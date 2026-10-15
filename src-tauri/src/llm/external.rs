//! The external OpenAI-compatible transport: SSE streaming with byte and
//! time caps, cancellation-aware waits, and provider error mapping.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::types::ChatMessage;

/// Hard cap on a single SSE event, delimiter included. Real events
/// carry a few token bytes in a JSON envelope, so anything near this
/// size means the endpoint is not speaking SSE (e.g. an HTML error
/// page or a CRLF-averse parser deadlock). 1 MiB.
const MAX_SSE_EVENT_BYTES: usize = 1 << 20;
/// Hard cap on the accumulated completion text across all events of
/// one generation. Far above the largest supported max_tokens budget
/// (4096 tokens), yet a valid-event-forever endpoint cannot grow
/// memory (or the streamed transcript) without end. 8 MiB.
pub(super) const MAX_OUTPUT_BYTES: usize = 8 << 20;
/// Overall budget for one external generation: request send, SSE
/// stream and JSON fallback combined. Generous (15 minutes) so slow
/// local endpoints and long completions always fit; a stalled
/// endpoint is caught much earlier by the per-chunk idle timeout,
/// and a trickling one is stopped here.
const GENERATION_OVERALL_BUDGET: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// Every wait phase (request send, SSE chunk, body read) re-checks
/// the cancel flag at least this often, so cancelling a generation
/// is honored well under a second instead of waiting out the idle
/// timeout.
const CANCEL_TICK: std::time::Duration = std::time::Duration::from_millis(150);

/// Bounds for one external generation. Production values are the
/// generous defaults above; tests inject small ones through
/// [`ExternalLlm::generate_with_limits`].
#[derive(Clone, Copy)]
struct GenerationLimits {
	/// Cap on a single SSE event, delimiter included. Checked with
	/// `>` so an event landing exactly at the cap is still valid.
	max_event_bytes: usize,
	/// Cap on the accumulated completion text, also checked with `>`
	/// before extending the output or invoking `on_chunk`.
	max_output_bytes: usize,
	/// Overall budget covering the whole generate call, including the
	/// JSON fallback.
	overall_budget: std::time::Duration,
	/// Cancel-flag polling interval for every wait phase.
	cancel_tick: std::time::Duration,
}

impl GenerationLimits {
	fn production() -> Self {
		Self {
			max_event_bytes: MAX_SSE_EVENT_BYTES,
			max_output_bytes: MAX_OUTPUT_BYTES,
			overall_budget: GENERATION_OVERALL_BUDGET,
			cancel_tick: CANCEL_TICK,
		}
	}
}

/// Why a bounded wait ended without its future completing. Kept
/// distinguishable from the completion path and from each other so
/// cap/deadline errors never masquerade as user cancellation.
enum WaitEnd {
	/// The user cancelled the generation.
	Cancelled,
	/// The overall generation budget ran out.
	Deadline,
}

/// Wait for `fut` until `deadline`, re-checking `cancel` every
/// `tick`. This keeps the existing AtomicBool cancellation API while
/// making every wait phase respond to it promptly (the tick is far
/// below the idle timeout) instead of only between reads.
async fn wait_bounded<F: std::future::Future>(
	fut: F,
	deadline: tokio::time::Instant,
	cancel: &AtomicBool,
	tick: std::time::Duration,
) -> Result<F::Output, WaitEnd> {
	tokio::pin!(fut);
	let mut next_tick = tokio::time::Instant::now() + tick;
	loop {
		tokio::select! {
			biased;
			_ = tokio::time::sleep_until(next_tick.min(deadline)) => {
				if tokio::time::Instant::now() >= deadline {
					return Err(WaitEnd::Deadline);
				}
				if cancel.load(Ordering::Relaxed) {
					return Err(WaitEnd::Cancelled);
				}
				next_tick = tokio::time::Instant::now() + tick;
			}
			out = &mut fut => return Ok(out),
		}
	}
}

/// Cap/deadline errors, distinguishable from user cancellation
/// ("generation cancelled") and never echoing request headers or raw
/// bodies.
fn overall_budget_error(budget: std::time::Duration) -> String {
	format!("external generation exceeded its overall time budget ({budget:?})")
}

fn event_cap_error(cap: usize) -> String {
	format!("external endpoint sent a single SSE event larger than {cap} bytes")
}

fn output_cap_error(cap: usize) -> String {
	format!("external endpoint generated more than {cap} bytes of completion text")
}

/// OpenAI-compatible chat completion endpoint (Ollama, llama.cpp server,
/// LM Studio, vLLM, ...). Streams tokens over SSE.
pub struct ExternalLlm {
	pub base_url: String,
	pub api_key: String,
	pub model: String,
	/// the context window the endpoint allows (drives prompt fitting
	/// with the estimator, since the endpoint reports no token counts)
	pub ctx_tokens: u32,
	client: reqwest::Client,
}

impl ExternalLlm {
	/// If no bytes arrive for this long, the endpoint is treated as stalled.
	const CHUNK_IDLE_TIMEOUT_SECS: u64 = 90;
	/// Overall budget for reading a (display-only) error body.
	const ERROR_BODY_TIMEOUT_SECS: u64 = 30;
	/// Overall budget for reading a non-streamed JSON completion (the
	/// server has already generated it when the headers arrive).
	const JSON_BODY_TIMEOUT_SECS: u64 = 120;

	pub fn new(
		base_url: &str,
		api_key: &str,
		model: &str,
		ctx_tokens: u32,
	) -> Result<Self, String> {
		// re-vet at use time, not only at save time: the setting can also
		// arrive from an environment override or an older build
		crate::models::validate_endpoint_url(base_url)?;
		Ok(Self {
			base_url: base_url.trim_end_matches('/').to_string(),
			api_key: api_key.to_string(),
			model: model.to_string(),
			ctx_tokens,
			client: Self::shared_client()?,
		})
	}

	/// One HTTP client for every external-endpoint call. A generation
	/// builds a fresh ExternalLlm, and a client per instance threw away
	/// its connection pool (TCP + TLS handshakes) on every message.
	/// reqwest::Client is a cheap Arc handle to the shared pool.
	fn shared_client() -> Result<reqwest::Client, String> {
		static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
		if let Some(client) = CLIENT.get() {
			return Ok(client.clone());
		}
		// Client::new() panics when the TLS backend cannot initialize;
		// surface that as an error instead (and retry on the next call).
		let client = reqwest::Client::builder()
			.connect_timeout(std::time::Duration::from_secs(10))
			.redirect(crate::models::hardened_redirect_policy())
			.build()
			.map_err(|e| format!("failed to build HTTP client: {e}"))?;
		Ok(CLIENT.get_or_init(|| client).clone())
	}

	fn completions_url(&self) -> String {
		if self.base_url.ends_with("/v1") {
			format!("{}/chat/completions", self.base_url)
		} else {
			format!("{}/v1/chat/completions", self.base_url)
		}
	}

	/// [`Self::generate_with_limits`] with the summary fit applied first:
	/// when `summary_render` is given, the conversation is fitted to
	/// [`crate::llm::DEFAULT_EXT_CTX`] (external endpoints report no
	/// tokenization of their own, so the estimator counts) and
	/// serialized into the single summary message, so an oversized
	/// session loses its oldest turns instead of the newest mid-JSON.
	/// Returns the text, the prompt token count (None: endpoints don't
	/// report one) and the number of dropped turns.
	#[allow(clippy::too_many_arguments)]
	pub async fn generate(
		&self,
		system: &str,
		messages: &[ChatMessage],
		summary_render: Option<super::SummaryRenderRef<'_>>,
		cancel: &AtomicBool,
		max_tokens: u32,
		temperature_override: Option<f32>,
		mut on_chunk: impl FnMut(String) + Send,
	) -> Result<(String, Option<usize>, usize), String> {
		// First attempt at the configured window; a 400 that names the
		// context (Ollama, LM Studio and friends) retries once with the
		// budget halved and the older turns dropped.
		match self
			.generate_attempt(
				system,
				messages,
				summary_render,
				cancel,
				max_tokens,
				temperature_override,
				self.ctx_tokens,
				&mut on_chunk,
			)
			.await
		{
			Ok(out) => Ok(out),
			Err(e) if is_context_overflow(&e) => {
				log::warn!("endpoint rejected the prompt as too long; retrying with half the context budget");
				self.generate_attempt(
					system,
					messages,
					summary_render,
					cancel,
					max_tokens,
					temperature_override,
					(self.ctx_tokens / 2).max(1_024),
					&mut on_chunk,
				)
				.await
			}
			Err(e) => Err(e),
		}
	}

	/// One fitted attempt at `ctx_tokens`: fit the conversation to that
	/// window (pairs for a chat, the whole-transcript render for a
	/// summary), then run the transfer.
	#[allow(clippy::too_many_arguments)]
	async fn generate_attempt(
		&self,
		system: &str,
		messages: &[ChatMessage],
		summary_render: Option<super::SummaryRenderRef<'_>>,
		cancel: &AtomicBool,
		max_tokens: u32,
		temperature_override: Option<f32>,
		ctx_tokens: u32,
		on_chunk: &mut (impl FnMut(String) + Send),
	) -> Result<(String, Option<usize>, usize), String> {
		let budget = super::prompt_budget(ctx_tokens, max_tokens);
		let (messages, dropped_turns) = match summary_render {
			Some(render) => {
				let (fitted, dropped) =
					super::fit_summary_turns(messages, budget, &super::approx_tokens, render);
				(
					vec![ChatMessage {
						role: "user".into(),
						content: render(&fitted),
					}],
					dropped,
				)
			}
			None => {
				// chat turns are fitted too: an endpoint with a small
				// window would otherwise 400 mid-interview
				let render = |msgs: &[ChatMessage]| -> String {
					msgs.iter()
						.map(|m| format!("[{}] {}\n", m.role, m.content))
						.collect::<String>()
				};
				let (fitted, _) =
					super::fit_chat_turns(messages, budget, &super::approx_tokens, &render);
				(fitted, 0)
			}
		};
		let text_and_tokens = self
			.generate_with_limits(
				system,
				&messages,
				cancel,
				max_tokens,
				temperature_override,
				on_chunk,
				GenerationLimits::production(),
			)
			.await?;
		Ok((text_and_tokens.0, text_and_tokens.1, dropped_turns))
	}

	/// [`ExternalLlm::generate`] with injectable bounds, so tests can
	/// exercise the caps and the overall budget without waiting out
	/// the generous production values.
	#[allow(clippy::too_many_arguments)]
	async fn generate_with_limits(
		&self,
		system: &str,
		messages: &[ChatMessage],
		cancel: &AtomicBool,
		max_tokens: u32,
		temperature_override: Option<f32>,
		mut on_chunk: impl FnMut(String) + Send,
		limits: GenerationLimits,
	) -> Result<(String, Option<usize>), String> {
		let temperature = temperature_override.unwrap_or(0.7);
		// (text, prompt-token count). External endpoints don't report their
		// prompt tokenization, hence the None.
		let mut body_messages = vec![];
		if !system.trim().is_empty() {
			body_messages.push(serde_json::json!({"role": "system", "content": system}));
		}
		for msg in messages {
			body_messages.push(serde_json::json!({"role": msg.role, "content": msg.content}));
		}

		let mut request = self
			.client
			.post(self.completions_url())
			.header("User-Agent", crate::models::USER_AGENT)
			.json(&serde_json::json!({
				"model": self.model,
				"messages": body_messages,
				"stream": true,
				"temperature": temperature,
				"max_tokens": max_tokens,
			}));
		if !self.api_key.is_empty() {
			request = request.bearer_auth(&self.api_key);
		}

		// The overall budget covers every phase of the call: request
		// send, headers, the SSE stream and the JSON fallback.
		let overall = tokio::time::Instant::now() + limits.overall_budget;
		let idle = std::time::Duration::from_secs(Self::CHUNK_IDLE_TIMEOUT_SECS);
		let send_deadline = (tokio::time::Instant::now() + idle).min(overall);
		let response =
			match wait_bounded(request.send(), send_deadline, cancel, limits.cancel_tick).await {
				Ok(Ok(response)) => response,
				Ok(Err(e)) => return Err(format!("request failed: {e}")),
				Err(WaitEnd::Cancelled) => return Err("generation cancelled".into()),
				Err(WaitEnd::Deadline) => {
					return Err(if tokio::time::Instant::now() >= overall {
						overall_budget_error(limits.overall_budget)
					} else {
						format!(
							"external endpoint stalled (no response for {}s)",
							Self::CHUNK_IDLE_TIMEOUT_SECS
						)
					});
				}
			};
		if !response.status().is_success() {
			let status = response.status();
			let body = Self::read_error_body(response, overall, cancel, &limits).await?;
			return Err(map_provider_error(status.as_u16(), &body));
		}
		let content_type = response
			.headers()
			.get(reqwest::header::CONTENT_TYPE)
			.and_then(|v| v.to_str().ok())
			.unwrap_or("")
			.to_ascii_lowercase();
		if content_type.contains("application/json") {
			// A server that ignores "stream": true answers with one plain
			// chat.completion object; take it as a single chunk.
			let deadline = (tokio::time::Instant::now()
				+ std::time::Duration::from_secs(Self::JSON_BODY_TIMEOUT_SECS))
			.min(overall);
			let body = match read_body_bounded(
				response,
				limits.max_event_bytes,
				idle,
				deadline,
				Some(overall),
				cancel,
				limits.cancel_tick,
			)
			.await
			{
				Ok(body) => body,
				Err(WaitEnd::Cancelled) => return Err("generation cancelled".into()),
				Err(WaitEnd::Deadline) => return Err(overall_budget_error(limits.overall_budget)),
			};
			if cancel.load(Ordering::Relaxed) {
				return Err("generation cancelled".into());
			}
			let content = json_completion_content(&body)?;
			ensure_usable_completion(&content)?;
			if content.len() > limits.max_output_bytes {
				return Err(output_cap_error(limits.max_output_bytes));
			}
			on_chunk(content.clone());
			return Ok((content, None));
		}
		if !content_type.is_empty() && !content_type.contains("text/event-stream") {
			let body = Self::read_error_body(response, overall, cancel, &limits).await?;
			return Err(format!(
				"endpoint did not return an SSE stream (content-type {content_type}): {}",
				truncate_body(&body)
			));
		}

		use futures_util::StreamExt;
		let mut stream = response.bytes_stream();
		let mut buffer: Vec<u8> = Vec::new();
		let mut output = String::new();

		loop {
			if cancel.load(Ordering::Relaxed) {
				return Err("generation cancelled".into());
			}
			if tokio::time::Instant::now() >= overall {
				return Err(overall_budget_error(limits.overall_budget));
			}
			let wait_until = (tokio::time::Instant::now() + idle).min(overall);
			let chunk =
				match wait_bounded(stream.next(), wait_until, cancel, limits.cancel_tick).await {
					Ok(Some(Ok(c))) => c,
					Ok(Some(Err(e))) => return Err(format!("stream interrupted: {e}")),
					Ok(None) => break,
					Err(WaitEnd::Cancelled) => return Err("generation cancelled".into()),
					Err(WaitEnd::Deadline) => {
						return Err(if tokio::time::Instant::now() >= overall {
							overall_budget_error(limits.overall_budget)
						} else {
							format!(
								"external endpoint stalled (no data for {}s)",
								Self::CHUNK_IDLE_TIMEOUT_SECS
							)
						});
					}
				};
			buffer.extend_from_slice(&chunk);
			// Bound the pending (still undelimited) event...
			if buffer.len() > limits.max_event_bytes && find_event_end(&buffer).is_none() {
				return Err(event_cap_error(limits.max_event_bytes));
			}

			// ...and every complete one, delimiter included, so a huge
			// event is rejected however the chunks happened to frame it.
			while let Some(pos) = find_event_end(&buffer) {
				if pos > limits.max_event_bytes {
					return Err(event_cap_error(limits.max_event_bytes));
				}
				let event_bytes: Vec<u8> = buffer.drain(..pos).collect();
				let event = String::from_utf8_lossy(&event_bytes);
				if consume_sse_event(&event, &mut output, &mut on_chunk, limits.max_output_bytes)? {
					ensure_usable_completion(&output)?;
					return Ok((output, None));
				}
			}
		}

		// A stream may legitimately end without a trailing blank line;
		// don't drop the final buffered event.
		if !buffer.is_empty() {
			let tail = String::from_utf8_lossy(&buffer);
			if consume_sse_event(&tail, &mut output, &mut on_chunk, limits.max_output_bytes)? {
				ensure_usable_completion(&output)?;
				return Ok((output, None));
			}
		}
		ensure_usable_completion(&output)?;
		Ok((output, None))
	}

	/// Read a display-only (error or non-SSE) body of at most 64 KiB
	/// under cancellation and the overall budget. A phase deadline or
	/// transport end keeps whatever arrived, so the status error can
	/// still be reported; only cancellation and the overall budget
	/// abort the generation outright.
	async fn read_error_body(
		response: reqwest::Response,
		overall: tokio::time::Instant,
		cancel: &AtomicBool,
		limits: &GenerationLimits,
	) -> Result<String, String> {
		let idle = std::time::Duration::from_secs(Self::CHUNK_IDLE_TIMEOUT_SECS);
		let deadline = (tokio::time::Instant::now()
			+ std::time::Duration::from_secs(Self::ERROR_BODY_TIMEOUT_SECS))
		.min(overall);
		match read_body_bounded(
			response,
			64 * 1024,
			idle,
			deadline,
			Some(overall),
			cancel,
			limits.cancel_tick,
		)
		.await
		{
			Ok(body) => Ok(body),
			Err(WaitEnd::Cancelled) => Err("generation cancelled".into()),
			Err(WaitEnd::Deadline) => Err(overall_budget_error(limits.overall_budget)),
		}
	}
}

/// Read a response body with a per-chunk idle deadline, an overall
/// deadline and a size cap, so a broken or hostile endpoint can neither
/// hang the caller (not even by trickling a byte just inside the idle
/// window forever) nor exhaust memory. Returns whatever arrived
/// (lossy-decoded) until the cap, the stream end, or a deadline;
/// display-only bodies should use a small cap. A body that is exactly
/// `cap` bytes is valid and arrives whole.
pub(crate) async fn read_body_capped(
	response: reqwest::Response,
	cap: usize,
	idle_timeout_secs: u64,
	total_timeout_secs: u64,
) -> String {
	// never cancelled, no overall budget beyond its own total deadline
	let not_cancelled = AtomicBool::new(false);
	let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(total_timeout_secs);
	read_body_bounded(
		response,
		cap,
		std::time::Duration::from_secs(idle_timeout_secs),
		deadline,
		None,
		&not_cancelled,
		CANCEL_TICK,
	)
	.await
	.unwrap_or_default()
}

/// Cancellation- and budget-aware body reader behind
/// [`read_body_capped`]. Ok always means "kept what arrived" (stream
/// end, cap reached, idle/phase deadline, transport error); only the
/// overall generation budget (`overall`) or the cancel flag turns
/// into an Err, so cap/deadline outcomes stay distinguishable from
/// user cancellation. Cap arithmetic is checked and exactly-at-cap
/// bodies are accepted.
async fn read_body_bounded(
	response: reqwest::Response,
	cap: usize,
	idle: std::time::Duration,
	phase_deadline: tokio::time::Instant,
	overall: Option<tokio::time::Instant>,
	cancel: &AtomicBool,
	tick: std::time::Duration,
) -> Result<String, WaitEnd> {
	use futures_util::StreamExt;
	let mut stream = response.bytes_stream();
	let mut body: Vec<u8> = Vec::new();
	// None (or zero) exactly at the cap: a valid, complete body
	while let Some(room) = cap.checked_sub(body.len()).filter(|room| *room > 0) {
		let mut wait_until = phase_deadline.min(tokio::time::Instant::now() + idle);
		if let Some(overall) = overall {
			wait_until = wait_until.min(overall);
		}
		let chunk = match wait_bounded(stream.next(), wait_until, cancel, tick).await {
			Ok(Some(Ok(c))) => c,
			Ok(Some(Err(_))) => break, // transport error: keep what arrived
			Ok(None) => break,         // stream end
			Err(WaitEnd::Cancelled) => return Err(WaitEnd::Cancelled),
			Err(WaitEnd::Deadline) => {
				// only the overall budget aborts the generation; an
				// idle/phase deadline keeps the partial body, like
				// read_body_capped always did
				if overall.is_some_and(|o| tokio::time::Instant::now() >= o) {
					return Err(WaitEnd::Deadline);
				}
				break;
			}
		};
		let take = room.min(chunk.len());
		body.extend_from_slice(&chunk[..take]);
	}
	Ok(String::from_utf8_lossy(&body).into_owned())
}

/// The assistant text of a non-streamed chat.completion body
/// (`choices[0].message.content`). A provider error object surfaces as
/// its message; anything else is a descriptive error, never silently
/// empty output.
fn json_completion_content(body: &str) -> Result<String, String> {
	let value: serde_json::Value = serde_json::from_str(body).map_err(|e| {
		format!(
			"endpoint returned invalid JSON ({e}): {}",
			truncate_body(body)
		)
	})?;
	if let Some(err) = value["error"]["message"].as_str() {
		return Err(map_provider_error(0, err));
	}
	value["choices"][0]["message"]["content"]
		.as_str()
		.map(str::to_string)
		.ok_or_else(|| {
			format!(
				"endpoint returned JSON with no completion (choices[0].message.content): {}",
				truncate_body(body)
			)
		})
}

/// A completion that produced no usable assistant content - an empty
/// body, comments/keepalives only, an empty [DONE], or nothing but
/// unparseable events - is a bounded error, never a successful empty
/// completion. Provider errors abort earlier, in [`consume_sse_event`].
fn ensure_usable_completion(output: &str) -> Result<(), String> {
	if output.is_empty() {
		return Err("endpoint returned an empty completion".into());
	}
	Ok(())
}

/// Parse one SSE event's `data:` lines, appending content deltas
/// under the accumulated-output cap. Returns `Ok(true)` when the
/// endpoint signalled `[DONE]`.
pub(super) fn consume_sse_event(
	event: &str,
	output: &mut String,
	on_chunk: &mut impl FnMut(String),
	max_output_bytes: usize,
) -> Result<bool, String> {
	for line in event.lines() {
		let line = line.trim();
		if let Some(data) = line.strip_prefix("data:") {
			let data = data.trim();
			if data == "[DONE]" {
				return Ok(true);
			}
			if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
				if let Some(delta) = value["choices"][0]["delta"]["content"].as_str() {
					if !delta.is_empty() {
						append_capped(delta, output, on_chunk, max_output_bytes)?;
					}
				} else if let Some(content) = value["choices"][0]["message"]["content"].as_str() {
					// some servers ignore "stream": true and answer with
					// one plain completion object; treat it as a single
					// (complete) chunk instead of silently empty output
					if !content.is_empty() {
						append_capped(content, output, on_chunk, max_output_bytes)?;
					}
				}
				if let Some(err) = value["error"]["message"].as_str() {
					return Err(map_provider_error(0, err));
				}
			}
		}
	}
	Ok(false)
}

/// Append one completion piece under the accumulated-output cap. The
/// cap uses checked lengths and is enforced BEFORE the output buffer
/// grows or `on_chunk` runs, so a piece landing exactly at the cap is
/// still accepted while a piece past it never streams partially.
fn append_capped(
	piece: &str,
	output: &mut String,
	on_chunk: &mut impl FnMut(String),
	max_output_bytes: usize,
) -> Result<(), String> {
	let next = output
		.len()
		.checked_add(piece.len())
		.ok_or_else(|| output_cap_error(max_output_bytes))?;
	if next > max_output_bytes {
		return Err(output_cap_error(max_output_bytes));
	}
	output.push_str(piece);
	on_chunk(piece.to_string());
	Ok(())
}

/// Find the end of the next SSE event, tolerating both `\n\n` and
/// `\r\n\r\n` separators.
fn find_event_end(buffer: &[u8]) -> Option<usize> {
	let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|p| p + 2);
	let crlf = buffer
		.windows(4)
		.position(|w| w == b"\r\n\r\n")
		.map(|p| p + 4);
	match (lf, crlf) {
		(Some(a), Some(b)) => Some(a.min(b)),
		(Some(a), None) => Some(a),
		(None, Some(b)) => Some(b),
		(None, None) => None,
	}
}

/// Error kind for a provider's content-filter rejection. The frontend
/// matches this prefix (helpers.ts isModerationError) and asks the user
/// to reword their message instead of showing a generic AI error; the
/// IPC contract test keeps the two sides in sync.
pub const MODERATION_ERROR: &str = "moderation: the AI provider flagged this message";

/// Map provider errors onto the error kinds the frontend understands.
/// A 400 whose body blames the context/token budget: the one failure
/// a smaller prompt can fix, so [`ExternalLlm::generate`] retries with
/// half the window before surfacing it.
fn is_context_overflow(error: &str) -> bool {
	let lower = error.to_lowercase();
	// the mapped form is "external endpoint error (400): <body>"
	lower.contains("(400):")
		&& (lower.contains("context") || lower.contains("token") || lower.contains("length"))
}

fn map_provider_error(status: u16, body: &str) -> String {
	let lower = body.to_lowercase();
	if status == 400 && (lower.contains("content_filter") || lower.contains("content_policy")) {
		return MODERATION_ERROR.into();
	}
	if status == 401 || status == 403 {
		return format!("external endpoint rejected credentials ({status})");
	}
	if status != 0 {
		return format!(
			"external endpoint error ({status}): {}",
			truncate_body(body)
		);
	}
	truncate_body(body)
}

pub(super) fn truncate_body(body: &str) -> String {
	if body.len() > 300 {
		// Slice at a char boundary: byte 300 can land mid-UTF-8-character
		// for a non-ASCII error body, and a plain [..300] would panic.
		let mut cut = 300;
		while !body.is_char_boundary(cut) {
			cut -= 1;
		}
		format!("{}...", &body[..cut])
	} else {
		body.to_string()
	}
}

/// Truncate a string to at most `max_bytes`, never splitting a character.
pub(super) fn truncate_at_boundary(s: &str, max_bytes: usize) -> &str {
	if s.len() <= max_bytes {
		return s;
	}
	let mut cut = max_bytes;
	while !s.is_char_boundary(cut) {
		cut -= 1;
	}
	&s[..cut]
}

#[cfg(test)]
mod sse_tests {
	use super::{consume_sse_event, find_event_end, map_provider_error};

	#[test]
	fn find_event_end_handles_lf_and_crlf() {
		assert_eq!(find_event_end(b"data: x\n\nrest"), Some(9));
		assert_eq!(find_event_end(b"data: x\r\n\r\nrest"), Some(11));
		// whichever separator comes first wins
		assert_eq!(find_event_end(b"a\n\nb\r\n\r\n"), Some(3));
		assert_eq!(find_event_end(b"no separator"), None);
		assert_eq!(find_event_end(b"trailing\n"), None);
	}

	#[test]
	fn consume_sse_event_appends_deltas_and_stops_on_done() {
		let mut output = String::new();
		let mut chunks = Vec::new();
		let done = consume_sse_event(
			"data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}",
			&mut output,
			&mut |c| chunks.push(c),
			super::MAX_OUTPUT_BYTES,
		)
		.expect("parse");
		assert!(!done);
		assert_eq!(output, "ab");
		assert_eq!(chunks, vec!["a".to_string(), "b".to_string()]);

		let done = consume_sse_event(
			"data: [DONE]",
			&mut output,
			&mut |_| {},
			super::MAX_OUTPUT_BYTES,
		)
		.expect("parse");
		assert!(done);
	}

	#[test]
	fn consume_sse_event_surfaces_error_objects() {
		let mut output = String::new();
		let err = consume_sse_event(
			"data: {\"error\":{\"message\":\"content filter flagged this\"}}",
			&mut output,
			&mut |_| {},
			super::MAX_OUTPUT_BYTES,
		)
		.expect_err("error objects must surface");
		assert!(err.contains("content filter"), "unexpected: {err}");
	}

	#[test]
	fn map_provider_error_maps_content_filter_to_moderation_kind() {
		assert_eq!(
			map_provider_error(400, r#"{"error":{"code":"content_filter"}}"#),
			super::MODERATION_ERROR
		);
		assert_eq!(
			map_provider_error(400, r#"{"error":{"type":"content_policy_violation"}}"#),
			super::MODERATION_ERROR
		);
		assert!(map_provider_error(401, "bad key").contains("rejected credentials"));
		assert!(map_provider_error(500, "boom").contains("500"));
	}
}

#[cfg(test)]
mod external_stream_tests {
	use super::{read_body_capped, ExternalLlm, GenerationLimits};

	/// Minimal loopback SSE server: writes the given events after the
	/// request head, keeps the socket open briefly.
	fn serve_sse(events: Vec<String>) -> String {
		serve_typed("text/event-stream", events.concat())
	}

	/// Loopback server answering one request with `body` as
	/// `content_type`.
	fn serve_typed(content_type: &'static str, body: String) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut request = String::new();
				loop {
					let mut byte = [0u8; 1];
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0] as char);
					if request.ends_with("\r\n\r\n") {
						break;
					}
				}
				let head = format!(
					"HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
					body.len()
				);
				let _ = sock.write_all(head.as_bytes());
				let _ = sock.write_all(body.as_bytes());
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(400));
			}
		});
		format!("http://{addr}/v1")
	}

	#[tokio::test]
	async fn a_context_overflow_400_retries_once_with_a_smaller_prompt() {
		// serves 400 (context) first, then a valid SSE stream; records
		// both request bodies so the retry's smaller prompt is provable
		use std::io::{Read, Write};
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		let (tx, rx) = std::sync::mpsc::channel::<String>();
		std::thread::spawn(move || {
			for first in [true, false] {
				let Ok((mut sock, _)) = listener.accept() else {
					return;
				};
				let mut request = String::new();
				loop {
					let mut byte = [0u8; 1];
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0] as char);
					if request.ends_with("\r\n\r\n") {
						break;
					}
				}
				// drain the JSON body per Content-Length
				let len: usize = request
					.split("\r\n")
					.find_map(|l| {
						let lower = l.to_ascii_lowercase();
						lower
							.strip_prefix("content-length: ")
							.map(|v| v.trim().to_string())
					})
					.and_then(|v| v.parse().ok())
					.unwrap_or(0);
				let mut body = vec![0u8; len];
				let _ = sock.read_exact(&mut body);
				let _ = tx.send(String::from_utf8_lossy(&body).to_string());
				let response = if first {
					let body = "{\"error\":{\"message\":\"context length exceeded\"}}";
					format!(
						"HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
						body.len(),
						body
					)
				} else {
					let body = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
					format!(
						"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
						body.len(),
						body
					)
				};
				let _ = sock.write_all(response.as_bytes());
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});

		// a conversation far beyond a tiny configured window
		let conversation: Vec<crate::types::ChatMessage> = (0..200)
			.map(|i| crate::types::ChatMessage {
				role: if i % 2 == 0 { "assistant" } else { "user" }.into(),
				content: format!("message number {i} with a decent amount of text"),
			})
			.collect();
		let client =
			ExternalLlm::new(&format!("http://{addr}/v1"), "", "m", 2_048).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let (output, _, _) = client
			.generate("", &conversation, None, &cancel, 32, None, |_| {})
			.await
			.expect("the retry succeeds");
		assert_eq!(output, "ok");

		let first = rx.recv().expect("first body");
		let second = rx.recv().expect("retry body");
		let count = |body: &str| body.matches("\"role\"").count();
		assert!(
			count(&second) < count(&first),
			"the retry must carry fewer messages ({} -> {})",
			count(&first),
			count(&second)
		);
	}

	#[tokio::test]
	async fn external_generate_streams_from_loopback_sse_server() {
		let url = serve_sse(vec![
			"data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n".into(),
			"data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n".into(),
			"data: [DONE]\n\n".into(),
		]);
		let client = ExternalLlm::new(&url, "", "test-model", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let mut chunks = Vec::new();
		let (output, prompt_tokens, _) = client
			.generate("you are a test", &[], None, &cancel, 32, None, |c| {
				chunks.push(c)
			})
			.await
			.expect("generate");
		assert_eq!(output, "Hello");
		assert_eq!(chunks, vec!["Hel".to_string(), "lo".to_string()]);
		assert!(
			prompt_tokens.is_none(),
			"external endpoints report no prompt tokens"
		);
	}

	#[tokio::test]
	async fn external_generate_rejects_non_sse_responses() {
		// neither SSE nor a JSON completion (a proxy's HTML error page):
		// must produce a descriptive error, and the capped reader must
		// bound the body
		let url = serve_typed(
			"text/html; charset=utf-8",
			"<html><body>Bad gateway</body></html>".into(),
		);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect_err("non-SSE must fail");
		assert!(
			err.contains("did not return an SSE stream"),
			"unexpected: {err}"
		);
		assert!(err.contains("Bad gateway"), "the body is shown: {err}");
	}

	#[tokio::test]
	async fn external_generate_accepts_a_server_that_ignores_stream_true() {
		// llama.cpp server builds, some proxies and vLLM configs answer a
		// streaming request with one plain chat.completion object
		let url = serve_typed(
			"application/json; charset=utf-8",
			r#"{"object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Hi there"}}]}"#.into(),
		);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let mut chunks = Vec::new();
		let (output, _, _) = client
			.generate("", &[], None, &cancel, 8, None, |c| chunks.push(c))
			.await
			.expect("a plain JSON completion is a valid reply");
		assert_eq!(output, "Hi there");
		assert_eq!(
			chunks,
			vec!["Hi there".to_string()],
			"delivered as one chunk"
		);
	}

	#[tokio::test]
	async fn external_clients_share_one_connection_pool() {
		// a server that accepts exactly ONE connection and answers two
		// keep-alive requests on it: a second generation only succeeds if
		// it reuses the pooled connection of the first
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			use std::io::{Read, Write};
			let Ok((mut sock, _)) = listener.accept() else {
				return;
			};
			for reply in ["one", "two"] {
				let mut head = Vec::new();
				let mut byte = [0u8; 1];
				while !head.ends_with(b"\r\n\r\n") {
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						return;
					}
					head.push(byte[0]);
				}
				let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
				let len: usize = head
					.lines()
					.find_map(|l| l.strip_prefix("content-length:"))
					.and_then(|v| v.trim().parse().ok())
					.unwrap_or(0);
				let mut body = vec![0u8; len];
				let _ = sock.read_exact(&mut body);
				let json = format!(r#"{{"choices":[{{"message":{{"content":"{reply}"}}}}]}}"#);
				let _ = sock.write_all(
					format!(
						"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{json}",
						json.len()
					)
					.as_bytes(),
				);
				let _ = sock.flush();
			}
			std::thread::sleep(std::time::Duration::from_millis(500));
		});
		let url = format!("http://{addr}/v1");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		for expected in ["one", "two"] {
			// a fresh ExternalLlm per generation, as the command layer does
			let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
			let (output, _, _) = tokio::time::timeout(
				std::time::Duration::from_secs(5),
				client.generate("", &[], None, &cancel, 8, None, |_| {}),
			)
			.await
			.expect("the second call must reuse the first call's connection")
			.expect("generate");
			assert_eq!(output, expected);
		}
	}

	#[tokio::test]
	async fn external_generate_rejects_json_without_a_completion() {
		let url = serve_typed("application/json", r#"{"not":"a completion"}"#.into());
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect_err("JSON without choices[0].message.content must fail");
		assert!(err.contains("no completion"), "unexpected: {err}");

		let url = serve_typed(
			"application/json",
			r#"{"error":{"message":"model not loaded"}}"#.into(),
		);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let err = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect_err("an error object must surface");
		assert!(err.contains("model not loaded"), "unexpected: {err}");
	}

	#[tokio::test]
	async fn external_generate_rejects_an_empty_sse_stream() {
		// A 200 text/event-stream with an empty body used to return
		// Ok(("", None)), so the command layer treated an unusable
		// completion as a success.
		let url = serve_sse(vec![]);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect_err("an empty SSE body is not a completion");
		assert!(err.contains("empty completion"), "unexpected: {err}");
	}

	#[tokio::test]
	async fn external_generate_rejects_a_stream_of_only_comments_and_keepalives() {
		let url = serve_sse(vec![": keepalive\n\n".into(), ": ping\n\n".into()]);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect_err("comments carry no completion");
		assert!(err.contains("empty completion"), "unexpected: {err}");
	}

	#[tokio::test]
	async fn external_generate_rejects_an_empty_done_marker_without_content() {
		let url = serve_sse(vec!["data: [DONE]\n\n".into()]);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect_err("[DONE] without any content is not a completion");
		assert!(err.contains("empty completion"), "unexpected: {err}");
	}

	#[tokio::test]
	async fn external_generate_rejects_a_stream_of_only_malformed_events() {
		let url = serve_sse(vec![
			"data: {not json\n\n".into(),
			"data: \"also broken\n\n".into(),
		]);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect_err("only unparseable events is not a completion");
		assert!(err.contains("empty completion"), "unexpected: {err}");
	}

	#[tokio::test]
	async fn external_generate_errors_when_the_provider_signals_an_error_after_text() {
		// the partial text must not be returned as a success
		let url = serve_sse(vec![
			"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n".into(),
			"data: {\"error\":{\"message\":\"model overloaded\"}}\n\n".into(),
		]);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect_err("a provider error event must fail the generation");
		assert!(err.contains("model overloaded"), "unexpected: {err}");
	}

	#[tokio::test]
	async fn external_generate_accepts_a_final_event_without_a_trailing_blank_line() {
		// compatibility: servers that end valid content with neither a
		// final blank line nor [DONE] keep working
		let url = serve_sse(vec![
			"data: {\"choices\":[{\"delta\":{\"content\":\"tail\"}}]}".into(),
		]);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let (output, _, _) = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect("valid content ending at EOF is a completion");
		assert_eq!(output, "tail");
	}

	#[tokio::test]
	async fn external_generate_rejects_an_empty_json_completion() {
		let url = serve_typed(
			"application/json",
			r#"{"choices":[{"index":0,"message":{"role":"assistant","content":""}}]}"#.into(),
		);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect_err("empty JSON content is not a completion");
		assert!(err.contains("empty completion"), "unexpected: {err}");
	}

	#[tokio::test]
	async fn external_generate_reassembles_utf8_split_across_stream_chunks() {
		// the emoji is split mid-character across two TCP writes: the
		// parser must buffer until the event is complete instead of
		// lossy-decoding each chunk separately
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut request = String::new();
				loop {
					let mut byte = [0u8; 1];
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0] as char);
					if request.ends_with("\r\n\r\n") {
						break;
					}
				}
				let body = format!(
					"data: {{\"choices\":[{{\"delta\":{{\"content\":\"brain {} story\"}}}}]}}\n\ndata: [DONE]\n\n",
					"🧠"
				);
				let bytes = body.as_bytes();
				// two bytes into the four-byte character: not a char boundary
				let split = body.find("🧠").expect("emoji") + 2;
				let head = format!(
					"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
					bytes.len()
				);
				let _ = sock.write_all(head.as_bytes());
				let _ = sock.write_all(&bytes[..split]);
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(100));
				let _ = sock.write_all(&bytes[split..]);
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		let url = format!("http://{addr}/v1");
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let (output, _, _) = client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
			.expect("a character split across stream chunks must not corrupt decoding");
		assert_eq!(output, "brain 🧠 story");
	}

	// keep the capped reader honest alongside the stream tests
	#[tokio::test]
	async fn read_body_capped_is_also_exercised_here() {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut byte = [0u8; 1];
				let mut request = String::new();
				loop {
					if sock.read(&mut byte).unwrap_or(0) == 0 {
						break;
					}
					request.push(byte[0] as char);
					if request.ends_with("\r\n\r\n") {
						break;
					}
				}
				let _ =
					sock.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 4\r\n\r\nnope");
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		let response = reqwest::Client::new()
			.get(format!("http://{addr}/"))
			.send()
			.await
			.expect("send");
		let body = read_body_capped(response, 64 * 1024, 5, 5).await;
		assert_eq!(body, "nope");
	}

	/// Loopback SSE server that keeps writing valid delta events every
	/// `interval_ms`, never ending the stream, until the client goes
	/// away (or ~6 s pass so the fixture thread always terminates).
	fn serve_trickle_sse(interval_ms: u64) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf); // drain the request head
				let _ =
					sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
				for _ in 0..240 {
					let event = "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n";
					if sock.write_all(event.as_bytes()).is_err() {
						break;
					}
					let _ = sock.flush();
					std::thread::sleep(std::time::Duration::from_millis(interval_ms));
				}
			}
		});
		format!("http://{addr}/v1")
	}

	#[tokio::test]
	async fn external_generate_trickling_valid_events_hits_the_overall_budget() {
		// Every event is valid and arrives well inside the idle
		// timeout, so a trickling endpoint could keep the stream alive
		// indefinitely. The (tiny, test-injected) overall budget must
		// end it on its own; production uses the documented 15 minutes.
		let url = serve_trickle_sse(25);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let started = std::time::Instant::now();
		let limits = GenerationLimits {
			overall_budget: std::time::Duration::from_millis(400),
			cancel_tick: std::time::Duration::from_millis(20),
			..GenerationLimits::production()
		};
		let result = tokio::time::timeout(
			std::time::Duration::from_secs(5),
			client.generate_with_limits("", &[], &cancel, 8, None, |_| {}, limits),
		)
		.await
		.expect("the overall budget must end a trickling stream");
		let err = result.expect_err("a never-ending stream is an error");
		assert!(
			err.contains("overall time budget"),
			"expected an overall-budget error, got: {err}"
		);
		assert!(
			started.elapsed() < std::time::Duration::from_secs(2),
			"the budget must fire long before the idle timeout, took {:?}",
			started.elapsed()
		);
	}

	#[tokio::test]
	async fn external_generate_caps_accumulated_output_across_many_small_events() {
		// Every single event is small and valid, but together the
		// deltas exceed the output cap; generation must stop instead
		// of accepting an unbounded completion. Events are written
		// (and flushed) one by one so the client sees small chunks,
		// like a real streaming provider.
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf); // drain the request head
				let event = format!(
					"data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
					"a".repeat(990)
				);
				let _ =
					sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
				// ~9 MB of valid events
				for i in 0..9_000 {
					if i % 200 == 0 {
						std::thread::sleep(std::time::Duration::from_millis(1));
					}
					if sock.write_all(event.as_bytes()).is_err() {
						return;
					}
					let _ = sock.flush();
				}
				let _ = sock.write_all(b"data: [DONE]\n\n");
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		let client =
			ExternalLlm::new(&format!("http://{addr}/v1"), "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let mut streamed = 0usize;
		let err = match client
			.generate("", &[], None, &cancel, 8, None, |c: String| {
				streamed += c.len()
			})
			.await
		{
			Ok((output, _, _)) => panic!(
				"output beyond the cap must fail: accepted {} bytes of completion",
				output.len()
			),
			Err(err) => err,
		};
		assert!(
			err.contains("bytes of completion text"),
			"unexpected: {err}"
		);
		assert!(
			streamed <= 8 * 1024 * 1024,
			"no content may stream past the cap ({streamed} bytes)"
		);
	}

	#[tokio::test]
	async fn external_generate_rejects_one_huge_delimited_event() {
		// The size check only rejected buffers with NO delimiter,
		// so a complete event whose delimiter is already buffered
		// passed straight through. The event is delivered in two
		// pieces: an undelimited prefix under the cap, then the rest
		// (delimiter included) as one final piece.
		let mk_event = |n: usize| {
			format!(
				"data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
				"a".repeat(n)
			)
		};
		let overhead = mk_event(1).len() - 1;
		// just over the 1 MiB event cap, with the split point under
		// the legacy 1_000_000 undelimited bound
		let total = (1 << 20) + 600;
		let event = mk_event(total - overhead);
		assert_eq!(event.len(), total, "fixture sizes the event exactly");
		let split = 999_999;
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf); // drain the request head
				let _ =
					sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
				let _ = sock.write_all(&event.as_bytes()[..split]);
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(150));
				let _ = sock.write_all(&event.as_bytes()[split..]);
				let _ = sock.write_all(b"data: [DONE]\n\n");
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(300));
			}
		});
		let client =
			ExternalLlm::new(&format!("http://{addr}/v1"), "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = match client
			.generate("", &[], None, &cancel, 8, None, |_| {})
			.await
		{
			Ok((output, _, _)) => panic!(
				"a single huge delimited event must fail: accepted {} bytes of completion",
				output.len()
			),
			Err(err) => err,
		};
		assert!(err.contains("single SSE event"), "unexpected: {err}");
	}

	/// Loopback server that accepts the connection and then never
	/// writes anything: an endpoint hanging before responding.
	fn serve_silent_endpoint() -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::Read;
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf); // drain the request head
				std::thread::sleep(std::time::Duration::from_millis(3_000));
			}
		});
		format!("http://{addr}/v1")
	}

	/// Loopback server that answers `head` immediately and then never
	/// sends any body bytes, holding the connection open: an endpoint
	/// hanging mid-body.
	fn serve_head_then_silence(head: &'static str) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf); // drain the request head
				let _ = sock.write_all(head.as_bytes());
				let _ = sock.flush();
				std::thread::sleep(std::time::Duration::from_millis(3_000));
			}
		});
		format!("http://{addr}/v1")
	}

	/// Cancellation core: the cancel flag is raised 150 ms in, when
	/// the request head has surely been answered and the body read is
	/// the phase being waited on. The generation must return
	/// "generation cancelled" within the ~150 ms cancel tick, not wait
	/// out the 90 s idle timeout.
	async fn assert_generation_cancels_promptly(url: String) {
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let flag = cancel.clone();
		tokio::spawn(async move {
			tokio::time::sleep(std::time::Duration::from_millis(150)).await;
			flag.store(true, std::sync::atomic::Ordering::Relaxed);
		});
		let started = std::time::Instant::now();
		let result = tokio::time::timeout(
			std::time::Duration::from_secs(2),
			client.generate("", &[], None, &cancel, 8, None, |_| {}),
		)
		.await
		.expect("cancel must return well before the idle timeout");
		let err = result.expect_err("a cancelled generation is an error");
		assert_eq!(
			err, "generation cancelled",
			"cancellation must stay distinguishable from other errors"
		);
		assert!(
			started.elapsed() < std::time::Duration::from_secs(1),
			"cancel must be honored within the tick, took {:?}",
			started.elapsed()
		);
	}

	#[tokio::test]
	async fn external_generate_cancel_during_request_send_returns_promptly() {
		assert_generation_cancels_promptly(serve_silent_endpoint()).await;
	}

	#[tokio::test]
	async fn external_generate_cancel_during_sse_body_read_returns_promptly() {
		assert_generation_cancels_promptly(serve_head_then_silence(
			"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n",
		))
		.await;
	}

	#[tokio::test]
	async fn external_generate_cancel_during_json_body_read_returns_promptly() {
		assert_generation_cancels_promptly(serve_head_then_silence(
			"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4096\r\n\r\n",
		))
		.await;
	}

	#[tokio::test]
	async fn external_generate_cancel_during_error_body_read_returns_promptly() {
		assert_generation_cancels_promptly(serve_head_then_silence(
			"HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: 4096\r\n\r\n",
		))
		.await;
	}

	#[tokio::test]
	async fn external_generate_accepts_an_event_exactly_at_the_event_cap() {
		// caps are checked with `>`: a complete event whose length,
		// delimiter included, lands exactly on the cap is valid
		let mk_event = |n: usize| {
			format!(
				"data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
				"a".repeat(n)
			)
		};
		let cap = 4_096usize;
		let n = 1 + (cap - mk_event(1).len()); // pad to exactly the cap
		let event = mk_event(n);
		assert_eq!(event.len(), cap, "fixture sizes the event exactly");
		let url = serve_sse(vec![event, "data: [DONE]\n\n".into()]);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let limits = GenerationLimits {
			max_event_bytes: cap,
			..GenerationLimits::production()
		};
		let (output, _) = client
			.generate_with_limits("", &[], &cancel, 8, None, |_| {}, limits)
			.await
			.expect("an event exactly at the cap is valid");
		assert_eq!(output.len(), n);
	}

	#[tokio::test]
	async fn external_generate_accepts_output_exactly_at_the_output_cap() {
		// 2 + 4 + 2 = 8 bytes of deltas land exactly on the cap
		let url = serve_sse(vec![
			"data: {\"choices\":[{\"delta\":{\"content\":\"ab\"}}]}\n\n".into(),
			"data: {\"choices\":[{\"delta\":{\"content\":\"🧠\"}}]}\n\n".into(),
			"data: {\"choices\":[{\"delta\":{\"content\":\"cd\"}}]}\n\n".into(),
			"data: [DONE]\n\n".into(),
		]);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let limits = GenerationLimits {
			max_output_bytes: 8,
			..GenerationLimits::production()
		};
		let (output, _) = client
			.generate_with_limits("", &[], &cancel, 8, None, |_| {}, limits)
			.await
			.expect("output landing exactly at the cap is valid");
		assert_eq!(output, "ab🧠cd");
	}

	#[tokio::test]
	async fn external_generate_multibyte_delta_at_the_output_cap_boundary() {
		// the cap counts bytes; a 4-byte character whose delta lands
		// exactly at the cap must arrive intact, and one byte past it
		// must reject the WHOLE delta (never a partial character)
		let url = serve_sse(vec![
			"data: {\"choices\":[{\"delta\":{\"content\":\"ab\"}}]}\n\n".into(),
			"data: {\"choices\":[{\"delta\":{\"content\":\"🧠\"}}]}\n\n".into(),
			"data: [DONE]\n\n".into(),
		]);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let at_limit = GenerationLimits {
			max_output_bytes: 6, // "ab" (2) + "🧠" (4)
			..GenerationLimits::production()
		};
		let (output, _) = client
			.generate_with_limits("", &[], &cancel, 8, None, |_| {}, at_limit)
			.await
			.expect("the emoji lands exactly at the cap");
		assert_eq!(output, "ab🧠");
		assert_eq!(output.chars().count(), 3, "no off-by-one at the boundary");

		let url = serve_sse(vec![
			"data: {\"choices\":[{\"delta\":{\"content\":\"ab\"}}]}\n\n".into(),
			"data: {\"choices\":[{\"delta\":{\"content\":\"🧠\"}}]}\n\n".into(),
			"data: [DONE]\n\n".into(),
		]);
		let client = ExternalLlm::new(&url, "", "m", 32_768).expect("client");
		let one_past = GenerationLimits {
			max_output_bytes: 5, // "ab" fits, "🧠" would make 6
			..GenerationLimits::production()
		};
		let mut chunks = Vec::new();
		let err = client
			.generate_with_limits("", &[], &cancel, 8, None, |c| chunks.push(c), one_past)
			.await
			.expect_err("one byte past the cap must fail");
		assert!(
			err.contains("bytes of completion text"),
			"unexpected: {err}"
		);
		assert_eq!(
			chunks,
			vec!["ab".to_string()],
			"the rejected delta never streams"
		);
	}
}

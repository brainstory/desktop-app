use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

use crate::types::ChatMessage;

/// Generation context size. Transcripts are conversational; 16k tokens
/// comfortably fits a long session plus the result document.
const N_CTX: u32 = 16384;
pub const MAX_NEW_TOKENS_RESPONSE: u32 = 1024;
pub const MAX_NEW_TOKENS_RESULT: u32 = 4096;
/// If no token completes for this long the generation is treated as
/// stalled and aborted. Generously above even slow-CPU token times.
const GENERATION_IDLE_TIMEOUT_SECS: u64 = 120;
/// Hard ceiling for one generation as a backstop; large enough that a
/// healthy max-length result on a slow machine still finishes.
const GENERATION_MAX_TOTAL_SECS: u64 = 900;
/// Thinking models (MiniCPM5) emit `<think>` reasoning before the answer;
/// reasoning tokens don't count against the answer budget but get this
/// separate allowance so a model reasoning forever can't stall a chat.
const MAX_THINK_TOKENS: u32 = 4096;
/// Prompt tokens are decoded in chunks of this size so cancel/timeout
/// checks stay responsive during long prompts.
const PROMPT_DECODE_CHUNK: usize = 512;

#[allow(dead_code)]
pub struct LocalLlm {
	backend: Arc<LlamaBackend>,
	model: Arc<LlamaModel>,
	pub model_id: String,
	/// general.architecture from the GGUF metadata
	architecture: String,
}

impl LocalLlm {
	pub fn load(backend: Arc<LlamaBackend>, path: &Path, model_id: &str) -> Result<Self, String> {
		// Offload every layer to the GPU when a backend (Metal/Vulkan) exists;
		// llama.cpp falls back to CPU compute automatically otherwise.
		let params = LlamaModelParams::default().with_n_gpu_layers(u32::MAX);
		let model = LlamaModel::load_from_file(&backend, path, &params)
			.map_err(|e| format!("failed to load model {path:?}: {e}"))?;
		let architecture = model
			.meta_val_str("general.architecture")
			.unwrap_or_default();
		Ok(Self {
			backend,
			model: Arc::new(model),
			model_id: model_id.to_string(),
			architecture,
		})
	}

	fn apply_llama_template(
		&self,
		template: &LlamaChatTemplate,
		chat: &[LlamaChatMessage],
	) -> Result<String, String> {
		self.model
			.apply_chat_template(template, chat, true)
			.map_err(|e| format!("failed to apply chat template: {e}"))
	}

	/// Neutralize the model's own turn control markers so message content
	/// (user text, or text imported from a share file) can't reshape the
	/// prompt structure in the manual-template path.
	fn neutralize_turn_markers(text: &str) -> String {
		text.replace("<|turn>", "<\\|turn>")
			.replace("<turn|>", "<turn\\|>")
	}

	/// Manual prompt format matching the model's native chat markers. Used
	/// when llama.cpp's built-in template applier can't handle the model's
	/// (e.g. the Gemma 4 canonical template, which its minja subset rejects).
	fn manual_prompt(&self, system: &str, messages: &[ChatMessage]) -> String {
		if self.architecture.starts_with("gemma") {
			// Gemma 4: <bos><|turn>role\ncontent<turn|>... ending with an
			// open model turn. The system turn (if any) comes first.
			let mut prompt = String::from("<bos>");
			if !system.trim().is_empty() {
				prompt.push_str(&format!("<|turn>system\n{}<turn|>\n", system.trim()));
			}
			for msg in messages {
				let role = if msg.role == "assistant" {
					"model"
				} else {
					"user"
				};
				let content = Self::neutralize_turn_markers(&msg.content);
				let content = if role == "user" {
					content.trim()
				} else {
					content.as_str()
				};
				prompt.push_str(&format!("<|turn>{role}\n{content}<turn|>\n"));
			}
			prompt.push_str("<|turn>model\n");
			prompt
		} else {
			// last resort: plain concatenation
			let mut prompt = String::new();
			if !system.trim().is_empty() {
				prompt.push_str(system.trim());
				prompt.push_str("\n\n");
			}
			for msg in messages {
				let role = if msg.role == "assistant" {
					"Assistant"
				} else {
					"User"
				};
				prompt.push_str(&format!("{role}: {}\n", msg.content));
			}
			prompt.push_str("Assistant:");
			prompt
		}
	}

	fn apply_template(&self, system: &str, messages: &[ChatMessage]) -> Result<String, String> {
		if let Ok(template) = self.model.chat_template(None) {
			let mut chat: Vec<LlamaChatMessage> = Vec::new();
			if !system.trim().is_empty() {
				chat.push(
					LlamaChatMessage::new("system".to_string(), system.to_string())
						.map_err(|e| e.to_string())?,
				);
			}
			for msg in messages {
				let role = if msg.role == "assistant" {
					"assistant"
				} else {
					"user"
				};
				chat.push(
					LlamaChatMessage::new(role.to_string(), msg.content.clone())
						.map_err(|e| e.to_string())?,
				);
			}
			if let Ok(prompt) = self.apply_llama_template(&template, &chat) {
				return Ok(prompt);
			}
			log::warn!(
				"built-in chat template failed, falling back to manual {} format",
				self.architecture
			);
		}
		Ok(self.manual_prompt(system, messages))
	}

	fn count_tokens(&self, prompt: &str) -> Result<usize, String> {
		Ok(self
			.model
			.str_to_token(prompt, AddBos::Never)
			.map_err(|e| e.to_string())?
			.len())
	}

	/// Build the final prompt, dropping older middle messages until it fits
	/// into the context window with room for `max_new_tokens`.
	fn build_prompt(
		&self,
		system: &str,
		messages: &[ChatMessage],
		max_new_tokens: u32,
	) -> Result<String, String> {
		let budget = (N_CTX as usize).saturating_sub(max_new_tokens as usize + 64);
		let mut msgs: Vec<ChatMessage> = messages.to_vec();
		let mut prompt = self.apply_template(system, &msgs)?;
		let mut n_tokens = self.count_tokens(&prompt)?;

		// Drop the second-oldest message repeatedly (keeps the opening
		// assistant framing and the most recent context).
		while n_tokens > budget && msgs.len() > 2 {
			msgs.remove(1);
			prompt = self.apply_template(system, &msgs)?;
			n_tokens = self.count_tokens(&prompt)?;
		}

		// Even with only the opening + final message left, one oversized
		// paste can exceed the budget. Hard-truncate the final message
		// (char-boundary safe) so decode succeeds instead of failing with
		// "prompt decode failed" or collapsing the generation cap to ~0.
		while n_tokens > budget {
			let Some(last) = msgs.last_mut() else {
				break;
			};
			if last.content.is_empty() {
				break;
			}
			// Estimate the cut from the overshoot (~4 bytes per token is a
			// safe upper bound) but always make progress.
			let overshoot = n_tokens - budget;
			let target = last
				.content
				.len()
				.saturating_sub(overshoot.saturating_mul(4))
				.max(last.content.len() / 2);
			let truncated = truncate_at_boundary(&last.content, target);
			// The truncation notice must never outweigh the shrink, or this
			// loop stops making progress.
			let mut candidate = format!("{truncated}\n[...truncated to fit the model context]");
			if candidate.len() >= last.content.len() {
				candidate = truncated.to_string();
			}
			if candidate.len() >= last.content.len() {
				break;
			}
			last.content = candidate;
			prompt = self.apply_template(system, &msgs)?;
			n_tokens = self.count_tokens(&prompt)?;
		}
		Ok(prompt)
	}

	/// Run generation, streaming pieces through `on_chunk`. Blocking and
	/// CPU/GPU heavy; run on a dedicated thread.
	pub fn generate(
		&self,
		system: &str,
		messages: &[ChatMessage],
		summarize: bool,
		cancel: &AtomicBool,
		mut on_chunk: impl FnMut(String),
	) -> Result<String, String> {
		let max_new = if summarize {
			MAX_NEW_TOKENS_RESULT
		} else {
			MAX_NEW_TOKENS_RESPONSE
		};
		let prompt = self.build_prompt(system, messages, max_new)?;
		let tokens = self
			.model
			.str_to_token(&prompt, AddBos::Never)
			.map_err(|e| e.to_string())?;
		if tokens.is_empty() {
			return Err("the model produced no tokens for this conversation; please try again".into());
		}

		// The context borrows the model, so it lives only within this call.
		let ctx_params = LlamaContextParams::default()
			.with_n_ctx(NonZeroU32::new(N_CTX))
			.with_n_batch(PROMPT_DECODE_CHUNK as u32);
		let mut ctx = self
			.model
			.new_context(&self.backend, ctx_params)
			.map_err(|e| format!("failed to create context: {e}"))?;

		let started = std::time::Instant::now();
		let mut last_progress = std::time::Instant::now();
		let stalled =
			|| format!("generation stalled (no progress for {GENERATION_IDLE_TIMEOUT_SECS}s)");
		let overdue = || format!("generation timed out after {GENERATION_MAX_TOTAL_SECS}s");

		// Decode the prompt in chunks: cancel stays responsive and a stalled
		// decode aborts instead of hanging the whole budget.
		let mut batch = LlamaBatch::new(PROMPT_DECODE_CHUNK, 1);
		let n_prompt = tokens.len();
		for (offset, chunk) in tokens.chunks(PROMPT_DECODE_CHUNK).enumerate() {
			if cancel.load(Ordering::Relaxed) {
				return Err("generation cancelled".into());
			}
			if last_progress.elapsed()
				> std::time::Duration::from_secs(GENERATION_IDLE_TIMEOUT_SECS)
			{
				return Err(stalled());
			}
			if started.elapsed() > std::time::Duration::from_secs(GENERATION_MAX_TOTAL_SECS) {
				return Err(overdue());
			}
			batch.clear();
			for (i, token) in chunk.iter().enumerate() {
				let pos = (offset * PROMPT_DECODE_CHUNK + i) as i32;
				let needs_logits = offset * PROMPT_DECODE_CHUNK + i + 1 == n_prompt;
				batch
					.add(*token, pos, &[0], needs_logits)
					.map_err(|e| e.to_string())?;
			}
			ctx.decode(&mut batch)
				.map_err(|e| format!("prompt decode failed: {e}"))?;
			last_progress = std::time::Instant::now();
		}

		// Gemma-recommended sampling: top_k 64, top_p 0.95. Summaries use a
		// lower temperature for more deterministic structure.
		let temperature = if summarize { 0.35 } else { 0.7 };
		let seed = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|d| d.subsec_nanos())
			.unwrap_or(42);
		let mut sampler = LlamaSampler::chain(
			[
				LlamaSampler::top_k(64),
				LlamaSampler::top_p(0.95, 1),
				LlamaSampler::temp(temperature),
				LlamaSampler::dist(seed),
			],
			true,
		);

		let mut decoder = encoding_rs::UTF_8.new_decoder();
		let mut output = String::new();
		let mut think_filter = ThinkFilter::new();
		let mut generated: u32 = 0;
		let mut answer_tokens: u32 = 0;
		// Absolute backstop across answer + reasoning tokens; can never
		// overflow the context window regardless of prompt length.
		let total_cap = max_new
			.saturating_add(MAX_THINK_TOKENS)
			.min((N_CTX - 1).saturating_sub(tokens.len() as u32));
		let mut pos = tokens.len() as i32;

		loop {
			if cancel.load(Ordering::Relaxed) {
				return Err("generation cancelled".into());
			}
			if last_progress.elapsed()
				> std::time::Duration::from_secs(GENERATION_IDLE_TIMEOUT_SECS)
			{
				return Err(stalled());
			}
			if started.elapsed() > std::time::Duration::from_secs(GENERATION_MAX_TOTAL_SECS) {
				return Err(overdue());
			}
			let token = sampler.sample(&ctx, -1);
			if self.model.is_eog_token(token) {
				break;
			}
			let piece = match self.model.token_to_piece(token, &mut decoder, false, None) {
				Ok(piece) => piece,
				// a token with no text piece is not an error; feed it back
				Err(llama_cpp_2::TokenToStringError::UnknownTokenType) => String::new(),
				Err(e) => return Err(e.to_string()),
			};
			if !piece.is_empty() {
				let safe = think_filter.push(&piece);
				if !safe.is_empty() {
					// `output` only ever holds think-stripped text, so every
					// caller (UI stream, stored history, result documents)
					// sees the final response, never reasoning blocks.
					output.push_str(&safe);
					on_chunk(safe);
					answer_tokens += 1;
				}
			}
			generated += 1;
			last_progress = std::time::Instant::now();
			if answer_tokens >= max_new || generated >= total_cap {
				break;
			}

			batch.clear();
			batch
				.add(token, pos, &[0], true)
				.map_err(|e| e.to_string())?;
			ctx.decode(&mut batch)
				.map_err(|e| format!("decode failed: {e}"))?;
			pos += 1;
		}

		let tail = think_filter.finish();
		if !tail.is_empty() {
			output.push_str(&tail);
			on_chunk(tail);
		}

		Ok(output)
	}
}

/// Streaming filter that removes `<think>...</think>` reasoning blocks.
/// Thinking models (e.g. MiniCPM5) emit their reasoning before the answer;
/// only the final response should be displayed or stored. Tags can split
/// across token pieces, so the filter buffers until a tag is confirmed.
struct ThinkFilter {
	buffer: String,
	in_think: bool,
}

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

impl ThinkFilter {
	fn new() -> Self {
		Self {
			buffer: String::new(),
			in_think: false,
		}
	}

	/// Feed one decoded piece; returns whatever is now safe to emit.
	fn push(&mut self, piece: &str) -> String {
		self.buffer.push_str(piece);
		let mut out = String::new();
		loop {
			if self.in_think {
				let Some(i) = self.buffer.find(THINK_CLOSE) else {
					// Reasoning so far; keep only a tail large enough to
					// still catch a closing tag split across pieces.
					let mut cut = self.buffer.len().saturating_sub(THINK_CLOSE.len() - 1);
					while !self.buffer.is_char_boundary(cut) {
						cut -= 1;
					}
					self.buffer.drain(..cut);
					break;
				};
				self.buffer.drain(..i + THINK_CLOSE.len());
				self.in_think = false;
				// The answer starts after the whitespace following the tag.
				let ws = self.buffer.len() - self.buffer.trim_start().len();
				self.buffer.drain(..ws);
				continue;
			}
			if let Some(i) = self.buffer.find(THINK_OPEN) {
				out.push_str(&self.buffer[..i]);
				self.buffer.drain(..i + THINK_OPEN.len());
				self.in_think = true;
				continue;
			}
			// Hold back a suffix that could be the start of `<think>`; the
			// matched bytes are ASCII, so the split point is a boundary.
			let mut hold = 0;
			for n in 1..=THINK_OPEN.len().min(self.buffer.len()) {
				if THINK_OPEN.as_bytes()[..n]
					== self.buffer.as_bytes()[self.buffer.len() - n..]
				{
					hold = n;
					break;
				}
			}
			let emit_end = self.buffer.len() - hold;
			out.push_str(&self.buffer[..emit_end]);
			self.buffer.drain(..emit_end);
			break;
		}
		out
	}

	/// Flush at end of generation. An unterminated think block was all
	/// reasoning; a dangling partial open tag stays as literal text.
	fn finish(&mut self) -> String {
		if self.in_think {
			self.buffer.clear();
			String::new()
		} else {
			std::mem::take(&mut self.buffer)
		}
	}
}

/// OpenAI-compatible chat completion endpoint (Ollama, llama.cpp server,
/// LM Studio, vLLM, ...). Streams tokens over SSE.
pub struct ExternalLlm {
	pub base_url: String,
	pub api_key: String,
	pub model: String,
	client: reqwest::Client,
}

impl ExternalLlm {
	/// If no bytes arrive for this long, the endpoint is treated as stalled.
	const CHUNK_IDLE_TIMEOUT_SECS: u64 = 90;
	/// SSE events are tiny; a bigger buffer means the endpoint isn't
	/// speaking SSE (e.g. CRLF-averse parser deadlock or an HTML error page).
	const MAX_SSE_BUFFER: usize = 1_000_000;

	pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
		Self {
			base_url: base_url.trim_end_matches('/').to_string(),
			api_key: api_key.to_string(),
			model: model.to_string(),
			client: reqwest::Client::builder()
				.connect_timeout(std::time::Duration::from_secs(10))
				.build()
				.unwrap_or_else(|_| reqwest::Client::new()),
		}
	}

	fn completions_url(&self) -> String {
		if self.base_url.ends_with("/v1") {
			format!("{}/chat/completions", self.base_url)
		} else {
			format!("{}/v1/chat/completions", self.base_url)
		}
	}

	pub async fn generate(
		&self,
		system: &str,
		messages: &[ChatMessage],
		cancel: &AtomicBool,
		max_tokens: u32,
		mut on_chunk: impl FnMut(String) + Send,
	) -> Result<String, String> {
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
			.header("User-Agent", "brainstory-desktop/0.1")
			.json(&serde_json::json!({
				"model": self.model,
				"messages": body_messages,
				"stream": true,
				"temperature": 0.7,
				"max_tokens": max_tokens,
			}));
		if !self.api_key.is_empty() {
			request = request.bearer_auth(&self.api_key);
		}

		let response = tokio::time::timeout(
			std::time::Duration::from_secs(Self::CHUNK_IDLE_TIMEOUT_SECS),
			request.send(),
		)
		.await
		.map_err(|_| {
			format!(
				"external endpoint stalled (no response for {}s)",
				Self::CHUNK_IDLE_TIMEOUT_SECS
			)
		})?
		.map_err(|e| format!("request failed: {e}"))?;
		if !response.status().is_success() {
			let status = response.status();
			let body = response.text().await.unwrap_or_default();
			return Err(map_provider_error(status.as_u16(), &body));
		}
		let content_type = response
			.headers()
			.get(reqwest::header::CONTENT_TYPE)
			.and_then(|v| v.to_str().ok())
			.unwrap_or("")
			.to_ascii_lowercase();
		if !content_type.is_empty() && !content_type.contains("text/event-stream") {
			let body = response.text().await.unwrap_or_default();
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
			let chunk = match tokio::time::timeout(
				std::time::Duration::from_secs(Self::CHUNK_IDLE_TIMEOUT_SECS),
				stream.next(),
			)
			.await
			{
				Err(_) => {
					return Err(format!(
						"external endpoint stalled (no data for {}s)",
						Self::CHUNK_IDLE_TIMEOUT_SECS
					))
				}
				Ok(Some(Ok(c))) => c,
				Ok(Some(Err(e))) => return Err(format!("stream interrupted: {e}")),
				Ok(None) => break,
			};
			buffer.extend_from_slice(&chunk);
			if buffer.len() > Self::MAX_SSE_BUFFER && find_event_end(&buffer).is_none() {
				return Err("external endpoint sent an oversized non-SSE response".into());
			}

			while let Some(pos) = find_event_end(&buffer) {
				let line_bytes: Vec<u8> = buffer.drain(..pos).collect();
				let line = String::from_utf8_lossy(&line_bytes);
				for line in line.lines() {
					let line = line.trim();
					if let Some(data) = line.strip_prefix("data:") {
						let data = data.trim();
						if data == "[DONE]" {
							return Ok(output);
						}
						if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
							if let Some(delta) = value["choices"][0]["delta"]["content"].as_str() {
								if !delta.is_empty() {
									output.push_str(delta);
									on_chunk(delta.to_string());
								}
							}
							if let Some(err) = value["error"]["message"].as_str() {
								return Err(map_provider_error(0, err));
							}
						}
					}
				}
			}
		}
		Ok(output)
	}
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

/// Map provider errors onto the brainstory protocol. HTTP 469 was the
/// original "inappropriate input" signal that the frontend knows how to
/// surface as a resend request.
fn map_provider_error(status: u16, body: &str) -> String {
	let lower = body.to_lowercase();
	if status == 400 && (lower.contains("content_filter") || lower.contains("content_policy")) {
		return "HttpError 469: Inappropriate input".into();
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

fn truncate_body(body: &str) -> String {
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
fn truncate_at_boundary(s: &str, max_bytes: usize) -> &str {
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
mod tests {
	use super::ThinkFilter;

	fn run(pieces: &[&str]) -> String {
		let mut filter = ThinkFilter::new();
		let mut out = String::new();
		for piece in pieces {
			out.push_str(&filter.push(piece));
		}
		out.push_str(&filter.finish());
		out
	}

	#[test]
	fn plain_text_passes_through() {
		assert_eq!(run(&["hello ", "world"]), "hello world");
	}

	#[test]
	fn leading_think_block_is_stripped() {
		assert_eq!(run(&["<think>\nplan\n</think>\n\nHello!"]), "Hello!");
	}

	#[test]
	fn tags_split_across_pieces() {
		assert_eq!(run(&["<th", "ink>reasoning</thi", "nk>ans", "wer"]), "answer");
	}

	#[test]
	fn unterminated_think_is_dropped() {
		assert_eq!(run(&["<think>half a thought, genera"]), "");
	}

	#[test]
	fn partial_open_tag_stays_literal() {
		assert_eq!(run(&["a < b", " and c"]), "a < b and c");
	}

	#[test]
	fn think_mid_answer_is_stripped() {
		assert_eq!(run(&["Wait. <think>reconsider</think> Done."]), "Wait. Done.");
	}

	#[test]
	fn truncate_body_never_splits_a_character() {
		use super::truncate_body;
		// byte 300 lands inside this multi-byte snowman
		let body = "☃".repeat(120); // 360 bytes
		let truncated = truncate_body(&body);
		assert!(truncated.ends_with("..."));
		assert!(truncated.is_char_boundary(truncated.len() - 3));
		assert_eq!(truncate_body("short"), "short");
	}

	#[test]
	fn truncate_at_boundary_respects_utf8() {
		use super::truncate_at_boundary;
		let s = "héllo wörld"; // multi-byte é, ö
		let cut = truncate_at_boundary(s, 4);
		assert!(s.starts_with(cut));
		assert!(cut.is_char_boundary(cut.len()));
		assert_eq!(truncate_at_boundary(s, 100), s);
	}
}


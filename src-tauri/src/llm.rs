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
use llama_cpp_2::SeqState;

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

/// Break control-token-shaped sequences so tokenizing the text with
/// parse_special=true can never turn user/imported content into turn
/// boundaries or header control tokens: Gemma 2/3 "<start_of_turn>" /
/// "<end_of_turn>", the "<bos>"/"<eos>" sequence tokens, every "<|..."
/// opener (Gemma 4 "<|turn>", ChatML "<|im_start|>", Llama 3
/// "<|eot_id|>") and every "...|>" closer (Gemma 4 "<turn|>").
/// The single place content is neutralized: generate() applies it to the
/// system prompt and every message before any template (built-in or
/// manual) adds the real markers. The inserted backslash keeps the text
/// readable.
pub fn neutralize_turn_markers(content: &str) -> String {
	content
		.replace("<start_of_turn>", "<\\start_of_turn>")
		.replace("<end_of_turn>", "<\\end_of_turn>")
		.replace("<bos>", "<\\bos>")
		.replace("<eos>", "<\\eos>")
		.replace("<|", "<\\|")
		.replace("|>", "\\|>")
}

/// How a restored KV cache lines up with the next prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvReuse {
	/// First prompt position that still decodes. Restored cells from here
	/// on are dropped, so this is also how many cached tokens stay: on an
	/// exact repeat it is one short of the shared prefix, and the dropped
	/// final token is re-decoded rather than kept twice.
	decode_from: usize,
}

impl KvReuse {
	/// The tokens the cache holds once the prompt tail is decoded.
	fn tokens_after_prompt<T: Clone>(&self, cached: &[T], prompt: &[T]) -> Vec<T> {
		let mut tokens = cached[..self.decode_from].to_vec();
		tokens.extend_from_slice(&prompt[self.decode_from..]);
		tokens
	}
}

/// Plan reusing a cache built from `cached` for `prompt`: only the tail
/// past the common prefix decodes, but at least the final prompt token
/// always does so logits exist for sampling even on an exact repeat.
/// None when nothing is shared.
fn plan_kv_reuse<T: PartialEq>(cached: &[T], prompt: &[T]) -> Option<KvReuse> {
	let common = cached
		.iter()
		.zip(prompt.iter())
		.take_while(|(a, b)| a == b)
		.count();
	if common == 0 {
		return None;
	}
	Some(KvReuse {
		decode_from: common.min(prompt.len().saturating_sub(1)),
	})
}

/// A captured KV cache plus the tokens it was built from. Restoring
/// this into a fresh context skips re-decoding the common prefix of the
/// next turn's prompt (the biggest per-turn latency cost).
struct GenerationState {
	kv: SeqState,
	tokens: Vec<llama_cpp_2::token::LlamaToken>,
}

pub struct LocalLlm {
	backend: Arc<LlamaBackend>,
	model: Arc<LlamaModel>,
	pub model_id: String,
	/// Reusable KV cache from the previous generation, if any. Mutex (not
	/// RwLock): restoration mutates the state by draining it.
	kv_state: std::sync::Mutex<Option<GenerationState>>,
	/// general.architecture from the GGUF metadata
	architecture: String,
	/// The model's trained context length (`{arch}.context_length` GGUF
	/// metadata), when present. The effective context is clamped to this so
	/// a small-trained-context model isn't run with a silently-degrading
	/// oversized window.
	trained_ctx: Option<u32>,
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
		let trained_ctx = model
			.meta_val_str(&format!("{architecture}.context_length"))
			.ok()
			.and_then(|v| v.trim().parse::<u64>().ok())
			.filter(|v| *v > 0)
			.map(|v| v.min(u32::MAX as u64) as u32);
		if let Some(trained) = trained_ctx {
			log::info!(
				"model {model_id} ({architecture}) trained context: {trained} tokens; \
				 effective context: {}",
				trained.min(N_CTX)
			);
		}
		Ok(Self {
			backend,
			model: Arc::new(model),
			model_id: model_id.to_string(),
			kv_state: std::sync::Mutex::new(None),
			architecture,
			trained_ctx,
		})
	}

	/// Context window actually used: the default, clamped to what the
	/// model was trained for.
	fn effective_ctx(&self) -> u32 {
		self.trained_ctx.map_or(N_CTX, |trained| trained.min(N_CTX))
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

	/// Manual prompt format matching the model's native chat markers. Used
	/// when llama.cpp's built-in template applier can't handle the model's
	/// (e.g. the Gemma 4 canonical template, which its minja subset rejects).
	/// `system` and `messages` arrive already neutralized (see
	/// [`neutralize_turn_markers`]); escaping again here would double the
	/// backslashes.
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
				let content = if role == "user" {
					msg.content.trim()
				} else {
					msg.content.as_str()
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
		let n_ctx = self.effective_ctx() as usize;
		let budget = n_ctx.saturating_sub(max_new_tokens as usize + 64);
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
	/// CPU/GPU heavy; run on a dedicated thread. Returns the final text
	/// together with the prompt token count (for the telemetry fields the
	/// command layer reports).
	pub fn generate(
		&self,
		system: &str,
		messages: &[ChatMessage],
		summarize: bool,
		cancel: &AtomicBool,
		mut on_chunk: impl FnMut(String),
	) -> Result<(String, usize), String> {
		let max_new = if summarize {
			MAX_NEW_TOKENS_RESULT
		} else {
			MAX_NEW_TOKENS_RESPONSE
		};
		// User/imported content must never tokenize into control tokens:
		// the pinned llama-cpp-2 tokenizes with parse_special=true and has
		// no special-off API, so neutralize the marker spellings in the
		// content (our own scaffolding is added by the chat template,
		// after this point, and stays intact).
		let system = neutralize_turn_markers(system);
		let messages: Vec<ChatMessage> = messages
			.iter()
			.map(|m| ChatMessage {
				role: m.role.clone(),
				content: neutralize_turn_markers(&m.content),
			})
			.collect();
		let prompt = self.build_prompt(&system, &messages, max_new)?;
		let tokens = self
			.model
			.str_to_token(&prompt, AddBos::Never)
			.map_err(|e| e.to_string())?;
		if tokens.is_empty() {
			return Err(
				"the model produced no tokens for this conversation; please try again".into(),
			);
		}
		let n_prompt = tokens.len();

		// The context borrows the model, so it lives only within this call;
		// the KV cache travels separately, as captured state bytes.
		let n_ctx = self.effective_ctx();
		let ctx_params = LlamaContextParams::default()
			.with_n_ctx(Some(
				NonZeroU32::new(n_ctx).expect("context size is never zero"),
			))
			.with_n_batch(PROMPT_DECODE_CHUNK as u32);
		let mut ctx = self
			.model
			.new_context(&self.backend, ctx_params)
			.map_err(|e| format!("failed to create context: {e}"))?;

		// Restore the previous KV cache when one exists and reuse its
		// common prefix with this prompt, so only the new tail decodes.
		// (A restore failure just falls back to a full decode.)
		// kv_tokens tracks what the cache holds, for the next turn's reuse.
		let mut kv_tokens: Vec<llama_cpp_2::token::LlamaToken> = tokens.clone();
		let mut decode_from = 0usize;
		if let Some(saved) = self
			.kv_state
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.take()
		{
			if let Some(plan) = plan_kv_reuse(&saved.tokens, &tokens) {
				match ctx.state_seq_set(&saved.kv, 0) {
					Ok(()) => {
						// The restored cache also holds the previous turn's
						// generated tokens; drop every cell past the reuse
						// point so the tail decodes into free cells.
						if let Err(e) =
							ctx.clear_kv_cache_seq(Some(0), Some(plan.decode_from as u32), None)
						{
							log::warn!("KV truncation failed; decoding full prompt: {e:?}");
						} else {
							decode_from = plan.decode_from;
							kv_tokens = plan.tokens_after_prompt(&saved.tokens, &tokens);
							log::info!(
								"reusing KV cache: {decode_from} cached tokens, decoding {} new",
								n_prompt - decode_from
							);
						}
					}
					Err(e) => {
						log::warn!("KV restore failed; decoding full prompt: {e:?}");
					}
				}
			}
		}

		let started = std::time::Instant::now();
		let mut last_progress = std::time::Instant::now();
		let stalled =
			|| format!("generation stalled (no progress for {GENERATION_IDLE_TIMEOUT_SECS}s)");
		let overdue = || format!("generation timed out after {GENERATION_MAX_TOTAL_SECS}s");

		// Decode the (possibly partial) prompt in chunks: cancel stays
		// responsive and a stalled decode aborts instead of hanging the
		// whole budget. The KV prefix is already in the context.
		let mut batch = LlamaBatch::new(PROMPT_DECODE_CHUNK, 1);
		for (offset, chunk) in tokens[decode_from..]
			.chunks(PROMPT_DECODE_CHUNK)
			.enumerate()
		{
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
				let pos = (decode_from + offset * PROMPT_DECODE_CHUNK + i) as i32;
				let needs_logits = decode_from + offset * PROMPT_DECODE_CHUNK + i + 1 == n_prompt;
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
			.min((n_ctx - 1).saturating_sub(tokens.len() as u32));
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
			kv_tokens.push(token);
			pos += 1;
		}

		let tail = think_filter.finish();
		if !tail.is_empty() {
			output.push_str(&tail);
			on_chunk(tail);
		}

		// Capture the KV for the next turn. Failures are non-fatal: the
		// next call just re-decodes from scratch.
		if let Ok(kv) = ctx.state_seq_get(0, llama_cpp_2::LlamaStateSeqFlags::empty()) {
			*self.kv_state.lock().unwrap_or_else(|e| e.into_inner()) = Some(GenerationState {
				kv,
				tokens: kv_tokens,
			});
		}

		Ok((output, n_prompt))
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
				if THINK_OPEN.as_bytes()[..n] == self.buffer.as_bytes()[self.buffer.len() - n..] {
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
	/// Overall budget for reading a (display-only) error body.
	const ERROR_BODY_TIMEOUT_SECS: u64 = 30;
	/// Overall budget for reading a non-streamed JSON completion (the
	/// server has already generated it when the headers arrive).
	const JSON_BODY_TIMEOUT_SECS: u64 = 120;
	/// SSE events are tiny; a bigger buffer means the endpoint isn't
	/// speaking SSE (e.g. CRLF-averse parser deadlock or an HTML error page).
	const MAX_SSE_BUFFER: usize = 1_000_000;

	pub fn new(base_url: &str, api_key: &str, model: &str) -> Result<Self, String> {
		Ok(Self {
			base_url: base_url.trim_end_matches('/').to_string(),
			api_key: api_key.to_string(),
			model: model.to_string(),
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

	pub async fn generate(
		&self,
		system: &str,
		messages: &[ChatMessage],
		cancel: &AtomicBool,
		max_tokens: u32,
		mut on_chunk: impl FnMut(String) + Send,
	) -> Result<(String, Option<usize>), String> {
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
			let body = read_body_capped(
				response,
				64 * 1024,
				Self::CHUNK_IDLE_TIMEOUT_SECS,
				Self::ERROR_BODY_TIMEOUT_SECS,
			)
			.await;
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
			let body = read_body_capped(
				response,
				Self::MAX_SSE_BUFFER,
				Self::CHUNK_IDLE_TIMEOUT_SECS,
				Self::JSON_BODY_TIMEOUT_SECS,
			)
			.await;
			if cancel.load(Ordering::Relaxed) {
				return Err("generation cancelled".into());
			}
			let content = json_completion_content(&body)?;
			if !content.is_empty() {
				on_chunk(content.clone());
			}
			return Ok((content, None));
		}
		if !content_type.is_empty() && !content_type.contains("text/event-stream") {
			let body = read_body_capped(
				response,
				64 * 1024,
				Self::CHUNK_IDLE_TIMEOUT_SECS,
				Self::ERROR_BODY_TIMEOUT_SECS,
			)
			.await;
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
				let event_bytes: Vec<u8> = buffer.drain(..pos).collect();
				let event = String::from_utf8_lossy(&event_bytes);
				if consume_sse_event(&event, &mut output, &mut on_chunk)? {
					return Ok((output, None));
				}
			}
		}

		// A stream may legitimately end without a trailing blank line;
		// don't drop the final buffered event.
		if !buffer.is_empty() {
			let tail = String::from_utf8_lossy(&buffer);
			if consume_sse_event(&tail, &mut output, &mut on_chunk)? {
				return Ok((output, None));
			}
		}
		Ok((output, None))
	}
}

/// Read a response body with a per-chunk idle deadline, an overall
/// deadline and a size cap, so a broken or hostile endpoint can neither
/// hang the caller (not even by trickling a byte just inside the idle
/// window forever) nor exhaust memory. Returns whatever arrived
/// (lossy-decoded) until the cap, the stream end, or a deadline;
/// display-only bodies should use a small cap.
pub(crate) async fn read_body_capped(
	response: reqwest::Response,
	cap: usize,
	idle_timeout_secs: u64,
	total_timeout_secs: u64,
) -> String {
	use futures_util::StreamExt;
	let mut stream = response.bytes_stream();
	let mut body: Vec<u8> = Vec::new();
	let idle = std::time::Duration::from_secs(idle_timeout_secs);
	let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(total_timeout_secs);
	while body.len() < cap {
		let wait_until = deadline.min(tokio::time::Instant::now() + idle);
		let chunk = match tokio::time::timeout_at(wait_until, stream.next()).await {
			Err(_) => break, // stalled or overdue: keep what arrived so far
			Ok(Some(Ok(c))) => c,
			Ok(Some(Err(_))) => break, // transport error: same
			Ok(None) => break,         // stream end
		};
		let room = cap - body.len();
		body.extend_from_slice(&chunk[..room.min(chunk.len())]);
	}
	String::from_utf8_lossy(&body).into_owned()
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

/// Parse one SSE event's `data:` lines, appending content deltas. Returns
/// `Ok(true)` when the endpoint signalled `[DONE]`.
fn consume_sse_event(
	event: &str,
	output: &mut String,
	on_chunk: &mut impl FnMut(String),
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
						output.push_str(delta);
						on_chunk(delta.to_string());
					}
				} else if let Some(content) = value["choices"][0]["message"]["content"].as_str() {
					// some servers ignore "stream": true and answer with
					// one plain completion object; treat it as a single
					// (complete) chunk instead of silently empty output
					if !content.is_empty() {
						output.push_str(content);
						on_chunk(content.to_string());
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
	use super::{read_body_capped, ThinkFilter};

	/// Serve raw bytes as an HTTP response on a loopback listener; the
	/// socket stays open until dropped so slow/never-ending bodies can
	/// be simulated.
	fn serve_raw(response: Vec<u8>) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf); // drain the request head
				let _ = sock.write_all(&response);
				let _ = sock.flush();
				// hold the connection open so the body has no stream end
				std::thread::sleep(std::time::Duration::from_millis(1_500));
			}
		});
		format!("http://{addr}/")
	}

	#[tokio::test]
	async fn read_body_capped_stops_at_the_cap() {
		// ~300 KB via chunked encoding, never completed by the server
		let mut raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
		for _ in 0..300 {
			raw.extend_from_slice(b"1000\r\n");
			raw.extend_from_slice(&vec![b'a'; 4096]);
			raw.extend_from_slice(b"\r\n");
		}
		let url = serve_raw(raw);
		let response = reqwest::Client::new().get(&url).send().await.expect("send");
		let body = read_body_capped(response, 64 * 1024, 10, 10).await;
		assert_eq!(body.len(), 64 * 1024, "reading stops exactly at the cap");
	}

	#[tokio::test]
	async fn read_body_capped_returns_what_arrived_before_the_deadline() {
		// one complete chunk, then the stream never continues
		let url =
			serve_raw(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n".to_vec());
		let response = reqwest::Client::new().get(&url).send().await.expect("send");
		let started = std::time::Instant::now();
		let body = read_body_capped(response, 64 * 1024, 1, 10).await;
		assert!(
			started.elapsed() < std::time::Duration::from_secs(5),
			"the deadline must end the read"
		);
		assert_eq!(body, "x", "bytes that arrived before the deadline are kept");
	}

	#[tokio::test]
	async fn read_body_capped_enforces_an_overall_deadline() {
		// a byte every 100 ms: never idle long enough for the idle
		// timeout, and it would keep going for ~20 s
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let addr = listener.local_addr().expect("addr");
		std::thread::spawn(move || {
			if let Ok((mut sock, _)) = listener.accept() {
				use std::io::{Read, Write};
				let mut buf = [0u8; 4096];
				let _ = sock.read(&mut buf);
				let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
				for _ in 0..200 {
					if sock.write_all(b"1\r\nx\r\n").is_err() {
						break;
					}
					let _ = sock.flush();
					std::thread::sleep(std::time::Duration::from_millis(100));
				}
			}
		});
		let response = reqwest::Client::new()
			.get(format!("http://{addr}/"))
			.send()
			.await
			.expect("send");
		let started = std::time::Instant::now();
		let body = read_body_capped(response, 64 * 1024, 5, 1).await;
		assert!(
			started.elapsed() < std::time::Duration::from_secs(4),
			"the overall deadline must end a trickling read, took {:?}",
			started.elapsed()
		);
		assert!(!body.is_empty(), "what arrived before the deadline is kept");
		assert!(body.chars().all(|c| c == 'x'));
	}

	#[test]
	fn consume_sse_event_accepts_non_streaming_completions() {
		// a server that ignored stream:true replies with one data event
		// carrying choices[0].message.content
		let event = "data: {\"choices\":[{\"message\":{\"content\":\"hello there\"}}]}";
		let mut output = String::new();
		let mut chunks = Vec::new();
		let done =
			super::consume_sse_event(event, &mut output, &mut |c| chunks.push(c)).expect("parse");
		assert!(!done, "no [DONE] marker yet");
		assert_eq!(output, "hello there");
		assert_eq!(chunks, vec!["hello there".to_string()]);
	}

	#[test]
	fn neutralize_turn_markers_escapes_all_chat_specials() {
		let f = super::neutralize_turn_markers;
		// Gemma 2/3 turn markers
		assert_eq!(
			f("<start_of_turn>model"),
			"<\\start_of_turn>model",
			"gemma opener broken"
		);
		assert_eq!(f("user<end_of_turn>"), "user<\\end_of_turn>");
		// Gemma 4 (the default catalog models): `<|name>` opens a turn and
		// `<name|>` closes it, so both halves must be broken
		assert_eq!(f("<|turn>model\nhi"), "<\\|turn>model\nhi");
		assert_eq!(f("hi<turn|>"), "hi<turn\\|>");
		assert_eq!(f("<|channel>x<channel|>"), "<\\|channel>x<channel\\|>");
		// ChatML / Llama 3 header style specials (`<|...|>`)
		assert_eq!(f("<|im_start|>system"), "<\\|im_start\\|>system");
		assert_eq!(f("<|eot_id|>"), "<\\|eot_id\\|>");
		// sequence control tokens
		assert_eq!(f("<bos>x<eos>"), "<\\bos>x<\\eos>");
		for raw in ["<|turn>", "<turn|>", "<|im_start|>", "<|eot_id|>", "a<||>b"] {
			let out = f(raw);
			// every closer must be escaped (`\|>`), no opener may survive
			assert!(
				!out.contains("<|") && !out.replace("\\|>", "").contains("|>"),
				"{raw:?} -> {out:?} still holds a special-token spelling"
			);
		}
		// plain text (including ordinary tags the prompts rely on) is kept
		assert_eq!(
			f("<idea author=\"x\">plain</idea>"),
			"<idea author=\"x\">plain</idea>"
		);
		assert_eq!(f("math: 5 < 10 > 2"), "math: 5 < 10 > 2");
	}

	#[test]
	fn kv_reuse_decodes_only_the_new_tail_of_a_grown_prompt() {
		use super::plan_kv_reuse;
		// cache = last prompt + the reply generated after it; the next
		// prompt repeats that history and appends a new user turn
		let cached = [1, 2, 3, 40, 41];
		let prompt = [1, 2, 3, 40, 41, 7, 8];
		let plan = plan_kv_reuse(&cached, &prompt).expect("shared prefix");
		assert_eq!(plan.decode_from, 5);
		assert_eq!(plan.tokens_after_prompt(&cached, &prompt), prompt);

		// the reply was re-templated differently: reuse stops at the split
		let prompt = [1, 2, 3, 9, 9];
		let plan = plan_kv_reuse(&cached, &prompt).expect("shared prefix");
		assert_eq!(plan.decode_from, 3);
		assert_eq!(plan.tokens_after_prompt(&cached, &prompt), prompt);
	}

	#[test]
	fn kv_reuse_of_an_exact_repeat_records_each_prompt_token_once() {
		use super::plan_kv_reuse;
		// the same prompt again (a retry, or a cache that ends exactly at
		// the prompt): the final token re-decodes for fresh logits, so its
		// cell is dropped and must not stay recorded alongside the re-decode
		let prompt = [1, 2, 3, 4];
		for cached in [&[1, 2, 3, 4][..], &[1, 2, 3, 4, 50, 51][..]] {
			let plan = plan_kv_reuse(cached, &prompt).expect("shared prefix");
			assert_eq!(plan.decode_from, 3, "the last token decodes again");
			assert_eq!(
				plan.tokens_after_prompt(cached, &prompt),
				prompt,
				"cache bookkeeping must match the cells actually held"
			);
		}
		// a one-token prompt still decodes its token
		let plan = plan_kv_reuse(&[1, 2], &[1]).expect("shared prefix");
		assert_eq!(plan.decode_from, 0);
		assert_eq!(plan.tokens_after_prompt(&[1, 2], &[1]), [1]);
	}

	#[test]
	fn kv_reuse_is_skipped_for_divergent_prompts() {
		use super::plan_kv_reuse;
		assert_eq!(plan_kv_reuse(&[1, 2, 3], &[7, 2, 3]), None);
		assert_eq!(plan_kv_reuse::<i32>(&[], &[1, 2]), None);
	}

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
		assert_eq!(
			run(&["<th", "ink>reasoning</thi", "nk>ans", "wer"]),
			"answer"
		);
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
		assert_eq!(
			run(&["Wait. <think>reconsider</think> Done."]),
			"Wait. Done."
		);
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
		)
		.expect("parse");
		assert!(!done);
		assert_eq!(output, "ab");
		assert_eq!(chunks, vec!["a".to_string(), "b".to_string()]);

		let done = consume_sse_event("data: [DONE]", &mut output, &mut |_| {}).expect("parse");
		assert!(done);
	}

	#[test]
	fn consume_sse_event_surfaces_error_objects() {
		let mut output = String::new();
		let err = consume_sse_event(
			"data: {\"error\":{\"message\":\"content filter flagged this\"}}",
			&mut output,
			&mut |_| {},
		)
		.expect_err("error objects must surface");
		assert!(err.contains("content filter"), "unexpected: {err}");
	}

	#[test]
	fn map_provider_error_maps_content_filter_to_469() {
		assert_eq!(
			map_provider_error(400, r#"{"error":{"code":"content_filter"}}"#),
			"HttpError 469: Inappropriate input"
		);
		assert!(map_provider_error(401, "bad key").contains("rejected credentials"));
		assert!(map_provider_error(500, "boom").contains("500"));
	}
}

#[cfg(test)]
mod external_stream_tests {
	use super::{read_body_capped, ExternalLlm};

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
	async fn external_generate_streams_from_loopback_sse_server() {
		let url = serve_sse(vec![
			"data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n".into(),
			"data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n".into(),
			"data: [DONE]\n\n".into(),
		]);
		let client = ExternalLlm::new(&url, "", "test-model").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let mut chunks = Vec::new();
		let (output, prompt_tokens) = client
			.generate("you are a test", &[], &cancel, 32, |c| chunks.push(c))
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], &cancel, 8, |_| {})
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let mut chunks = Vec::new();
		let (output, _) = client
			.generate("", &[], &cancel, 8, |c| chunks.push(c))
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
			let client = ExternalLlm::new(&url, "", "m").expect("client");
			let (output, _) = tokio::time::timeout(
				std::time::Duration::from_secs(5),
				client.generate("", &[], &cancel, 8, |_| {}),
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], &cancel, 8, |_| {})
			.await
			.expect_err("JSON without choices[0].message.content must fail");
		assert!(err.contains("no completion"), "unexpected: {err}");

		let url = serve_typed(
			"application/json",
			r#"{"error":{"message":"model not loaded"}}"#.into(),
		);
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let err = client
			.generate("", &[], &cancel, 8, |_| {})
			.await
			.expect_err("an error object must surface");
		assert!(err.contains("model not loaded"), "unexpected: {err}");
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
}

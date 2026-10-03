use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::SeqState;

use crate::types::ChatMessage;

/// Default generation context size. Transcripts are conversational; 16k
/// tokens comfortably fits a long session plus the result document. The
/// user can opt into a larger window (the `llm_ctx_tokens` setting);
/// the window a load actually uses is [`effective_ctx_for`].
pub const DEFAULT_N_CTX: u32 = 16384;
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

/// The context window a load actually uses: the app default when the
/// requested setting is 0 (or absent), otherwise the request - always
/// clamped to what the model was trained for, so a small-trained-context
/// model is never run with a silently-degrading oversized window. A
/// model whose trained context is unknown (None) keeps the default.
/// Pure, so the loader's already-loaded check and the engine build can
/// never disagree about the window.
pub fn effective_ctx_for(requested: u32, trained: Option<u32>) -> u32 {
	let requested = if requested == 0 {
		DEFAULT_N_CTX
	} else {
		requested
	};
	match trained {
		Some(trained) => requested.min(trained).max(1),
		None => DEFAULT_N_CTX,
	}
}

/// Tokens of room the assembled prompt may occupy in a `n_ctx` window:
/// the window minus the generation budget and a small margin. Extracted
/// pure so the truncation budget provably derives from (and scales
/// with) the effective context; see [`LocalLlm::build_prompt`].
fn prompt_budget(n_ctx: u32, max_new_tokens: u32) -> usize {
	(n_ctx as usize).saturating_sub(max_new_tokens as usize + 64)
}

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

/// Decode one token's bytes with a streaming UTF-8 decoder: a multi-byte
/// character can be split across tokens, so the decoder keeps the
/// incomplete tail for the next call. Mirrors the decode step llama-cpp-2
/// did inside LlamaModel::token_to_piece before 0.1.158 moved
/// detokenization to LlamaVocab, which returns raw bytes.
fn decode_piece(decoder: &mut encoding_rs::Decoder, bytes: &[u8]) -> Result<String, String> {
	// decode_to_string never grows its destination; this bound includes
	// any incomplete sequence held over from the previous token
	let capacity = decoder
		.max_utf8_buffer_length(bytes.len())
		.ok_or("token output is too large to decode")?;
	let mut out = String::with_capacity(capacity);
	let (result, read, _) = decoder.decode_to_string(bytes, &mut out, false);
	if !matches!(result, encoding_rs::CoderResult::InputEmpty) || read != bytes.len() {
		return Err("UTF-8 decoder did not consume the whole token".into());
	}
	Ok(out)
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
	/// The context window this engine was actually built for:
	/// [`effective_ctx_for`] of the requested setting and the trained
	/// context at load time. A same-model settings change that alters it
	/// must rebuild the engine.
	built_ctx: u32,
}

impl LocalLlm {
	pub fn load(
		backend: Arc<LlamaBackend>,
		path: &Path,
		model_id: &str,
		requested_ctx: u32,
	) -> Result<Self, String> {
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
		let built_ctx = effective_ctx_for(requested_ctx, trained_ctx);
		if let Some(trained) = trained_ctx {
			log::info!(
				"model {model_id} ({architecture}) trained context: {trained} tokens; \
				 effective context: {built_ctx}"
			);
		}
		Ok(Self {
			backend,
			model: Arc::new(model),
			model_id: model_id.to_string(),
			kv_state: std::sync::Mutex::new(None),
			architecture,
			trained_ctx,
			built_ctx,
		})
	}

	/// The context window this engine was built for.
	pub fn built_ctx(&self) -> u32 {
		self.built_ctx
	}

	/// The model's trained context length, when the GGUF reports one.
	pub fn trained_ctx(&self) -> Option<u32> {
		self.trained_ctx
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
		Ok(self.tokenize_prompt(prompt).len())
	}

	/// Tokenize an assembled prompt: no BOS added (the template writes its
	/// own), and special-token spellings ARE parsed - the chat scaffolding
	/// depends on it. User and imported text is made safe for that by
	/// neutralize_turn_markers before it is assembled into the prompt.
	fn tokenize_prompt(&self, prompt: &str) -> Vec<llama_cpp_2::token::LlamaToken> {
		self.model.vocab().tokenize(prompt.as_bytes(), false, true)
	}

	/// Build the final prompt, dropping older middle messages until it fits
	/// into the context window this engine was built for, with room for
	/// `max_new_tokens`. The budget derives from `built_ctx`, so a larger
	/// configured window automatically truncates less.
	fn build_prompt(
		&self,
		system: &str,
		messages: &[ChatMessage],
		max_new_tokens: u32,
	) -> Result<String, String> {
		let budget = prompt_budget(self.built_ctx, max_new_tokens);
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
		let tokens = self.tokenize_prompt(&prompt);
		if tokens.is_empty() {
			return Err(
				"the model produced no tokens for this conversation; please try again".into(),
			);
		}
		let n_prompt = tokens.len();

		// The context borrows the model, so it lives only within this call;
		// the KV cache travels separately, as captured state bytes.
		let n_ctx = self.built_ctx;
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

		let vocab = self.model.vocab();
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
			if vocab.is_eog(token) {
				break;
			}
			// a token with no text piece yields no bytes; it is still fed back
			let piece = decode_piece(&mut decoder, &vocab.token_to_piece(token, false, None))?;
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

/// Hard cap on a single SSE event, delimiter included. Real events
/// carry a few token bytes in a JSON envelope, so anything near this
/// size means the endpoint is not speaking SSE (e.g. an HTML error
/// page or a CRLF-averse parser deadlock). 1 MiB.
const MAX_SSE_EVENT_BYTES: usize = 1 << 20;
/// Hard cap on the accumulated completion text across all events of
/// one generation. Far above the largest supported max_tokens budget
/// (4096 tokens), yet a valid-event-forever endpoint cannot grow
/// memory (or the streamed transcript) without end. 8 MiB.
const MAX_OUTPUT_BYTES: usize = 8 << 20;
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
		on_chunk: impl FnMut(String) + Send,
	) -> Result<(String, Option<usize>), String> {
		self.generate_with_limits(
			system,
			messages,
			cancel,
			max_tokens,
			on_chunk,
			GenerationLimits::production(),
		)
		.await
	}

	/// [`ExternalLlm::generate`] with injectable bounds, so tests can
	/// exercise the caps and the overall budget without waiting out
	/// the generous production values.
	async fn generate_with_limits(
		&self,
		system: &str,
		messages: &[ChatMessage],
		cancel: &AtomicBool,
		max_tokens: u32,
		mut on_chunk: impl FnMut(String) + Send,
		limits: GenerationLimits,
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
fn consume_sse_event(
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

	#[tokio::test]
	async fn read_body_capped_accepts_a_body_exactly_at_the_cap() {
		// a valid body that is exactly `cap` bytes must arrive whole
		let url = serve_raw(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n0123456789".to_vec());
		let response = reqwest::Client::new().get(&url).send().await.expect("send");
		let body = read_body_capped(response, 10, 5, 5).await;
		assert_eq!(body, "0123456789", "a body exactly at the cap is valid");
	}

	#[test]
	fn consume_sse_event_accepts_non_streaming_completions() {
		// a server that ignored stream:true replies with one data event
		// carrying choices[0].message.content
		let event = "data: {\"choices\":[{\"message\":{\"content\":\"hello there\"}}]}";
		let mut output = String::new();
		let mut chunks = Vec::new();
		let done = super::consume_sse_event(
			event,
			&mut output,
			&mut |c| chunks.push(c),
			super::MAX_OUTPUT_BYTES,
		)
		.expect("parse");
		assert!(!done, "no [DONE] marker yet");
		assert_eq!(output, "hello there");
		assert_eq!(chunks, vec!["hello there".to_string()]);
	}

	#[test]
	fn decode_piece_reassembles_characters_split_across_tokens() {
		let mut decoder = encoding_rs::UTF_8.new_decoder();
		// a 4-byte character
		let emoji = "🧠".as_bytes();
		// the first token carries half the character: nothing to emit yet
		assert_eq!(super::decode_piece(&mut decoder, &emoji[..2]).unwrap(), "");
		// the next token completes it, plus plain text
		let mut rest = emoji[2..].to_vec();
		rest.extend_from_slice(b" ok");
		assert_eq!(super::decode_piece(&mut decoder, &rest).unwrap(), "🧠 ok");
		// a token with no text piece decodes to nothing
		assert_eq!(super::decode_piece(&mut decoder, &[]).unwrap(), "");
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

	#[test]
	fn effective_ctx_resolves_the_requested_window_against_the_trained_one() {
		use super::{effective_ctx_for, DEFAULT_N_CTX};
		// no request: the app default
		assert_eq!(effective_ctx_for(0, None), DEFAULT_N_CTX);
		assert_eq!(effective_ctx_for(0, Some(131072)), DEFAULT_N_CTX);
		// a model trained below the default clamps the default down
		assert_eq!(effective_ctx_for(0, Some(8192)), 8192);
		// an unknown trained context keeps the default regardless of the ask
		assert_eq!(effective_ctx_for(65536, None), DEFAULT_N_CTX);
		// an explicit request wins up to the trained window...
		assert_eq!(effective_ctx_for(32768, Some(131072)), 32768);
		assert_eq!(effective_ctx_for(131072, Some(131072)), 131072);
		// ...and clamps to it beyond
		assert_eq!(effective_ctx_for(131072, Some(65536)), 65536);
		// a degenerate zero trained value must never yield a zero context
		// (load filters those out; the pure fn stays safe for any input)
		assert_eq!(effective_ctx_for(8192, Some(0)), 1);
	}

	#[test]
	fn the_truncation_budget_scales_with_the_effective_context() {
		use super::{effective_ctx_for, prompt_budget, DEFAULT_N_CTX, MAX_NEW_TOKENS_RESULT};
		let at_default = prompt_budget(effective_ctx_for(0, Some(131072)), MAX_NEW_TOKENS_RESULT);
		assert_eq!(
			at_default,
			(DEFAULT_N_CTX - MAX_NEW_TOKENS_RESULT - 64) as usize
		);
		let at_64k = prompt_budget(
			effective_ctx_for(65536, Some(131072)),
			MAX_NEW_TOKENS_RESULT,
		);
		assert_eq!(at_64k, (65536 - MAX_NEW_TOKENS_RESULT - 64) as usize);
		assert!(
			at_64k > at_default,
			"a larger setting shrinks what gets truncated"
		);
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

	#[tokio::test]
	async fn external_generate_rejects_an_empty_sse_stream() {
		// A 200 text/event-stream with an empty body used to return
		// Ok(("", None)), so the command layer treated an unusable
		// completion as a success.
		let url = serve_sse(vec![]);
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], &cancel, 8, |_| {})
			.await
			.expect_err("an empty SSE body is not a completion");
		assert!(err.contains("empty completion"), "unexpected: {err}");
	}

	#[tokio::test]
	async fn external_generate_rejects_a_stream_of_only_comments_and_keepalives() {
		let url = serve_sse(vec![": keepalive\n\n".into(), ": ping\n\n".into()]);
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], &cancel, 8, |_| {})
			.await
			.expect_err("comments carry no completion");
		assert!(err.contains("empty completion"), "unexpected: {err}");
	}

	#[tokio::test]
	async fn external_generate_rejects_an_empty_done_marker_without_content() {
		let url = serve_sse(vec!["data: [DONE]\n\n".into()]);
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], &cancel, 8, |_| {})
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], &cancel, 8, |_| {})
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], &cancel, 8, |_| {})
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let (output, _) = client
			.generate("", &[], &cancel, 8, |_| {})
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = client
			.generate("", &[], &cancel, 8, |_| {})
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let (output, _) = client
			.generate("", &[], &cancel, 8, |_| {})
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let started = std::time::Instant::now();
		let limits = GenerationLimits {
			overall_budget: std::time::Duration::from_millis(400),
			cancel_tick: std::time::Duration::from_millis(20),
			..GenerationLimits::production()
		};
		let result = tokio::time::timeout(
			std::time::Duration::from_secs(5),
			client.generate_with_limits("", &[], &cancel, 8, |_| {}, limits),
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
		let client = ExternalLlm::new(&format!("http://{addr}/v1"), "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let mut streamed = 0usize;
		let err = match client
			.generate("", &[], &cancel, 8, |c| streamed += c.len())
			.await
		{
			Ok((output, _)) => panic!(
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
		let client = ExternalLlm::new(&format!("http://{addr}/v1"), "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let err = match client.generate("", &[], &cancel, 8, |_| {}).await {
			Ok((output, _)) => panic!(
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let flag = cancel.clone();
		tokio::spawn(async move {
			tokio::time::sleep(std::time::Duration::from_millis(150)).await;
			flag.store(true, std::sync::atomic::Ordering::Relaxed);
		});
		let started = std::time::Instant::now();
		let result = tokio::time::timeout(
			std::time::Duration::from_secs(2),
			client.generate("", &[], &cancel, 8, |_| {}),
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let limits = GenerationLimits {
			max_event_bytes: cap,
			..GenerationLimits::production()
		};
		let (output, _) = client
			.generate_with_limits("", &[], &cancel, 8, |_| {}, limits)
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let limits = GenerationLimits {
			max_output_bytes: 8,
			..GenerationLimits::production()
		};
		let (output, _) = client
			.generate_with_limits("", &[], &cancel, 8, |_| {}, limits)
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
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		let at_limit = GenerationLimits {
			max_output_bytes: 6, // "ab" (2) + "🧠" (4)
			..GenerationLimits::production()
		};
		let (output, _) = client
			.generate_with_limits("", &[], &cancel, 8, |_| {}, at_limit)
			.await
			.expect("the emoji lands exactly at the cap");
		assert_eq!(output, "ab🧠");
		assert_eq!(output.chars().count(), 3, "no off-by-one at the boundary");

		let url = serve_sse(vec![
			"data: {\"choices\":[{\"delta\":{\"content\":\"ab\"}}]}\n\n".into(),
			"data: {\"choices\":[{\"delta\":{\"content\":\"🧠\"}}]}\n\n".into(),
			"data: [DONE]\n\n".into(),
		]);
		let client = ExternalLlm::new(&url, "", "m").expect("client");
		let one_past = GenerationLimits {
			max_output_bytes: 5, // "ab" fits, "🧠" would make 6
			..GenerationLimits::production()
		};
		let mut chunks = Vec::new();
		let err = client
			.generate_with_limits("", &[], &cancel, 8, |c| chunks.push(c), one_past)
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

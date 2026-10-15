//! The in-process llama.cpp engine: model loading, KV-cache reuse across
//! turns, prompt assembly and the streaming output filters.

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

use super::external::truncate_at_boundary;
use super::{
	approx_tokens, effective_ctx_for, neutralize_turn_markers, prompt_budget,
	MAX_NEW_TOKENS_RESPONSE, MAX_NEW_TOKENS_RESULT,
};
use crate::types::ChatMessage;

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

/// How a restored KV cache lines up with the next prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct KvReuse {
	/// First prompt position that still decodes. Restored cells from here
	/// on are dropped, so this is also how many cached tokens stay: on an
	/// exact repeat it is one short of the shared prefix, and the dropped
	/// final token is re-decoded rather than kept twice.
	pub(super) decode_from: usize,
}

impl KvReuse {
	/// The tokens the cache holds once the prompt tail is decoded.
	pub(super) fn tokens_after_prompt<T: Clone>(&self, cached: &[T], prompt: &[T]) -> Vec<T> {
		let mut tokens = cached[..self.decode_from].to_vec();
		tokens.extend_from_slice(&prompt[self.decode_from..]);
		tokens
	}
}

/// Plan reusing a cache built from `cached` for `prompt`: only the tail
/// past the common prefix decodes, but at least the final prompt token
/// always does so logits exist for sampling even on an exact repeat.
/// None when nothing is shared.
pub(super) fn plan_kv_reuse<T: PartialEq>(cached: &[T], prompt: &[T]) -> Option<KvReuse> {
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
pub(super) fn decode_piece(
	decoder: &mut encoding_rs::Decoder,
	bytes: &[u8],
) -> Result<String, String> {
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
		// An oversized system prompt (a huge feedback idea) can never be
		// trimmed by dropping turns: say so instead of failing inside the
		// decoder with a cryptic error.
		if self.count_tokens(system)? > budget {
			return Err(
				"This idea is too long for the current context window. Raise it under Settings > AI Models."
					.into(),
			);
		}
		// Drop the oldest user+assistant pair after the opener (never a
		// single message: roles must keep alternating so the chat template
		// keeps applying, and pair-sized steps keep the KV prefix
		// reusable), with hysteresis so following turns fit untrimmed.
		let count = |s: &str| self.count_tokens(s).unwrap_or_else(|_| approx_tokens(s));
		let render = |msgs: &[ChatMessage]| -> String {
			self.apply_template(system, msgs).unwrap_or_default()
		};
		let (fitted, dropped) = crate::llm::fit_chat_turns(messages, budget, &count, &render);
		if dropped > 0 {
			log::info!("dropped {dropped} older turns to fit the context");
		}
		let mut msgs: Vec<ChatMessage> = fitted;
		let mut prompt = self.apply_template(system, &msgs)?;
		let mut n_tokens = self.count_tokens(&prompt)?;

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
	/// CPU/GPU heavy; run on a dedicated thread. Returns the final text,
	/// the prompt token count (for the telemetry fields the command layer
	/// reports) and how many transcript turns a summary fit had to drop
	/// (0 for chat turns).
	///
	/// `summary_render` (Some = this is a summary generation) serializes the
	/// fitted conversation into the single summary message: the turns are
	/// fitted to this engine's context BEFORE serialization (see
	/// [`crate::llm::fit_summary_turns`]), so an oversized session loses
	/// its oldest turns instead of having the newest cut off mid-JSON.
	#[allow(clippy::too_many_arguments)]
	pub fn generate(
		&self,
		system: &str,
		messages: &[ChatMessage],
		summary_render: Option<super::SummaryRenderRef<'_>>,
		cancel: &AtomicBool,
		temperature_override: Option<f32>,
		mut on_chunk: impl FnMut(String),
	) -> Result<(String, usize, usize), String> {
		let summarize = summary_render.is_some();
		let max_new = if summarize {
			MAX_NEW_TOKENS_RESULT
		} else {
			MAX_NEW_TOKENS_RESPONSE
		};
		let (messages, dropped_turns) = match summary_render {
			Some(render) => {
				let budget = prompt_budget(self.built_ctx, max_new);
				let count = |s: &str| self.count_tokens(s).unwrap_or_else(|_| approx_tokens(s));
				let (fitted, dropped) =
					crate::llm::fit_summary_turns(messages, budget, &count, render);
				(
					vec![ChatMessage {
						role: "user".into(),
						content: render(&fitted),
					}],
					dropped,
				)
			}
			None => (messages.to_vec(), 0),
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
		// CPU threads: llama.cpp defaults to 4, which leaves most of an
		// 8-16 core CPU-only machine idle. Physical cores only (the
		// hyperthreads slow generation down), clamped so an inflated
		// core count cannot spawn an absurd pool.
		let n_threads = num_cpus::get_physical().clamp(1, 32) as i32;
		// On macOS the Metal backend does the heavy lifting; a large host
		// thread pool for batch decode only competes for memory
		// bandwidth. CPU-only builds (Windows/Linux) use every core.
		#[cfg(target_os = "macos")]
		let n_threads_batch = n_threads.min(4);
		#[cfg(not(target_os = "macos"))]
		let n_threads_batch = n_threads;
		let ctx_params = LlamaContextParams::default()
			.with_n_ctx(Some(
				NonZeroU32::new(n_ctx).expect("context size is never zero"),
			))
			.with_n_batch(PROMPT_DECODE_CHUNK as u32)
			.with_n_threads(n_threads)
			.with_n_threads_batch(n_threads_batch);
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

		// The shared sampling profile (llm::Sampling): penalties and
		// min_p sit ahead of temperature in the chain, per llama.cpp
		// guidance; a temperature override (the malformed-summary retry)
		// wins over the profile's value.
		let sampling = crate::llm::sampling::effective(summarize);
		let temperature = temperature_override.unwrap_or(sampling.temperature);
		let seed = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|d| d.subsec_nanos())
			.unwrap_or(42);
		let mut sampler = LlamaSampler::chain(
			[
				LlamaSampler::penalties(
					0,
					sampling.repeat_last_n,
					sampling.repeat_penalty,
					0.0,
					0.0,
				),
				LlamaSampler::top_k(sampling.top_k),
				LlamaSampler::top_p(sampling.top_p, 1),
				LlamaSampler::min_p(sampling.min_p, 1),
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

		Ok((output, n_prompt, dropped_turns))
	}
}

/// Streaming filter that removes `<think>...</think>` reasoning blocks.
/// Thinking models (e.g. MiniCPM5) emit their reasoning before the answer;
/// only the final response should be displayed or stored. Tags can split
/// across token pieces, so the filter buffers until a tag is confirmed.
pub(super) struct ThinkFilter {
	buffer: String,
	in_think: bool,
}

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

impl ThinkFilter {
	pub(super) fn new() -> Self {
		Self {
			buffer: String::new(),
			in_think: false,
		}
	}

	/// Feed one decoded piece; returns whatever is now safe to emit.
	pub(super) fn push(&mut self, piece: &str) -> String {
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
	pub(super) fn finish(&mut self) -> String {
		if self.in_think {
			self.buffer.clear();
			String::new()
		} else {
			std::mem::take(&mut self.buffer)
		}
	}
}

//! The in-process llama.cpp engine: model loading, prompt assembly and
//! the streaming output filters, with inference on a dedicated worker
//! thread that owns a persistent `LlamaContext`.
//!
//! `LlamaContext` borrows `LlamaModel`, so the two cannot both be
//! fields of one struct - the constructor moves them into a worker
//! thread instead, and [`LocalLlm`] keeps a request sender. The worker
//! creates the context on its first request (the full-window KV
//! allocation - 1.5-2.35 GB at 16k ctx - happens once per engine, not
//! once per call) and reuses it: each request keeps the longest token
//! prefix it shares with what the context already holds, drops the KV
//! cells from the first differing token on, and decodes only the new
//! tail. No per-call context build, and no `state_seq_get`/`state_seq_set`
//! round-trip of the KV state through host RAM.
//!
//! Requests are one `mpsc` message each; the channel is the queue, so a
//! second concurrent `generate()` simply waits its turn (callers already
//! run inside `spawn_blocking`; `Sender::send` takes `&self`, so no
//! mutex is needed). Shutdown rides on channel close: the worker holds
//! only the `Receiver`, and the last [`LocalLlm`] dropping (vacate /
//! swap_engine) closes it - the thread then finishes any in-flight
//! request (whose caller still holds an `Arc`, keeping the engine alive
//! and the sender open) and exits. Nothing joins the thread, so vacate
//! can never hang on it.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

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
/// How often the calling side of `generate()` polls the cancel flag
/// while waiting for worker output. The worker cannot see the caller's
/// `&AtomicBool` (it must not outlive the call), so cancellation is
/// bridged: the caller copies the flag into the request's shared abort
/// flag at this cadence - and on every arriving chunk - while the
/// worker checks abort at every decode step the pre-thread code checked
/// cancel at. Cancellation therefore stays as responsive as before,
/// plus at most one interval.
const CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// How a live KV cache lines up with the next prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct KvReuse {
	/// First prompt position that still decodes. Cached cells from here
	/// on are dropped, so this is also how many cached tokens stay: on an
	/// exact repeat it is one short of the shared prefix, and the dropped
	/// final token is re-decoded rather than kept twice.
	pub(super) decode_from: usize,
}

impl KvReuse {
	/// The cells the context holds once the tail has decoded through
	/// prompt position `decoded_to`: the surviving prefix plus however
	/// much of the tail actually decoded. The worker maintains this same
	/// state incrementally (truncate to `decode_from`, then extend per
	/// decoded chunk), so a cancelled or failed decode leaves an honest
	/// prefix - `decoded_to < prompt.len()` is exactly that case.
	pub(super) fn mirror_after<T: Clone>(
		&self,
		cached: &[T],
		prompt: &[T],
		decoded_to: usize,
	) -> Vec<T> {
		let mut mirror = cached[..self.decode_from].to_vec();
		mirror.extend_from_slice(&prompt[self.decode_from..decoded_to]);
		mirror
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

/// One generation job for the inference worker. The prompt is already
/// assembled and tokenized: `LlamaModel` is `Send + Sync` (and concurrent
/// generates already tokenized against it before the worker existed), so
/// that half of the old `generate()` stays on the calling thread. The
/// worker owns the context and runs decode + sampling.
struct GenRequest {
	/// The full prompt token stream, chat scaffolding included.
	tokens: Vec<LlamaToken>,
	/// The generation budget for this request (chat or summary cap).
	max_new: u32,
	/// Whether this is a summary generation (selects the sampling
	/// profile, as before).
	summarize: bool,
	temperature_override: Option<f32>,
	/// The worker-side cancel flag, bridged from the caller's
	/// `&AtomicBool` (see [`CANCEL_POLL_INTERVAL`]).
	abort: Arc<AtomicBool>,
	/// Per-request reply channel: chunks stream first, the final
	/// `Done` lands last (mpsc is FIFO, so the order holds).
	reply: mpsc::Sender<WorkerMsg>,
}

/// A message from the inference worker to one waiting `generate()` call.
enum WorkerMsg {
	/// One think-filtered piece of the response, delivered as produced.
	Chunk(String),
	/// The generation outcome: final text plus prompt token count.
	Done(Result<(String, usize), String>),
}

pub struct LocalLlm {
	/// The model, shared with the worker thread for prompt assembly on
	/// the calling thread (tokenize / chat template).
	model: Arc<LlamaModel>,
	pub model_id: String,
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
	/// Requests for the inference worker. `send` takes `&self` and is
	/// internally synchronized, so concurrent `generate()` calls queue
	/// through the channel without a mutex; the worker's `Receiver`
	/// closes when the last `LocalLlm` drops, which is the thread's
	/// shutdown signal (see the module doc).
	tx: mpsc::Sender<GenRequest>,
}

impl LocalLlm {
	pub fn load(
		backend: Arc<LlamaBackend>,
		path: &Path,
		model_id: &str,
		requested_ctx: u32,
	) -> Result<Self, String> {
		Self::load_with_progress(backend, path, model_id, requested_ctx, None)
	}

	/// [`Self::load`] with a load-progress observer: called with
	/// progress in 0..=1; returning `false` from it aborts the load
	/// (llama.cpp fails the load with an error).
	pub fn load_with_progress(
		backend: Arc<LlamaBackend>,
		path: &Path,
		model_id: &str,
		requested_ctx: u32,
		on_progress: Option<Box<dyn FnMut(f32) -> bool + Send>>,
	) -> Result<Self, String> {
		// Offload every layer to the GPU when a backend (Metal/Vulkan) exists;
		// llama.cpp falls back to CPU compute automatically otherwise.
		let mut params = LlamaModelParams::default().with_n_gpu_layers(u32::MAX);
		if let Some(on_progress) = on_progress {
			params = params.with_progress_callback(on_progress);
		}
		let model = Arc::new(
			LlamaModel::load_from_file(&backend, path, &params)
				.map_err(|e| format!("failed to load model {path:?}: {e}"))?,
		);
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
		// The worker owns model + backend and, from its first request,
		// the persistent context (they cannot live in this struct:
		// LlamaContext borrows LlamaModel - see the module doc).
		let (tx, rx) = mpsc::channel();
		std::thread::Builder::new()
			.name("llm-inference".to_string())
			.spawn({
				let model = Arc::clone(&model);
				let backend = Arc::clone(&backend);
				move || inference_worker(&model, &backend, built_ctx, rx)
			})
			.map_err(|e| format!("failed to spawn the inference thread: {e}"))?;
		Ok(Self {
			model,
			model_id: model_id.to_string(),
			architecture,
			trained_ctx,
			built_ctx,
			tx,
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
	fn tokenize_prompt(&self, prompt: &str) -> Vec<LlamaToken> {
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
	/// its oldest turns instead of having its newest cut off mid-JSON.
	///
	/// The prompt is assembled here (the model is Sync); the tokens then
	/// go to the persistent-context worker, and this call blocks in
	/// [`pump_reply`] streaming the worker's chunks back out.
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
		// Hand the tokens to the inference worker (the persistent context
		// and its KV cache live there) and relay its reply. `abort` is the
		// worker-visible half of `cancel`: this call's reference cannot
		// cross threads, so pump_reply bridges the two while waiting.
		let abort = Arc::new(AtomicBool::new(false));
		let (reply_tx, reply_rx) = mpsc::channel();
		self.tx
			.send(GenRequest {
				tokens,
				max_new,
				summarize,
				temperature_override,
				abort: Arc::clone(&abort),
				reply: reply_tx,
			})
			.map_err(|_| "the inference worker is not running".to_string())?;
		let (output, n_prompt) = pump_reply(reply_rx, cancel, &abort, &mut on_chunk)?;
		Ok((output, n_prompt, dropped_turns))
	}
}

/// The calling side of one generation: forwards worker chunks to
/// `on_chunk` as they arrive (the UI renders partial text live; nothing
/// is buffered whole) and returns the worker's final result.
///
/// While waiting, the call's `cancel` flag is polled and copied into
/// `abort`, which is what the worker checks - see
/// [`CANCEL_POLL_INTERVAL`]. Once abort is set the loop still waits for
/// the worker's `Done` (it notices abort at its next decode-step check,
/// bounded by one chunk), so a cancelled call returns the same
/// "generation cancelled" error the inline loop used to.
fn pump_reply(
	reply: mpsc::Receiver<WorkerMsg>,
	cancel: &AtomicBool,
	abort: &AtomicBool,
	on_chunk: &mut dyn FnMut(String),
) -> Result<(String, usize), String> {
	let cancelled = || {
		if cancel.load(Ordering::Relaxed) {
			abort.store(true, Ordering::Relaxed);
		}
	};
	loop {
		// The short timeout keeps the cancel poll running even while the
		// worker is deep in a (chunked) prompt decode and emits nothing.
		match reply.recv_timeout(CANCEL_POLL_INTERVAL) {
			Ok(WorkerMsg::Chunk(piece)) => {
				cancelled();
				on_chunk(piece);
			}
			Ok(WorkerMsg::Done(result)) => return result,
			Err(mpsc::RecvTimeoutError::Timeout) => cancelled(),
			// The worker thread died mid-request - only a panic can do
			// this; every decode error is a normal Done - and its context
			// died with it. Fail instead of hanging; later calls fail at
			// send with "the inference worker is not running".
			Err(mpsc::RecvTimeoutError::Disconnected) => {
				return Err("the inference worker stopped unexpectedly".into());
			}
		}
	}
}

/// The inference worker: owns the model and (from its first request on)
/// the persistent context, serving requests strictly one at a time in
/// channel order. Exits when every request sender drops - the last
/// `Arc<LocalLlm>` going away - after finishing any in-flight request
/// (see the module doc). Nothing here blocks on anything but the
/// channel, so shutdown is prompt.
fn inference_worker(
	model: &LlamaModel,
	backend: &LlamaBackend,
	built_ctx: u32,
	requests: mpsc::Receiver<GenRequest>,
) {
	// Created on the first request (and retried if creation fails): the
	// full-window KV allocation happens once per engine, not per call.
	// The context borrows `model`, which is why all of this lives on
	// this thread rather than in LocalLlm.
	let mut ctx: Option<LlamaContext> = None;
	// Mirror of the cells the context actually holds (seq 0, positions
	// 0..len) - the prefix the next request reuses. It replaces the old
	// captured-state `GenerationState` and is only ever extended as
	// cells decode, so a cancelled or failed generation leaves an honest
	// prefix behind (the old capture-on-cancel recorded the whole
	// planned prompt even when just part of it had decoded).
	let mut held: Vec<LlamaToken> = Vec::new();
	for req in requests {
		let GenRequest {
			tokens,
			max_new,
			summarize,
			temperature_override,
			abort: cancel,
			reply,
		} = req;
		let result = generate_on_context(
			model,
			backend,
			built_ctx,
			&mut ctx,
			&mut held,
			&tokens,
			max_new,
			summarize,
			temperature_override,
			&cancel,
			&reply,
		);
		// A dropped receiver (the caller's thread panicked) is not an
		// error for the worker: the context is simply done with this
		// request and moves on.
		let _ = reply.send(WorkerMsg::Done(result));
	}
}

/// The persistent inference context. Same shape the per-call context
/// had: the full effective window, prompt-decode-chunk batches, physical
/// cores clamped for generation threads (a small batch pool on macOS,
/// where Metal does the heavy lifting; every core elsewhere).
fn new_context<'m>(
	backend: &LlamaBackend,
	model: &'m LlamaModel,
	n_ctx: u32,
) -> Result<LlamaContext<'m>, llama_cpp_2::LlamaContextLoadError> {
	// CPU threads: llama.cpp defaults to 4, which leaves most of an
	// 8-16 core CPU-only machine idle. Physical cores only (the
	// hyperthreads slow generation down), clamped so an inflated
	// core count cannot spawn an absurd pool.
	let n_threads = num_cpus::get_physical().clamp(1, 32) as i32;
	#[cfg(target_os = "macos")]
	let n_threads_batch = n_threads.min(4);
	#[cfg(not(target_os = "macos"))]
	let n_threads_batch = n_threads;
	let params = LlamaContextParams::default()
		.with_n_ctx(Some(
			NonZeroU32::new(n_ctx).expect("context size is never zero"),
		))
		.with_n_batch(PROMPT_DECODE_CHUNK as u32)
		.with_n_threads(n_threads)
		.with_n_threads_batch(n_threads_batch);
	model.new_context(backend, params)
}

/// Decode + sample one request against the persistent context. This is
/// the worker half of the old `generate()`: KV reuse from the first
/// differing token, chunked prompt decode with cancel/stall/total-time
/// checks, the sampling chain, and the think-filtered streaming loop.
/// On cancel or failure the context simply keeps the cells it decoded
/// (`held` records exactly those), so the next request reuses them.
#[allow(clippy::too_many_arguments)]
fn generate_on_context<'m>(
	model: &'m LlamaModel,
	backend: &LlamaBackend,
	built_ctx: u32,
	ctx_slot: &mut Option<LlamaContext<'m>>,
	held: &mut Vec<LlamaToken>,
	tokens: &[LlamaToken],
	max_new: u32,
	summarize: bool,
	temperature_override: Option<f32>,
	cancel: &AtomicBool,
	reply: &mpsc::Sender<WorkerMsg>,
) -> Result<(String, usize), String> {
	let n_prompt = tokens.len();
	let ctx = match ctx_slot {
		Some(ctx) => ctx,
		None => {
			// A failed creation leaves the slot empty, so the next
			// request retries - matching the per-call creation this
			// replaced, where every generate() got its own chance.
			let ctx = new_context(backend, model, built_ctx)
				.map_err(|e| format!("failed to create context: {e}"))?;
			ctx_slot.insert(ctx)
		}
	};

	let started = std::time::Instant::now();
	let mut last_progress = std::time::Instant::now();
	let stalled =
		|| format!("generation stalled (no progress for {GENERATION_IDLE_TIMEOUT_SECS}s)");
	let overdue = || format!("generation timed out after {GENERATION_MAX_TOTAL_SECS}s");

	// Reuse the live KV from the first differing token: keep the common
	// prefix (minus its final token, which re-decodes for fresh logits),
	// drop every cell past it, decode only the tail. No shared prefix
	// (or an empty cache) keeps nothing - the equivalent of the fresh
	// context this path used to build, expressed as the same clear from
	// position 0.
	let mut reuse = plan_kv_reuse(held, tokens).unwrap_or(KvReuse { decode_from: 0 });
	if let Err(e) = ctx.clear_kv_cache_seq(Some(0), Some(reuse.decode_from as u32), None) {
		log::warn!("KV truncation failed; decoding full prompt: {e:?}");
		reuse.decode_from = 0;
	} else if reuse.decode_from > 0 {
		log::info!(
			"reusing KV cache: {} cached tokens, decoding {} new",
			reuse.decode_from,
			n_prompt - reuse.decode_from
		);
	}
	// The mirror now holds the surviving cells - the plan played out
	// through its split with nothing decoded past it yet; the decode
	// loop extends it per chunk so it always reflects decoded reality.
	*held = reuse.mirror_after(held, tokens, reuse.decode_from);
	let decode_from = reuse.decode_from;

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
		if last_progress.elapsed() > std::time::Duration::from_secs(GENERATION_IDLE_TIMEOUT_SECS) {
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
		// the chunk's cells are now live in the context
		held.extend_from_slice(chunk);
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
			LlamaSampler::penalties(0, sampling.repeat_last_n, sampling.repeat_penalty, 0.0, 0.0),
			LlamaSampler::top_k(sampling.top_k),
			LlamaSampler::top_p(sampling.top_p, 1),
			LlamaSampler::min_p(sampling.min_p, 1),
			LlamaSampler::temp(temperature),
			LlamaSampler::dist(seed),
		],
		true,
	);

	let vocab = model.vocab();
	let mut decoder = encoding_rs::UTF_8.new_decoder();
	let mut output = String::new();
	let mut think_filter = ThinkFilter::new();
	let mut generated: u32 = 0;
	let mut answer_tokens: u32 = 0;
	// Absolute backstop across answer + reasoning tokens; can never
	// overflow the context window regardless of prompt length.
	let total_cap = max_new
		.saturating_add(MAX_THINK_TOKENS)
		.min((built_ctx - 1).saturating_sub(tokens.len() as u32));
	let mut pos = tokens.len() as i32;

	loop {
		if cancel.load(Ordering::Relaxed) {
			return Err("generation cancelled".into());
		}
		if last_progress.elapsed() > std::time::Duration::from_secs(GENERATION_IDLE_TIMEOUT_SECS) {
			return Err(stalled());
		}
		if started.elapsed() > std::time::Duration::from_secs(GENERATION_MAX_TOTAL_SECS) {
			return Err(overdue());
		}
		let token = sampler.sample(ctx, -1);
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
				let _ = reply.send(WorkerMsg::Chunk(safe));
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
		held.push(token);
		pos += 1;
	}

	let tail = think_filter.finish();
	if !tail.is_empty() {
		output.push_str(&tail);
		let _ = reply.send(WorkerMsg::Chunk(tail));
	}

	Ok((output, n_prompt))
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

/// Unit tests for the request plumbing (the protocol half of the
/// persistent-context engine): these run without a model by scripting
/// the worker's side of the reply channel.
#[cfg(test)]
mod pump_tests {
	use super::{pump_reply, WorkerMsg};
	use std::sync::atomic::{AtomicBool, Ordering};
	use std::sync::{mpsc, Arc};
	use std::time::{Duration, Instant};

	/// Blocks up to `secs` waiting for `flag`; a bridging failure then
	/// surfaces through the mock worker's reply instead of a hung test.
	fn await_flag(flag: &Arc<AtomicBool>, secs: u64) -> bool {
		let deadline = Instant::now() + Duration::from_secs(secs);
		while Instant::now() < deadline {
			if flag.load(Ordering::Relaxed) {
				return true;
			}
			std::thread::sleep(Duration::from_millis(2));
		}
		flag.load(Ordering::Relaxed)
	}

	#[test]
	fn chunks_stream_through_in_order_before_the_result() {
		let (tx, rx) = mpsc::channel();
		std::thread::spawn(move || {
			for piece in ["He", "llo", " wor", "ld"] {
				tx.send(WorkerMsg::Chunk(piece.to_string())).unwrap();
				std::thread::sleep(Duration::from_millis(5));
			}
			tx.send(WorkerMsg::Done(Ok(("Hello world".into(), 7))))
				.unwrap();
		});
		let mut seen = Vec::new();
		let mut on_chunk = |piece: String| seen.push(piece);
		let (output, n_prompt) = pump_reply(
			rx,
			&AtomicBool::new(false),
			&AtomicBool::new(false),
			&mut on_chunk,
		)
		.expect("the worker's result flows back");
		assert_eq!(seen, ["He", "llo", " wor", "ld"], "chunks arrive in order");
		assert_eq!(output, "Hello world");
		assert_eq!(n_prompt, 7);
	}

	#[test]
	fn a_cancel_seen_on_a_chunk_bridges_into_abort() {
		// cancel is already set: the first arriving chunk must raise the
		// abort flag the worker polls, and the worker's outcome (here the
		// cancellation error) must flow back unchanged
		let (tx, rx) = mpsc::channel();
		let abort = Arc::new(AtomicBool::new(false));
		let worker_abort = Arc::clone(&abort);
		std::thread::spawn(move || {
			tx.send(WorkerMsg::Chunk("x".into())).unwrap();
			let observed = await_flag(&worker_abort, 5);
			tx.send(WorkerMsg::Done(Err(if observed {
				"generation cancelled".to_string()
			} else {
				"abort was never set".to_string()
			})))
			.unwrap();
		});
		let err = pump_reply(rx, &AtomicBool::new(true), &abort, &mut |_| {})
			.expect_err("a cancelled call returns the worker's error");
		assert_eq!(err, "generation cancelled");
	}

	#[test]
	fn a_cancel_seen_while_nothing_arrives_bridges_into_abort() {
		// the worker is mid prompt-decode and sends nothing: the poll
		// timeout arm is what notices cancel
		let (tx, rx) = mpsc::channel();
		let abort = Arc::new(AtomicBool::new(false));
		let worker_abort = Arc::clone(&abort);
		std::thread::spawn(move || {
			let observed = await_flag(&worker_abort, 5);
			tx.send(WorkerMsg::Done(Err(if observed {
				"generation cancelled".to_string()
			} else {
				"abort was never set".to_string()
			})))
			.unwrap();
		});
		let err = pump_reply(rx, &AtomicBool::new(true), &abort, &mut |_| {})
			.expect_err("a cancelled call returns the worker's error");
		assert_eq!(err, "generation cancelled");
	}

	#[test]
	fn a_dead_worker_fails_the_call_instead_of_hanging() {
		let (tx, rx) = mpsc::channel();
		drop(tx);
		let err = pump_reply(
			rx,
			&AtomicBool::new(false),
			&AtomicBool::new(false),
			&mut |_| {},
		)
		.expect_err("a dropped reply channel is an error");
		assert!(err.contains("worker"), "the error names the worker: {err}");
	}
}

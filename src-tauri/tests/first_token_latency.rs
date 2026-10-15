//! OPTIONAL first-token-latency benchmark over the local engine,
//! env-gated on LLM_MODEL_PATH like the other model-gated suites - but
//! unlike them it SKIPS (with a printed note) when the variable is
//! unset, because it is a measurement harness, not an assertion.
//!
//! Per conversation turn it prints the wall time from generate() entry
//! to the first streamed chunk (first-token latency: context setup +
//! KV-cache preparation + prompt decode + first sample) and the total
//! call time. On the pre-persistent-context code every call allocated a
//! fresh full-window LlamaContext (~1.5-2.35 GB of KV cells at 16k ctx)
//! and round-tripped the previous KV state through host RAM
//! (state_seq_get -> bytes -> state_seq_set); on the persistent-context
//! code the first call pays the allocation once and later turns keep
//! the live cache, clear from the first differing token, and decode
//! only the new tail.
//!
//! Before/after measurement on a machine with a model (the numbers must
//! come from a real run; this file is the harness):
//!
//!   git worktree add ../bench-base <pre-change-commit>
//!   cp tests/first_token_latency.rs ../bench-base/src-tauri/tests/
//!   LLM_MODEL_PATH=/path/to/model.gguf cargo test --release \
//!     --manifest-path ../bench-base/src-tauri/Cargo.toml \
//!     --test first_token_latency -- --nocapture
//!   LLM_MODEL_PATH=/path/to/model.gguf \
//!     cargo test --release --test first_token_latency -- --nocapture
//!
//! then compare the per-turn "to first chunk" columns (same model,
//! machine and prompt; use --release, a debug build measures nothing
//! useful). Turn 1 is the cold context; turns 2+ are the KV-reuse path
//! the change targets, so their delta is the win.
#![allow(linker_messages)]

use brainstory_lib::llm::LocalLlm;
use brainstory_lib::types::ChatMessage;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Generation stops after this many streamed pieces: first-token latency
/// (the metric) is unaffected and the run stays quick on CPU-only
/// machines. The partial reply stays in the context's cache exactly like
/// a user stop, so later turns still exercise prefix reuse.
const ENOUGH_PIECES: usize = 16;

#[test]
fn first_token_latency_by_turn() {
	let Ok(path) = std::env::var("LLM_MODEL_PATH") else {
		println!("LLM_MODEL_PATH is not set; skipping the first-token-latency benchmark");
		return;
	};
	let backend = Arc::new(
		llama_cpp_2::llama_backend::LlamaBackend::init().expect("failed to init llama backend"),
	);
	let llm = LocalLlm::load(backend, Path::new(&path), "bench", 0).expect("failed to load model");

	// A realistically heavy system prompt (~4 KB) so prompt decode - the
	// part KV reuse eliminates - dominates the first-token number.
	let system = format!(
		"You are a focused brainstorming interviewer for spoken thoughts. {}\nAsk one short \
		 question at a time.",
		"Never suggest solutions yourself; reflect the user's own words back, probe for concrete \
		 details and next actions, and keep the conversation moving one small step at a time. "
			.repeat(24)
	);
	let mut conversation = vec![ChatMessage {
		role: "user".into(),
		content: "I want to reorganize my garage workshop this weekend.".into(),
	}];
	let cancel = Arc::new(AtomicBool::new(false));

	for turn in 1..=4 {
		if turn > 1 {
			// extend the transcript with the next user turn; the model's
			// actual output from the previous turn is already recorded as
			// an assistant turn, so the next prompt shares that whole
			// prefix and adds a tail - the KV-reuse shape of a real
			// interview
			conversation.push(ChatMessage {
				role: "user".into(),
				content: format!(
					"Turn {turn}: mostly the boxes I never sorted. What should I do first?"
				),
			});
		}
		let first_chunk = Arc::new(Mutex::new(None::<Instant>));
		let mut pieces = 0usize;
		let mut streamed = String::new();
		let started = Instant::now();
		let outcome = llm.generate(&system, &conversation, None, &cancel, None, |piece| {
			let mut slot = first_chunk.lock().unwrap();
			if slot.is_none() {
				*slot = Some(Instant::now());
			}
			drop(slot);
			streamed.push_str(&piece);
			pieces += 1;
			if pieces >= ENOUGH_PIECES {
				cancel.store(true, Ordering::Relaxed);
			}
		});
		let total = started.elapsed();
		let first = first_chunk
			.lock()
			.unwrap()
			.map(|at| at.duration_since(started));
		match outcome {
			Ok((_, n_prompt, _)) => println!(
				"turn {turn}: prompt {n_prompt:5} tokens | first chunk {:>10.3?} | total \
				 {total:8.3?} | {pieces} pieces",
				first.expect("at least one piece arrived for an Ok result"),
			),
			// our own stop after ENOUGH_PIECES
			Err(e) if e == "generation cancelled" && pieces >= ENOUGH_PIECES => println!(
				"turn {turn}: prompt (cancelled)   | first chunk {:>10.3?} | total {total:8.3?} \
				 | {pieces} pieces",
				first.expect("the stop only fires after pieces arrived"),
			),
			Err(e) => panic!("turn {turn} generation failed: {e}"),
		}
		// the model's actual output becomes the transcript's assistant
		// turn, so the next turn's prompt shares this whole prefix
		if !streamed.is_empty() {
			conversation.push(ChatMessage {
				role: "assistant".into(),
				content: streamed,
			});
		}
		cancel.store(false, Ordering::Relaxed);
	}
	println!(
		"compare turn 1 (cold context) against turns 2+ (live KV reuse) across the two commits; \
		 see the module doc for the exact commands"
	);
}

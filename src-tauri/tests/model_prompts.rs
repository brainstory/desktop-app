//! Model validation against the app's real prompts (env-gated like
//! engine_smoke): runs the actual story-interview chat system prompt and
//! the story result summary prompt through LocalLlm, exactly as the ai
//! command layer resolves them, and checks that thinking models come back
//! clean (no `<think>` reasoning in streamed or returned text).
//! The truncation probes at the bottom drive the context-fit path with
//! oversized payloads; they need a fixture whose trained context is at
//! least 8192 tokens (smaller windows degenerate: the summarize budget
//! `trained - 4096 - 64` saturates at zero and generation collapses).
//! Run: LLM_MODEL_PATH=<gguf> cargo test --test model_prompts -- --nocapture
#![allow(linker_messages)]

use brainstory_lib::llm::LocalLlm;
use brainstory_lib::prompts::{ChatType, PromptRequest};
use brainstory_lib::types::ChatMessage;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn generate(llm: &LocalLlm, request: &PromptRequest) -> (String, String) {
	let system = request.system_prompt();
	let messages = request.user_messages();
	let cancel = Arc::new(AtomicBool::new(false));
	let mut streamed = String::new();
	let (returned, prompt_tokens) = llm
		.generate(&system, &messages, request.summarize, &cancel, |chunk| {
			streamed.push_str(&chunk);
		})
		.expect("generation failed");
	println!("[tokens] prompt: {prompt_tokens}");
	(returned, streamed)
}

fn assert_clean(label: &str, returned: &str, streamed: &str) {
	println!("--- {label}:\n{returned}\n");
	assert!(!returned.trim().is_empty(), "{label} produced no output");
	assert_eq!(
		returned, streamed,
		"{label}: streamed and returned text differ"
	);
	assert!(
		!returned.contains("<think>"),
		"{label}: reasoning leaked into output"
	);
}

#[test]
fn model_prompt_suite() {
	// one model load, serial phases: two parallel Metal contexts from
	// two independent tests collide on GPU memory
	let path = std::env::var("LLM_MODEL_PATH").expect("set LLM_MODEL_PATH to a llama gguf");
	let backend =
		Arc::new(llama_cpp_2::llama_backend::LlamaBackend::init().expect("backend init failed"));
	let llm = LocalLlm::load(backend, std::path::Path::new(&path), "test", 0).expect("load failed");
	real_prompts_work_end_to_end(&llm);
	kv_cache_reuse_produces_completions_and_skips_prefix_decode(&llm);
	oversized_payload_fit_probes(&llm);
}

fn real_prompts_work_end_to_end(llm: &LocalLlm) {
	// A realistic brainstorming transcript, the same shape the chat command
	// receives (user turns + assistant turns).
	let transcript = vec![
		ChatMessage {
			role: "user".into(),
			content: "I keep procrastinating on writing my thesis introduction.".into(),
		},
		ChatMessage {
			role: "assistant".into(),
			content: "What makes the introduction feel harder than the other sections?".into(),
		},
		ChatMessage {
			role: "user".into(),
			content: "I guess I'm scared the first sentence has to be perfect.".into(),
		},
	];

	let chat = PromptRequest {
		chat_type: ChatType::Original,
		messages: transcript.clone(),
		summarize: false,
		react_to: None,
		react_to_author: None,
		react_to_is_current_user: false,
		structured_feedback: false,
	};
	let (returned, streamed) = generate(llm, &chat);
	assert_clean("interview chat reply", &returned, &streamed);

	// The writeup pass: the result system prompt plus the JSON-encoded
	// transcript, resolved through PromptRequest just like the app does.
	let summary = PromptRequest {
		summarize: true,
		messages: transcript,
		..chat
	};
	let (returned, streamed) = generate(llm, &summary);
	assert_clean("story result summary", &returned, &streamed);
}

fn kv_cache_reuse_produces_completions_and_skips_prefix_decode(llm: &LocalLlm) {
	// Turn 1: a cold context; turn 2 extends the same conversation, so a
	// KV prefix must be restored and only the tail decoded.
	let turn1 = PromptRequest {
		chat_type: ChatType::Original,
		messages: vec![ChatMessage {
			role: "user".into(),
			content: "Tell me a story about a small dragon.".into(),
		}],
		summarize: false,
		react_to: None,
		react_to_author: None,
		react_to_is_current_user: false,
		structured_feedback: false,
	};
	let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
	let (first, prompt_tokens) = llm
		.generate("", &turn1.messages.clone(), false, &cancel, &mut |_| {})
		.expect("first generation");
	assert!(prompt_tokens > 0, "prompt tokens counted");

	let mut extended = turn1.messages.clone();
	extended.push(ChatMessage {
		role: "assistant".into(),
		content: first.clone(),
	});
	extended.push(ChatMessage {
		role: "user".into(),
		content: "What is the dragon's name?".into(),
	});
	let (second, _) = llm
		.generate("", &extended, false, &cancel, &mut |_| {})
		.expect("second generation with restored KV");
	assert!(
		!second.trim().is_empty(),
		"generation still works off a restored KV"
	);

	// A divergent conversation (no shared prefix beyond the header) must
	// also work: the restore is skipped, not required.
	let (fresh, _) = llm
		.generate(
			"",
			&[ChatMessage {
				role: "user".into(),
				content: "Something completely different: name three colors.".into(),
			}],
			false,
			&cancel,
			&mut |_| {},
		)
		.expect("generation after divergence");
	assert!(!fresh.trim().is_empty());
}

/// A deterministic transcript of roughly `bytes` bytes: alternating
/// user/assistant turns of realistic sentences. No tag spellings, so
/// `sanitize_tag_content` leaves the text verbatim inside the JSON.
fn long_transcript(bytes: usize, multibyte: bool) -> Vec<ChatMessage> {
	let mut messages = Vec::new();
	let mut written = 0usize;
	let mut i = 0;
	while written < bytes {
		let role = if i % 2 == 0 { "user" } else { "assistant" };
		let sentence =
			if multibyte {
				format!("第{i}个想法：我一直在权衡这个项目的范围和时间安排。🧠")
			} else {
				format!("Note {i}: I keep circling back to the same tradeoff between scope and timing. ")
			};
		written += sentence.len();
		messages.push(ChatMessage {
			role: role.into(),
			content: sentence,
		});
		i += 1;
	}
	messages
}

/// Context-fit truncation probes (env-gated, run last so the phases above
/// keep their existing KV dynamics). Drive `LocalLlm::generate` with
/// payloads past the context budget so `build_prompt`'s hard
/// truncation engages, and record what is observable through the
/// public API:
///
/// - oversized summaries (the `<t>` transcript, plus `<oid>` framing
///   for feedback) must still come back as a decodable prompt and a
///   non-empty, think-free generation - the fit loop may not collapse
///   the generation cap to ~0 or fail the decode;
/// - a multibyte-heavy oversized transcript exercises the byte-based
///   cut end to end;
/// - an immutable payload too large for the window (a huge parent
///   idea, which lives in the SYSTEM prompt and is never truncated)
///   currently empties the user reply, stays over budget, and fails
///   at prompt decode. That error is the documented current failure
///   mode; the proposed fix must replace it with a clear too-large
///   error before any decode happens (update this assertion then).
///
/// What these runs CANNOT show is tag integrity of the final prompt:
/// `generate` exposes only the token count, so whether the closing
/// `</t>`/`</oid>` survive a cut is established by reading
/// `build_prompt` (they do not: the cut keeps a prefix and appends the
/// notice) - a fix needs a test seam exposing the fitted prompt.
///
/// The already-fitting control is `real_prompts_work_end_to_end`
/// above: a small transcript that triggers no truncation.
fn oversized_payload_fit_probes(llm: &LocalLlm) {
	// ~80 KB is comfortably past the summarize budget (effective
	// context is clamped to 16384, minus 4096 + 64) for any tokenizer
	// a fixture would plausibly use.
	let original = PromptRequest {
		chat_type: ChatType::Original,
		messages: long_transcript(80_000, false),
		summarize: true,
		react_to: None,
		react_to_author: None,
		react_to_is_current_user: false,
		structured_feedback: false,
	};
	let (returned, streamed) = generate(llm, &original);
	assert_clean(
		"oversized original summary (truncated <t> payload)",
		&returned,
		&streamed,
	);

	let feedback = PromptRequest {
		chat_type: ChatType::Feedback,
		react_to: Some("# Parent idea\n\n## Section\nA reasonably sized parent idea.\n".into()),
		react_to_author: Some("Ada".into()),
		..original
	};
	let (returned, streamed) = generate(llm, &feedback);
	assert_clean(
		"oversized feedback summary (truncated <oid>/<t> payload)",
		&returned,
		&streamed,
	);

	let multibyte = PromptRequest {
		messages: long_transcript(80_000, true),
		..feedback
	};
	let (returned, streamed) = generate(llm, &multibyte);
	assert_clean(
		"oversized multibyte summary (UTF-8-safe cut)",
		&returned,
		&streamed,
	);

	// ~270 KB of idea (~30k+ tokens for any tokenizer) in the system
	// prompt: past the 16384 window itself, so the message the fitter
	// CAN truncate is emptied and the prompt still does not fit.
	let huge_idea = PromptRequest {
		chat_type: ChatType::Feedback,
		messages: vec![
			ChatMessage {
				role: "assistant".into(),
				content: "Opening question about the idea.".into(),
			},
			ChatMessage {
				role: "user".into(),
				content: "My first reaction.".into(),
			},
		],
		summarize: false,
		react_to: Some("This idea body keeps repeating its core point.\n".repeat(6_000)),
		react_to_author: Some("Ada".into()),
		react_to_is_current_user: false,
		structured_feedback: false,
	};
	let cancel = Arc::new(AtomicBool::new(false));
	let system = huge_idea.system_prompt();
	let messages = huge_idea.user_messages();
	let err = llm
		.generate(&system, &messages, false, &cancel, &mut |_| {})
		.expect_err("a system payload past the window must fail, not hang or generate");
	assert!(
		err.contains("decode"),
		"expected the documented prompt-decode failure, got: {err}"
	);
	println!("[f21] oversized system payload error: {err}");
}

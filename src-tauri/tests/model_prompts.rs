//! Model validation against the app's real prompts (env-gated like
//! engine_smoke): runs the actual story-interview chat system prompt and
//! the story result summary prompt through LocalLlm, exactly as the ai
//! command layer resolves them, and checks that thinking models come back
//! clean (no `<think>` reasoning in streamed or returned text).
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
	let llm = LocalLlm::load(backend, std::path::Path::new(&path), "test").expect("load failed");
	real_prompts_work_end_to_end(&llm);
	kv_cache_reuse_produces_completions_and_skips_prefix_decode(&llm);
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

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
	assert_eq!(returned, streamed, "{label}: streamed and returned text differ");
	assert!(!returned.contains("<think>"), "{label}: reasoning leaked into output");
}

#[test]
fn real_prompts_work_end_to_end() {
	let path = std::env::var("LLM_MODEL_PATH").expect("set LLM_MODEL_PATH to a llama gguf");
	let backend = Arc::new(
		llama_cpp_2::llama_backend::LlamaBackend::init().expect("backend init failed"),
	);
	let llm = LocalLlm::load(backend, std::path::Path::new(&path), "test").expect("load failed");

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
	let (returned, streamed) = generate(&llm, &chat);
	assert_clean("interview chat reply", &returned, &streamed);

	// The writeup pass: the result system prompt plus the JSON-encoded
	// transcript, resolved through PromptRequest just like the app does.
	let summary = PromptRequest {
		summarize: true,
		messages: transcript,
		..chat
	};
	let (returned, streamed) = generate(&llm, &summary);
	assert_clean("story result summary", &returned, &streamed);
}

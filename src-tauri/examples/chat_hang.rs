//! Reproduces the exact generate_response command path (prompt building +
//! llama generation) against a real model. Times each phase.
//!
//! Usage: cargo run --release --example chat_hang -- /path/to/model.gguf

use brainstory_lib::llm::LocalLlm;
use brainstory_lib::prompts::{ChatType, PromptRequest};
use brainstory_lib::types::ChatMessage;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

fn main() {
	let path = std::env::args().nth(1).expect("model path required");
	let now = || Instant::now();

	let t = now();
	let backend = Arc::new(
		llama_cpp_2::llama_backend::LlamaBackend::init().expect("backend init"),
	);
	let llm = LocalLlm::load(backend, std::path::Path::new(&path), "test")
		.expect("model load");
	eprintln!("[timing] model load: {:?}", t.elapsed());

	let request = PromptRequest {
		chat_type: ChatType::Original,
		messages: vec![
			ChatMessage {
				role: "assistant".into(),
				content: "Hi, how's it going? What's on your mind?".into(),
			},
			ChatMessage {
				role: "user".into(),
				content: "I have been thinking about how I want to structure my week so I have more time for deep work.".into(),
			},
		],
		summarize: false,
		react_to: None,
		react_to_author: None,
		react_to_is_current_user: false,
		structured_feedback: false,
	};

	let t = now();
	let system = request.system_prompt();
	eprintln!("[timing] system prompt: {:?} ({} chars)", t.elapsed(), system.len());

	let user_messages = request.user_messages();
	let cancel = Arc::new(AtomicBool::new(false));

	let t = now();
	let mut chunk_count = 0usize;
	let output = llm
		.generate(&system, &user_messages, false, &cancel, |_| {
			chunk_count += 1;
		})
		.expect("generation failed");
	eprintln!(
		"[timing] generate: {:?} ({chunk_count} chunks, {} chars)",
		t.elapsed(),
		output.len()
	);
	println!("--- output ---");
	println!("{output}");
}

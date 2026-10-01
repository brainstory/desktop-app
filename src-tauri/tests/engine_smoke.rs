//! Verifies that whisper.cpp and llama.cpp can run in the same process
//! despite each vendoring its own ggml copy (duplicate symbol link warnings).
//! Point the env vars at any whisper ggml + llama gguf.
#![allow(linker_messages)]

use brainstory_lib::llm::LocalLlm;
use brainstory_lib::stt::SttEngine;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[test]
fn whisper_and_llama_coexist() {
	let whisper_path = std::env::var("WHISPER_MODEL_PATH")
		.expect("set WHISPER_MODEL_PATH to a whisper ggml model");
	let llama_path =
		std::env::var("LLM_MODEL_PATH").expect("set LLM_MODEL_PATH to a tiny llama gguf");

	// --- whisper: transcribe one second of silence ---
	let engine = SttEngine::load(std::path::Path::new(&whisper_path), "test")
		.expect("failed to load whisper model");
	let samples = vec![0.0f32; 16000];
	let transcript = engine
		.transcribe(&samples, "en-US")
		.expect("whisper transcription failed");
	println!("whisper transcript of silence: {transcript:?}");
	// whisper annotates silence ("[BLANK_AUDIO]"); none of it is speech
	assert_eq!(transcript, "", "silence must transcribe to nothing");

	// --- llama.cpp: generate a few tokens ---
	let backend = Arc::new(
		llama_cpp_2::llama_backend::LlamaBackend::init().expect("failed to init llama backend"),
	);
	let llm = LocalLlm::load(backend, std::path::Path::new(&llama_path), "test")
		.expect("failed to load llama model");
	// A few pieces prove generation runs; stop there instead of a full
	// reply (up to 1024 tokens). On the CI macOS VM's paravirtual GPU most
	// of this tiny model's ops fall back to the CPU (63 graph splits vs 2
	// on real Apple Silicon), about a second per token, so a full reply
	// ran into the 900 s generation ceiling.
	const ENOUGH_PIECES: usize = 8;
	let cancel = Arc::new(AtomicBool::new(false));
	let mut chunks = Vec::new();
	let result = llm.generate(
		"",
		&[brainstory_lib::types::ChatMessage {
			role: "user".into(),
			content: "Once upon a time".into(),
		}],
		false,
		&cancel,
		|piece| {
			chunks.push(piece);
			if chunks.len() >= ENOUGH_PIECES {
				cancel.store(true, std::sync::atomic::Ordering::Relaxed);
			}
		},
	);
	match result {
		Ok((_, prompt_tokens)) => println!("prompt tokenized to {prompt_tokens} tokens"),
		// our own stop after ENOUGH_PIECES
		Err(e) if e == "generation cancelled" && chunks.len() >= ENOUGH_PIECES => {}
		Err(e) => panic!("llama generation failed: {e:?}"),
	}
	// sanity: generation produced something and did not crash
	println!("generated: {:?}", chunks.concat());
	assert!(!chunks.is_empty());
}

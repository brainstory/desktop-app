#![allow(linker_messages)]

//! One binary for every model-gated integration test: each file here
//! statically links llama.cpp, whisper.cpp and onnxruntime, so separate
//! test binaries (one per file) linked the whole stack up to six times
//! per `cargo test` / `cargo clippy --all-targets`. Every suite keeps
//! its env-var gate and its own module; run one suite by filtering:
//!
//!     cargo test --test model_gated special_tokens -- --nocapture

mod apple_stt_smoke;
mod engine_smoke;
mod model_prompts;
mod special_tokens;
mod speech_pipeline;

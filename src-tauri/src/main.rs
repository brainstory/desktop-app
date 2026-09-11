// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// whisper.cpp and llama.cpp each vendor their own ggml; the linker warns
// about duplicate symbols, but resolution is verified at runtime by the
// `engine_smoke` integration test (both engines run in one process).
#![allow(linker_messages)]

fn main() {
	brainstory_lib::run()
}

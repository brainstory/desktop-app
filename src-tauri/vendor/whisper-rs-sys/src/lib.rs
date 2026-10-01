#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

// Brainstory: whisper.cpp is built against llama-cpp-sys-2's ggml and does
// not bundle its own. Referencing the crate makes rustc load it as this
// crate's dependency, so its ggml static libraries are linked, and linked
// after libwhisper (GNU ld resolves static archives left to right).
extern crate llama_cpp_sys_2 as _;

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

# Vendored whisper-rs

whisper-rs and whisper-rs-sys live here instead of coming from crates.io so
that whisper.cpp can be built against **the same ggml as llama.cpp**. Two
copies of ggml in one binary collide (duplicate symbols; the linker keeps
one copy of each function for both engines), and once the copies drift
apart the engines crash - e.g. llama-cpp-sys-2 0.1.158 (ggml 0.24) aborts on
`GGML_ASSERT(a->op == GGML_OP_FLASH_ATTN_EXT)` when linked next to
whisper-rs-sys 0.15's ggml 0.9.5, whose op enum is numbered differently.

## Origins

| Path | Source | Version |
| --- | --- | --- |
| `whisper-rs/` | crates.io `whisper-rs` | 0.16.0 (codeberg.org/tazz4843/whisper-rs @ 7558e1b) |
| `whisper-rs-sys/` (Rust parts) | crates.io `whisper-rs-sys` | 0.15.0 |
| `whisper-rs-sys/whisper.cpp/` | github.com/ggml-org/whisper.cpp | a44e078 (2026-09-22), the last commit on ggml 0.24.0 |

Only the parts of whisper.cpp the library build needs are kept: `CMakeLists.txt`,
`cmake/`, `include/`, `src/`, `LICENSE`, and `bindings/javascript/package-tmpl.json`
(read by its CMake when built standalone). Its bundled `ggml/` is deliberately
not vendored: ggml comes from llama-cpp-sys-2.

Licenses: whisper-rs is released into the public domain (Unlicense), whisper.cpp
is MIT (`whisper-rs-sys/whisper.cpp/LICENSE`).

## Local changes

Every change to the upstream files is in this repository's git history on
top of the commit that added them unmodified ("Vendor whisper-rs ...").

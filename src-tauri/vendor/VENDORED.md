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
| `whisper-rs-sys/whisper.cpp/` | github.com/ggml-org/whisper.cpp | 6e4ab85 (2026-09-28, master) |

whisper.cpp 6e4ab85 bundles ggml 0.25.1, but its library code is identical
to a44e078 (2026-09-22, the last commit written for ggml 0.24.0) apart from
one fix in `src/whisper.cpp` (language auto-detection honours the abort
callback), which uses no new ggml API - so it builds against llama's ggml
0.24.

Only the parts of whisper.cpp the library build needs are kept: `CMakeLists.txt`,
`cmake/`, `include/`, `src/`, `LICENSE`, and `bindings/javascript/package-tmpl.json`
(read by its CMake when built standalone). Its bundled `ggml/` is deliberately
not vendored: ggml comes from llama-cpp-sys-2.

Licenses: whisper-rs is released into the public domain (Unlicense), whisper.cpp
is MIT (`whisper-rs-sys/whisper.cpp/LICENSE`).

## Local changes

Every change to the upstream files is in this repository's git history on
top of the commit that added them unmodified ("Vendor whisper-rs ..."). In
short, all in `whisper-rs-sys/` (marked `Brainstory:` in the code):

- `Cargo.toml`: depends on `llama-cpp-sys-2` (same exact version as the app's
  pin). That makes cargo pass `DEP_LLAMA_GGML_CMAKE_DIR`, the ggml CMake
  package llama-cpp-sys-2 installs and exports for this purpose.
- `build.rs`: configures whisper.cpp with `WHISPER_USE_SYSTEM_GGML=ON` and
  `ggml_DIR` pointing at that package (approach from whisper-rs PR #260,
  using llama's ggml instead of a system-installed one); generates bindings
  against its headers; links only `libwhisper`, never a `ggml*` library; and
  reads whisper.cpp's newer `set(WHISPER_VERSION_MAJOR ...)` version format.
- `wrapper.h`: includes `<ggml.h>` from that package.
- `src/lib.rs`: `extern crate llama_cpp_sys_2` so rustc links llama's ggml
  after libwhisper (static archives resolve left to right on Linux).

## Updating

- llama-cpp-2 / llama-cpp-sys-2: bump the app's pins and the version in
  `whisper-rs-sys/Cargo.toml` together, then pick the whisper.cpp commit whose
  `ggml/CMakeLists.txt` has the same `GGML_VERSION_*` as llama.cpp's.
- Run the env-gated engine tests against real models (README, "Manual model
  tests") before merging: they are the only proof both engines still work.

# Vendored crates

## whisper-rs

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

### Local changes (whisper-rs)

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

### Updating (whisper-rs)

- llama-cpp-2 / llama-cpp-sys-2: bump the app's pins and the version in
  `whisper-rs-sys/Cargo.toml` together, then pick the whisper.cpp commit whose
  `ggml/CMakeLists.txt` has the same `GGML_VERSION_*` as llama.cpp's.
- Run the env-gated engine tests against real models (README, "Manual model
  tests") before merging: they are the only proof both engines still work.

## glib-macros / gtk3-macros

glib-macros and gtk3-macros (the proc-macro crates behind gtk-rs, pulled in
via tauri's Linux GTK stack) are vendored to drop the **unmaintained
`proc-macro-error` crate** ([RUSTSEC-2024-0370](https://rustsec.org/advisories/RUSTSEC-2024-0370.html)).
Tauri 2.x pins gtk-rs to the 0.18 series, whose macros crates depend on
`proc-macro-error` (and through it `syn 1.x`). Upstream removed the dependency,
but only in newer series than tauri allows:

- `glib-macros`: removed in [gtk-rs/gtk-rs-core#1288](https://github.com/gtk-rs/gtk-rs-core/pull/1288)
  (commit `c74a40a16`), first released in **0.19.0**; the 0.18 series ended at
  0.18.5 without a backport.
- `gtk3-macros`: removed in commit
  [`45782251fa3`](https://github.com/gtk-rs/gtk3-rs/commit/45782251fa35ecf8bfe759ffceb3db32659a0c97),
  first released in **0.19.0**; 0.18.2 is the last 0.18.

## Origins (gtk)

| Path | Source | Version |
| --- | --- | --- |
| `glib-macros/` | crates.io `glib-macros` | 0.18.5 (gtk-rs/gtk-rs-core @ 42b9caf, tag `0.18.5`) |
| `gtk3-macros/` | crates.io `gtk3-macros` | 0.18.2 (gtk-rs/gtk3-rs @ 00133512bf) |

Only the published crate contents are kept (`src/`, `Cargo.toml`, `LICENSE`,
`COPYRIGHT`); each crate's `tests/` (dev-only, needs the full glib/gtk
workspace) is omitted.

Licenses: both are MIT (`glib-macros/LICENSE`, `gtk3-macros/LICENSE`).

### Local changes (gtk)

Each crate replaces `proc-macro-error`'s `abort!`/`abort_call_site!` /
`#[proc_macro_error]` with explicit `syn::Result` propagation, exactly
mirroring the upstream commits above (the changes are on top of the commit
that vendored the crates unmodified). `proc-macro-error` is removed from each
`Cargo.toml`. The generated code and the compile errors users see are
unchanged; only the error-reporting mechanism differs (a `syn::Error` rendered
as `compile_error!` instead of a proc-macro panic).

### Updating (gtk)

Drop the `[patch.crates-io]` entries for `glib-macros`/`gtk3-macros` (and
delete these directories) once tauri moves to a gtk-rs series whose macros no
longer need `proc-macro-error` - i.e. glib-macros >= 0.19 and gtk3-macros >=
0.19. Until then, if the 0.18 versions are ever bumped, re-apply the two
removal commits on top of the new crates.io sources.

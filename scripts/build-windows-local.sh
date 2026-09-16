#!/usr/bin/env bash
# Local Windows cross-compile (macOS -> x86_64-pc-windows-msvc) via cargo-xwin.
# The real Windows builds run on GitHub Actions windows runners (native MSVC);
# this script is only for iterating locally without a Windows machine.
#
# Requirements (one-time):
#   brew install llvm lld cmake ninja
#   rustup target add x86_64-pc-windows-msvc
#   cargo install cargo-xwin
#
# Known quirk: whisper-rs-sys 0.15 build scripts evaluate
# `cfg!(target_os = "macos")` against the HOST when cross-compiling, so they
# emit a link directive for `ggml-blas.lib` even though the Windows cmake
# build has BLAS disabled. Pass 1 runs cmake (creating the build dir), the
# script drops an empty stub archive into it, and pass 2 links cleanly. The
# same duplicates are tolerated on real Windows builds via /FORCE:MULTIPLE
# (.cargo/config.toml).
set -euo pipefail
cd "$(dirname "$0")/.."

LLVM="$(brew --prefix llvm)"
LLD="$(brew --prefix lld)"
export PATH="$HOME/.cargo/bin:$LLVM/bin:$LLD/bin:$PATH"

# cargo-xwin replaces RUSTFLAGS, which overrides .cargo/config.toml
# rustflags - so the Windows-target flags are passed here instead. Keep in
# sync with src-tauri/.cargo/config.toml [target.x86_64-pc-windows-msvc].
export RUSTFLAGS="-C link-arg=/FORCE:MULTIPLE -L native=$(pwd)/src-tauri/windows-xwin-stub"

STUB="src-tauri/windows-xwin-stub/ggml-blas.lib"

# Pass 1: compile + cmake configure (may fail at the missing ggml-blas)
pnpm tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis ||
  echo "(pass 1: expected to stop at whisper-rs-sys on first run / after clean)"

# Drop the stub into every whisper-rs-sys cmake build dir
for d in src-tauri/target/x86_64-pc-windows-msvc/release/build/whisper-rs-sys/*/out/build; do
  [ -d "$d" ] || continue
  cp "$STUB" "$d/ggml-blas.lib"
  echo "stubbed $d/ggml-blas.lib"
done

# Pass 2: link completes and NSIS installer is bundled
pnpm tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis

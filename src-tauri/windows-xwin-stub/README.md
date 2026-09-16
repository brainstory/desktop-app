# Local Windows cross-compile stub

`whisper-rs-sys 0.15` build scripts evaluate `cfg!(target_os = "macos")`
against the HOST when cross-compiling, so building the Windows target from
a Mac emits a link directive for `ggml-blas.lib` even though the Windows
CMake build has BLAS disabled. This directory holds an empty COFF archive
that satisfies that directive.

Generate: clang-cl --target=x86_64-pc-windows-msvc -c empty.c, then
llvm-lib /out:ggml-blas.lib empty.obj (see .cargo/config.toml, which adds
this dir to the Windows-MSVC link search path).

Unused on real Windows builds (CI): there the host IS Windows and the
directive is never emitted.

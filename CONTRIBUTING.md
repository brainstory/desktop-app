# Contributing to Brainstory

## Toolchain

- Rust (stable; `rust-toolchain.toml` pins the channel and components)
- Node >= 24 and pnpm (`packageManager` in `package.json` pins the exact version)
- cmake + a C++ toolchain (llama.cpp and whisper.cpp are compiled from source)
- On macOS: Xcode; the Apple Speech bridge needs the Xcode 26 SDK (older SDKs stub
  it out with a build warning). On Linux: the apt packages listed in
  `.github/workflows/release.yml` under "Install Linux dependencies".

## Commands - keep all of these green

```sh
pnpm install --frozen-lockfile
pnpm -C app/frontend typecheck      # tsc --noEmit && astro check
pnpm -C app/frontend lint           # eslint, zero warnings
pnpm -C app/frontend format:check   # prettier
pnpm -C app/frontend test           # vitest
pnpm -C app/frontend build

cd src-tauri
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --lib --locked
```

## Conventions

- **Indentation is tabs everywhere** (`rustfmt.toml`, `.prettierrc`). Don't reformat
  unrelated code.
- One logical change per commit; the message says what and why.
- Every behaviour fix ships with a test that failed before and passes after.
- `cargo fmt`/prettier run before every commit that touches the relevant side.

## Prompt files (`prompts/*.txt`)

Prompt files are **user-facing product behaviour**, embedded at compile time. When
editing them, keep the input/output contracts intact:

- `<idea author="..." is_current_user="...">` appended to the react prompt,
- `<oid oida="..." is_current_user="...">` + `<t>` framing the summary user message,
- the feedback JSON schema (`feedback_items[].oid_heading_text` as
  `<index>##<heading>`, `matched_spans`, `feedback_text`, `labels` as
  `[{name, emoji}]`).

The prompt contract tests in `src-tauri/src/prompts.rs` assert these tags; run them
with `cargo test --lib prompts`.

## Pinned inference crates

`llama-cpp-2`, `whisper-rs`, and `speech` in `src-tauri/Cargo.toml` are exact-pinned
on purpose (comments in the file explain why). **Never bump them without running the
engine tests against real models**:

```sh
WHISPER_MODEL_PATH=... LLM_MODEL_PATH=... cargo test --test engine_smoke
LLM_MODEL_PATH=... cargo test --test model_prompts
```

## Releasing

1. `node scripts/bump-version.mjs <version>` - updates `tauri.conf.json`,
   `Cargo.toml` + `Cargo.lock`, and both `package.json` files. Commit.
2. Tag `v<version>` and push the tag. CI runs the checks, builds all three OSes,
   and creates a **draft** release.
3. Review the draft release (artifacts, `latest.json`, `SHA256SUMS.txt`) and publish
   it. Publishing is what serves the update to installed clients.

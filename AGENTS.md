# Agent notes

Ground rules for AI-assisted work in this repository.

## Commands that must stay green

```sh
pnpm install --frozen-lockfile
pnpm -C app/frontend typecheck
pnpm -C app/frontend lint            # zero warnings
pnpm -C app/frontend format:check
pnpm -C app/frontend test
pnpm -C app/frontend build
cd src-tauri && cargo fmt --check
cd src-tauri && cargo clippy --all-targets --locked -- -D warnings
cd src-tauri && cargo test --lib --locked
```

Run the relevant ones before every commit. Never delete or weaken a test to make
CI pass. If you cannot build the Rust side in your environment, say so and
restrict yourself to `cargo fmt --check` plus careful reading - never claim tests
passed that you did not run.

## Hard rules

1. **Never change pinned versions** of `llama-cpp-2`, `whisper-rs`, or `speech`
   in `src-tauri/Cargo.toml`. They are exact-pinned on purpose. Bumping them
   requires running the env-gated engine tests against real model files.
2. **Never force-push, rewrite history, touch the updater public key in
   `tauri.conf.json`, or commit secrets.**
3. **No new heavyweight dependencies** without a one-line justification in the
   commit message.
4. **Every behaviour fix ships with a test** that failed before and passes after;
   say so in the commit message.
5. **`prompts/*.txt` are product behaviour.** Keep the tag contracts
   (`<idea author=...>`, `<oid ...>`, `<t>`, the feedback JSON schema) intact.
6. Indentation is tabs everywhere. Small, focused commits; the message says what
   and why; never mix refactors with behaviour fixes.
7. Verify before you fix: re-read the cited code before changing it. When a fix
   is ambiguous, take the smaller/safer option and flag the alternative.

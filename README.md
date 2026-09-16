# Brainstory Desktop

Think out loud — rebuilt as a local-first desktop app. Everything (speech-to-text,
the LLM, your ideas, streaks, settings) runs and lives on your machine. No accounts,
no servers.

## Layout

```
desktop-app/
├── app/frontend/       The Brainstory frontend (Astro + React), imported from the
│                       original repo via `git subtree` (full git history preserved)
│                       and modified for Tauri (no auth/sharing/paywall, local AI)
├── prompts/            The prompting system, adapted from brainstory/prompts and the
│                       improved Claude Code rewrites (see prompts/README.md)
└── src-tauri/          The Rust backend
    └── src/
        ├── llm.rs        llama.cpp engine (Gemma 4) + OpenAI-compatible client
        ├── stt.rs        whisper.cpp engine + OpenAI-compatible client, WAV decoding
        ├── models.rs     model catalog, downloader with progress, engine runtime
        ├── db.rs         SQLite storage (ideas, logs, surveys, streaks, settings)
        ├── prompts.rs    system prompt assembly (<t>/<oid>/<idea> tag formats)
        ├── reminders.rs  daily reminder scheduler
        └── commands/     the Tauri command surface (the old REST API)
```

## AI models

First launch (or from **Profile → AI Models**), download:

- **LLM** — Google Gemma 4 E2B (QAT 4-bit GGUF, ~3.3 GB, default), Gemma 4 E4B
  (QAT 4-bit, ~5.2 GB), or MiniCPM5 2B (~1.6 GB) for lighter machines.
  Runs via llama.cpp with Metal acceleration on Apple Silicon.
- **STT** — whisper.cpp tiny.en (~78 MB), base.en (~148 MB, default), small.en
  (~488 MB), or large-v3-turbo (~1.6 GB).

Downloads are verified against pinned file sizes and SHA-256 hashes before
being activated, so a truncated or corrupted transfer can't brick a model slot.
Downloads can be cancelled from the same card; a hard quit leaves no garbage
behind (partial `.part` files are swept at the next launch). Switching models
unloads the old engine only when the new one is ready to take over — if the
new model fails to load, the previous one is restored and the app keeps
working. Optionally paste a HuggingFace access token (Profile → AI Models) —
authenticated downloads are faster and never hit anonymous rate limits. The
token is stored locally and never sent back to the UI, only a `••••1234`
style hint.

Recording is captured as 16 kHz mono WAV in the webview and transcribed locally.

**External offload**: Profile → AI Models → External endpoints lets you point the LLM
and/or STT at any OpenAI-compatible server (Ollama, llama.cpp server, LM Studio, ...).
If an external STT URL is set it takes precedence over the local whisper model;
likewise the LLM mode switch toggles local vs external.

## Sharing without accounts

Instead of the original share links, ideas travel as JSON files that you send however
you like (email, AirDrop, USB stick...):

- **Export** an idea (or feedback) from the idea page → `brainstory-*.json`
  containing the document, author name and a stable share id.
- **Import** from the dashboard → an idea lands in your library attributed to its
  author; feedback JSON attaches to the matching local idea, attributed to the
  sender and marked unread.
- Give feedback on any idea with **Give Feedback** (the normal feedback interview),
  then export the feedback and send it back.

The author name shown to others is your profile name (Profile → General).

## Daily reminders

Set a time under Profile → General → Notifications (or toggle it from the tray icon
menu). The app keeps running in the system tray when the window is closed and sends
a system notification once per day at the configured time. Quit fully from the tray
menu.

## Development

```sh
pnpm install          # installs the tauri CLI + frontend deps (workspace)

pnpm tauri:dev        # dev: astro dev server + debug build
pnpm tauri:build      # release build + .app + DMG (via scripts/make-dmg.sh,
                      # using plain hdiutil - tauri's own dmg bundler needs
                      # AppleScript control of Finder and fails headless)
pnpm -C app/frontend typecheck   # tsc --noEmit + astro check (strict)
pnpm -C app/frontend test        # vitest
pnpm -C app/frontend lint        # eslint
```

The frontend is TypeScript (`.ts` / `.tsx`; `.astro` pages stay `.astro`).
Keep `pnpm -C app/frontend typecheck` green alongside tests and lint.

Requirements: Rust, Node >= 24, pnpm, cmake (brew install cmake ninja), macOS 12+.

Local Windows cross-compile (no Windows machine needed):

```sh
brew install llvm lld makensis cmake ninja
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin
./scripts/build-windows-local.sh   # -> .../bundle/nsis/Brainstory_*_x64-setup.exe
```

The binary is cross-compiled but never executed locally; runtime verification
happens via the nightly/release Windows CI builds.
The `MACOSX_DEPLOYMENT_TARGET=12.0` needed by llama.cpp/whisper.cpp is set in
`src-tauri/.cargo/config.toml`.

Platform support: macOS (Apple Silicon) is the only tested target. The code
is mostly portable (the mic capture accepts F32/I16/U16 devices, and
notifications/dialogs use cross-platform Tauri plugins), but Dock/tray
presence handling and the release build are macOS-specific and
Windows/Linux builds are unverified.

Data locations (macOS): `~/Library/Application Support/ai.brainstory.desktop/`
(`brainstory.db` + `models/`).

## Notes / shortcuts taken

- whisper.cpp and llama.cpp each vendor their own ggml, so the link emits
  duplicate-symbol warnings; this is verified harmless by the
  `engine_smoke` integration test (`WHISPER_MODEL_PATH=... LLM_MODEL_PATH=...
  cargo test --release --test engine_smoke`), which runs both engines in one
  process. The lint is suppressed in the binary.
- HTTP 469 ("inappropriate input") moderation from the original backend is not
  implemented locally; some external providers' content-filter errors map to it.
- The daily-intent/context interview prompt is the original one; the other prompts
  use the improved Claude Code rewrites (adapted back to system-message form).
- Reminders fire while the app process is alive (tray). Scheduling while fully
  quit would need native per-platform notification scheduling.
- Logs (both stdout and a rotating file under `~/Library/Logs/ai.brainstory.desktop/`)
  are written via `tauri-plugin-log`; include `brainstory.log` when reporting
  model-load or storage problems.
- Share files are plain signed-by-nothing JSON: anyone can author one with any
  author name or share id. That is inherent to the file-based flow; imports
  are still size-bounded so a hostile file can't exhaust memory.

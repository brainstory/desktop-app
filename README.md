<div align="center">
  <img src="docs/og.png" alt="Brainstory" width="440" />
  <p><strong>Think out loud — rebuilt as a local-first desktop app.</strong></p>
  <p>Everything (speech-to-text, the LLM, your ideas, streaks, settings) runs and lives<br/>
  on your machine. No accounts, no servers.</p>
  <img src="docs/screenshot.png" alt="Brainstory dashboard" width="800" />
</div>

## AI models

First launch (or from **Settings → AI Models**), download:

- **LLM** — Google Gemma 4 E2B (QAT 4-bit GGUF, ~3.3 GB, default), Gemma 4 E4B
  (QAT 4-bit, ~5.2 GB), or MiniCPM5 2B (~1.6 GB) for lighter machines.
  Runs via llama.cpp with Metal acceleration on Apple Silicon.
- **STT** — whisper.cpp tiny.en (~78 MB), base.en (~148 MB, default), small.en
  (~488 MB), or large-v3-turbo (~1.6 GB). On macOS 26+, the built-in Apple
  Speech engine (Speech.framework, on-device) is available instead — no model
  download at all, with a selectable language and whisper as the automatic
  fallback. New installs default to "Auto" (Apple Speech where available,
  whisper otherwise); upgraded installs keep whisper and can switch under
  Settings → AI Models. Apple Speech needs one-time permission under System
  Settings → Privacy & Security → Speech Recognition.

Downloads are verified against pinned file sizes and SHA-256 hashes before
being activated, so a truncated or corrupted transfer can't brick a model slot.
Downloads can be cancelled from the same card; a hard quit leaves no garbage
behind (partial `.part` files are swept at the next launch). Switching models
unloads the old engine only when the new one is ready to take over — if the
new model fails to load, the previous one is restored and the app keeps
working. Optionally paste a HuggingFace access token (Settings → AI Models) —
authenticated downloads are faster and never hit anonymous rate limits. The
token is stored locally and never sent back to the UI, only a `••••1234`
style hint.

Recording is captured as 16 kHz mono WAV in the webview and transcribed locally.

**External offload**: Settings → AI Models lets you point the LLM and/or STT at any
OpenAI-compatible server (Ollama, llama.cpp server, LM Studio, ...). Each has its own
"External … endpoint" section (URL, model, API key, Test) and its own "Use external …
endpoint" switch: save and test the URL, then turn the switch on. While the STT switch
is on, transcription goes to the server instead of Apple Speech or the local whisper
model. (Installs that already had an STT URL saved before the switch existed start with
it on.)

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
- A feedback file also carries your emoji reactions on the idea's sections; they show
  on the original idea attributed to you. Reactions on feedback comments stay local.

The author name shown to others is your profile name (Settings → General).

## Daily reminders

Set a time under Settings → General → Notifications (or toggle it from the tray icon
menu). The app keeps running in the system tray when the window is closed and sends
a system notification once per day at the configured time. Quit fully from the tray
menu.

## Install

Grab the latest build for your OS from the [Releases page](https://github.com/brainstory/desktop-app/releases):

- **macOS** — download the `.dmg` (Apple Silicon only; there is no Intel build). Drag
  Brainstory to Applications and launch it from there.
- **Windows** — run the `Brainstory_*_x64-setup.exe` installer (NSIS).
- **Linux** — install the `.deb`, `.rpm`, or run the `.AppImage`.

**The builds are not code-signed.** There are no developer accounts for signing
certificates, so a warning on first launch is expected:

- **macOS** — the first launch is blocked ("Brainstory" Not Opened / can't be opened
  because Apple cannot check it for malicious software). Since macOS 15 Sequoia,
  right-click → **Open** no longer gets past this. Instead click **Done**, open
  **System Settings → Privacy & Security**, scroll to the message about Brainstory
  and click **Open Anyway**, then confirm (with your password or Touch ID). From then
  on it launches normally. If macOS instead says the app "is damaged and can't be
  opened", the download's quarantine flag is the cause; clear it with
  `xattr -dr com.apple.quarantine /Applications/Brainstory.app` and launch again.
- **Windows** — SmartScreen shows "Windows protected your PC": click **More info**,
  then **Run anyway**.

## Updates

Brainstory checks for updates on startup (at most once every 6 hours) against the
GitHub releases of this repository. When a newer version exists it shows a banner —
nothing is downloaded or installed until you click **Restart to update**, which
downloads the update through Tauri's updater, verifies it against the release signing
key built into the app, installs it and restarts.

To opt out, turn off **Check for updates automatically** in Settings (on by default).
The app then never contacts GitHub, and it warns you that you will not hear about new
versions — including security fixes — so check the
[Releases page](https://github.com/brainstory/desktop-app/releases) yourself now and
then. The app works fully offline after the models are downloaded. Update artifacts
and their checksums are attached to every release (`latest.json` + `SHA256SUMS.txt`).

## Where your data lives

Everything is local:

| What | Where |
| --- | --- |
| Ideas, drafts, daily logs, settings | SQLite database `brainstory.db` in the app data folder: `~/Library/Application Support/ai.brainstory.desktop/` (macOS), `%APPDATA%\ai.brainstory.desktop\` (Windows), `~/.local/share/ai.brainstory.desktop/` (Linux, or `$XDG_DATA_HOME/ai.brainstory.desktop/`) |
| Downloaded models | the HuggingFace hub cache, shared with other HF tooling: `$HF_HUB_CACHE`, else `$HF_HOME/hub`, else `$XDG_CACHE_HOME/huggingface/hub`, else `~/.cache/huggingface/hub/` (also on macOS; `%USERPROFILE%\.cache\huggingface\hub\` on Windows); legacy app-folder copies are migrated there (hash-verified) at first launch |
| HuggingFace token, external API keys | the system keyring under the service name `ai.brainstory.desktop` (accounts `hf_token`, `ext_llm_api_key`, `ext_stt_api_key`) — never in the database, except as a plaintext fallback row where no keyring service is available |
| Logs | rotating `brainstory.log` in the OS log folder: `~/Library/Logs/ai.brainstory.desktop/` (macOS), `%LOCALAPPDATA%\ai.brainstory.desktop\logs\` (Windows), `~/.local/share/ai.brainstory.desktop/logs/` (Linux, or `$XDG_DATA_HOME/...`) |

A corrupted database is not deleted: on next launch it is quarantined as
`brainstory.db.corrupt-<timestamp>` next to the fresh one, so your data is never
silently discarded.

To remove everything Brainstory stored, quit it fully (tray menu → Quit), then:

1. **Models** — delete them first, with **Delete** on each downloaded model under
   **Settings → AI Models**, which removes exactly Brainstory's files. Doing it by hand
   means deleting the matching `models--<org>--<repo>` folders from the hub cache —
   but that cache is shared, and other apps (Python `transformers`, llama.cpp, other
   local-AI tools) may have downloaded or be using the same models, so delete
   deliberately; never wipe the whole cache just for Brainstory.
2. **Database and logs** — delete the app data folder and the log folder above.
3. **Keyring entries** (service `ai.brainstory.desktop`):
   - macOS — in **Keychain Access**, search for `ai.brainstory.desktop` and delete the
     items, or run `security delete-generic-password -s ai.brainstory.desktop -a hf_token`
     (repeat with `-a ext_llm_api_key` and `-a ext_stt_api_key`).
   - Windows — **Credential Manager → Windows Credentials**: remove the generic
     credentials named `hf_token.ai.brainstory.desktop`,
     `ext_llm_api_key.ai.brainstory.desktop` and `ext_stt_api_key.ai.brainstory.desktop`.
   - Linux — in **Passwords and Keys** (seahorse) or your keyring manager, delete the
     entries for `ai.brainstory.desktop`, or run
     `secret-tool clear service ai.brainstory.desktop`.

## Network access

The app is local-first, but it is not network-free. It contacts exactly:

- `huggingface.co` — model downloads (and only when you click Download). Users behind
  the Great Firewall can point downloads at a mirror (e.g. `https://hf-mirror.com`) in
  **Settings → AI Models → HuggingFace download endpoint**, or via the `HF_ENDPOINT`
  environment variable; models already downloaded into a HuggingFace hub cache
  (`~/.cache/huggingface/hub`) are recognized and reused as-is. If you set a
  HuggingFace token, it is sent with every model download to whichever endpoint is
  configured — including a mirror or `HF_ENDPOINT` — so only combine a token with a
  mirror you trust.
- `github.com` — update checks on startup (unless turned off, see [Updates](#updates)),
  and the update download itself when you click **Restart to update**.
- Any **external AI endpoint you configure** yourself (Settings → AI Models).

Model and update downloads follow HTTP redirects, so the actual file transfer may come
from the CDN/storage hosts those services redirect to (HuggingFace's `*.hf.co` /
`*.huggingface.co` file hosts, GitHub's `*.githubusercontent.com` release-asset host,
or whatever a mirror redirects to).

Nothing else. There are no accounts, analytics, or telemetry servers — but "no
servers" here means *no Brainstory servers*, not "no network".

## Manual model tests

Two integration tests need real model files and never run in CI's unit-test job (the
nightly workflow runs the engine smoke test with tiny downloaded models):

```sh
# whisper + llama coexisting in one process, with a tiny (~1 MB) test llama:
WHISPER_MODEL_PATH=/path/to/ggml-tiny.en.bin \
LLM_MODEL_PATH=/path/to/tiny.gguf \
cargo test --test engine_smoke

# prompt quality against a real model:
LLM_MODEL_PATH=/path/to/model.gguf cargo test --test model_prompts

# Apple Speech smoke test (macOS 26 host, terminal needs speech permission):
APPLE_STT_SMOKE=1 cargo test --test apple_stt_smoke
```

## Development

```sh
pnpm install          # installs frontend deps + the tauri CLI

pnpm tauri:dev        # dev: astro dev server + debug build
pnpm tauri:build      # release build + .app + DMG (via scripts/make-dmg.sh,
                      # using plain hdiutil - tauri's own dmg bundler needs
                      # AppleScript control of Finder and fails headless)
pnpm typecheck        # tsc --noEmit + astro check (strict)
pnpm test             # vitest
pnpm lint             # eslint
pnpm vendor:check     # vendored icons/WASM complete, icon names valid
```

The frontend is TypeScript (`.ts` / `.tsx`; `.astro` pages stay `.astro`).
Keep `pnpm typecheck` green alongside tests and lint.

**Dark mode:** the window is pinned to a light theme (`"theme": "Light"` in
`tauri.conf.json`). This is deliberate for now — the accent palette is tuned for
light surfaces — and will change if/when dark variants of the tokens are designed.

Requirements: Rust (the exact version is pinned in `rust-toolchain.toml`; rustup
installs it automatically), Node >= 24, pnpm (the version in `package.json`'s
`packageManager` field), cmake (brew install cmake ninja), macOS 12+.
Building on macOS additionally uses the active Xcode's Swift toolchain; with an
Xcode 26 SDK the Apple Speech engine (macOS 26+) compiles in, and with an older
SDK it is stubbed out with a build warning (the app still runs, whisper handles
transcription). CI enforces the 26 SDK via `REQUIRE_MACOS26_SDK=1` so a stubbed
build can never ship. The env-gated Apple Speech smoke test runs with
`APPLE_STT_SMOKE=1 cargo test --test apple_stt_smoke` on a macOS 26 machine
(synthesizes audio via `say`, needs speech-recognition permission for the
terminal).

Local Windows cross-compile (no Windows machine needed):

```sh
brew install llvm lld makensis cmake ninja
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin
./scripts/build-windows-local.sh   # -> .../bundle/nsis/Brainstory_*_x64-setup.exe
```

The binary is cross-compiled but never executed locally; runtime verification
happens via the nightly/release Windows CI builds.

## Releasing

1. From the repository root (cargo must be on `PATH`), bump every version that must
   agree — `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` + `Cargo.lock`, and
   both `package.json` files:

   ```sh
   node scripts/bump-version.mjs 0.3.0   # semver; a pre-release like 0.3.0-rc.1 works too
   ```

2. Commit the result (e.g. `Bump version to 0.3.0`) and push it to `main`.
3. Tag that commit and push the tag:

   ```sh
   git tag v0.3.0
   git push origin v0.3.0
   ```

4. The **Release** workflow runs the full CI checks, refuses to build if the tag does
   not match the committed versions, builds all three platforms (signing the updater
   artifacts with the updater key; the installers themselves are not code-signed), and
   creates a **draft** GitHub release with the installers, the updater feed
   (`latest.json`), `SHA256SUMS.txt` and build-provenance attestations. Versions with
   a `-` suffix are marked as pre-releases.
5. A human reviews the draft and publishes it. Until then installed apps keep seeing
   the previous release — the updater reads `releases/latest/download/latest.json`,
   which never points at a draft.

## License

GPL-3.0-only — see [LICENSE](LICENSE). This covers the whole app: frontend,
Rust backend, and the prompts under `prompts/`.

# Architecture

A Tauri 2 app: an Astro 7 / React 19 / TypeScript / Tailwind 4 frontend in the
webview, and a Rust backend that owns the database, the local engines, and all I/O.

```
app/frontend/                Astro + React frontend
  src/pages/*.astro          one page per route; each mounts one React root
  src/components/            React components (chat/, idea/, dashboard/, profile/,
                             recording-ui/, global/)
  src/helpers/api/*.ts       thin wrappers around Tauri invoke("<command>", {...})
  src/helpers/*.ts           pure helpers (vitest)
src-tauri/src/
  lib.rs                     Tauri builder, tray, close-to-tray, startup model loader
  db.rs                      SQLite (rusqlite, WAL), migrations via PRAGMA user_version
  models.rs                  model catalog, AiSettings, AppState, engine load/swap, downloader
  llm.rs                     LocalLlm (llama.cpp) + ExternalLlm (OpenAI-compatible SSE)
  stt.rs / stt_apple.rs / apple.rs / voice.rs
                             whisper, Apple Speech, cpal mic capture
  prompts.rs                 include_str! of prompts/*.txt; system/user message builders
  secrets.rs                 OS keychain via `keyring`, DB-row fallback
  reminders.rs               daily reminder loop
  commands/{data,ai,settings,models_cmd,share}.rs   the #[tauri::command] layer
prompts/*.txt                LLM system prompts (GPL, embedded at compile time)
```

## IPC surface

Every frontend `invoke` is a registered command (enforced by the contract test in
`app/frontend/src/helpers/api/contract.test.ts`): data commands
(`get_user`, `get_daily_status`, `get_all_ideas`, `get_idea`, `get_idea_children`,
`create_idea`, `update_idea`, `mark_idea_read`, `delete_idea`, `get_log_questions`,
`submit_log`, `get_survey_fields`, `submit_survey`, `get_notifications`), AI
commands (`transcribe`, `generate_response`, `generate_streaming_response`,
`cancel_generation`, `start_voice_capture`, `stop_voice_capture`), settings
commands (`get_user_settings`, `save_user_settings`, `set_app_presence`,
`get_ai_settings`, `save_ai_settings`, `test_llm_endpoint`, `test_stt_endpoint`),
model commands (`list_models`, `get_runtime_status`, `get_apple_stt_status`,
`get_free_disk_space`, `download_model`, `cancel_download`, `delete_model`,
`activate_model`), and share commands (`export_idea`, `import_share`).

Events (backend -> frontend): `llm-status`, `stt-status` (engine state machine:
`ready | loading | error | missing | external`), and `model-download`
(`progress | done | error | load-error`).

## Engine load/swap flow

`AppState` holds `runtime.llm` / `runtime.stt` behind a mutex plus per-engine
`*_loading` guards (one load at a time; a concurrent activate is refused, not
queued). `load_llm`/`load_stt` drop the previous engine before mmap'ing the new
file (peak memory stays at one model) and roll back to the previous model if the
new one fails. The settings row only records a model as active after the engine
actually loaded. Status transitions are always emitted as events; on external
or missing configurations the resident engine is dropped so the runtime matches
the reported status.

## Database schema and migrations

SQLite in WAL mode, foreign keys enforced. Schema version lives in
`PRAGMA user_version`; each migration step runs in one transaction committed
together with its version bump, and a database from a newer schema version is
refused rather than downgraded. `local_date` columns freeze each activity's
local calendar day at write time (imported rows keep it empty so someone else's
activity never feeds your streak) and are indexed (schema v4).

## Share-file trust model

Share files are plain JSON received out of band; nothing cryptographically ties
them to an author. The import path treats them as untrusted input: file size,
title length, result size and idea type are validated; author names are
sanitized before entering any prompt; imports never count as local activity.
See the comment at the top of `src-tauri/src/commands/share.rs`.

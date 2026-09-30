# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[SemVer](https://semver.org/).

## [Unreleased]

### Fixed

- Download bookkeeping no longer wedges a model as "downloading" when its setup
  fails; both registration maps update under one lock.
- Databases written by a newer app version are refused (with a clear message)
  instead of being silently re-migrated; migrations are transactional and the
  activity-day backfill is self-healing.
- Corruption quarantine moves the real SQLite sidecars (`-wal`/`-shm`).
- Activating a model now reports load failures to the UI; the post-download
  auto-load surfaces a distinct "load error" instead of posing as a failed
  download.
- Daily-intent drafts reset the day's completed flag; a new draft no longer
  shows the day as finished.
- Local whisper honors the configured language (multilingual models); English
  models stay English.
- External endpoint reads are bounded by size and idle time; non-streaming
  replies and servers that ignore `stream: true` work.
- Imported-only libraries render; imports never feed the importer's streak; the
  importer validates file type/size and sanitizes author names against
  prompt injection (as does every prompt boundary).
- Frontend: default chat greeting reachable, unfinished daily intents resume in
  chat, controlled settings switches that can't desync from saved state, no
  unhandled promise rejections, no microphone leak when stopping fails, debounced
  chat autosave, numeric feedback ordering with stable comment identity,
  UTC-correct dates and local-midnight question-of-the-day, uncontrolled timers
  removed, a real input for idea titles with rollback, and many smaller fixes.

### Changed

- Heavy commands (model listing/deletion, settings, Apple Speech status) run off
  the main thread; the signing key is scoped to the build steps; releases are
  test-gated drafts with checksums and build provenance; CI covers Linux and
  Windows, audits dependencies, and enforces formatting.
- Settings are validated (modes, model ids, endpoint URLs) and parsed into enums.
- Prompts: quoted-material rule, no scripted openers (the app sends turn 1), a
  rewritten daily-intent prompt, language-consistent result documents, and a
  stricter JSON feedback contract.

### Added

- Generation cancel button; a shared AI-status store feeding chat and the
  dashboard; accessible switches, tabs, dialogs, live regions and focus handling;
  component test setup (jsdom + testing-library) and a large test expansion on
  both sides; CONTRIBUTING/SECURITY/ARCHITECTURE docs.

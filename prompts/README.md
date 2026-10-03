# Brainstory Prompts (desktop app)

> **Coupled to the frontend:** the interview prompts (`story_interview_*`) describe the first assistant
> message, which the app sends itself rather than the model. They paraphrase the exact opener strings in
> `getFirstPrompt` in `src/helpers/chat.ts` (the feedback opener's three options, the daily
> intent "walk me through your day" question). Change the opener and the matching prompt together.

The Brainstory prompting system, as used by the desktop app.

These are adapted from three sources:

- The original [brainstory/prompts](https://github.com/brainstory/prompts) system messages
  (interview flow, `<t>` / `<oid>` / `<idea>` tag input formats, and the feedback JSON schema
  consumed by the frontend: `oid_heading_text` with `<index>##<heading>` format, `matched_spans`,
  `feedback_text`). The original schema's LLM-chosen `labels` (emoji per item) were dropped: emoji
  reactions are now chosen by people in the app, never guessed by the model.
- The improved rewrites from the Claude Code plugin (`../claude-code-brainstory`): explicit
  no-information/no-affirmation rules, questioning strategies, conversation modes, thinking
  frameworks, thread tracking, and the extended synthesis sections in result documents.

- Patterns from the `socratic-*` skills in tomzorz/congruens (a sibling Socratic-interviewing system):
  a wrap-up signal when answers stop producing new content, per-stance response handling in the feedback
  interview, permission to quote the shared document when asked, and "silence is not agreement" rules for
  the generated documents.

Claude Code specific mechanics (file/session storage, slash commands, frontmatter) were removed,
and the input/output contracts the app relies on were preserved.

## Guidelines

`story_interview_system_message`

Original idea conversation.

`story_interview_context_system_message`

Original idea with context conversation (daily intent flow).

`story_interview_react_system_message`

Feedback idea conversation. The idea document is appended to this system message wrapped in
`<idea author="..." is_current_user="...">...</idea>`.

`story_result_system_message`

Original idea result document.

`feedback_result_system_message`

Unstructured (prose) feedback idea result.

`feedback_json_result_system_message`

Structured JSON feedback result. Stored as the idea's `structured_result` and rendered as
comment cards in the UI.

## Runtime overrides

The prompts are compiled into the binary. At startup the app also looks for
`<app data dir>/prompts/<name>.txt` (for example `story_result_system_message.txt`) and, when found,
uses it instead of the built-in prompt, so a prompt fix can ship without a new release. The app data
dir is the platform's Tauri app data directory (on macOS `~/Library/Application Support/ai.brainstory.desktop/`).

- Only the six file names above are read; anything else in the folder is ignored.
- A file that is empty, not UTF-8, larger than 256 KB or unreadable is skipped with a warning, and
  the built-in prompt is used.
- Overrides are read once at launch (restart the app to pick up changes), and every applied
  override is logged.
- An override replaces the whole prompt, so it must keep the tag contracts above intact.

## License

GPL-3.0-only. See [LICENSE](../LICENSE) at the repo root.

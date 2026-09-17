# Brainstory Prompts (desktop app)

The Brainstory prompting system, as used by the desktop app.

These are adapted from two sources:

- The original [brainstory/prompts](https://github.com/brainstory/prompts) system messages
  (interview flow, `<t>` / `<oid>` / `<idea>` tag input formats, and the feedback JSON schema
  consumed by the frontend: `oid_heading_text` with `<index>##<heading>` format, `matched_spans`,
  `labels` as `[{name, emoji}]`).
- The improved rewrites from the Claude Code plugin (`../claude-code-brainstory`): explicit
  no-information/no-affirmation rules, questioning strategies, conversation modes, thinking
  frameworks, thread tracking, and the extended synthesis sections in result documents.

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

`feedback_result_reflection_system_message`

Reflection evaluation from a transcript and feedback. (Kept from the original set; currently
unused by the app.)

## License

GPL-3.0-only. See [LICENSE](../LICENSE) at the repo root.

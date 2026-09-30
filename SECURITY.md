# Security policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately to the repository maintainers
(open a GitHub security advisory on the repo, or contact the maintainers directly).
Do not open a public issue for security problems. We will respond as quickly as we
can and credit reporters in the release notes.

## Scope notes

- **Update signing.** Release artifacts are signed with the Tauri updater key; the
  public key in `tauri.conf.json` pins what installed clients accept. The private
  key lives only in GitHub Actions secrets and is exposed to the build steps alone.
  The updater public key must never change without a coordinated client release.
- **Local trust model.** All data (ideas, transcripts, settings) is stored locally
  in SQLite; API tokens live in the OS keychain. Share files are untrusted input:
  sizes, types and prompt-injection surfaces are bounded in code, but a crafted
  share file can still choose its author name and share id by design of the
  peer-to-peer flow.
- **Dependencies** are audited in CI (`cargo audit` + `pnpm audit`); GitHub Actions
  are pinned to commit SHAs.

---
status: in progress
priority: P2
complexity: 2
readiness: 100%
agent: Claude Code / claude-opus-5-5
---

# T978. `rtok doctor` warns when Claude Code does not trust the folder

Moved from `roadmap.md` (approved by the creator 2026-10-24).

Claude Code and the desktop Code tab show "This workspace isn't trusted" for a folder whose
trust dialog was never accepted. In that folder they skip the project's hooks and MCP servers,
so rtok silently does nothing.

Done when: `rtok doctor` detects that the current folder, or a worktree outside a trusted
parent, is untrusted by Claude Code. It reports one problem that says rtok hooks and MCP are
inactive there and how to trust the folder: run `claude` in it and accept the dialog, or
accept it in the desktop app. Read-only: doctor never writes the trust flag, and `doctor --fix`
offers no fix for it, since trusting a folder is the user's security decision. A missing or
unreadable Claude config is no problem and no warning (fail open). Only the trust entry is
read; nothing else in the file is parsed beyond what that needs.

Check: tests with a temp home for a trusted folder (no problem), an untrusted one (the warning), a subfolder of a trusted parent (as Claude Code inherits it) and a missing config (no problem); `just check`.

Plan:
1. Confirmed from the vendor docs (`research.md` §48): `~/.claude.json`
   `projects["<path>"].hasTrustDialogAccepted`, keyed on the repository root (the main checkout
   for a worktree) or, outside a repository, the start folder covering its subfolders.
2. `src/doctor/trust.rs`: one `untrusted-folder` problem (agent `claude`, never fixable), read
   through the doctor `Probes` from `[doctor] claude_json`; wired into `checks()` in
   `src/doctor.rs` and rendered by `hooks::render`. No new dependency.
3. Unit tests with an in-memory mock: trusted repo, untrusted repo, linked worktree, trusted
   parent outside a repo, missing or broken config and the home folder.
4. Docs: `docs/claude-code.md` (no `ru`/`uk` copy exists); regenerate `ws.schema.json` and
   `snapshot.gen.ts` for the `kind` doc comment.

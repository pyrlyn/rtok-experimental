---
status: in progress
priority: P2
complexity: 2
readiness: 100%
agent: Claude Code / claude-opus-5-5
---

# T975. Doctor warns about outdated rtok skills and offers to update them

The creator's request. `rtok agents install|update` copies `skills/<name>/` into each host's
skill root, but a copy stays as it was until the next update: after a ketch upgrade the
skills in `~/.claude/skills` and the other roots silently lag the binary (found on
2026-10-23: `rtok-tasks` and `rtok-worktrees` were the 0.17.0-alpha.2 text under alpha.3).
Nothing told the user.

Done: `rtok doctor` lists every installed rtok skill whose copy differs from the one this
binary ships, as a WARN line with the command that updates it. `rtok doctor --fix` gains the
`stale-skills` class: a dry run lists what it would update, a terminal asks once before
writing, `--yes` writes; the update is `agents::skill::sync`, the same copy `agents update`
makes. `--agent <host>` limits it like the hook checks. A missing skill is not reported
(a host may run rtok without skills); a foreign directory is never touched.

Plan:

1. `rtok-agent-sdk`: `SkillCopy::outdated` — present, ours, and `skill_plan` would copy.
2. `agents/skill.rs`: `outdated(cfg)` over every host with a skill root.
3. `doctor.rs`: `Report.stale_skills` and its WARN lines with the hint.
4. `cli/doctor.rs`: `FixClass::StaleSkills`; config fix and skill update run per selection.
5. Unit tests for the SDK decision and the render; `just check`.

Check: `cargo test -p rtok-agent-sdk outdated` and `cargo test --lib doctor` pass; `just check` is green.

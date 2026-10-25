# T933. Done

Current `main` already lets every 2026-10-16 command through; the refusals came from an
installed rtok binary older than T437 (#799). No change to `runs_git`. The exact lines are now
pinned in a test.

**Where the lines came from.** The host refuses a command, so rtok's store never saw the 18
refusals (`calls`/`logs` hold none). They are in the weft session transcript
(`~/.claude/projects/-Users-listepo-GitHub-listepo-apps-weft/21a7f99d-…/subagents/agent-{a10131a73e855515f,ab8377fb4288559b1,a9a9ccf12b6aa3ef6}.jsonl`),
3 worktree-isolated sub-agents, 2026-10-16 21:08-21:09 UTC, 18 refusals of 12 distinct
command lines (`git fetch origin -q && git checkout -B … origin/main …`, `git -C <worktree>
fetch origin -q`, `git status`, `git status --short --branch`, `/usr/bin/git fetch …`,
`/usr/bin/git status | head -3`, `/usr/bin/env git log --oneline -S"@class" | cat; …`).

**Which binary refused.** The `PreToolUse:Bash` hook attachment next to each refusal shows the
rewrite: `rtok run --agent a10131a73e855515f -- 'git fetch origin -q'`. The hook had the
sub-agent id (it embedded it in `--agent`) and still wrapped git, so that binary lacked the
T437 branch (`cx.agent_id().is_some() && runs_git(full)`), which keys off the same id. T437 is
in v0.16.0 and later, not in v0.15.1. Sub-agent git was still being wrapped up to
2026-10-18T00:29Z across all transcripts and never after; v0.17.0-alpha.2 was installed on
2026-10-18 and is the only version left in `~/.ketch/store/rtok`, so the exact older version
on 2026-10-16 cannot be named from the machine (it predates v0.16.0). Claude Code 2.1.299.

**Replay.** `weft_2026_10_16_refused_commands_are_never_wrapped` (`src/plugins/cmd/hook.rs`)
feeds the 12 distinct lines to `pre_tool` with the real sub-agent id `a10131a73e855515f`: all
12 pass through unwrapped. The same lines from the main session (no id, no host guard) keep
their wrap, so compression is unchanged outside sub-agents; the existing
`sub_agent_non_git_commands_stay_wrapped` still pins that non-git sub-agent commands wrap.
Nothing in the hook or in `runs_git` was weakened.

Checked (disk was nearly full, so the full `just check` could not finish: its workspace clippy
hit ENOSPC; the touched crate was checked on its own, CI runs the rest):
`cargo test --lib plugins::cmd::hook` (25 passed), `cargo test --test plan_dir` (2 passed),
`cargo clippy -p rtok --lib --bins -- -D warnings` and `--lib --tests -- -D warnings -A
clippy::disallowed_methods` (clean), `cargo fmt --check` (clean).

Found on the way, not part of this task: `main`'s `Cargo.lock` has `serde-saphyr`/`granit-parser`
2.0.0 and `toml` 0.9 while `Cargo.toml` pins `serde-saphyr = "1.3.0"`, so every unlocked cargo run
rewrites the lock (downgrade to 1.3.0); `--locked` fails. This PR leaves `Cargo.lock` alone.

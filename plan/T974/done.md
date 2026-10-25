# T974. Done

No code change: the failure is already fixed on main.

Every recorded failure (Claude Code session logs, 2026-10-20 15:16Z to 2026-10-21 00:14Z) was the
same assert, `the dump must predate _rtok`. The dump held the developer's own completions
(`_rtok _rustup _grok _zshz _afew`), so the `compinit` that built it had inherited an exported
`FPATH`. T943 (#1191, merged 2026-10-20 22:25Z) cleared the environment only for the
interactive shell. e1bf6354e (#1200, 2026-10-21 00:10Z) clears it for the dump step too, and the
test comment records why. The 00:13–00:14Z failures ran on branches cut before #1200.

Checked:
- `cargo nextest run --test completions_shells` three times on 92cf8915a: 4/4 passed each time
  (zsh 5.9, `/bin/zsh`).
- The last 25 failed CI runs of pyrlyn/rtok (`gh run view --log-failed`): none mention the test.

PR: this branch.

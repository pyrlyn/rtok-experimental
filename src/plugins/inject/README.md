# `inject`

The only door into SessionStart and UserPromptSubmit context. Everything that wants to say
something to the model at those points goes through here, under one budget.

| | |
|---|---|
| Surfaces | SessionStart, UserPromptSubmit hooks |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | on |

## Mechanism

1. Collect `Injection { plugin, text, priority }` from every enabled plugin.
2. Sort by priority (higher first).
3. Emit until `plugins.inject.budget_tokens` (default 800) is reached; drop the rest.
4. Record `Measurement { kind: "inject" }` with what was emitted and what was dropped.

SessionStart output is byte-identical across runs with unchanged state — no timestamps, no
counters — so the host's prompt cache keeps hitting.

Modes (`terse`, `yagni`; aliases `cave`→terse, `pony`→yagni) are markdown files under
`~/.rtok/modes/` (builtins via `include_str!`), ≤ 250 estimated prose tokens each,
enabled with `rtok agents install --mode terse,yagni`, injected once per session at priority 5.
Enriched terse captures caveman-style full-intensity rules (shortest reply, fence-safe
exactness, auto-clarity); yagni captures the ponytail YAGNI ladder. Structured helpers live
in `rtok::modes` (`compress_prose`, `evaluate_ladder`) — native safety nets beyond prompt-only
modes, not wraps of third-party tools (D6). Fixture comparison vs weak caveman-lite /
naive-Minimum baselines (and why we win): `research.md` modes subsection and
`docs/comparison.md` § Prompt-level; re-run with `cargo test --test mode_bench -- --nocapture`.

## Config

```toml
[plugins.inject]
enabled = true
budget_tokens = 800             # per turn, all injections together
modes_dir = "~/.rtok/modes"
modes = []                      # e.g. ["terse", "yagni"]
```

`core.inject_budget_tokens` is the old spelling (moved in T12.1). It is still read and
carried over to `plugins.inject.budget_tokens` when that key is left at its default.

## Tasks

See `roadmap.md` § `inject`. Checks in `plan/`.

T2.4 plugin + budget · T7.1 modes as data.

## Status

Implemented: budgeted SessionStart / UserPromptSubmit injection (T2.4), modes as data (T7.1).

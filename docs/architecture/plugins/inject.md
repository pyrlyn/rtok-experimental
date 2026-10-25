# inject plugin

![inject: hook offerings from every plugin pass one priority-sorted token budget before they reach the model as additional context](inject.svg)

Source: [`inject.d2`](inject.d2). Plugin docs: [README](../../../src/plugins/inject/README.md). Index: [all diagrams](../README.md).

`inject` is the door into SessionStart, PostCompact, UserPromptSubmit and SubagentStart context. The hook
dispatcher collects an `Injection { plugin, text, priority }` from every enabled plugin, then
`inject::apply` emits them under one budget, so nothing reaches the model unbudgeted. The plugin
also contributes its own offerings (the compaction checkpoint, the previous session's handoff and
the configured prompt modes) and writes the checkpoint at `PreCompact`.

## Walkthrough

1. **A hook event fires.** `hooks::inject_event` in `src/hooks/mod.rs` calls `session_start`,
   `prompt_submit` or `subagent_start` on each enabled plugin (a `PostCompact` event is dispatched as
   `session_start` with `source: "compact"`). A panicking plugin is logged and skipped.
2. **Extra offerings join.** The dispatcher adds the agent id line (SessionStart and SubagentStart),
   the claimed-task line (SessionStart and PostCompact) and pending messages (UserPromptSubmit),
   all of which ride the same budget.
3. **`InjectPlugin::session_start` picks its text.** On `compact` it asks `checkpoint::offer` for this
   session's checkpoint; on `startup`, when `[plugins.memory] startup_recall` is on, it asks
   `checkpoint::offer_session` for the project's last session handoff. Either one is priority 9.
4. **Modes are appended.** `modes_text` takes `[plugins.inject] modes`, falling back to
   `[setup] modes`. Each name resolves to `<modes_dir>/<name>.md`, or to the builtin
   (`terse` / `cave`, `yagni` / `pony`, `nudges`, embedded with `include_str!`), is read with a
   bounded `read_mode`, and fitted to the budget. The text is joined into the same `Injection`, whose priority is 5, or 9
   when a checkpoint or handoff text came first. Only `session_start` adds mode text.
5. **`apply` enforces the budget.** `inject::apply` sorts by priority (ties by plugin id), emits
   candidates while `used < budget_tokens`, and for a candidate over the budget keeps a
   `fit_budget` prefix plus a `dropped:<plugin>:<est>` marker. The rest are named by markers only.
6. **A measurement is recorded.** `Measurement { kind: "inject" }` carries the offered and emitted
   bytes and estimates; `ref_id` holds the first dropped marker. With no offerings at all, nothing
   is recorded.
7. **PreCompact saves.** `InjectPlugin::pre_compact` calls `checkpoint::save`, which streams the
   transcript and upserts a `notes` row with kind `checkpoint:<session>`; SessionEnd writes the
   `session:<session>` row that `offer_session` reads later.

## Where it lives

| Box | Code |
| --- | --- |
| `hooks::inject_event` | `src/hooks/mod.rs` (`inject_event`) |
| `InjectPlugin::session_start`, `pre_compact` | `src/plugins/inject/mod.rs` (`impl Plugin for InjectPlugin`) |
| `modes_text`, `read_mode`, `builtin` | `src/plugins/inject/mod.rs` |
| `inject::apply` | `src/plugins/inject/mod.rs` (`apply`) |
| `checkpoint::offer`, `offer_session`, `save` | `src/plugins/checkpoint.rs` |
| builtin modes | `modes/terse.md`, `modes/yagni.md`, `modes/nudges.md` |
| budget key | `crates/rtok-config/src/lib.rs` (`plugins.inject.budget_tokens`, `modes_dir`) |

## Invariants

- The budget is a hard cap: `plugins.inject.budget_tokens` (default 800) bounds what `apply` emits,
  and what is cut is named by a `dropped:` marker. Test: `three_500_budget_800_drops_one_and_is_byte_stable`.
- SessionStart output is byte-stable for unchanged state: no timestamps or counters in the text
  `inject` offers.
- Modes are data in markdown files, not Rust strings; a missing or unreadable file falls back to the
  builtin, and an unknown name is skipped.
- `UserPromptSubmit` never carries mode text (`session_start` is the only place it is added).
- `core.inject_budget_tokens` is a removed alias, accepted from old configs and migrated to
  `plugins.inject.budget_tokens`.

## Related

[Overview](../overview.md) · [Plugin contract](../plugin-contract.md) · [Hook flow](../hook-flow.md) · [memory plugin](memory.md) · [guard plugin](guard.md)

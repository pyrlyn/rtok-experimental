# Skill management

`rtok skills` lists, enables and disables `SKILL.md` skills of the agent rtok runs in.
Claude Code, Cursor and GitHub Copilot are supported; other agents plug in by adding a
row to `Roots::skills_dir` and a value to the `Agent` enum in the contract.

## How it works

Every supported agent discovers a skill as `<skills dir>/<name>/SKILL.md`:

| Agent   | User scope                                     | Project scope        |
|---------|------------------------------------------------|----------------------|
| claude  | `~/.claude/skills`                             | `.claude/skills`     |
| cursor  | `~/.cursor/skills`                             | `.cursor/skills`     |
| copilot | `$COPILOT_HOME/skills` (default `~/.copilot`)  | `.github/skills`     |

Disabling moves `<skills dir>/<name>` to the sibling `<skills dir>-disabled/<name>`
(for example `~/.claude/skills-disabled/<name>`), a directory no agent scans. Enabling
moves it back. Files are never edited or deleted, so every change is reversible with
the opposite command or a plain `mv`.

The agent comes from `--agent`, else `RTOK_AGENT`, else the variables each agent sets
for the commands it runs: `CLAUDECODE` (Claude Code), `CURSOR_AGENT` / `CURSOR_TRACE_ID`
(Cursor), `COPILOT_CLI` / `COPILOT_AGENT_SESSION_ID` (Copilot). Plugins and hooks can
call the binary without flags from inside the agent.

### Contract

The JSON shapes are defined once in [`openapi/skills.yaml`](../openapi/skills.yaml)
(OpenAPI 3.0). The Rust models in `src/skills/api/` are generated with openapi-generator
(rust generator, models only) and committed, so building rtok needs no Java:

```sh
python3 tools/gen_openapi.py          # regenerate after editing the spec
python3 tools/gen_openapi.py --check  # fail if committed models are stale
```

`--json` output of every subcommand is the `SkillList` / `ToggleResult` schema, so a
TypeScript or shell plugin can generate its own client from the same spec.

## Enable

```sh
rtok skills enable my-skill                 # agent detected from the environment
rtok skills enable my-skill --agent cursor --scope project
```

## Disable

```sh
rtok skills disable my-skill --agent claude
rtok skills disable my-skill --dry-run      # print the move, change nothing
```

If a name exists in both user and project scope, pass `--scope`.

## List

```sh
rtok skills list --agent copilot
rtok skills list --json
```

## Size

Every skill carries `size_bytes` (contract field, total bytes of all files in its
directory). Display rule: under 1000 bytes as `999 B`, from 1000 bytes in KB
(1 KB = 1000 bytes, one decimal: `1.0 KB`). Web UIs put the exact byte count in the
size cell's `title` (hover). The terminal has no hover, so `rtok skills list --bytes`
adds an exact-bytes column and `--json` always includes `size_bytes`.

## Test

Each part is tested in its own language:

| Part | Runner | Command |
|------|--------|---------|
| Core logic, all agents x scopes, size formatting (999/1000/1001) | cargo | `cargo test --lib skills::` |
| CLI end to end, fake HOME per agent | cargo | `cargo test --test skills_cli` |
| Plugin launchers (`hook.sh`, `mcp.sh`) + `rtok skills` from sh | POSIX sh | `RTOK_BIN=target/debug/rtok sh tests/sh/plugin_scripts_test.sh` |
| JSON contract as TS plugins read it | vitest | `RTOK_BIN=target/debug/rtok vitest run` |
| Generator / committed models | pytest | `uvx pytest tools/tests/test_gen_openapi.py` |

Manual: `rtok skills disable <name> --agent claude`, start a new Claude Code session,
check `/skills` no longer shows it, then `rtok skills enable <name>`.

Manual: `rtok skills disable <name> --agent claude`, start a new Claude Code session,
check `/skills` no longer shows it, then `rtok skills enable <name>`.

## Limitations

- Agents read skills at session start; a running session may keep a skill until restart.
- Only directory skills with `SKILL.md` are managed. Loose `*.md` files, skills shipped
  inside agent plugins (for example Claude Code plugin skills) and Cursor rules are not.
- Project scope is the given `--project` or the current directory, not the git root.
- Agent detection relies on environment variables the agents set today; Copilot's are
  the least certain, so pass `--agent` or `RTOK_AGENT` from scripts.
- Cursor and Copilot also read `.claude/skills` for compatibility; a skill disabled for
  `claude` there disappears for them too.
- Symlinked skills are moved as links; `<dir>-disabled` sits at the same depth, so
  relative links keep resolving.
- There is no web skills page yet; the size/hover rule is part of the contract for when one lands.
- No HTTP server implements the OpenAPI paths yet; the spec is the shared data contract
  for the CLI and plugins.

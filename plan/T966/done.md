rtok found a host app in several drifting ways: `agents::app_path`/`present` (bundle, then PATH), `find_on_path(<bin>)` checks in the host modules, `host_program(bin)` for `run_cli` and `app_version`, `restart.rs` expanding `apps[]` itself, and `doctor::detected_hosts`, which looked at config folders instead of the app.

`src/agents/locate.rs` is now the one lookup: `locate(cfg, host, variant)` tries the user's `[setup.paths] <host>` path, then the app bundles, then PATH. `present`, `programs`/`cli_program`, `cli`, `app_paths` and `absent` are built on it, and every caller above (install/update/info/list, junk, mcp ping, model rows, restart, doctor) uses them. `find_on_path` stays the PATH primitive for non-agent programs.

The key is a flat map, `[setup.paths] claude = "~/.local/bin/claude"`, not the per-variant table the plan named: the path itself picks the variant (an `.app` or an app file name is the desktop app, a binary is the CLI). The project layer already rejects `setup.*`; the trust test now covers `setup.paths`, since rtok runs that program.

`agents install|update|info <host>` on a terminal, when no wanted variant is found, asks for the path, checks it (exists, executable, a folder only as a desktop `.app`), shows agent, path and config files and asks `Is this correct? [y/N]`. Yes runs the command with it, no asks again, empty stops. A second question saves it through `config set` (skipped on `--dry-run`). Pipes, `--json`, hooks, MCP, `list` and `doctor` never ask; `install` fails and `update` warns with `rtok config set setup.paths.<host> <path>`.

Check: `agents::locate::tests` (lookup order, user path per variant, restart, path checks, escapes, empty/EOF, no-then-yes, save only on yes, dry-run, reload from the file, no terminal); trycmd `cli` (`config-init`, and `report-md` now in the host sandbox because doctor sees real apps); `just check` green.

Status: done 2026-10-23
Model: Claude Code / claude-opus-5-5

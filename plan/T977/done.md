Done: `rtok doctor --project [PATH]` adds a `project <root>` section for the git checkout holding
PATH (default: the working directory): languages by file count with the language server rtok
would start, whether it is installed and the install command when it is not, a note for a
missing root marker (C: how to generate `compile_commands.json`) or a tags-only language; the
graph index (registered, state, files, health score with reasons and fixes, backend and the mode
that served it, next commands); the `AGENTS.md` / `CLAUDE.md` audit scoped to that root; and the
catalogue plugins left off there with the file that may turn each on. Every saving figure comes
from the user's `Measurement` rows of the last 30 days (`measurement_saved_by_project`); with no
rows the line reads `no measurements yet` and shows no number. `--json` carries the section as
`Report.project`; the dashboard fills it for the checkout rtok runs in and the doctor page shows
it as the `project` panel (story `DoctorProjectQuiet`, `?sample` data with one installed server,
one missing, a tags-only language, an unhealthy index and plugins with and without rows).
`docs/lsp.md` has the install column and a `doctor --project` section, `docs/commands.md` a row
(en, ru, uk).

PRs: #1283 (backend, CLI flag, schema), #1286 (trycmd goldens, `cargo fmt`, clippy on Rust
1.99.1: `fetch_update` -> `try_update`, `redundant_field_names` allowed in rtok-store for the
`QueryableByName` derive), #1288 (web panel, docs, this report).

Checked: `cli_trycmd`, `commands_e2e` (`doctor_project_refuses_a_missing_path`,
`doctor_project_reports_the_language_server_from_path`), `config_coverage`, `surface_parity`,
`docs_structure`, `host_docs` pass; `just lint` passes; full workspace nextest with
`--no-fail-fast` minus the two FSEvents watcher tests that fail on a clean `main` on this host
(see T976): 4272 of 4275 passed, the 3 left are `graph_lsp_gate lsp_backend_outlines_dart_main`
(host-only, as in T976) and two load flakes that pass alone (`hook_deadline
rtok_hook_answers_when_the_host_never_closes_stdin`, `cli_snapshots_ignore_ambient_rust_log`).
Web: `just spa-test` 505/505, `just js`, `just spa-typecheck`, `just spa-stories` 261/261,
`just spa-e2e` 45/45; the panel was checked by eye in `?sample` at desktop and 375 px width (no
horizontal scroll).

---
status: in progress
priority: P2
complexity: 4
readiness: 45%
agent: Claude Code / claude-opus-5-5
---

# T977. `rtok doctor --project`: per-project setup advice with savings

`rtok doctor` reports on rtok as a whole and ignores where it runs. A user who opens a new
project has no way to learn which language servers rtok could use there, whether the graph
index exists, how heavy the project's instruction files are, or which plugins left off would
save tokens on it, and how much.

Done when `rtok doctor --project [PATH]` (PATH defaults to the working directory; its git
checkout root is analysed) adds a project section to the text and `--json` report, and the
TUI doctor tab and the web Doctor page show the same section for the project they run in.
The section has four parts, each with a concrete next step:

- **Languages and LSP**: languages found by marker files and file extensions (gitignore-aware,
  bounded walk), the server rtok spawns for each (`lsp::LANGS`), whether it is on `PATH`, and
  the install command; a language rtok has no server for says "tags only"; C without
  `compile_commands.json` says how to generate it.
- **Graph index**: registered, indexed, health score and its reasons, the `backend` in effect,
  and the command or config change that fixes each finding.
- **Instructions**: the project's `AGENTS.md` / `CLAUDE.md` audit (tokens, over-budget warn),
  scoped to that root.
- **Plugins**: catalogue plugins off for this project, with where to turn each on
  (`.rtok.toml`, or user config for keys `.rtok.toml` may not set).

Savings are approximate and come only from the user's own `Measurement` rows: the average
saving per call of that plugin/kind, times this project's calls in the window when sessions
map to it. With no rows the part says "no measurements yet" and shows no number.

Check: `just check`, `just spa-test`, `just spa-stories`; unit tests with fixture projects and
a fake `PATH` (no real language server, no real agent); `rtok doctor --project` on this
checkout shows Rust with rust-analyzer and the graph state.

## Execution plan

1. `crates/rtok-store`: `measurement_saved_by_project(since)` beside
   `measurement_saved_by_host` (group by session, map session to `sessions.project` in Rust);
   per plugin/kind count and saved totals. Diesel only.
2. `crates/rtok-config`: load a config for a given directory (expose the existing
   `assemble(.., cwd, ..)` path) and a public `project_allowed(key)` over
   `project_restricted`, so advice names the right file.
3. `src/plugins/graph/lsp.rs`: an install hint per server next to `LANGS` (one table) and
   the extension-to-language list used for counting; widen `projects::row` to `pub(crate)`.
4. `src/doctor/project.rs`: `ProjectReport` and `audit(cfg, root)`; reuse `graph::walk`
   (`ignore`), `lsp::probe`, `projects::row` / `health::score`, `instruction_audit` (take a
   root instead of `current_dir`), `CATALOGUE` + `plugin_enabled`. File-count cap and time
   budget; a cut walk says so.
5. `src/doctor.rs`: `Report.project: Option<ProjectReport>`, a `project` section in
   `render()` (TUI tab gets it through `to_text`); `src/cli/doctor.rs`: `--project [PATH]`
   (conflicts with `--fix`); `doctor_for_snapshot` fills it for the process's own checkout.
6. Schema bless (`committed_schema_is_current`), `npm --prefix web run gen:api`, a Project
   panel in `web/src/pages/Doctor.tsx` with a story and a Vitest case; `?sample` data.
7. Docs: doctor and `docs/lsp.md` install table (en, ru, uk), trycmd help and completion
   goldens, `toolchain.md` unchanged (no new dependency).

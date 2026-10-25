# `rtok skills audit`

## What it is

`rtok skills audit [PATH] [--json]` lists every agent skill this machine will load for a
project and gives each one a percentage: how likely it is to be used in that project. It is a
static analysis. No model is called, nothing leaves the machine, and the same project with the
same skills always gets the same numbers.

## Why it is needed

Every listed skill rides in the system prompt of every request: its name, path and
`description:` (`rtok doctor` prices that listing). A machine collects skills from many places:
`~/.claude/skills`, plugins, `~/.agents/skills`, a project's `.claude/skills`. Many of them can
never apply to the project at hand, such as .NET build skills in a Rust repository or Figma
skills in a CLI. The audit names those, so you can disable a plugin or move a skill to the
projects that need it. `rtok doctor` says what the listing costs; `rtok skills audit` says which
part of it this project is likely to use.

## How it works

1. **Skills.** The same discovery as the skills section of `rtok doctor`
   (`doctor::found_skills`): user roots under `$HOME` (`.claude/skills`, `.agents/skills`,
   `.codex/skills`, `.cursor/skills`, `.gemini/skills`, `.copilot/skills`), the project roots
   under `PATH` (`.claude/skills`, `.agents/skills`), and the skill dirs of every enabled Claude
   Code plugin (`installed_plugins.json`). `name:` and `description:` come from each
   `SKILL.md` frontmatter (folded `>` and `|` blocks included). A plugin's id is part of the
   skill text, since `dotnet-diag@…` names its stack.
2. **Project profile.** A gitignore-aware walk of `PATH` (the `ignore` crate, the same walk as
   `search` and `tree`):
   - **languages** by file extension, with each language's share of the source files;
   - **manifests**: `Cargo.toml`, `package.json`, `pyproject.toml`, `requirements.txt`,
     `setup.py`, `go.mod`, `Package.swift`, `pubspec.yaml`, `Gemfile`, `composer.json`,
     Gradle, Maven, CMake, Docker files, plus their dependency names and the project's own
     package name;
   - **imports**: the first 200 lines of up to 2000 Rust, Python, TypeScript/JavaScript and Go
     files, mapped to known technologies (`import React` → react);
   - **tests**: `tests/`, `__tests__/`, `*_test.go`, `test_*.py`, `*.test.ts`, …;
   - **CI**: `.github/workflows`, `.gitlab-ci.yml`, `Jenkinsfile`, …
3. **Matching.** A fixed vocabulary maps words to technology keys (`cargo`, `clippy` → rust;
   `msbuild`, `nuget`, `.net` → csharp; `pytest` → testing). A skill's keys are the vocabulary
   words in its name, description and plugin id, plus the project's dependency names it
   mentions.
4. **Score.** Each project key has a strength from 0 to 1: a manifest or CI file gives 1, a
   language gives `0.3 + 2.8 × share` (capped at 1). Then:
   - the skill names a specific technology (a language, framework or tool): only those count,
     `5 + 90 × Σ strength(present) / named`. When it names a technology, a dependency that is
     also an English word (`tracing`, `regex`) is ignored; a dependency named alone counts 0.7,
     the project's own package name counts in full;
   - it names only general things (CI, GitHub, git, tests, docs, Docker): `50 + 40 × share`;
   - it names nothing: 50%, no evidence either way.

   Rows are sorted by percentage, then name. `has …` and `lacks …` show the evidence.

## Output

On a fixture project (Rust, `tests/`, a GitHub workflow) with four skills:

```text
$ rtok skills audit tests/trycmd/input/skills-audit/project
skills 4 · languages: rust 100% · signals: ci, github, rust, testing
  95%  rust-crates   user      has rust
  90%  ci-check      user      has ci, github
  50%  write-emails  user      generic: names no technology
   5%  figma-use     user      lacks figma
```

On rtok itself, on a machine with the .NET agent-skills plugins installed (cut):

```text
$ rtok skills audit .
skills 76 · languages: rust 63%, typescript 27%, python 7%, javascript 1% · signals: ci, css, docker, git, github, html, javascript, markdown, python, react, rust, sql, tailwind, testing, tokio, typescript
  95%  project-plan                     user      has rtok
  95%  rtok                             user      has rtok
  95%  rtok-tasks                       user      has rtok
  95%  rtok-worktrees                   user      has rtok
  90%  dogfood                          project   has testing
  90%  dry-refactoring                  user      has ci
  90%  dry-refactoring                  project   has ci
  90%  surgical-patch                   user      has testing
  90%  understand-diff                  user      has git, github
  90%  worktrees                        user      has git, github
  68%  core                             project   has browser
  68%  penpot                           plugin:penpotx@penpotx  has browser
  50%  build-perf-diagnostics           plugin:dotnet-msbuild@dotnet-agent-skills  has javascript; lacks csharp
  50%  codebase-memory                  user      generic: names no technology
  50%  create-datadriven-aspnetcore     plugin:dotnet-data@dotnet-agent-skills  has css; lacks csharp
  50%  csharp-scripts                   plugin:dotnet-advanced@dotnet-agent-skills  has python; lacks csharp
  50%  eval-performance                 plugin:dotnet-msbuild@dotnet-agent-skills  has javascript; lacks csharp
  50%  figma-use                        user      has javascript; lacks figma
  50%  find-skills                      user      generic: names no technology
  50%  investigate-first                user      generic: names no technology
  50%  jscpd                            user      generic: names no technology
  50%  lean-build                       user      generic: names no technology
  50%  migration                        user      generic: names no technology
  50%  optimizing-ef-core-queries       plugin:dotnet-data@dotnet-agent-skills  has sql; lacks csharp
  50%  safe-refactor                    user      generic: names no technology
  50%  technology-selection             plugin:dotnet-ai@dotnet-agent-skills  has python; lacks csharp
  50%  understand                       user      generic: names no technology
  50%  understand-chat                  user      generic: names no technology
  50%  understand-dashboard             user      generic: names no technology
  50%  understand-domain                user      generic: names no technology
  50%  understand-explain               user      generic: names no technology
  50%  understand-knowledge             user      generic: names no technology
  50%  understand-onboard               user      generic: names no technology
  50%  verify-and-stop                  user      generic: names no technology
  28%  binlog-generation                plugin:dotnet-msbuild@dotnet-agent-skills  has javascript; lacks c, csharp, java
  28%  msbuild-modernization            plugin:dotnet-msbuild@dotnet-agent-skills  has javascript; lacks c, csharp, java
   5%  agent-browser                    project   lacks figma
   5%  analyzing-dotnet-performance     plugin:dotnet-diag@dotnet-agent-skills  lacks csharp
   5%  android-tombstone-symbolication  plugin:dotnet-diag@dotnet-agent-skills  lacks csharp, java, kotlin, swift
```

`--json` prints the same report: `root`, `languages` (files per language), `signals` and
`skills` (`name`, `source`, `percent`, `matched`, `missing`).

## Enable and disable

The command is part of every build and has no config: it reads files and changes nothing. Run
it when you want it; there is nothing to turn off. To act on its result, remove or move a skill
directory, or disable the plugin that ships it (`/plugin` in Claude Code).

## Test

- `cargo test --lib skills_audit`: scoring, frontmatter, imports and the profile walk.
- `cargo test --test cli_trycmd`: `tests/trycmd/skills-audit.toml` runs the command against
  the fixture in `tests/trycmd/input/skills-audit/` with `HOME` pointed at its skills.

## Limitations

- Word matching, not understanding: a skill whose description names no technology gets 50%,
  and a word used in another sense (`node`, `branch`, `deck`) can count as a match.
- The vocabulary is fixed in `src/skills_audit.rs`; a technology outside it only counts
  through a dependency name.
- Imports are a lexical scan of the first 200 lines of up to 2000 files; local modules are not
  told apart from packages, so imports only map to known technologies.
- Monorepos are profiled as one project: a Python script in a Rust repo raises Python skills a
  little, in proportion to its share.
- The percentage is a heuristic likelihood, not a measured rate. `rtok doctor` shows measured
  invocations from transcripts; the two are not merged yet.

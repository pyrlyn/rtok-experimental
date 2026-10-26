# Development

The local gate, test selection and manual packaging. The README smoke block that `just readme-check` runs stays in [README.md](../README.md#development).

## Development

```bash
just check
just example
just readme-check
just dist-plan
```

`just check` is the gate for what the change touches: it diffs against the merge base with
`main` (committed and uncommitted) and runs format, lint, the copy-paste, JS and Python checks
and the tests only for that diff. A docs-only change runs no cargo tests; a change to
`Cargo.toml`, `build.rs`, `justfile`, `crates/` or `config/` runs everything. `just full-check`
is the whole gate regardless of the diff, and CI runs the whole gate too.

The selection is `just test-changed`, which also works on its own against any revision. It
builds and runs only the test targets the diff can reach, which is what makes the loop short:
all integration tests are one binary (`it`, `tests/it/main.rs`, one module per file), so
the selection is a filter on module names and `it` is not linked at all when nothing reaches
it. Profiles, split debug info and `target/` size: [build-profiles.md](build-profiles.md). The unit-test binary is trimmed the same way, which keeps the slow
TUI tests out of an unrelated edit. It picks targets by name, so it can miss a test that
exercises a module without naming it — run `just full-check` when that matters.

Packaging for npm, PyPI and crates.io is manual and local; no workflow publishes. Build and
try the npm package for this machine without touching a registry:

```bash
just npm-build                                   # target/npm/dist/rtok-cli-*.tgz
tmp=$(mktemp -d) && cd "$tmp" && npm init -y >/dev/null
npm i <repo>/target/npm/dist/rtok-cli-*.tgz      # rtok-cli + the platform tarball
npx rtok --version
```

`just pypi-build` does the same for a wheel (`uv venv && uv pip install target/pypi/dist/*.whl`),
and every `*-publish` recipe takes `--dry-run`. The full steps are in
[docs/release.md](release.md#npm-pypi-and-cratesio).

The README smoke examples below are executed by `just readme-check`.

```bash
# check
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
rtok() {
  RTOK_HOME="$tmp/home" \
  RTOK_STATS_TRANSCRIPTS_DIR="$tmp/transcripts" \
  RTOK_SETUP_CLAUDE_SETTINGS_PATH="$tmp/settings.json" \
  mise exec -- cargo run -q -- "$@"
}
rtok --version
rtok config init
rtok config validate
rtok agents install claude --dry-run
rtok stats --since 1h
rtok plugins
rtok proxy --dry-run
rtok otel status
```

Read [`plan/README.md`](../plan/README.md) for how tasks are kept (one folder per task under
`plan/`, closed ones carry a `done.md`), and [`docs/plugin-authoring.md`](plugin-authoring.md) to build an
external plugin.

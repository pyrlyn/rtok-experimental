# T930. Done

The PreToolUse(Read) gate no longer denies a large native Read that `mcp__rtok__read` cannot
serve.

- **Gate.** `src/plugins/read/hook.rs` lets the native Read through (after the size check) when
  the path has an image or PDF extension, or when `plugins::read::resolve` refuses it. That is
  the function MCP `read` calls for its root guard, with the process cwd and
  `plugins.read.allow_paths`, so scratchpad logs, `tasks/*.output`, `tool-results` and
  `~/.cargo/registry` pass and the two sides cannot drift. The hook does not enable the extra
  worktree roots (`roots::enable`, which spawns git), so a file in a sibling worktree counts as
  outside and passes natively; the hook keeps its 10 ms budget. No existing image detection
  existed; the extension list is a small const next to the check.
- **Text.** The deny already names `mcp__rtok__read` plus the `ToolSearch select:` line on
  Claude (T384, `hook.host` defaults to `claude`). `rtok read` remains only for a host whose
  tool naming is unknown, unchanged.
- **Docs.** `src/plugins/read/README.md` and `AGENTS.md` state the new exemption.
- **Tests.** `a_file_rtok_read_cannot_serve_passes_native`: a large in-project file stays
  denied; a large file outside the roots and `.png`/`.PNG`/`.pdf` files pass. The other hook
  tests keep their temp-dir files reachable through `allow_paths`.

Checked: `cargo test --lib plugins::read`, `cargo fmt --check`, `cargo clippy --lib --tests
-p rtok -- -D warnings`. A full `just check` ran out of disk (other sessions' builds); the PR's
CI runs the whole gate.

# T963 combined validation

Combined branch: `codex/t963-combine-security-fixes`, based on fresh `origin/main`
`b7183a94105bf3f3a49d62cec6c48eb8fc45d430`.

T960 and T961 were cherry-picked in order. The only conflict was the A10/A11 status rows
in `plan/README.md`; both fixes were retained, together with A13. No production source
conflict required manual edits. Source worktrees were left untouched.

The task reports now reference the integrated code commits, and T960's historical A10
deferral points to T961's subsequent fix. No new behavior or dependencies were introduced.

Combined `just check`: **passed, exit 0**, selecting `full-check`. Commands unset inherited `RUST_LOG` and use
`CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2`.
Platform: macOS arm64.

- Workspace nextest: **4,196 passed, 8 skipped**. Run ID:
  `5c8180ba-c37f-40cc-b3dd-d0abc2f2c400`.
- Existing tests `rtok::module_graph::production_modules_are_a_layered_dag` and
  `rtok::proxy_bench::proxy_session_saves_over_the_floor` received `LEAK` markers;
  nextest counted both as passed and returned success.
- Python: **43 passed**.
- Formatting, jscpd, JS/TS lint/format and all three Clippy invocations passed.
- Minimal production build (`--no-default-features --features measure`) passed with
  unused-import/dead-code warnings in unchanged files.
- `ptools swarfr` skipped its check because `swarfr` is unavailable.
- Combined diff check and conflict-marker scan passed; no unmerged paths remain.
- Source and test files match their respective T960/T961 source branches exactly.

No Linux/Windows execution or dependency-advisory scan was performed locally. The Linux-only
FIFO fixture was not executed on macOS. Publication targets canonical `pyrlyn/rtok`:
GitHub resolves the configured origin's old `listepo/rtok` URL to the same repository ID.

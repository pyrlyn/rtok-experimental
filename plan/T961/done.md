# T961. Restrict untrusted project configuration (A10)

Fixed A10 by filtering sensitive keys from automatic checkout TOML and dotenv providers
before merging. Projects cannot redirect telemetry, API endpoints, embedding credentials,
extractor commands, read roots or protected storage/host settings. Both sources use one
policy, including legacy aliases. Rejected dotenv keys cannot shadow trusted home values.

Warnings name the file and key without printing rejected values. Source diagnostics show
the retained safe value as `ignored (project)`. Trusted legacy config, environment, flags,
explicit config files, safe project settings and linked-worktree discovery retain precedence.
No dependencies or schema fields were added. See the config guides for settings that must
move from project files to trusted sources.

Commit: `f94a7a21f` (`fix: T961 restrict sensitive project configuration`).

Check: three new regression tests, all 98 config tests and 2 plan-layout tests passed;
normalized `just check` selected full-check and exited 0: 4,191 Rust tests and 43 Python
tests passed, 8 skipped. One existing test was marked leaky; swarfr was unavailable.
Clippy, formatting, duplication checks and minimal production build passed. Exact commands,
earlier failures, compatibility and platform limits: [validation](validation.md).

Status: done 2026-10-22

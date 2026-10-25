# T960. HTTP archive isolation and resilient log rotation

Fixed A11: HTTP expansion now accepts only archive IDs granted by the running server,
including prefix resolution. Identical content archived by another session remains usable
after this server archives it. Restarting resets grants; CLI and stdio keep their behavior.
Commit: `4f6fe6ccc` (`fix: T960 restrict HTTP expansion to server archives`).

Fixed A13: rotation uses a private sibling lock, rejects Unix symlinks and acquires the
lock without blocking. A busy lock or failed rotation no longer discards the incoming line.
Reused the existing rustix version and consolidated runtime archive writes in one method.
Commit: `9fc3f43de` (`fix: T960 preserve logging when rotation is unavailable`).

Check: regression tests failed before the changes and passed afterward; normalized
`env -u RUST_LOG mise exec -- just check` exited 0 (4,193 Rust tests and 43 Python tests
passed, 8 skipped). One test was marked leaky; swarfr was unavailable. See
[validation](validation.md) for exact commands, environment failures and platform limits.

The [review](review.md) documents compatibility, remaining audit items and three feature
proposals. A10 was deferred in T960 and subsequently fixed in T961; this task does not claim
a complete security audit.

Status: done 2026-10-22

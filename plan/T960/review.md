# T960 audit review

Scope: current `origin/main` at `b7183a941`, HTTP MCP archive access and rotating logs.
This is a focused source review with regression tests, not a complete security certification
or a dependency-advisory scan. Findings A1–A9 already have fixes in the baseline.

Primary sources: [HTTP transport](../../src/mcp/http.rs),
[archive resolution](../../src/expand.rs), [runtime capabilities](../../src/plugin.rs),
[rotating log](../../crates/rtok-log/src/lib.rs), and
[configuration layers](../../crates/rtok-config/src/layers.rs), reviewed against the baseline
above and this task's diff. Regression cases live alongside the changed code.

## Confirmed defects

- **A11, medium: cross-session archive disclosure.** `http_server` used unrestricted
  `expand::resolve_id`, which queried every archive and accepted full IDs without scoping.
  The regression returned another session's private payload before the fix. HTTP now resolves
  only IDs successfully archived through its own runtime; unauthorized and unknown IDs have
  the same response, and ambiguous prefixes reveal no candidate IDs. A fresh UUID session
  prevents a reused process ID from reviving old read-cache pointers. CLI and stdio retain
  global expansion. All clients sharing one HTTP server/token still share its capabilities.
- **A13, low: rotation coordination could suppress logging.** A predictable lock in the
  shared temporary directory could be precreated by another account. Blocking lock acquisition
  could also stall a writer, and a rotation I/O error discarded the incoming line. The new
  sibling lock uses owner-only creation and no-follow opening on Unix, acquisition is
  nonblocking, and rotation errors preserve appends. Log size may temporarily exceed the
  configured cap when rotation is unavailable. Existing loose log modes (A14) are unchanged.

## Refactoring and reuse

`Runtime::put_archive` now delegates to `put_archive_for`, so successful writes and HTTP
capability grants have one implementation. Expansion still uses the existing rendering,
range validation, accounting and store code. No schema migration is necessary. A session
column filter was rejected because content-addressed archives retain the first writer:
later HTTP writes of identical bytes must remain expandable.

Checked the existing archive capabilities, store prefix lookup, `fs-kit`, `path-gates`, and
the shared crate inventory. None provides this runtime-specific grant policy. Reused
standard `BTreeSet` prefix ranges, standard file locks and the project's existing `rustix`
version for Unix `O_NOFOLLOW`; no new crate version or custom system-call wrapper.

## Next priorities and feature proposals

1. **Project trust and an egress preview** (A10, existing proposal): before allowing a
   checkout's configuration to change endpoints or execute an extractor, show the requested
   destinations and actions and record explicit trust by canonical project. The high-severity
   project-config issue remains outside this patch; it needs coordinated config-layer work.
2. **A security view in `doctor`** (I-S1/A14, existing proposals): show network exposure,
   authentication and legacy file permissions together, with a concrete dry-run of repairs.
3. **Rotation health reporting** (new proposal): expose skipped/failed rotations and current
   log size in diagnostics without copying log contents. This makes best-effort size limits
   visible before a persistently unavailable rotation fills storage.

These are proposals only. A12 (WASM resource limits), A15 (aggregate proxy buffering), and
A16 (telemetry content defaults) also remain open in the existing audit. No savings claim,
benchmark result, external service change, or feature implementation is part of this task.

reviewer: openai/prism-ml/bonsai-27b



`crates/rtok-store/src/tests_store.rs` (test `migration_0052_drops_observe_rows_only`): **Medium**
The test compares the result of `measurement_totals()` directly to a hardcoded vector, but SQLite does not guarantee row order without an explicit `ORDER BY` clause. This will cause non-deterministic test failures across different SQLite versions or when rows are inserted in varying orders. Add `ORDER BY plugin, kind` to the underlying query or sort the collected results in Rust before assertion.

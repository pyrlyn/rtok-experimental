reviewer: openai/prism-ml/bonsai-27b



`crates/rtok-store/src/tests_store.rs:245`: **High** - Invalid Diesel syntax in `.values()`. Calling `column.eq(value)` returns a predicate, which cannot be used inside `.values()`; it should simply pass the literal values or simple expressions (e.g., `"s"`, `plugin`). This will cause a compilation error.

No issues found.

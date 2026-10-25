reviewer: openai/prism-ml/bonsai-27b



`crates/rtok-store/src/tests_store.rs:238` | low | The test asserts equality on an unsorted `Vec`, which may flake if SQLite returns rows in a different order. Consider sorting the result before comparison or using `assert_eq!(left.sort_by(|a, b| a.0.cmp(&b.0)), ...)` to ensure deterministic behavior across environments.

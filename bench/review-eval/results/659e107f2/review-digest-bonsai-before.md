reviewer: openai/prism-ml/bonsai-27b



**src/tasks/plan_dir.rs::find: High**
The method attempts to destructure the return value of `scan()` as `Scan { found, skipped }`, but `plan_dir::scan` returns a tuple `(Vec<Task>, Vec<TaskId>)`. Additionally, `skipped` contains `TaskId`s, not `Skipped` structs with an `.error` field. This will cause a compilation error. Fix: change to `let (found, skipped) = self.scan(&dir)?;` and replace the check with `if skipped.contains(id) { return Err(anyhow::bail!("task folder {} does not parse", id)); }` (or re-parse the folder to preserve the original parse error).

**No other issues found.** The rest of the changes correctly adapt `disk.rs` and `plan_dir.rs` to skip unparseable tasks while preserving their IDs for allocation, maintain trait contracts via `list`/`get`/`max_id`, and include comprehensive test updates. The `warn_skipped` deduplication logic is thread-safe and functionally sound.

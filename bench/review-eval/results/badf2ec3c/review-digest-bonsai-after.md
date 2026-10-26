reviewer: openai/prism-ml/bonsai-27b



**tests/latency.rs**: High severity. The loop `for i in 0..N` and subsequent assertions reference an undefined variable `N`. Based on the doc comment (`n=200 interleaved`), add `const N: usize = 200;` at the top of the function to fix the compilation error.

**No other issues found.** The regex change in `src/plugins/memory/scrub.rs` correctly replaces `[\s\S]` with `.` under the `(?is)` flag (which enables DOTALL mode), preserving exact semantics while improving compile-time performance as documented. The added unit test and latency benchmark are structurally sound and properly scoped to release builds.

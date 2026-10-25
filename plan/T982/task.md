---
status: todo
priority: P2
complexity: 3
readiness: 0%
agent:
---

# T982. Blocking store calls leave the tokio workers in `proxy` and `web`

From `ideas.md` I-96. Diesel calls run inline on tokio workers: the dashboard `socket_loop` (`src/web/mod.rs`) opens the store and runs the snapshot queries on every 2 s tick, and the proxy `handle` does request bookkeeping in the async hot path (`src/proxy/mod.rs`). A slow store can stall unrelated proxy requests. Wrap those calls in `tokio::task::spawn_blocking` (tokio is already in the tree). All async stays on tokio; `rtok mcp` stays synchronous.

Done means: a slow store no longer delays other proxy requests, and proxy latency with the dashboard open does not get worse.

Check: a test where the store blocks for a while shows a second proxy request completing without waiting for it; proxy p95 under concurrent requests with the dashboard open, before and after, recorded in `research.md`.

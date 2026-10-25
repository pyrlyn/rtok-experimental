---
status: todo
priority: P2
complexity: 2
readiness: 0%
agent:
---

# T984. Bench the archive pointer's `min_tokens` floor

From `ideas.md` I-116 (`research.md` §16.8, T385.8). Old tool results of 300–1,499 tokens are 7.22 % of input and sit under the 1500 floor; `min_tokens = 300` would double the eligible share from 6.54 % to 13.76 %. A lower floor also churns more blocks (cache busts) and raises `expand` calls, so 1500 may be right. This needs no compressor and is cheaper than P28.

Blocked by T983: the comparison is only fair once bench scores each run from a clean fixture. The live runs use a real agent, so the creator runs them; the agent prepares the command set and records the result.

Done means: cost per passed task and `expand` rate at 1500 / 800 / 300 are recorded, and the default either changes to the winner or stays at 1500 with the reason.

Check: three `rtok bench` rows (1500 / 800 / 300) in `docs/agents/benchmarks.md`, and the default in config matches the decision written next to them.

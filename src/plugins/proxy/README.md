# `proxy`

A local `ANTHROPIC_BASE_URL` hop that records real token usage and, in `compress` mode, lets
other plugins rewrite the request.

| | |
|---|---|
| Surfaces | `rtok proxy [--port 8790] [--upstream URL] [--mode passthrough\|compress]`; serves `ANTHROPIC_BASE_URL` and `OPENAI_BASE_URL` |
| Spec | the **Replaces** column of [`docs/plugins.md`](../../../docs/plugins.md) |
| Default | on |

## Behaviour

- Forwards `POST /v1/messages` (and everything else) to `RTOK_UPSTREAM`
  (default `https://api.anthropic.com`; may be another local proxy during A/B).
- Wire formats (decision D11, phase P11): Anthropic Messages, OpenAI Chat Completions
  (`/v1/chat/completions` → `RTOK_OPENAI_UPSTREAM`), OpenAI Responses (`/v1/responses`).
  Each is a `Wire` adapter that exposes tool results and `usage` in one normalised shape;
  `archive` and `toon` never see the raw format.
- Streams SSE responses unchanged; parses `usage` from the final `message_delta` or the
  non-streaming body into the `usage` table.
- `passthrough` mode changes zero bytes. `compress` mode runs every enabled plugin's
  `proxy_filter` (today: `archive`, optionally `toon`).
- `/health` → `{"ok":true,"mode":"passthrough"}`.
- `rtok agents install claude --proxy` sets `env.ANTHROPIC_BASE_URL` in settings (with backup) and
  prints how to revert.

Caveat (research.md §3): setting a base URL disables MCP tool search by default in Claude
Code; `rtok doctor` flags this.

## Tasks

See `roadmap.md` § `proxy`. Checks in `plan/`.

T5.1 passthrough · T5.2 lifecycle/health/setup · T5.5 cache-health report (with `measure`) ·
T11.1 `Wire` adapter · T11.2 OpenAI Chat · T11.3 OpenAI Responses · T11.5 OpenAI host setup.

## Status

Manifest only. First task: T5.1.

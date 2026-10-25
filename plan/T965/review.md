# T965 review

## Dependency maintenance

`bincode 1.3.3` was an unmaintained transitive dependency, not a direct serialization
format used by rtok. The path was printpdf's default `html` feature through azul-layout
and hyphenation. Native PDF operations used by the report need none of that feature.
Disable printpdf defaults without changing its version or replacing the renderer.
The two PDF integration tests cover headings, ordering and charts before and after.
Root and fuzz lockfiles lose only the 15 now-unused packages and their dependency edges.

The fuzz lockfile was already stale against its manifest before this task: locked Cargo
metadata fails on T962 as well. A speculative full refresh was reverted; unrelated
package versions are not authorized by this task. The root lockfile is Cargo-resolved.

## A14: upgrade existing Unix permissions explicitly

Doctor reports loose permission bits and unsafe entries in its text/JSON model. The
web doctor renders the same findings; the TUI/report text inherits them. Permission
repair uses `doctor --fix --only permissions --yes`; without `--yes`, or with
`--dry-run`/`--stat`, it previews. The hook checklist cannot implicitly approve chmod.

Use existing rtok-sys/rustix rather than add a filesystem dependency. Open path
components and descendants relative to held descriptors with NOFOLLOW; inspect owner,
type and link count, chmod that descriptor, then verify it. No file contents are read,
rewritten or deleted. Directories become 0700, regular files 0600. Refuse symbolic links,
multiply-linked files, foreign owners and non-regular entries. Independent paths still
repair after a refusal, so a retry is safe after resolving the refused entry.

Scope is the rtok home, its config/env files, configured database/sidecars, archive tree,
and log directory plus known live/error/rotated logs. Do not recursively chmod unrelated
log siblings. External configured paths require manual review. Each recursive scan is
bounded by 100,000 entries and 64 levels; reaching a bound is reported. At most 100
configured rotated siblings per log are inspected. Windows ACLs and additional Unix
ACL entries are outside this permission-bit check. Same-user processes can still change
permissions after verification. No repair was run against the user's real data.

## A15: bound proxy buffering across all modes

Typed positive limits default to 8 MiB per request/retained response prefix and eight
global in-flight requests. Zero cannot disable the global bound. Acquire before reading
the request, with no global wait queue; overload returns 429 and Retry-After. Existing
lane gates remain secondary policy. Resource bounds also apply with lanes disabled,
proxy bookkeeping disabled, and on subsequent proxy hops.

The global permit covers downstream streaming and detached CPU/store work retaining
buffers, including cancellation. The tee notices downstream disconnection even when
upstream goes silent. Oversized requests return 413; broken request streams remain 400.
Oversized responses are forwarded whole, but recording is capped and truncated responses
cannot populate the semantic cache. Plain mode retains no response-recording buffer.
These are application buffer/concurrency bounds, not a claim about total process RSS.
JSON parsing, plugin work, transport buffers and other process state also consume memory.

## A16: telemetry content is opt-in

Default `otel.content` is false whether the endpoint is explicit or comes from the
generic OTLP environment variable. Existing explicit true remains true; no user config
is rewritten. Tests exercise hook prompts/tool results, MCP arguments/results and proxy
messages, with explicit opt-in preserving the existing content and truncation behavior.

## Integration boundary

A10, A11 and A13 are implemented by T960/T961 and collected by T963 in PR #1249
<https://github.com/pyrlyn/rtok/pull/1249>. That PR was open when inspected. Its foreign
branches were neither modified nor merged here; this branch continues our T962 fixes.

No new feature, benchmark or saving claim is part of this task. See validation.md for
actual checks and remaining validation limitations.

## Compatibility and rollback

Existing config files need no rewrite. Applications sending larger requests must raise
the positive `proxy.max_body_bytes`; concurrency can likewise be tuned explicitly.
Existing explicit telemetry content opt-in is preserved. Downgrading the binary leaves
data and tightened file modes intact. A permission repair prints each previous/target
mode so an operator can deliberately restore a particular mode if sharing was intended;
there is no automatic rollback that silently makes private data readable again. Failed
repairs preserve contents and return an error; successful independent repairs remain in
place and need no replay on the next run.

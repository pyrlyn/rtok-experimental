// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! PreToolUse(Read) advice (plan T4.6): deny native Read of large files not just edited.

use rtok_plugin_sdk::{Class, Ctx, Measurement, PreToolDecision, PreToolUse};

/// Tail of the first deny, the same for every host: `measure::stats` matches it in a
/// transcript to find a denied Read (T383), and it also sits inside the pre-T384 text.
pub(crate) const REASON: &str = "before Edit run native Read(limit=1) — it satisfies the edit gate";

/// Head of the post-edit deny; `measure::stats` matches it like [`REASON`].
pub(crate) const DELTA_REASON: &str = "file changed since last read; use ";

/// What the deny calls rtok's `read`, and the line that loads it where the host defers MCP
/// schemas (T384). Without the exact name 55 % of the denies in `research.md` §29.2 were
/// followed by a `ToolSearch` turn. Byte-stable per host.
struct ReadTool {
    name: String,
    hint: String,
}

impl ReadTool {
    /// Fail open: a host not listed keeps the generic `rtok read` and no hint, as before T384.
    fn of(host: &str) -> Self {
        let server = rtok_mcp::registry::RTOK.name;
        let (name, defers) = match host {
            // `mcp__<server>__<tool>`, tool search on by default
            // (https://code.claude.com/docs/en/mcp, read 2026-10-04).
            "claude" => (format!("mcp__{server}__read"), true),
            // `mcp_{serverName}_{toolName}`, tools discovered at startup, not deferred
            // (https://geminicli.com/docs/tools/mcp-server/, read 2026-10-04).
            "gemini" => (format!("mcp_{server}_read"), false),
            _ => (format!("{server} read"), false),
        };
        let hint = if defers {
            format!(" (ToolSearch select:{name} loads it)")
        } else {
            String::new()
        };
        Self { name, hint }
    }
}

pub fn pre_tool(ev: &PreToolUse<'_>, cx: &Ctx) -> Option<PreToolDecision> {
    let cfg = cx.plugin_config::<crate::config::Read>("read");
    if !cfg.advice || ev.tool_name != "Read" {
        return None;
    }
    let path = super::path_arg(ev.tool_input)?;
    let len = std::fs::metadata(path).ok()?.len();
    if len <= cfg.native_max_bytes {
        return None;
    }
    // T930: a deny that names a tool which cannot serve the file only costs a turn, and the
    // agent then falls back to `cat`/`sed`, which shorten the output again.
    if rtok_cannot_serve(path, &cfg.allow_paths) {
        return None;
    }
    // T127/T383: a ranged native Read passes whatever the file size. The host's `Edit` wants
    // a native `Read` first (an MCP `read` does not count), and 70% of the denied Reads in
    // `research.md` §29 were already ranged, so denying them cost a turn for nothing. A zero
    // limit is unranged for the host too, so it stays denied.
    let limit = ev.tool_input.get("limit").and_then(|l| l.as_u64());
    if limit.is_some_and(|l| (1..=u64::from(cfg.range_max_lines)).contains(&l)) {
        return None;
    }
    if recently_edited(cx, path) {
        // T929: with a sub-agent in the session the id may name a body this window never saw.
        if cfg.delta
            && !cx.shared_session()
            && let Some(id) = last_read_id(cx, path)
        {
            let t = tool(cx);
            return Some(deny(
                cx,
                format!("{DELTA_REASON}{}(mode=diff) vs {id:.8}{}", t.name, t.hint),
            ));
        }
        return None;
    }
    let t = tool(cx);
    Some(deny(cx, format!("use {}{}; {REASON}", t.name, t.hint)))
}

/// Extensions the host's native Read renders (vision, PDF pages) and MCP `read` cannot:
/// it returns text only.
const NATIVE_MEDIA: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "avif", "pdf",
];

/// True when MCP `read` would not serve `path`: a media file, or a path its root guard
/// refuses (scratchpads, task outputs, `~/.cargo/registry`). The guard is
/// [`super::resolve`] itself, so the two cannot drift. The hook never enables the extra
/// worktree roots (that spawns git, and the hook has a 10 ms budget), so a sibling-worktree
/// file is treated as outside: fail open towards the native Read.
fn rtok_cannot_serve(path: &str, allow_paths: &[std::path::PathBuf]) -> bool {
    let media = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| NATIVE_MEDIA.iter().any(|m| e.eq_ignore_ascii_case(m)));
    if media {
        return true;
    }
    let Ok(cwd) = super::roots::cwd() else {
        return false;
    };
    super::resolve(&cwd, std::path::Path::new(path), allow_paths).is_err()
}

/// Read only on a deny, the rare branch: `config` serializes the whole configuration.
fn tool(cx: &Ctx) -> ReadTool {
    ReadTool::of(&cx.config::<crate::config::Hook>("hook").host)
}

/// Every deny is a cost row, never a saving (T383): the reason is tokens the model reads
/// for a call that returned nothing, and `rtok stats --plugin read` nets it against the
/// savings. Fail open: a store error must not turn a deny into a hook failure.
fn deny(cx: &Ctx, reason: String) -> PreToolDecision {
    let _ = cx.record(&Measurement {
        plugin: "read",
        kind: "deny",
        before_bytes: 0,
        after_bytes: reason.len() as u64,
        est_before: 0,
        est_after: cx.estimate(&reason, Class::Code),
        ref_id: None,
        call_id: None,
    });
    PreToolDecision::Deny { reason }
}

/// Archive id of the last MCP `read` of this path (`mode=full`), if still cached.
fn last_read_id(cx: &Ctx, path: &str) -> Option<String> {
    let abs = fs_kit::path::canonicalize(path).ok()?;
    let key = super::cache::key(abs.to_string_lossy().as_ref(), "full", None);
    cx.get_read_cache(&key).ok().flatten()?.0
}

/// The edit window: the last 5 finished tool calls (PostToolUse rows).
/// T202: `hook_event_name` lives in `calls.name` (`record_call`, `src/hooks/mod.rs`), so the
/// filter and this limit are now a `WHERE`/`LIMIT` in the query itself
/// (`recent_hook_inputs_for_event`) — a session with a long history of other events, or of
/// large PostToolUse bodies further back, costs nothing here.
const WINDOW_TOOL_CALLS: usize = 5;

/// True when a PostToolUse(Edit|Write) for `path` sits in the window.
/// Fail open: a store error allows the Read (unmodified input, D1), and so does a hook
/// body the store could not keep (empty string) — the window cannot rule out an edit of
/// this file, and denying a Read of a file the agent just wrote is the worse error.
fn recently_edited(cx: &Ctx, path: &str) -> bool {
    let bodies = match cx.recent_hook_inputs_for_event("PostToolUse", WINDOW_TOOL_CALLS as i64) {
        Ok(b) => b,
        Err(_) => return true,
    };
    for b in &bodies {
        if b.is_empty() {
            return true;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(b) else {
            continue;
        };
        if edits_path(&v, path) {
            return true;
        }
    }
    false
}

fn edits_path(v: &serde_json::Value, path: &str) -> bool {
    let tool = v.get("tool_name").and_then(|t| t.as_str()).unwrap_or("");
    if tool != "Edit" && tool != "Write" {
        return false;
    }
    let edited = v
        .get("tool_input")
        .and_then(|i| i.get("file_path").or_else(|| i.get("path")))
        .and_then(|p| p.as_str())
        .unwrap_or("");
    crate::fs::same_spelling(edited, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtok_plugin_sdk::Ctx;
    use serde_json::json;
    use std::fs;

    /// The test directory is outside the process cwd, so it is an `allow_paths` root:
    /// otherwise MCP `read` would refuse its files and the gate would let them through (T930).
    fn cx(name: &str) -> crate::plugin::Runtime {
        let (mut c, dir) = crate::testutil::config(name);
        c.plugins.read.allow_paths = vec![dir];
        crate::plugin::Runtime::open(c, name).unwrap()
    }

    fn ev<'a>(input: &'a serde_json::Value) -> PreToolUse<'a> {
        PreToolUse {
            tool_name: "Read",
            tool_input: input,
        }
    }

    #[test]
    fn edited_file_reads_on() {
        let cx = cx("edit");
        let dir = cx.config.core.archive_dir.parent().unwrap().to_path_buf();
        let p = dir.join("edit.txt");
        fs::write(&p, "x".repeat(100 * 1024)).unwrap();
        let path = p.to_str().unwrap();
        let edit = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Edit",
            "tool_input": {"file_path": path},
        });
        let id = cx.record_call("hook", "hook", Some("PostToolUse")).unwrap();
        cx.store
            .insert_call_io(
                id,
                Some(&serde_json::to_vec(&edit).unwrap()),
                None,
                65536,
                None,
            )
            .unwrap();
        let input = json!({"file_path": path});
        assert!(pre_tool(&ev(&input), &Ctx::new(&cx)).is_none());
    }

    #[test]
    fn edited_reread_advises_diff_when_cached() {
        let cx = cx("delta-advice");
        let dir = cx.config.core.archive_dir.parent().unwrap().to_path_buf();
        let p = dir.join("delta.txt");
        fs::write(&p, "x".repeat(100 * 1024)).unwrap();
        let path = p.to_str().unwrap();
        let abs = fs_kit::path::canonicalize(&p).unwrap();
        let key = crate::plugins::read::cache::key(abs.to_str().unwrap(), "full", None);
        let ctx = Ctx::new(&cx);
        let id = ctx.put_archive(b"previous body").unwrap();
        ctx.put_read_cache(&key, "h", Some(&id)).unwrap();
        let edit = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Edit",
            "tool_input": {"file_path": path},
        });
        let hid = cx.record_call("hook", "hook", Some("PostToolUse")).unwrap();
        cx.store
            .insert_call_io(
                hid,
                Some(&serde_json::to_vec(&edit).unwrap()),
                None,
                65536,
                None,
            )
            .unwrap();
        let input = json!({"file_path": path});
        match pre_tool(&ev(&input), &Ctx::new(&cx)) {
            Some(PreToolDecision::Deny { reason }) => {
                assert!(reason.contains("mode=diff"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Mirrors `src/hooks/mod.rs`'s real dispatch: `calls.name` is the hook event name (T202
    /// filters on it), taken from the same `hook_event_name` field this fixture's body sets.
    fn hook_row(cx: &crate::plugin::Runtime, body: serde_json::Value) {
        let event = body.get("hook_event_name").and_then(|e| e.as_str());
        let id = cx.record_call("hook", "hook", event).unwrap();
        let bytes = serde_json::to_vec(&body).unwrap();
        cx.store
            .insert_call_io(id, Some(&bytes), None, 65536, None)
            .unwrap();
    }

    /// Non-tool rows (prompts, PreToolUse) do not push an edit out of the window; the
    /// sixth finished tool call after it does.
    #[test]
    fn the_window_counts_tool_calls_not_hook_rows() {
        let cx = cx("window");
        let p = cx.config.core.archive_dir.parent().unwrap().join("w.txt");
        fs::write(&p, "x".repeat(100 * 1024)).unwrap();
        let path = p.to_str().unwrap();
        hook_row(
            &cx,
            json!({"hook_event_name": "PostToolUse", "tool_name": "Write", "tool_input": {"file_path": path}}),
        );
        for _ in 0..12 {
            hook_row(&cx, json!({"hook_event_name": "UserPromptSubmit"}));
        }
        let input = json!({"file_path": path});
        assert!(
            pre_tool(&ev(&input), &Ctx::new(&cx)).is_none(),
            "still in window"
        );
        for _ in 0..WINDOW_TOOL_CALLS {
            hook_row(
                &cx,
                json!({"hook_event_name": "PreToolUse", "tool_name": "Bash"}),
            );
            hook_row(
                &cx,
                json!({"hook_event_name": "PostToolUse", "tool_name": "Bash"}),
            );
        }
        assert!(pre_tool(&ev(&input), &Ctx::new(&cx)).is_some(), "aged out");
    }

    #[test]
    fn hundred_kb_is_denied() {
        let cx = cx("big");
        let p = cx.config.core.archive_dir.parent().unwrap().join("big.txt");
        fs::write(&p, "x".repeat(100 * 1024)).unwrap();
        let input = json!({"file_path": p.to_str().unwrap()});
        let d = pre_tool(&ev(&input), &Ctx::new(&cx)).expect("deny");
        match d {
            PreToolDecision::Deny { reason } => {
                assert!(reason.contains("Read(limit=1)"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// T930: a large file MCP `read` cannot serve is not denied: an image or PDF anywhere, and
    /// a text file outside the roots (scratchpad log, task output). An in-root one still is.
    #[test]
    fn a_file_rtok_read_cannot_serve_passes_native() {
        let tag = "native-passthrough";
        let (c, dir) = crate::testutil::config(tag);
        let cx = crate::plugin::Runtime::open(c, tag).unwrap();
        let big = "x".repeat(100 * 1024);
        let denied = |p: &std::path::Path| {
            let input = json!({"file_path": p.to_str().unwrap()});
            pre_tool(&ev(&input), &Ctx::new(&cx)).is_some()
        };
        let inside = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("t930-{}.log", std::process::id()));
        fs::create_dir_all(inside.parent().unwrap()).unwrap();
        fs::write(&inside, &big).unwrap();
        let inside_denied = denied(&inside);
        fs::remove_file(&inside).unwrap();
        assert!(inside_denied, "a large in-project file stays denied");
        let outside = dir.join("scratch.log");
        fs::write(&outside, &big).unwrap();
        assert!(!denied(&outside), "outside the roots rtok read refuses it");
        for name in ["shot.png", "SHOT.PNG", "doc.pdf"] {
            let p = dir.join(name);
            fs::write(&p, &big).unwrap();
            assert!(!denied(&p), "{name}");
        }
    }

    /// The deny text of a 100 KB native Read on `host`, and what a post-edit re-read says.
    fn denies_on(host: &str) -> (String, String) {
        let tag = format!("deny-text-{host}");
        let (mut c, dir) = crate::testutil::config(&tag);
        c.hook.host = host.into();
        c.plugins.read.allow_paths = vec![dir.clone()];
        let cx = crate::plugin::Runtime::open(c, &tag).unwrap();
        let p = dir.join("big.txt");
        fs::write(&p, "x".repeat(100 * 1024)).unwrap();
        let path = p.to_str().unwrap();
        let reason = |input: &serde_json::Value| match pre_tool(&ev(input), &Ctx::new(&cx)) {
            Some(PreToolDecision::Deny { reason }) => reason,
            other => panic!("{other:?}"),
        };
        let first = reason(&json!({"file_path": path}));
        let abs = fs_kit::path::canonicalize(&p).unwrap();
        let key = crate::plugins::read::cache::key(abs.to_str().unwrap(), "full", None);
        let id = Ctx::new(&cx).put_archive(b"previous body").unwrap();
        Ctx::new(&cx).put_read_cache(&key, "h", Some(&id)).unwrap();
        hook_row(
            &cx,
            json!({"hook_event_name": "PostToolUse", "tool_name": "Edit", "tool_input": {"file_path": path}}),
        );
        let delta = reason(&json!({"file_path": path}));
        (first, delta.replace(&format!("{id:.8}"), "ID"))
    }

    const TAIL: &str = "before Edit run native Read(limit=1) — it satisfies the edit gate";

    /// T384: Claude Code defers MCP schemas, so the deny names the exact tool and the
    /// `ToolSearch` call that loads it (no extra turn to find it).
    #[test]
    fn claude_deny_names_the_tool_and_how_to_load_it() {
        let (first, delta) = denies_on("claude");
        assert_eq!(
            first,
            format!("use mcp__rtok__read (ToolSearch select:mcp__rtok__read loads it); {TAIL}")
        );
        assert_eq!(
            delta,
            "file changed since last read; use mcp__rtok__read(mode=diff) vs ID \
             (ToolSearch select:mcp__rtok__read loads it)"
        );
    }

    /// T384: Gemini CLI names MCP tools `mcp_<server>_<tool>` and discovers them at startup.
    #[test]
    fn gemini_deny_names_the_tool_without_a_load_hint() {
        let (first, delta) = denies_on("gemini");
        assert_eq!(first, format!("use mcp_rtok_read; {TAIL}"));
        assert_eq!(
            delta,
            "file changed since last read; use mcp_rtok_read(mode=diff) vs ID"
        );
    }

    /// T384: a host whose tool naming is not known keeps the generic text (fail open).
    #[test]
    fn an_unlisted_host_keeps_the_generic_deny() {
        let (first, delta) = denies_on("copilot");
        assert_eq!(first, format!("use rtok read; {TAIL}"));
        assert_eq!(
            delta,
            "file changed since last read; use rtok read(mode=diff) vs ID"
        );
    }

    /// T127/T383: a ranged native Read up to `range_max_lines` passes (limit 1 is the edit
    /// gate); a wider range, no limit and `limit: 0` (unranged for the host) are denied.
    #[test]
    fn a_ranged_read_passes_up_to_the_configured_cap() {
        let cx = cx("gate");
        let p = cx
            .config
            .core
            .archive_dir
            .parent()
            .unwrap()
            .join("gate.txt");
        fs::write(&p, "x".repeat(100 * 1024)).unwrap();
        let path = p.to_str().unwrap();
        let denied = |input: serde_json::Value| pre_tool(&ev(&input), &Ctx::new(&cx)).is_some();
        assert_eq!(cx.config.plugins.read.range_max_lines, 300);
        assert!(!denied(json!({"file_path": path, "limit": 1})));
        assert!(!denied(json!({"file_path": path, "limit": 5})));
        assert!(!denied(json!({"file_path": path, "limit": 300})));
        assert!(denied(json!({"file_path": path, "limit": 301})));
        assert!(denied(json!({"file_path": path, "limit": 2000})));
        assert!(denied(json!({"file_path": path, "limit": 0})));
        assert!(denied(json!({"file_path": path})));
    }

    /// T383: both deny reasons write one `read`/`deny` cost row — no bytes before, the
    /// reason's bytes after — and a pass writes none.
    #[test]
    fn every_deny_writes_a_cost_row() {
        let cx = cx("deny-row");
        let p = cx.config.core.archive_dir.parent().unwrap().join("d.txt");
        fs::write(&p, "x".repeat(100 * 1024)).unwrap();
        let path = p.to_str().unwrap();
        let rows = || cx.store.list_measurements("read").unwrap();
        let ok = json!({"file_path": path, "limit": 300});
        assert!(pre_tool(&ev(&ok), &Ctx::new(&cx)).is_none());
        assert!(rows().is_empty(), "a pass is not a deny");
        let wide = json!({"file_path": path, "limit": 301});
        assert!(pre_tool(&ev(&wide), &Ctx::new(&cx)).is_some());
        let m = &rows()[0];
        assert_eq!((m.kind.as_str(), m.before_bytes), ("deny", 0));
        let (first, _) = denies_on("claude");
        assert_eq!(m.after_bytes as usize, first.len());
        assert_eq!(m.est_before, 0);
        assert!(m.est_after > 0);

        let abs = fs_kit::path::canonicalize(&p).unwrap();
        let key = crate::plugins::read::cache::key(abs.to_str().unwrap(), "full", None);
        let id = Ctx::new(&cx).put_archive(b"previous body").unwrap();
        Ctx::new(&cx).put_read_cache(&key, "h", Some(&id)).unwrap();
        hook_row(
            &cx,
            json!({"hook_event_name": "PostToolUse", "tool_name": "Edit", "tool_input": {"file_path": path}}),
        );
        assert!(pre_tool(&ev(&wide), &Ctx::new(&cx)).is_some());
        let all = rows();
        assert_eq!(all.len(), 2, "the delta reason writes its own row");
        assert!(all.iter().all(|m| m.kind == "deny"));
        let (_, delta) = denies_on("claude");
        assert_eq!(
            all[1].after_bytes as usize,
            delta.replace("ID", "12345678").len()
        );
    }

    /// T55.13: Copilot's adapted `Read` carries `path`, not `file_path`; the large-file
    /// advice must still deny.
    #[test]
    fn copilot_path_key_gets_the_read_advice() {
        let cx = cx("copilot-path");
        let dir = cx.config.core.archive_dir.parent().unwrap().to_path_buf();
        let p = dir.join("copilot-big.txt");
        fs::write(&p, "x".repeat(100 * 1024)).unwrap();
        let input = json!({"path": p.to_str().unwrap()});
        match pre_tool(&ev(&input), &Ctx::new(&cx)) {
            Some(PreToolDecision::Deny { reason }) => {
                assert!(reason.contains(TAIL), "{reason}")
            }
            other => panic!("{other:?}"),
        }
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn two_kb_is_silent() {
        let cx = cx("small");
        let p = cx
            .config
            .core
            .archive_dir
            .parent()
            .unwrap()
            .join("small.txt");
        fs::write(&p, "x".repeat(2048)).unwrap();
        let input = json!({"file_path": p.to_str().unwrap()});
        assert!(pre_tool(&ev(&input), &Ctx::new(&cx)).is_none());
    }

    #[test]
    fn same_path_rejects_suffix_false_positive() {
        use crate::fs::same_spelling as same_path;
        assert!(!same_path("/repo/main.rs", "ain.rs"));
        assert!(same_path("/repo/src/main.rs", "src/main.rs"));
        assert!(same_path("src/main.rs", "/repo/src/main.rs"));
        assert!(!same_path("", "src/main.rs"));
    }

    /// T504: a Windows tool call can spell the drive and folders in another case than the
    /// path the hook was asked about; on case-sensitive hosts they stay different files.
    #[test]
    fn same_path_follows_the_platform_case_rule() {
        let folds = cfg!(windows);
        assert_eq!(
            crate::fs::same_spelling(r"C:\Repo\Src\Main.rs", r"c:\repo\src\main.rs"),
            folds
        );
        assert_eq!(
            crate::fs::same_spelling("/Repo/Src/Main.rs", "/repo/src/main.rs"),
            folds
        );
        assert_eq!(
            crate::fs::same_spelling("/Repo/Src/Main.rs", "src/MAIN.rs"),
            folds
        );
    }

    /// A hook body the store could not keep (above `core.call_io_inline_bytes`, no archive
    /// dir) comes back empty. The window must fail open there: a big `Write` followed by a
    /// native `Read` of the same file used to be denied because the write was invisible.
    #[test]
    fn an_unreadable_hook_body_allows_the_read() {
        let cx = cx("elided");
        let dir = cx.config.core.archive_dir.parent().unwrap().to_path_buf();
        let p = dir.join("elided-big.txt");
        fs::write(&p, "x".repeat(100 * 1024)).unwrap();
        let big = "x".repeat(70 * 1024);
        let stdin = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Write",
            "tool_input": {"file_path": p.to_str().unwrap()},
            "pad": big,
        });
        let id = cx.record_call("hook", "hook", Some("PostToolUse")).unwrap();
        cx.store
            .insert_call_io(
                id,
                Some(&serde_json::to_vec(&stdin).unwrap()),
                None,
                65536,
                None,
            )
            .unwrap();
        assert!(
            cx.store.recent_hook_inputs(&cx.session, 10).unwrap()[0].is_empty(),
            "the body is elided, not stored"
        );
        let input = json!({"file_path": p.to_str().unwrap()});
        assert!(
            pre_tool(&ev(&input), &Ctx::new(&cx)).is_none(),
            "an unreadable window must not deny a file this session may have written"
        );
    }

    /// T202 Check: 50 unrelated 60 KB `PostToolUse` rows in this session must not make a
    /// native `Read` PreToolUse pay for their bodies. Wall time here is a generous
    /// debug-build smoke bound only (CI must not flake on it); the actual p95 < 10 ms budget
    /// is `cargo test --release latency` (T2.2). The real guard is the query itself: it must
    /// fetch at most the window size, not all 50 rows (~3 MB) to find nothing in it.
    #[test]
    fn hook_pre_tool_read_stays_under_budget_with_50_large_rows() {
        let cx = cx("big-history");
        let dir = cx.config.core.archive_dir.parent().unwrap().to_path_buf();
        let p = dir.join("target.txt");
        fs::write(&p, "x".repeat(100 * 1024)).unwrap();
        let path = p.to_str().unwrap();

        // 50 unrelated PostToolUse rows, ~60 KB each, none of them touching `path`.
        let pad = "x".repeat(60 * 1024);
        for n in 0..50 {
            let body = json!({
                "hook_event_name": "PostToolUse",
                "tool_name": "Write",
                "tool_input": {"file_path": format!("/repo/unrelated_{n}.rs")},
                "pad": pad,
            });
            let id = cx.record_call("hook", "hook", Some("PostToolUse")).unwrap();
            cx.store
                .insert_call_io(
                    id,
                    Some(&serde_json::to_vec(&body).unwrap()),
                    None,
                    200_000,
                    None,
                )
                .unwrap();
        }

        let input = json!({"file_path": path});
        let start = std::time::Instant::now();
        let decision = pre_tool(&ev(&input), &Ctx::new(&cx));
        let elapsed = start.elapsed();
        assert!(
            decision.is_some(),
            "no edit of `path` sits in the 50-row history; the large file must still deny"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "debug-build smoke bound (real budget: release latency test): {elapsed:?}"
        );

        // The real guard: the query returns at most the window, not the full history.
        let rows = cx
            .store
            .recent_hook_inputs_for_event(&cx.session, "PostToolUse", WINDOW_TOOL_CALLS as i64)
            .unwrap();
        assert!(
            rows.len() <= WINDOW_TOOL_CALLS,
            "fetched {} rows, want at most the {WINDOW_TOOL_CALLS}-row window",
            rows.len()
        );
        let bytes: usize = rows.iter().map(String::len).sum();
        assert!(
            bytes < 500 * 1024,
            "fetched {bytes} bytes across {} rows; want a few small rows, not the ~3 MB the \
             full 50-row history holds",
            rows.len()
        );
    }
}

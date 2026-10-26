// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T38.2: one e2e case per catalogue plugin through its surface (Measurement rows where owed).
use rtok::config::Config;
use rtok::proxy::{ProxyState, app};
use rtok::store::MeasRow;
use rtok::tokens::{self, Class};
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tmp(n: &str) -> Home {
    let t = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("rtok-t382-{n}-{}-{t}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Home(d)
}
fn run(home: &Home, args: &[&str], input: &str, cwd: &Path) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rtok"));
    c.args(args).env("RTOK_HOME", &home.0);
    c.env("HOME", &home.0).current_dir(cwd);
    c.stdin(Stdio::piped()).stdout(Stdio::piped());
    c.stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    drop(child.stdin.take().unwrap().write_all(input.as_bytes()));
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{args:?} {err}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn js(s: &str) -> serde_json::Value {
    serde_json::from_str(s).unwrap()
}
fn kinds(home: &Home, plugin: &str) -> Vec<String> {
    let s = rtok::store::Store::open(&home.0.join("rtok.db")).unwrap();
    let rows = s.list_measurements(plugin).unwrap();
    rows.into_iter().map(|r| r.kind).collect()
}
/// T239: full rows (not just kinds) for asserting `before`/`after` against real bytes.
fn rows(home: &Home, plugin: &str) -> Vec<MeasRow> {
    let s = rtok::store::Store::open(&home.0.join("rtok.db")).unwrap();
    s.list_measurements(plugin).unwrap()
}
fn tool(home: &Home, cwd: &Path, name: &str, args: &str) -> String {
    let p = format!("{{\"name\":\"{name}\",\"arguments\":{args}}}");
    let req = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{p}}}");
    let v = js(&run(home, &["mcp"], &req, cwd));
    let t = &v["result"]["content"][0]["text"];
    t.as_str().unwrap_or("").into()
}
const UP: &str = r#"{"id":"m","type":"message","role":"assistant","content":[{"type":"text","text":"ok"}],"model":"m","stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":2}}"#;
type Srv = (String, Arc<ProxyState>);
async fn serve(home: &Home, mode: &str, patch: impl FnOnce(&mut Config)) -> Srv {
    let srv = Box::leak(Box::new(httpmock::MockServer::start()));
    let m = srv.mock(|w, t| {
        w.method(httpmock::Method::POST).path("/v1/messages");
        t.status(200).body(UP);
    });
    let _ = Box::leak(Box::new(m));
    let mut cfg = Config::load_from(&home.0).unwrap();
    cfg.proxy.upstream = srv.base_url();
    cfg.proxy.mode = mode.into();
    patch(&mut cfg);
    let st = Arc::new(ProxyState::new(&cfg).unwrap());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap().to_string();
    tokio::spawn(axum::serve(l, app(st.clone())).into_future());
    (a, st)
}
async fn post(a: &str, body: Vec<u8>) -> String {
    let r = reqwest::Client::new().post(format!("http://{a}/v1/messages"));
    let r = r.header("content-type", "application/json");
    let r = r.body(body).send().await.unwrap();
    r.text().await.unwrap()
}
fn user_msg(t: usize, content: &str) -> String {
    let h = r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu-"#;
    format!("{h}{t}\",\"content\":{content}}}]}}")
}
fn turns(s: &str, mk: impl Fn(usize) -> String) -> Vec<u8> {
    let ms = (1..=6).map(mk).collect::<Vec<_>>().join(",");
    format!(r#"{{"model":"m","messages":[{ms}],"metadata":{{"user_id":"{s}"}}}}"#).into_bytes()
}
fn big(t: usize, p: &str) -> String {
    let v: Vec<_> = (1..=600).map(|i| format!("{t}:{i}:{p}")).collect();
    user_msg(t, &serde_json::to_string(&v.join("\n")).unwrap())
}
fn tab(t: usize) -> String {
    let v: Vec<_> = (1..=8).map(|i| json!({"a":i,"b":i,"c":i})).collect();
    user_msg(t, &serde_json::to_string(&v).unwrap())
}
#[test]
fn measure_stats_json_parses() {
    let home = tmp("measure");
    let out = run(&home, &["stats", "--json"], "", &home.0);
    assert!(js(&out).get("sessions").is_some(), "{out}");
}
#[test]
fn cmd_run_records_measurement() {
    let home = tmp("cmd");
    let out = run(&home, &["run", "echo", "hello-t382"], "", &home.0);
    assert!(out.contains("hello-t382"), "{out}");
    assert!(!kinds(&home, "cmd").is_empty());
}
#[test]
fn read_dedup_on_second_mcp_read() {
    let home = tmp("read");
    std::fs::write(home.0.join("a.txt"), "line one\nline two\n").unwrap();
    // One `rtok mcp` process is one session (read_cache is per session), so both reads go
    // down the same stdin.
    let req = |id: u8| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"read","arguments":{{"path":"a.txt"}}}}}}"#
        )
    };
    let out = run(
        &home,
        &["mcp"],
        &format!("{}\n{}\n", req(1), req(2)),
        &home.0,
    );
    let texts: Vec<String> = out
        .lines()
        .map(|l| {
            js(l)["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .into()
        })
        .collect();
    let (first, second) = (&texts[0], &texts[1]);
    assert!(first.contains("line one"), "{first}");
    assert!(second.contains("unchanged id "), "{second}");
    assert!(kinds(&home, "read").iter().any(|k| k == "dedup"));
}
#[test]
fn memory_save_then_search() {
    let home = tmp("memory");
    let a = r#"{"kind":"note","title":"t382 milk","body":"oat milk"}"#;
    let saved = tool(&home, &home.0, "mem_save", a);
    let found = tool(&home, &home.0, "mem_search", r#"{"query":"milk"}"#);
    assert!(saved.contains("id"), "{saved}");
    assert!(found.contains("t382 milk"), "{found}");
}
#[test]
fn memory_save_refuses_a_repeat_and_names_the_stored_note() {
    let home = tmp("memory-admit");
    let first = r#"{"title":"cache","body":"The walrus cache evicts the oldest entry."}"#;
    let twin = r#"{"title":"cache again","body":"the walrus cache evicts the OLDEST entry."}"#;
    let stored: serde_json::Value =
        serde_json::from_str(&tool(&home, &home.0, "mem_save", first)).unwrap();
    let refused: serde_json::Value =
        serde_json::from_str(&tool(&home, &home.0, "mem_save", twin)).unwrap();
    assert_eq!(refused["refused"], "duplicate", "{refused}");
    assert_eq!(refused["id"], stored["id"], "{refused}");
    let tiny = r#"{"title":"tiny","body":"ok"}"#;
    let tiny: serde_json::Value =
        serde_json::from_str(&tool(&home, &home.0, "mem_save", tiny)).unwrap();
    assert_eq!(tiny["refused"], "too_short", "{tiny}");
}
#[test]
fn memory_pack_returns_a_tier_and_records() {
    let home = tmp("memory-pack");
    let saved = tool(
        &home,
        &home.0,
        "mem_save",
        r#"{"kind":"decision","title":"hooks","body":"fail open in 10 ms"}"#,
    );
    assert!(saved.contains("id"), "{saved}");
    let packed = tool(
        &home,
        &home.0,
        "mem_pack",
        r#"{"query":"hooks","max_tokens":400}"#,
    );
    assert!(packed.contains("hooks"), "{packed}");
    assert!(
        packed.contains("\tabstract\n")
            || packed.contains("\tfull\n")
            || packed.contains("\toverview\n"),
        "{packed}"
    );
    assert!(
        kinds(&home, "memory").iter().any(|k| k == "mem_pack"),
        "{:?}",
        kinds(&home, "memory")
    );
}
#[test]
fn graph_outline_caps_with_measurement() {
    let home = tmp("graph");
    // T181: an answer under the cap is unchanged and owes no row; only a capped one does.
    std::fs::write(home.0.join("a.rs"), "fn a() {}\n").unwrap();
    let out = tool(&home, &home.0, "outline", r#"{"path":"a.rs"}"#);
    assert!(out.contains("fn a"), "{out}");
    assert!(
        kinds(&home, "graph").is_empty(),
        "{:?}",
        kinds(&home, "graph")
    );
    let big: String = (0..800).map(|i| format!("fn f{i}() {{}}\n")).collect();
    std::fs::write(home.0.join("b.rs"), big).unwrap();
    let out = tool(&home, &home.0, "outline", r#"{"path":"b.rs"}"#);
    assert!(out.contains(" more, expand "), "{out}");
    assert_eq!(kinds(&home, "graph"), ["cap"]);
}
#[test]
fn graph_session_start_map_off_by_default_and_on_when_capped() {
    let home = tmp("graph-map");
    let repo = home.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("a.rs"),
        "fn hot() {}\nfn cold() { hot(); hot(); }\n",
    )
    .unwrap();
    let arg = repo.to_string_lossy().into_owned();
    let _ = run(&home, &["graph", "index", &arg], "", &home.0);
    // Built with `json!`: a Windows path's `\` spliced into a JSON literal is an invalid
    // escape, the hook fails open and prints `{}` (T83.14).
    let input = json!({
        "session_id": "s-map",
        "cwd": repo,
        "hook_event_name": "SessionStart",
        "source": "startup"
    })
    .to_string();
    let off = run(&home, &["hook", "SessionStart"], &input, &home.0);
    let off_v = js(&off);
    let off_ctx = off_v
        .get("hookSpecificOutput")
        .and_then(|v| v.get("additionalContext"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        !off_ctx.contains("repo map"),
        "default map_tokens=0 must not inject: {off}"
    );
    std::fs::write(
        home.0.join("config.toml"),
        "[plugins.graph]\nmap_tokens = 200\n",
    )
    .unwrap();
    let on = run(&home, &["hook", "SessionStart"], &input, &home.0);
    let on_v = js(&on);
    let on_ctx = on_v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or("");
    assert!(on_ctx.contains("repo map"), "{on}");
    assert!(on_ctx.contains("hot"), "{on}");
}
/// T87: Devin's `PostCompaction` must reach the PostCompact plugins after `adapt_devin`
/// renames it. Graph's session-start repo map (source=`compact`) is the signal.
#[test]
fn graph_post_compaction_devin_reaches_post_compact_plugins() {
    let home = tmp("graph-post-compaction-devin");
    let repo = home.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("a.rs"),
        "fn hot() {}\nfn cold() { hot(); hot(); }\n",
    )
    .unwrap();
    let arg = repo.to_string_lossy().into_owned();
    let _ = run(&home, &["graph", "index", &arg], "", &home.0);
    std::fs::write(
        home.0.join("config.toml"),
        "[plugins.graph]\nmap_tokens = 200\n",
    )
    .unwrap();
    let input = json!({
        "session_id": "s-compact-devin",
        "cwd": repo,
        "hook_event_name": "PostCompaction",
        "summary": null
    })
    .to_string();
    let out = run(
        &home,
        &["hook", "PostCompaction", "--host", "devin"],
        &input,
        &home.0,
    );
    let on_v = js(&out);
    let ctx = on_v
        .get("hookSpecificOutput")
        .and_then(|v| v.get("additionalContext"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(ctx.contains("repo map"), "{out}");
    assert!(ctx.contains("hot"), "{out}");
}
/// T87 Check: a live-captured Devin `exec` PreToolUse (2026-09-26, `devin 3000.11.3`)
/// yields the same decision as the equivalent Claude `Bash` payload; an unknown tool is `{}`.
#[test]
fn devin_captured_exec_pre_tool_use_matches_claude_bash() {
    let home = tmp("devin-captured-exec");
    let exec = r#"{"hook_event_name":"PreToolUse","tool_name":"exec","tool_input":{"command":"echo rtok-t87"},"tool_use_id":"call_e53ac12625584a319924ddb5","session_id":"regal-name","prompt_id":"9d55c94c-4032-4f0d-a353-9a19d43d39d0"}"#;
    let bash = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"echo rtok-t87"},"tool_use_id":"call_e53ac12625584a319924ddb5","session_id":"regal-name","prompt_id":"9d55c94c-4032-4f0d-a353-9a19d43d39d0"}"#;
    let unknown = r#"{"hook_event_name":"PreToolUse","tool_name":"not_a_real_tool","tool_input":{},"session_id":"regal-name"}"#;
    let devin = js(&run(
        &home,
        &["hook", "PreToolUse", "--host", "devin"],
        exec,
        &home.0,
    ));
    let claude = js(&run(&home, &["hook", "PreToolUse"], bash, &home.0));
    assert_eq!(devin, claude, "devin={devin} claude={claude}");
    let unk = js(&run(
        &home,
        &["hook", "PreToolUse", "--host", "devin"],
        unknown,
        &home.0,
    ));
    assert_eq!(unk, json!({}));
}
#[test]
fn inject_session_start_records_measurement() {
    let home = tmp("inject");
    let c = home.0.join("config.toml");
    std::fs::write(c, "[plugins.inject]\nmodes=[\"terse\"]\n").unwrap();
    let input = r#"{"session_id":"s-inject","cwd":"/tmp","hook_event_name":"SessionStart","source":"startup"}"#;
    let out = run(&home, &["hook", "SessionStart"], input, &home.0);
    assert!(js(&out).is_object(), "{out}");
    assert!(kinds(&home, "inject").iter().any(|k| k == "inject"));
}
#[test]
fn guard_denies_repeat_bash() {
    let home = tmp("guard");
    let post = r#"{"session_id":"s-guard","cwd":"/tmp","tool_name":"Bash","tool_input":{"command":"cat dup-t382"},"hook_event_name":"PostToolUse","tool_response":{"stdout":"dup-output-body"}}"#;
    run(&home, &["hook", "PostToolUse"], post, &home.0);
    let pre = r#"{"session_id":"s-guard","cwd":"/tmp","tool_name":"Bash","tool_input":{"command":"cat dup-t382"},"hook_event_name":"PreToolUse"}"#;
    let out = run(&home, &["hook", "PreToolUse"], pre, &home.0);
    let d = &js(&out)["hookSpecificOutput"]["permissionDecision"];
    assert!(d == "deny", "{out}");
    assert!(kinds(&home, "guard").iter().any(|k| k == "guard"));
}
/// `cargo test` → Edit → `cargo test` must run again: the second result is new information.
/// A read-only command repeated after an Edit is allowed too (the Edit drops Bash keys).
#[test]
fn guard_allows_repeat_after_edit() {
    let home = tmp("guard-edit");
    let bash = |event: &str, cmd: &str| {
        format!(
            r#"{{"session_id":"s-guard-edit","cwd":"/tmp","tool_name":"Bash","tool_input":{{"command":"{cmd}"}},"hook_event_name":"{event}","tool_response":{{"stdout":"body"}}}}"#
        )
    };
    let decision = |cmd: &str| {
        let out = run(
            &home,
            &["hook", "PreToolUse"],
            &bash("PreToolUse", cmd),
            &home.0,
        );
        js(&out)["hookSpecificOutput"]["permissionDecision"].clone()
    };
    let edit = r#"{"session_id":"s-guard-edit","cwd":"/tmp","tool_name":"Edit","tool_input":{"file_path":"/tmp/x.rs"},"hook_event_name":"PostToolUse","tool_response":{}}"#;
    run(
        &home,
        &["hook", "PostToolUse"],
        &bash("PostToolUse", "cargo test"),
        &home.0,
    );
    run(&home, &["hook", "PostToolUse"], edit, &home.0);
    assert!(
        decision("cargo test").is_null(),
        "cargo test is never a duplicate"
    );
    run(
        &home,
        &["hook", "PostToolUse"],
        &bash("PostToolUse", "cat x.rs"),
        &home.0,
    );
    assert_eq!(decision("cat x.rs"), "deny");
    run(&home, &["hook", "PostToolUse"], edit, &home.0);
    assert!(
        decision("cat x.rs").is_null(),
        "an Edit clears the Bash keys"
    );
}
#[tokio::test]
async fn proxy_passthrough_records_usage() {
    let home = tmp("proxy");
    let (a, st) = serve(&home, "passthrough", |_| {}).await;
    let body = r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"metadata":{"user_id":"s-proxy"}}"#.as_bytes().to_vec();
    assert_eq!(post(&a, body).await, UP);
    for _ in 0..200 {
        if st.store.usage_rows("s-proxy").unwrap().len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(st.store.usage_rows("s-proxy").unwrap().len(), 1);
}
#[tokio::test]
async fn archive_compress_rewrites_old_blocks() {
    let home = tmp("archive");
    let (a, st) = serve(&home, "compress", |_| {}).await;
    let req = turns("s-archive", |t| big(t, "padding"));
    assert_eq!(post(&a, req.clone()).await, UP);
    assert_eq!(post(&a, req).await, UP);
    assert_eq!(st.store.measurement_count("archive").unwrap(), 4);
}
#[tokio::test]
async fn toon_encode_tabular_blocks() {
    let home = tmp("toon");
    let (a, st) = serve(&home, "compress", |c| c.plugins.toon.enabled = true).await;
    let req = turns("s-toon", tab);
    assert_eq!(post(&a, req.clone()).await, UP);
    assert_eq!(post(&a, req).await, UP);
    assert_eq!(st.store.measurement_count("toon").unwrap(), 4);
    assert_eq!(st.store.measurement_count("archive").unwrap(), 0);
}
#[tokio::test]
async fn compress_summary_row_visible_in_stats() {
    let home = tmp("compress");
    let (a, st) = serve(&home, "compress", |c| c.plugins.compress.enabled = true).await;
    let req = turns("s-compress", |t| big(t, &"x".repeat(90)));
    assert_eq!(post(&a, req.clone()).await, UP);
    assert_eq!(post(&a, req).await, UP);
    assert!(st.store.measurement_count("compress").unwrap() > 0);
    let a = ["stats", "--plugin", "compress", "--json"];
    let out = run(&home, &a, "", &home.0);
    let v = js(&out);
    let rows = v["rows"].as_array().unwrap();
    let hit = rows.iter().any(|r| r["kind"] == "summary");
    assert!(hit, "{out}");
}

// T239: `Measurement` rows must match the bytes each surface actually returned, not just
// carry the right `kind`. Each test below recomputes `before`/`after` independently from
// what the surface really sent back (never copied from the row) using the same estimator
// (`rtok::tokens::estimate`) and config rates the production code used.

/// Hook surface: `cmd` has no `PostToolUse` handler (`grep -n "fn post_tool" src/plugins/cmd`
/// finds none) — the whole saving happens at `PreToolUse`, which rewrites `Bash` to
/// `rtok run --` (`src/plugins/cmd/hook.rs`); the host then executes that, and its stdout
/// *is* what `PostToolUse` would see. So this test confirms the rewrite, then runs it.
///
/// Default config: 200 lines pass `trailer_min_lines`, so stdout ends with the
/// `[rtok <id8> · N lines]` trailer, which `after` must count too (T247).
#[test]
fn cmd_hook_measurement_matches_returned_bytes() {
    let home = tmp("cmd-bytes");
    let body: String = (1..=200).map(|i| format!("line {i}\n")).collect();
    let file = home.0.join("big.txt");
    std::fs::write(&file, &body).unwrap();
    // Built with `json!` (not `format!`) so a Windows `C:\...` path serializes as valid
    // JSON — a raw backslash in a hand-written string literal breaks parsing and the hook
    // fails open silently, per `tests/commands_e2e.rs`'s `hook_rewrite_of_sleep_cat_tail_runs_through_rtok`.
    let original = format!("cat {}", file.display());
    let pre = json!({
        "session_id": "s",
        "cwd": home.0,
        "tool_name": "Bash",
        "tool_input": {"command": original},
        "hook_event_name": "PreToolUse",
    })
    .to_string();
    let modified = js(&run(&home, &["hook", "PreToolUse"], &pre, &home.0))["hookSpecificOutput"]
        ["updatedInput"]["command"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(modified.contains("rtok run -- "), "{modified}");
    let out = run(&home, &["run", "cat", file.to_str().unwrap()], "", &home.0);
    assert!(out.contains(" lines]"), "{out}");
    let rows = rows(&home, "cmd");
    let row = rows
        .iter()
        .find(|r| r.kind != "raw")
        .expect("shortened row");
    let cfg = Config::load_from(&home.0).unwrap();
    assert_eq!(row.before_bytes as usize, body.len());
    assert_eq!(
        row.est_before,
        tokens::estimate(&body, Class::Code, &cfg.estimator) as i32
    );
    assert_eq!(row.after_bytes as usize, out.len(), "{row:?}");
    assert_eq!(
        row.est_after,
        tokens::estimate(&out, Class::Code, &cfg.estimator) as i32
    );
    assert!(row.after_bytes <= row.before_bytes, "{row:?}");
}

/// MCP surface: `read` on a large file in `stripped` mode. `src/plugins/read/mod.rs`
/// records `before_bytes`/`after_bytes` from the exact `raw`/`out` strings it returns, so
/// there is no intermediate value to drift from what the tool answers with.
#[test]
fn read_stripped_measurement_matches_returned_text() {
    let home = tmp("read-bytes");
    let mut src = String::new();
    for i in 1..=120 {
        src.push_str(&format!(
            "// comment line {i} filler filler filler filler\n"
        ));
    }
    src.push_str("fn f() { let x = 1; println!(\"{}\", x); }\n");
    std::fs::write(home.0.join("a.rs"), &src).unwrap();
    let out = tool(
        &home,
        &home.0,
        "read",
        r#"{"path":"a.rs","mode":"stripped"}"#,
    );
    assert!(out.len() < src.len(), "{out}");
    let cfg = Config::load_from(&home.0).unwrap();
    let rows = rows(&home, "read");
    let row = rows
        .iter()
        .find(|r| r.kind == "stripped")
        .expect("stripped row");
    assert_eq!(row.before_bytes as usize, src.len());
    assert_eq!(
        row.est_before,
        tokens::estimate(&src, Class::Code, &cfg.estimator) as i32
    );
    assert_eq!(row.after_bytes as usize, out.len());
    assert_eq!(
        row.est_after,
        tokens::estimate(&out, Class::Code, &cfg.estimator) as i32
    );
}

/// T299: `map`, `signatures` (outline) and `search` over one realistic fixture must keep
/// saving at least `floor` percent of the raw file's tokens (same estimator the code uses).
/// Floors are the saving measured at authoring time minus 5 points.
/// T300: `map`/`signatures` now also write exactly one fresh `read` row (`before` the raw
/// fixture, `after` the returned text). `search`'s only "before" is its own full output, and
/// this fixture is small enough that `cap` never truncates it, so it writes none.
#[test]
fn read_modes_keep_a_saving_floor() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/read_modes_sample.rs"
    ))
    .unwrap();
    // (tool, args, floor %, expected `read` row kind): measured 95.1 / 88.8 / 88.9
    // (1350 -> 66 / 151 / 150 tokens).
    let cases = [
        (
            "read",
            r#"{"path":"sample.rs","mode":"map"}"#,
            90.1,
            Some("map"),
        ),
        (
            "read",
            r#"{"path":"sample.rs","mode":"signatures"}"#,
            83.8,
            Some("signatures"),
        ),
        (
            "search",
            r#"{"pattern":"pub fn","path":"sample.rs"}"#,
            83.9,
            None,
        ),
    ];
    for (name, args, floor, expected_kind) in cases {
        let home = tmp("read-modes");
        std::fs::write(home.0.join("sample.rs"), &src).unwrap();
        let out = tool(&home, &home.0, name, args);
        let cfg = Config::load_from(&home.0).unwrap();
        let before = tokens::estimate(&src, Class::Code, &cfg.estimator);
        let after = tokens::estimate(&out, Class::Code, &cfg.estimator);
        let pct = 100.0 * (1.0 - f64::from(after) / f64::from(before));
        assert!(
            pct >= floor,
            "{name} {args}: saved {pct:.1}% < floor {floor}% ({before} -> {after} tokens): {out}"
        );
        let read_rows = rows(&home, "read");
        match expected_kind {
            Some(kind) => {
                let matching: Vec<_> = read_rows.iter().filter(|r| r.kind == kind).collect();
                assert_eq!(matching.len(), 1, "{name} {args}: {read_rows:?}");
                let row = matching[0];
                assert_eq!(row.before_bytes as usize, src.len(), "{row:?}");
                assert_eq!(row.after_bytes as usize, out.len(), "{row:?}");
            }
            None => {
                assert!(
                    read_rows.is_empty(),
                    "{name} {args}: unexpected rows {read_rows:?}"
                );
            }
        }
    }
}

/// T300: `search`/`tree` write a `Measurement` only when `cap` actually cuts their output —
/// a small `max_chars` forces that, and the row's `before` must be the full uncapped text
/// (recovered via the row's `ref_id`, the same archive `cap` made for the returned pointer).
#[test]
fn search_and_tree_cap_measurement_matches_full_and_returned_text() {
    let home = tmp("read-cap");
    std::fs::write(
        home.0.join("config.toml"),
        "[plugins.read]\nmax_chars = 200\n",
    )
    .unwrap();
    let nested = home.0.join("nest");
    std::fs::create_dir_all(&nested).unwrap();
    for i in 0..40 {
        std::fs::write(nested.join(format!("f{i}.rs")), format!("fn f{i}() {{}}\n")).unwrap();
    }

    for (name, args, kind) in [
        ("search", r#"{"pattern":"fn","path":"nest"}"#, "search_cap"),
        ("tree", r#"{"path":"nest"}"#, "tree_cap"),
    ] {
        let out = tool(&home, &home.0, name, args);
        assert!(out.contains("archived"), "{name}: {out}");
        let cfg = Config::load_from(&home.0).unwrap();
        let read_rows = rows(&home, "read");
        let matching: Vec<_> = read_rows.iter().filter(|r| r.kind == kind).collect();
        assert_eq!(matching.len(), 1, "{name}: {read_rows:?}");
        let row = matching[0];
        assert_eq!(row.after_bytes as usize, out.len(), "{row:?}");
        assert_eq!(
            row.est_after,
            tokens::estimate(&out, Class::Code, &cfg.estimator) as i32
        );
        let id = row.ref_id.as_deref().expect("cap archives, so ref_id");
        let full = rtok::store::Store::open(&home.0.join("rtok.db"))
            .unwrap()
            .get_archive(id, None)
            .unwrap()
            .expect("archived full text");
        let full = String::from_utf8(full).unwrap();
        assert_eq!(row.before_bytes as usize, full.len(), "{row:?}");
        assert_eq!(
            row.est_before,
            tokens::estimate(&full, Class::Code, &cfg.estimator) as i32
        );
        assert!(row.after_bytes < row.before_bytes, "{row:?}");
    }
}

/// Proxy surface: `compress` mode archives a `tool_result` older than `keep_turns`
/// (`src/plugins/archive/mod.rs::rewrite_block`), which records `before`/`after` from the
/// same `text`/`live` strings it substitutes into the request — so the row is checked
/// against the bytes actually forwarded to the upstream mock, captured via a custom matcher.
#[tokio::test]
async fn proxy_archive_measurement_matches_forwarded_bytes() {
    let home = tmp("archive-bytes");
    let captured: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let cap = captured.clone();
    let srv = httpmock::MockServer::start();
    srv.mock(|w, t| {
        w.method(httpmock::Method::POST)
            .path("/v1/messages")
            .is_true(move |req: &httpmock::HttpMockRequest| {
                *cap.lock().unwrap() = Some(req.body_vec());
                true
            });
        t.status(200).body(UP);
    });
    let mut cfg = Config::load_from(&home.0).unwrap();
    cfg.proxy.upstream = srv.base_url();
    cfg.proxy.mode = "compress".into();
    // `compress` (on by default) would further shrink the archive pointer into an
    // extractive summary and record its own chained row (src/plugins/compress/mod.rs) —
    // off here so `archive`'s pointer is what actually reaches the upstream mock.
    cfg.plugins.compress.enabled = false;
    let st = Arc::new(ProxyState::new(&cfg).unwrap());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap().to_string();
    tokio::spawn(axum::serve(l, app(st.clone())).into_future());

    // 5 user turns, `keep_turns` (default 4) means only the oldest (turn 4, counted from
    // the end) is eligible; the other 4 are small and stay untouched, so exactly one row.
    let text: String = (1..=600)
        .map(|i| format!("1:{i}:padding"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut ms = vec![user_msg(1, &serde_json::to_string(&text).unwrap())];
    ms.extend((2..=5).map(|t| user_msg(t, "\"ok\"")));
    let req = format!(
        r#"{{"model":"m","messages":[{}],"metadata":{{"user_id":"s-archive-bytes"}}}}"#,
        ms.join(",")
    )
    .into_bytes();
    assert_eq!(post(&a, req).await, UP);

    let db_rows = st.store.list_measurements("archive").unwrap();
    assert_eq!(db_rows.len(), 1, "{db_rows:?}");
    let row = &db_rows[0];
    assert_eq!(row.before_bytes as usize, text.len());
    assert_eq!(
        row.est_before,
        tokens::estimate(&text, Class::Code, &cfg.estimator) as i32
    );
    let body = captured.lock().unwrap().clone().expect("upstream request");
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let live = v["messages"][0]["content"][0]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert_eq!(row.after_bytes as usize, live.len());
    assert_eq!(
        row.est_after,
        tokens::estimate(&live, Class::Code, &cfg.estimator) as i32
    );
    assert!(row.after_bytes <= row.before_bytes, "{row:?}");
}

/// A body below the rule's threshold prints verbatim (`emit_filtered`'s early `raw` return,
/// src/plugins/cmd/run.rs): the row must show zero saving, never `after > before`.
#[test]
fn cmd_run_short_body_records_zero_not_negative_saving() {
    let home = tmp("cmd-short");
    let out = run(&home, &["run", "echo", "hi"], "", &home.0);
    let rows = rows(&home, "cmd");
    let row = rows.last().expect("a row for the short body");
    assert_eq!(row.kind, "raw");
    assert_eq!(row.before_bytes, row.after_bytes, "{row:?}");
    assert_eq!(row.after_bytes as usize, out.len());
    assert!(row.after_bytes <= row.before_bytes);
}

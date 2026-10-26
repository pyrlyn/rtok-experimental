// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T70.5: `rtok guard check` prints the same allow/deny `plugins::guard` returns on PreToolUse.
use serde_json::{Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(n: &str) -> Home {
    let t = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("rtok-t705-{n}-{}-{t}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Home(d)
}

fn run(home: &Home, args: &[&str], stdin: &str) -> (bool, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rtok"));
    c.args(args)
        .env("RTOK_HOME", &home.0)
        .env("HOME", &home.0)
        .current_dir(&home.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    drop(child.stdin.take().unwrap().write_all(stdin.as_bytes()));
    let out = child.wait_with_output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

// No `cwd`, like the pi and OpenCode plugins: `guard check` has none either, and the Bash key
// carries the cwd, so a recording with one would never match the check.
fn post(home: &Home, session: &str, tool: &str, input: &str, stdout: &str) {
    let body = format!(
        r#"{{"session_id":"{session}","tool_name":"{tool}","tool_input":{input},"hook_event_name":"PostToolUse","tool_response":{{"stdout":"{stdout}"}}}}"#
    );
    let (ok, out) = run(home, &["hook", "PostToolUse"], &body);
    assert!(ok, "{out}");
}

fn check(home: &Home, session: &str, tool: &str, input: &str) -> Value {
    let (ok, out) = run(
        home,
        &[
            "guard",
            "check",
            "--tool",
            tool,
            "--json",
            input,
            "--session",
            session,
        ],
        "",
    );
    assert!(ok, "{out}");
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}"))
}

fn kinds(home: &Home) -> Vec<String> {
    let s = rtok::store::Store::open(&home.0.join("rtok.db")).unwrap();
    s.list_measurements("guard")
        .unwrap()
        .into_iter()
        .map(|r| r.kind)
        .collect()
}

#[test]
fn first_call_allows_repeat_denies_with_reason() {
    let home = tmp("deny");
    let input = r#"{"command":"cat dup-t705"}"#;
    assert_eq!(check(&home, "s", "bash", input)["allow"], json!(true));
    post(&home, "s", "Bash", input, "dup-output-body");
    let v = check(&home, "s", "bash", input);
    assert_eq!(v["allow"], json!(false), "{v}");
    let reason = v["reason"].as_str().unwrap_or("");
    assert!(reason.contains("rtok expand"), "{reason}");
    assert!(
        kinds(&home).iter().any(|k| k == "guard"),
        "{:?}",
        kinds(&home)
    );
}

#[test]
fn unparsable_json_allows() {
    let home = tmp("bad-json");
    let v = check(&home, "s", "bash", "not-json");
    assert_eq!(v["allow"], json!(true));
}

#[test]
fn find_delete_allows_the_next_ls() {
    let home = tmp("false-deny");
    let ls = r#"{"command":"ls"}"#;
    post(&home, "s", "Bash", ls, "out");
    assert_eq!(check(&home, "s", "Bash", ls)["allow"], json!(false));
    post(
        &home,
        "s",
        "Bash",
        r#"{"command":"find . -delete"}"#,
        "gone",
    );
    assert_eq!(
        check(&home, "s", "bash", ls)["allow"],
        json!(true),
        "find -delete must drop bash keys so the next ls is allowed"
    );
}

#[test]
fn sed_n_is_keyed_sed_i_is_not() {
    let home = tmp("sed");
    let ro = r#"{"command":"sed -n 1,40p f"}"#;
    post(&home, "s", "Bash", ro, "body");
    assert_eq!(check(&home, "s", "bash", ro)["allow"], json!(false));
    post(&home, "s", "Bash", r#"{"command":"sed -i s/a/b/ f"}"#, "x");
    assert_eq!(
        check(&home, "s", "bash", ro)["allow"],
        json!(true),
        "sed -i must not leave a stale deny on sed -n"
    );
}

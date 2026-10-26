// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T59.4 Check: `rtok mcp -- <server>` is a lossless wrapper. A `sh` fake server answers
//! `tools/list` with a fixed frame, `tools/call` id 2 with a 3000-line text block and id 3
//! with an `isError` result; the wrapper must forward the first and last byte-for-byte,
//! cut the second behind an `expand` trailer, and exit with the server's code.
#![cfg(unix)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const LIST: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"big","inputSchema":{"type":"object"}}]}}"#;
const ERR: &str = r#"{"jsonrpc":"2.0","id":3,"result":{"isError":true,"content":[{"type":"text","text":"boom"}]}}"#;

fn big() -> String {
    let text: String = (1..=3000).map(|i| format!("line {i}\\n")).collect();
    format!(
        r#"{{"jsonrpc":"2.0","id":2,"result":{{"content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

/// A stdio server that replies by request id, in the framing `$FRAMING` names, then exits 3.
fn fake_server(dir: &Path) -> std::path::PathBuf {
    let script = format!(
        r#"#!/bin/sh
LIST='{LIST}'
BIG='{big}'
ERR='{ERR}'
reply() {{
  case "$1" in *'"id":1'*) r="$LIST";; *'"id":2'*) r="$BIG";; *'"id":3'*) r="$ERR";; *) return;; esac
  if [ "$FRAMING" = header ]; then printf 'Content-Length: %d\r\n\r\n%s' "${{#r}}" "$r"; else printf '%s\n' "$r"; fi
}}
if [ "$FRAMING" = header ]; then
  while IFS= read -r h; do n=$(printf '%s' "$h" | tr -dc 0-9); IFS= read -r _blank; reply "$(dd bs=1 count="$n" 2>/dev/null)"; done
else
  while IFS= read -r line; do reply "$line"; done
fi
exit 3
"#,
        big = big()
    );
    let path = dir.join("fake-server.sh");
    std::fs::write(&path, script).unwrap();
    path
}

/// A stdio server (T194) that emits one broken header block — `Content-Length: 99` but
/// only ~2/3 of that many bytes actually follow before it exits, EOF-ing mid-body — then
/// exits. The declared length swallows a well-formed frame that follows right behind the
/// short body, since nothing marks a frame boundary inside a body of the wrong length.
fn malformed_server(dir: &Path, valid: &str) -> std::path::PathBuf {
    let script = format!(
        r#"#!/bin/sh
printf 'content-length: 99\r\n\r\n{{}}'
printf 'Content-Length: %d\r\n\r\n%s' {vlen} '{valid}'
"#,
        vlen = valid.len(),
    );
    let path = dir.join("malformed-server.sh");
    std::fs::write(&path, script).unwrap();
    path
}

fn wrap(home: &Path, server: &Path, framing: &str, input: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["mcp", "--", "sh"])
        .arg(server)
        .env("RTOK_HOME", home)
        .env("FRAMING", framing)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp --");
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn expand(home: &Path, id: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["expand", id])
        .env("RTOK_HOME", home)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn trailer_id(text: &str) -> &str {
    let tail = text.rsplit("[rtok ").next().expect("trailer");
    tail.split(' ').next().unwrap()
}

fn check(short: &str, home: &Path) {
    let v: serde_json::Value = serde_json::from_str(short).unwrap();
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.lines().count() < 40, "not cut:\n{text}");
    assert!(text.starts_with("line 1\n"), "{text}");
    assert!(
        text.contains("· 3000 lines · expand: rtok expand "),
        "{text}"
    );
    let raw = expand(home, trailer_id(text));
    assert_eq!(raw.lines().count(), 3000, "expand returned {raw:.80}");
    assert!(raw.contains("line 2999\n"));
}

#[test]
fn newline_framing_cuts_long_results_and_forwards_the_rest_verbatim() {
    let home = std::env::temp_dir().join(format!("rtok-wrap-line-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let server = fake_server(&home);
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"big"}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"big"}}"#,
        "\n"
    );
    let out = wrap(&home, &server, "line", input.as_bytes());
    assert_eq!(out.status.code(), Some(3), "server exit code is propagated");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout}");
    assert_eq!(lines[0], LIST, "tools/list is byte-identical");
    assert_eq!(lines[2], ERR, "isError results are untouched");
    assert!(lines[1].len() < big().len() / 4, "{}", lines[1]);
    check(lines[1], &home);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn header_framing_is_kept_and_cut_the_same_way() {
    let home = std::env::temp_dir().join(format!("rtok-wrap-hdr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let server = fake_server(&home);
    let mut input = Vec::new();
    for body in [
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"big"}}"#,
    ] {
        write!(input, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    }
    let out = wrap(&home, &server, "header", &input);
    assert_eq!(out.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let first = format!(
        "Content-Length: {}\r\n\r\n{LIST}Content-Length: ",
        LIST.len()
    );
    assert!(stdout.starts_with(&first), "{stdout:.200}");
    let (head, body) = stdout[first.len()..].split_once("\r\n\r\n").unwrap();
    assert_eq!(head.parse::<usize>().unwrap(), body.len(), "{stdout:.200}");
    check(body, &home);
    let _ = std::fs::remove_dir_all(&home);
}

/// T194 Check: `content-length: 99\r\n\r\n{}` (short body) followed by a valid frame — the
/// wrapper must not stop forwarding at the broken header. The malformed header plus every
/// byte the server actually sent (the short body and the frame behind it, both swallowed
/// by the wrong declared length) reach stdout unchanged, byte-for-byte.
#[test]
fn short_body_is_forwarded_unchanged_instead_of_ending_the_pipe() {
    let home = std::env::temp_dir().join(format!("rtok-wrap-short-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let valid = r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#;
    let server = malformed_server(&home, valid);
    let out = wrap(&home, &server, "n/a", b"");
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let want = format!(
        "content-length: 99\r\n\r\n{{}}Content-Length: {}\r\n\r\n{valid}",
        valid.len()
    );
    assert_eq!(stdout, want, "malformed bytes must forward byte-for-byte");
    assert!(
        stdout.contains(valid),
        "the valid frame's bytes must still reach stdout: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// T366: a server killed by a signal reports `128 + signal`, as a shell would, not `1`; a
/// plain non-zero exit keeps its own code. The `kill` targets the test's own `sh` (`$$`).
#[test]
fn a_server_killed_by_a_signal_exits_128_plus_the_signal() {
    let home = std::env::temp_dir().join(format!("rtok-wrap-sig-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    for (body, code) in [
        ("kill -TERM $$", 143),
        ("kill -KILL $$", 137),
        ("exit 5", 5),
    ] {
        let server = home.join("sig-server.sh");
        std::fs::write(&server, format!("#!/bin/sh\n{body}\n")).unwrap();
        let out = wrap(&home, &server, "line", b"");
        assert_eq!(out.status.code(), Some(code), "{body}");
    }
    let _ = std::fs::remove_dir_all(&home);
}

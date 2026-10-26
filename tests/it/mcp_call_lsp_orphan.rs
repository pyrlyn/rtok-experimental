// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T142: `rtok mcp --call` used to never shut the cached LSP session down, unlike `run()`
//! (which shuts it down at stdin EOF) — every `--call` of a graph tool under
//! `graph.backend = "lsp"` left rust-analyzer/clangd/… running as an orphan after `rtok`
//! exited, because the session lives in a `static` that is never dropped on its own.
//!
//! Uses a fake `clangd` (`tests/common/fake_lsp.rs`, unix-only) first on `PATH`, so the
//! whole thing runs in milliseconds instead of waiting out `lsp.rs`'s 40s readiness poll.
//! Liveness is checked through a lock file the fake holds (see that module's docs) — never
//! through `ps`/`kill -0`/`kill` on a pid, which the creator has banned for these tests.
#![cfg(unix)]

use crate::it::common;

use common::fake_lsp;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// A fresh `RTOK_HOME` with `graph.backend = "lsp"` and a workspace `clangd` will pick
/// (`compile_commands.json`, no `Cargo.toml`) containing one `main.c`. Returns
/// `(home, workspace, bin_dir, file_uri)`.
fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf, String) {
    let root = std::env::temp_dir().join(format!("rtok-lsp-orphan-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let workspace = root.join("workspace");
    let bin_dir = root.join("bin");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "[plugins.graph]\nbackend = \"lsp\"\n",
    )
    .unwrap();
    std::fs::write(workspace.join("compile_commands.json"), "[]").unwrap();
    let main_c = workspace.join("main.c");
    std::fs::write(&main_c, "int main() {\n    return 0;\n}\n").unwrap();
    fake_lsp::write_fake_clangd(&bin_dir);
    let abs_main_c = std::fs::canonicalize(&main_c).unwrap();
    let file_uri = format!("file://{}", abs_main_c.display());
    (home, workspace, bin_dir, file_uri)
}

fn path_with(bin_dir: &Path) -> String {
    format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

/// `Command::new(rtok)` pre-wired with the isolated home/workspace/PATH/lock-file env this
/// whole test file shares.
fn rtok_cmd(
    home: &Path,
    workspace: &Path,
    bin_dir: &Path,
    file_uri: &str,
    lock_path: &Path,
) -> Command {
    let mut cmd = Command::new(crate::it::common::bin_path("rtok"));
    cmd.env("RTOK_HOME", home)
        .env("PATH", path_with(bin_dir))
        .env("FAKE_LSP_LOCK_FILE", lock_path)
        .env("FAKE_LSP_FILE_URI", file_uri)
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

#[test]
fn call_shuts_down_lsp_after_a_successful_tool_call() {
    let (home, workspace, bin_dir, file_uri) = setup("ok");
    let lock_path = fake_lsp::lock_path(&home);
    let mut child = rtok_cmd(&home, &workspace, &bin_dir, &file_uri, &lock_path)
        .arg("mcp")
        .arg("--call")
        .arg("symbol")
        .arg("--json")
        .arg(r#"{"name":"Widget"}"#)
        .spawn()
        .expect("spawn rtok mcp --call");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "stdout {} stderr {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        fake_lsp::wait_for_fake_death(&lock_path, Duration::from_secs(2)),
        "fake LSP server still held its lock 2s after `rtok mcp --call` exited"
    );
    let _ = std::fs::remove_dir_all(home.parent().unwrap());
}

/// Same shutdown guarantee on the error path: the LSP session is already up (past
/// `initialize`) when the server errors, which is exactly the case `call()`'s
/// early `?`/`Err` returns must still cover.
#[test]
fn call_shuts_down_lsp_after_a_tool_call_that_errors() {
    let (home, workspace, bin_dir, file_uri) = setup("err");
    let lock_path = fake_lsp::lock_path(&home);
    let mut child = rtok_cmd(&home, &workspace, &bin_dir, &file_uri, &lock_path)
        .arg("mcp")
        .arg("--call")
        .arg("symbol")
        .arg("--json")
        .arg(r#"{"name":"BoomTrigger"}"#)
        .spawn()
        .expect("spawn rtok mcp --call");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait");
    // T376: a server error no longer fails the call; it falls back to the tags answer and
    // names the reason. The session was still torn down on the way, which is the point here.
    assert!(out.status.success(), "expected the tags fallback to answer");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("(tags; lsp:") && stdout.contains("boom"),
        "stdout {stdout}"
    );

    assert!(
        fake_lsp::wait_for_fake_death(&lock_path, Duration::from_secs(2)),
        "fake LSP server still held its lock 2s after a fallen-back `rtok mcp --call` exited"
    );
    let _ = std::fs::remove_dir_all(home.parent().unwrap());
}

/// Regression guard for `run()`'s own shutdown-at-EOF path (already correct before T142,
/// but nothing here pinned it against a real spawned child): the stdio `rtok mcp` server
/// must reap the fake LSP too once stdin closes.
#[test]
fn run_stdio_server_shuts_down_lsp_at_stdin_eof() {
    let (home, workspace, bin_dir, file_uri) = setup("run-eof");
    let lock_path = fake_lsp::lock_path(&home);
    let mut child = rtok_cmd(&home, &workspace, &bin_dir, &file_uri, &lock_path)
        .arg("mcp")
        .spawn()
        .expect("spawn rtok mcp");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        let init = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}
        });
        let call = serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "symbol", "arguments": {"name": "Widget"}}
        });
        writeln!(stdin, "{init}").unwrap();
        writeln!(stdin, "{call}").unwrap();
        // Dropping `stdin` here closes it (EOF), which is what must trigger `run()`'s
        // shutdown of the cached LSP session.
    }
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "stdout {} stderr {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        fake_lsp::wait_for_fake_death(&lock_path, Duration::from_secs(2)),
        "fake LSP server still held its lock 2s after `rtok mcp` exited at stdin EOF"
    );
    let _ = std::fs::remove_dir_all(home.parent().unwrap());
}

// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Token floors on real captured command output (rtok#1071 harness, rtok#1077).
//!
//! Each `tests/fixtures/bench/<case>/raw.txt` is what the command printed (stderr, then
//! stdout) in the clean bench container at `fixture_commit`; `case.toml` holds the argv,
//! the exit code and `min_savings_pct`. The built binary filters the bytes through the
//! same path as `rtok run` (`rtok filter --archive`), so no cargo/git/ls runs here, no
//! Docker, no network. Tokens are o200k_base, the tokenizer the harness counts with.
//! Regenerate with `python3 bench/harness/run.py rtk --refresh-fixtures <dir>` in #1071.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(serde::Deserialize)]
struct Case {
    argv: Vec<String>,
    exit: i32,
    min_savings_pct: f64,
}

fn filtered(home: &Path, case: &Case, raw: &[u8], no_ref: bool) -> Vec<u8> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtok"));
    cmd.args(["filter", "--archive", "--cmd", &case.argv.join(" ")])
        .args(["--exit", &case.exit.to_string()])
        .env("RTOK_HOME", home)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if no_ref {
        cmd.arg("--no-ref");
    }
    let mut child = cmd.spawn().expect("spawn rtok");
    child.stdin.take().unwrap().write_all(raw).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "rtok filter failed");
    out.stdout
}

#[test]
#[allow(clippy::print_stderr, reason = "per-case savings table")]
fn bench_fixtures_meet_their_floors() {
    let bpe = tiktoken_rs::o200k_base().expect("o200k_base");
    let tok = |b: &[u8]| {
        bpe.encode_with_special_tokens(&String::from_utf8_lossy(b))
            .len()
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bench");
    let mut dirs: Vec<_> = std::fs::read_dir(&root)
        .expect("tests/fixtures/bench")
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("case.toml").is_file())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no bench fixtures");
    let home = std::env::temp_dir().join(format!("rtok-bench-fixtures-{}", std::process::id()));
    let mut fails = Vec::new();
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let case: Case = toml::from_str(&std::fs::read_to_string(dir.join("case.toml")).unwrap())
            .unwrap_or_else(|e| panic!("{name}/case.toml: {e}"));
        let raw = std::fs::read(dir.join("raw.txt")).unwrap();
        let raw_t = tok(&raw);
        for no_ref in [false, true] {
            let _ = std::fs::remove_dir_all(&home);
            std::fs::create_dir_all(&home).unwrap();
            let out_t = tok(&filtered(&home, &case, &raw, no_ref));
            let pct = if raw_t == 0 {
                0.0
            } else {
                (raw_t as f64 - out_t as f64) * 100.0 / raw_t as f64
            };
            let mode = if no_ref { "--no-ref" } else { "default" };
            eprintln!("{name:26} {mode:9} raw {raw_t:5} rtok {out_t:5} save {pct:6.1}%");
            if out_t > raw_t {
                fails.push(format!("{name} ({mode}): {out_t} tokens > raw {raw_t}"));
            } else if !no_ref && pct + 1e-9 < case.min_savings_pct {
                fails.push(format!(
                    "{name}: {pct:.1}% < floor {}%",
                    case.min_savings_pct
                ));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        fails.is_empty(),
        "bench fixture floors:\n{}",
        fails.join("\n")
    );
}

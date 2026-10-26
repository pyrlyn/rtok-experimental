// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T50.2: `rules.d` drop-ins merge after the user file, a broken drop-in is
//! skipped at runtime (fail open) and named by `rtok config validate`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t502-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A home whose `config.toml` points `rules` + `rules_dir` at fixtures under it.
fn seed(home: &Path) -> (PathBuf, PathBuf) {
    let rules = home.join("rules.toml");
    let dir = home.join("rules.d");
    fs::create_dir_all(&dir).unwrap();
    let h = home.display().to_string().replace('\\', "/");
    fs::write(
        home.join("config.toml"),
        format!("[plugins.cmd]\nrules = \"{h}/rules.toml\"\nrules_dir = \"{h}/rules.d\"\n"),
    )
    .unwrap();
    (rules, dir)
}

fn validate(home: &Path, cfg: &Path) -> std::process::Output {
    Command::new(bin())
        .args(["--config", cfg.to_str().unwrap(), "config", "validate"])
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .output()
        .unwrap()
}

#[test]
fn validate_names_a_broken_drop_in_and_passes_once_fixed() {
    let h = home("validate");
    let (rules, dir) = seed(&h);
    let cfg = h.join("config.toml");
    fs::write(&rules, "[echo]\nmax_lines = 5\n").unwrap();
    fs::write(dir.join("bad.toml"), "[echo]\nmax_lines = \"many\"\n").unwrap();

    let out = validate(&h, &cfg);
    assert!(!out.status.success(), "a broken drop-in fails validate");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("bad.toml"), "{err}");
    assert!(err.contains("max_lines"), "{err}");

    fs::remove_file(dir.join("bad.toml")).unwrap();
    let out = validate(&h, &cfg);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ok"),
        "fixed validates"
    );
    let _ = fs::remove_dir_all(&h);
}

/// The drop-in wins over the user file at runtime, and a broken sibling does
/// not stop it (fail open).
#[test]
fn filter_applies_drop_ins_in_order_and_skips_broken_ones() {
    let h = home("runtime");
    let (rules, dir) = seed(&h);
    fs::write(
        &rules,
        "[echo]\nmax_lines = 5\nhead = 5\ntail = 0\ndedupe = false\n",
    )
    .unwrap();
    fs::write(
        dir.join("echo.toml"),
        "[echo]\nmax_lines = 3\nhead = 1\ntail = 1\ndedupe = false\n",
    )
    .unwrap();
    fs::write(dir.join("bad.toml"), "[echo\nmax_lines = \n").unwrap();

    let body = (0..100)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut child = Command::new(bin())
        .args(["filter", "--cmd", "echo"])
        .env("RTOK_HOME", &h)
        .env("HOME", &h)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "a broken sibling must not fail the run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.lines().count() <= 4,
        "the drop-in's max_lines=3 wins:\n{s}"
    );
    assert!(s.contains("line 0") && s.contains("line 99"), "{s}");
    let _ = fs::remove_dir_all(&h);
}

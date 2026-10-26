// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T329.31: `rtok graph export --format svg|png` through the binary: the picture of a project's
//! scope, the same picture from a saved JSON, PNG at 1x, 2x and 4x, and the refused combinations.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t32931-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let project = dir.join("proj");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("lib.rs"),
        "pub fn caller() {\n    callee();\n}\n\npub fn callee() {}\n\npub struct Holder;\n",
    )
    .unwrap();
    fs_kit::path::canonicalize(&dir).unwrap()
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["graph", "export"])
        .args(args)
        .current_dir(home.join("proj"))
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .output()
        .expect("rtok runs")
}

fn ok(home: &Path, args: &[&str]) -> String {
    let out = run(home, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn png_size(file: &Path) -> (u32, u32) {
    let png = fs::read(file).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let be = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
    (be(16), be(20))
}

#[test]
fn svg_draws_the_scope_and_a_saved_json_draws_the_same_picture() {
    let home = fixture("svg");
    let svg = ok(&home, &["--level", "symbols", "--format", "svg"]);
    assert!(svg.starts_with("<svg"), "{svg}");
    for want in ["proj", "caller", "callee", "backend ", "rtok "] {
        assert!(svg.contains(want), "missing {want:?}");
    }
    let json = home.join("graph.json");
    ok(&home, &["--level", "symbols", "-o", json.to_str().unwrap()]);
    let again = ok(
        &home,
        &["--from", json.to_str().unwrap(), "--format", "svg"],
    );
    let twice = ok(
        &home,
        &["--from", json.to_str().unwrap(), "--format", "svg"],
    );
    assert_eq!(again, twice, "a saved export draws deterministically");
    assert!(again.contains("caller") && again.contains("callee"));
}

#[test]
fn png_is_written_at_1x_2x_and_4x() {
    let home = fixture("png");
    let mut sizes = Vec::new();
    for scale in ["1", "2", "4"] {
        let file = home.join(format!("g{scale}.png"));
        ok(
            &home,
            &[
                "--level",
                "symbols",
                "--format",
                "png",
                "--scale",
                scale,
                "-o",
                file.to_str().unwrap(),
            ],
        );
        sizes.push(png_size(&file));
    }
    assert_eq!((sizes[1].0, sizes[1].1), (sizes[0].0 * 2, sizes[0].1 * 2));
    assert_eq!((sizes[2].0, sizes[2].1), (sizes[0].0 * 4, sizes[0].1 * 4));
}

#[test]
fn refused_combinations_say_why() {
    let home = fixture("refused");
    let bare = run(&home, &["--format", "png"]);
    assert!(!bare.status.success());
    assert!(
        String::from_utf8_lossy(&bare.stderr).contains("--output"),
        "a PNG is never printed to a terminal"
    );
    let scaled = run(&home, &["--format", "svg", "--scale", "2"]);
    assert!(!scaled.status.success());
    assert!(String::from_utf8_lossy(&scaled.stderr).contains("--scale is for --format png"));
}

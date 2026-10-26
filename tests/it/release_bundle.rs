// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T81/T111/T310.9: the web UI ships inside the release binary, and the places that say so
//! stay in step. Pure file reads — the release itself is built in CI, but a dropped
//! embed guard, a hand-edited `release.yml` or a SPA build that stops feeding `build.rs` is
//! caught here.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::{Path, PathBuf};

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    let path = repo(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// A ketch install keeps only the executable, so the SPA is compiled in. The release job must
/// demand it — otherwise `build.rs` quietly embeds the "UI not built" placeholder.
#[test]
fn the_release_build_requires_the_embedded_spa() {
    let build = read("build.rs");
    assert!(
        build.contains(r#"Ok("require")"#) && build.contains("web/dist"),
        "build.rs no longer embeds web/dist or no longer honours RTOK_WEB_EMBED=require"
    );
    for rel in [".github/build-setup.yml", ".github/workflows/release.yml"] {
        assert!(
            read(rel).contains("RTOK_WEB_EMBED=require"),
            "{rel} no longer exports RTOK_WEB_EMBED=require before dist build"
        );
    }
    let manifest = read("Cargo.toml");
    assert!(
        !manifest.contains(r#""web/dist/""#),
        "the archive would carry a second copy of the embedded SPA"
    );
}

/// The SPA is built before the first cargo step of the release job, in every matrix job; the
/// `require` flag is exported by the same step, so a failed build cannot fall through to a
/// placeholder release.
#[test]
fn the_release_job_builds_the_spa_before_cargo() {
    for rel in [".github/build-setup.yml", ".github/workflows/release.yml"] {
        let text = read(rel);
        let spa = text
            .find("npm --prefix web run build")
            .unwrap_or_else(|| panic!("{rel} no longer builds the SPA"));
        assert!(
            text.contains("npm --prefix web ci"),
            "{rel} builds the SPA without a locked install"
        );
        let cargo = text
            .find("cargo build --locked")
            .unwrap_or_else(|| panic!("{rel}: no cargo build step"));
        assert!(spa < cargo, "{rel} builds the SPA after cargo has started");
    }
    let setup = read(".github/build-setup.yml");
    assert!(
        setup.contains("install_args: rust node"),
        "the release job no longer installs node from mise.toml"
    );
}

/// CI builds the SPA before it archives the test binaries that embed it, so the shards that run
/// them see the SPA and not the placeholder.
#[test]
fn ci_builds_the_spa_before_the_test_suite() {
    let ci = read(".github/workflows/ci.yml");
    let archive = ci
        .find("run: mise exec -- cargo nextest archive")
        .expect("ci archives the test binaries");
    let job = ci[..archive]
        .rfind("\n  build-tests:")
        .expect("the build-tests job archives them");
    let spa = ci[..archive]
        .rfind("just spa-install spa-build")
        .expect("ci builds the SPA");
    assert!(
        spa > job,
        "build-tests archives the tests without building the SPA first"
    );
}

/// `release.yml` is generated from `dist-workspace.toml` + `build-setup.yml`
/// (`just dist-generate`); a change to the setup that was never regenerated would
/// leave CI running the old steps.
#[test]
fn the_generated_workflow_is_in_step_with_build_setup() {
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains("npm --prefix web run build") && release.contains("\"rust node\""),
        "run `just dist-generate` and commit .github/workflows/release.yml"
    );
}

/// `vite build` writes `web/dist`, which `build.rs` embeds; `precompress.mjs` runs after it so
/// the `.br`/`.gz` files are in the same directory.
#[test]
fn the_spa_build_writes_where_build_rs_embeds_it() {
    let package = read("web/package.json");
    let build = package
        .lines()
        .find(|l| l.contains(r#""build":"#))
        .expect("build script in web/package.json");
    assert!(
        build.find("vite build") < build.find("scripts/precompress.mjs")
            && build.contains("scripts/precompress.mjs"),
        "npm run build must precompress after vite build: {build}"
    );
    assert!(
        read("web/vite.config.ts").contains(r#"outDir: "dist""#),
        "vite no longer builds into web/dist"
    );
    let dist = repo("web/dist");
    if !Path::new(&dist).is_dir() {
        eprintln!("skip: no SPA built — run `just spa-build`");
        return;
    }
    assert!(
        dist.join("index.html").is_file(),
        "index.html missing from web/dist"
    );
    assert!(
        dist.join("index.html.br").is_file() && dist.join("index.html.gz").is_file(),
        "web/dist lacks the precompressed index"
    );
}

/// The CSP forbids inline scripts, so the pre-paint theme script must stay a file.
#[test]
fn the_spa_index_has_no_inline_script() {
    let index = read("web/index.html");
    for tag in index.split("<script").skip(1) {
        let open = tag.split('>').next().unwrap_or("");
        assert!(
            open.contains("src="),
            "web/index.html has an inline <script>, which the CSP in src/web/spa.rs blocks"
        );
    }
}

/// T319: every archive carries share/ — dist `include`s it and the release job generates it
/// with tools/share-files.sh before `dist build` (it is not committed).
#[test]
fn the_release_job_generates_the_share_dir_dist_includes() {
    assert!(
        read("Cargo.toml").contains(r#""share/"]"#),
        "[package.metadata.dist] include no longer lists share/"
    );
    for rel in [".github/build-setup.yml", ".github/workflows/release.yml"] {
        assert!(
            read(rel).contains("tools/share-files.sh"),
            "{rel} no longer generates share/ before dist build"
        );
    }
    assert!(
        read(".gitignore").contains("\n/share/\n"),
        "share/ must stay out of git"
    );
}

/// T319: the script writes every man page and one completion script per shell.
#[cfg(unix)]
#[test]
fn share_files_writes_man_pages_and_every_completion_script() {
    let out = std::env::temp_dir().join(format!("rtok-share-{}", std::process::id()));
    let status = std::process::Command::new("bash")
        .arg(repo("tools/share-files.sh"))
        .arg(env!("CARGO_BIN_EXE_rtok"))
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    for name in [
        "rtok.bash",
        "_rtok",
        "rtok.fish",
        "rtok.ps1",
        "rtok.elv",
        "rtok.lua",
    ] {
        let body = std::fs::read_to_string(out.join("completions").join(name)).unwrap();
        assert!(body.contains("rtok"), "{name} is empty or unrelated");
    }
    let man = out.join("man/man1");
    for page in ["rtok.1", "rtok-agents.1", "rtok-completions.1"] {
        assert!(man.join(page).is_file(), "missing {page}");
    }
    let _ = std::fs::remove_dir_all(&out);
}

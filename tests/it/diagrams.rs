// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T959: every `docs/architecture/*.d2` has a fresh `.svg` beside it, rendered by `d2-render`'s
//! native backend. Freshness is the source-hash stamp the renderer writes into the SVG, so the
//! check does not depend on font metrics or platform. `RTOK_DIAGRAMS=overwrite` (`just diagrams`)
//! re-renders the stale ones and fails on any construct the native backend would not draw, so a
//! committed picture never silently drops part of its source.

use d2_render::{Format, RenderOptions, Renderer};
use std::path::{Path, PathBuf};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/architecture")
}

fn renderer() -> Renderer {
    Renderer::native().options(RenderOptions::new().theme(0).pad(24).stamp(true))
}

#[test]
fn architecture_diagrams_are_fresh() {
    let r = renderer();
    if std::env::var("RTOK_DIAGRAMS").as_deref() == Ok("overwrite") {
        let reports = r
            .refresh_dir(&dir(), Format::Svg)
            .expect("render docs/architecture");
        let warned: Vec<String> = reports
            .iter()
            .flat_map(|rep| {
                rep.warnings
                    .iter()
                    .map(move |w| format!("{}: {w}", rep.input.display()))
            })
            .collect();
        assert!(
            warned.is_empty(),
            "the native backend would drop:\n{}",
            warned.join("\n")
        );
        return;
    }
    let stale = r
        .verify_dir(&dir(), Format::Svg)
        .expect("verify docs/architecture");
    let names: Vec<String> = stale.iter().map(|s| s.to_string()).collect();
    assert!(
        names.is_empty(),
        "stale diagrams (run `just diagrams`):\n{}",
        names.join("\n")
    );
}

#[test]
fn every_diagram_has_its_page() {
    let sources = d2_render::fresh::find_sources(&dir()).expect("docs/architecture");
    assert!(!sources.is_empty(), "no .d2 under docs/architecture");
    let missing: Vec<String> = sources
        .iter()
        .filter(|p| !p.with_extension("md").is_file())
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "diagram without a .md page:\n{}",
        missing.join("\n")
    );
}

/// CONTRIBUTING: every English doc ships with its `docs/ru/` and `docs/uk/` translation.
#[test]
fn every_page_is_translated() {
    let root = dir();
    let docs = root.parent().expect("docs/");
    let mut pages: Vec<PathBuf> = d2_render::fresh::find_sources(&root)
        .expect("docs/architecture")
        .into_iter()
        .map(|p| p.with_extension("md"))
        .collect();
    pages.push(root.join("README.md"));
    let missing: Vec<String> = pages
        .iter()
        .flat_map(|p| {
            let rel = p.strip_prefix(docs).expect("under docs/").to_path_buf();
            ["ru", "uk"].map(move |lang| Path::new(lang).join(&rel))
        })
        .filter(|rel| !docs.join(rel).is_file())
        .map(|rel| format!("docs/{}", rel.display()))
        .collect();
    assert!(
        missing.is_empty(),
        "untranslated architecture pages:\n{}",
        missing.join("\n")
    );
}

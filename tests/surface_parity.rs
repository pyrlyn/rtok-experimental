// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! D23's gate: `rtok web` and `rtok tui` are two renderings of one operator model, so
//! a page that exists on one surface and not the other is a defect. Each surface
//! contributes its own set here and the assert fails naming the page that drifted;
//! when `rtok tui` lands (T15.1+) its tabs join the same compare.
//!
//! D27's gate (T15.12) is the second test: every command `Cli::command()` offers is
//! either a reading command that renders a page the model offers, or exempt with a
//! reason. A command in neither list fails by name, so a reading command cannot land
//! without its page and no new command can land unclassified.

use clap::{Command, CommandFactory};
use rtok::cli::Cli;
use rtok::config::Config;
use rtok::testutil::{config_in, tmp_dir};
use rtok::web::frame;
use rtok::web::model;

fn config() -> Config {
    config_in(&tmp_dir("parity"))
}

/// The pages the model offers, by name.
fn model_pages() -> Vec<String> {
    let mut names: Vec<String> = model::pages()
        .iter()
        .map(|(page, _)| page.to_string())
        .collect();
    names.sort();
    names
}

/// The pages the web surface serves: the keys of the frame `/ws` sends, envelope
/// dropped and each key read back to its page name. A key with no entry in the
/// model's table is its own page, so a web-only page names itself in the failure.
fn web_pages(cfg: &Config) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(&frame(cfg)).expect("frame is json");
    let mut names: Vec<String> = v
        .as_object()
        .expect("frame is an object")
        .keys()
        // Envelope, the T60.4 expand map, the T329.12 registry rows (data of the graph page) and
        // the T330.7 junk card (data of the hosts page), not pages of their own.
        .filter(|key| !["type", "ref_ids", "projects", "junk"].contains(&key.as_str()))
        .map(|key| {
            model::pages()
                .iter()
                .find(|(_, wire_key)| *wire_key == key.as_str())
                .map_or_else(|| key.clone(), |(page, _)| page.to_string())
        })
        .collect();
    names.sort();
    names
}

#[test]
fn web_serves_exactly_the_pages_the_model_offers() {
    let cfg = config();
    assert_eq!(
        model_pages(),
        web_pages(&cfg),
        "a page exists on one surface and not the other (D23)"
    );
}

/// T310.4: the React SPA's route tree, nav and tab bar are `PAGES` in `web/src/pages.ts`, a
/// TypeScript copy of `model::pages()`. Both the id and the snapshot field must match, in
/// order, so a page added to the model cannot be missing from the SPA.
#[test]
fn spa_page_list_matches_the_model() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/src/pages.ts"),
    )
    .expect("web/src/pages.ts is readable");
    let re = regex::Regex::new(r#"\{ id: "([^"]+)", field: "([^"]+)" \}"#).unwrap();
    let spa: Vec<(String, String)> = re
        .captures_iter(&src)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect();
    let model: Vec<(String, String)> = model::pages()
        .iter()
        .map(|(page, field)| (page.to_string(), field.to_string()))
        .collect();
    assert_eq!(
        spa, model,
        "web/src/pages.ts PAGES drifted from model::pages() (id, snapshot field, in order)"
    );
}

/// T60.10: the TUI's page match has no placeholder fallback any more, so a model
/// page without a TUI body names itself here. The check reads the match arms out of
/// `src/tui/view.rs` — the same source-scan shape the WASM `PAGE_IDS` check uses.
#[test]
fn every_model_page_has_a_tui_body() {
    let view = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui/view.rs"),
    )
    .expect("src/tui/view.rs is readable");
    for (page, _) in model::pages() {
        assert!(
            view.contains(&format!("\"{page}\" =>")),
            "page `{page}` has no TUI body in src/tui/view.rs — the placeholder that used to \
             hide the gap is gone (T60.10), so the page must render for real"
        );
    }
}

/// T60.3: the TUI renders session drill-down from the model's accessor (D23).
#[test]
fn session_detail_exists_on_both_surfaces() {
    let model = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/model/mod.rs"));
    let tui = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tui/view.rs"));
    assert!(
        model.contains("pub fn session_detail"),
        "the one accessor lives on the model (D23)"
    );
    assert!(
        tui.contains("model::session_detail"),
        "the TUI renders model::session_detail"
    );
}

/// The sources every "both surfaces" case reads: the model, the TUI view and the TUI app.
struct Surfaces {
    model: &'static str,
    tui: &'static str,
    app: &'static str,
}

const SURFACES: Surfaces = Surfaces {
    model: include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/model/mod.rs")),
    tui: include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tui/view.rs")),
    app: include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tui/app.rs")),
};

/// T60.4: both surfaces render archive expand from the same model accessor.
#[test]
fn expand_payload_exists_on_both_surfaces() {
    let Surfaces { model, tui, app } = SURFACES;
    let inbound = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/web/mod.rs"));
    assert!(
        model.contains("pub fn expand_payload"),
        "the one accessor lives on the model (D23)"
    );
    assert!(
        tui.contains("expand_pane") && app.contains("open_expand"),
        "the TUI opens model::expand_payload"
    );
    assert!(
        inbound.contains("ClientMessage::Expand") && inbound.contains("expand_payload"),
        "web inbound answers expand through expand_payload"
    );
}

/// T63.1: both surfaces render the skills page from the same model accessor.
#[test]
fn skills_page_exists_on_both_surfaces() {
    let Surfaces { model, tui, app } = SURFACES;
    assert!(
        model.contains("pub fn skills_from"),
        "the one accessor lives on the model (D23)"
    );
    assert!(
        model.contains("(\"skills\", \"skills\")"),
        "pages() offers skills"
    );
    assert!(
        tui.contains("\"skills\" =>") && app.contains("skills_key"),
        "the TUI renders the skills page"
    );
}

/// T260: the TUI Sessions tab filters to live rows with `l` (KEYS entry in
/// `src/tui/app.rs`), reading the `ended_at`-derived `live` flag that already rides the
/// snapshot wire. The check fails by name if the TUI drops it.
#[test]
fn sessions_live_filter_exists_on_both_surfaces() {
    let Surfaces { app, .. } = SURFACES;
    assert!(
        app.contains("(\"sessions\", \"l\", \"live-only filter\")"),
        "the TUI's KEYS table documents the sessions live-only filter"
    );
    assert!(
        app.contains("self.sessions.live_only = !self.sessions.live_only"),
        "the TUI Sessions tab toggles live_only on `l`"
    );
}

/// T478 (D27): the doctor `--fix` checklist is a write action on both surfaces, and both reach
/// the engine through the same `doctor::web` pair, so neither has a second code path.
#[test]
fn doctor_fix_exists_on_both_surfaces() {
    let Surfaces { app, .. } = SURFACES;
    let web = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/web/mod.rs"));
    let tui = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tui/doctor_fix.rs"
    ));
    assert!(
        web.contains("ClientMessage::Doctor")
            && web.contains("web::plan_here")
            && web.contains("web::apply_here"),
        "the web page plans and applies through doctor::web"
    );
    assert!(
        tui.contains("web::plan_here") && tui.contains("web::apply_here"),
        "the TUI plans and applies through the same pair"
    );
    assert!(
        app.contains("(\"doctor\", \"f\", \"fix checklist\")")
            && app.contains("(\"doctor\", \"Enter/y\", \"apply selected (confirm)\")"),
        "the TUI's KEYS table lists the doctor fix keys"
    );
}

/// T479: the Hosts page's clear-safe-junk action is on both surfaces and both call
/// `agents::junk_web`'s plan/apply pair, never a second clear path.
#[test]
fn clear_safe_junk_exists_on_both_surfaces() {
    let Surfaces { app, .. } = SURFACES;
    let web = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/web/mod.rs"));
    let tui = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tui/junk_clear.rs"
    ));
    assert!(
        web.contains("junk_web::plan") && web.contains("junk_web::apply"),
        "the web page plans and applies through agents::junk_web"
    );
    assert!(
        tui.contains("junk_web::plan") && tui.contains("junk_web::apply"),
        "the TUI plans and applies through the same pair"
    );
    assert!(
        app.contains("(\"hosts\", \"c\", \"clear safe junk (plan)\")")
            && app.contains("(\"hosts\", \"y\", \"remove planned (confirm)\")"),
        "the TUI's KEYS table lists the clear-safe-junk keys"
    );
}

/// T227: both surfaces render the Stats page from the same model accessor — `rtok
/// stats --price`'s table plus `rtok stats --cache`'s table, D27's one page for two
/// commands.
#[test]
fn stats_page_exists_on_both_surfaces() {
    let Surfaces { model, tui, .. } = SURFACES;
    assert!(
        model.contains("(\"stats\", \"stats\")"),
        "pages() offers stats"
    );
    assert!(
        model.contains("fn stats_page_text"),
        "the one accessor lives on the model (D23)"
    );
    assert!(
        tui.contains("\"stats\" =>"),
        "the TUI renders the stats page"
    );
}

/// T230: both surfaces render the Graph page — `graph status`'s index health plus
/// `graph dead`'s list — from the same model accessor, so `graph status`/`graph dead`
/// can leave EXEMPT for COMMAND_PAGES.
#[test]
fn graph_page_exists_on_both_surfaces() {
    let Surfaces { model, tui, .. } = SURFACES;
    assert!(
        model.contains("(\"graph\", \"graph\")"),
        "pages() offers graph"
    );
    assert!(
        model.contains("fn graph_page_text"),
        "the one accessor lives on the model (D23)"
    );
    assert!(
        tui.contains("\"graph\" =>"),
        "the TUI renders the graph page"
    );
}

/// T480 (D27, T484): the live calls view is on both surfaces, fed by one reader of `graph_events`
/// and one aggregation: the web poller folds each batch into the `CallsStore` and sends the totals
/// in the `calls` frame, which the page only renders; the TUI folds the same batches in the same
/// store, and the pane drives the web's `Reader` instead of opening its own.
#[test]
fn live_calls_view_exists_on_both_surfaces() {
    let Surfaces { app, .. } = SURFACES;
    let web_live = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/web/live.rs"));
    let view = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/web/calls_view.rs"
    ));
    let page = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/src/pages/graph3d/live/useCalls.ts"
    ));
    let port = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/web/calls_store.rs"
    ));
    let page_feed = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/src/pages/graph3d/live/CallFeed.tsx"
    ));
    let pane = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tui/live_calls.rs"
    ));
    // The pane's own tests open a store to feed the poller; only the code above them counts.
    let pane = pane.split("#[cfg(test)]\nmod tests").next().unwrap();
    assert!(
        web_live.contains("pub struct Reader") && web_live.contains("Reader::open"),
        "the web poller reads through the shared Reader"
    );
    assert!(
        web_live.contains("store.fold(") && web_live.contains("CallsView::of"),
        "the web poller folds into the CallsStore and sends its view"
    );
    assert!(
        view.contains("store.window_totals("),
        "the frame's windows are the store's window totals"
    );
    assert!(
        !page.contains("fold(") && page.contains("useCallStream"),
        "the page renders the server's frames and keeps no fold of its own"
    );
    for part in [
        "pub fn fold",
        "pub fn sweep",
        "pub fn window_totals",
        "pub fn filter_feed",
    ] {
        assert!(port.contains(part), "the shared store has `{part}`");
    }
    assert!(
        pane.contains("Reader::open")
            && pane.contains("CallsStore")
            && !pane.contains("Store::open"),
        "the TUI pane reads through the web's Reader and folds with the web module's store"
    );
    assert!(
        app.contains("(\"graph\", \"f\", \"freeze/unfreeze calls\")"),
        "the TUI's KEYS table documents the freeze key"
    );
    // T329.34: both surfaces count from their own start, replayed from the store by the one
    // Reader, and show the caller names the Reader resolves.
    assert!(
        web_live.contains("r.replay(") || web_live.contains(".replay(since)"),
        "the web poller replays the store since the server started"
    );
    assert!(
        pane.contains("r.replay(") && pane.contains("live::head_of"),
        "the TUI pane replays the store since the TUI started"
    );
    assert!(
        pane.contains("r.caller") && page_feed.contains("r.caller"),
        "both surfaces show the caller name, not the session id"
    );
}

/// T486 (D27): the live calls view shows the same five call metrics on both surfaces, read from the
/// one `Totals` fold (`latency`, `symbols`, `crossed`, `fallbacks`, `caps`) with the page's wording.
#[test]
fn call_metrics_exist_on_both_surfaces() {
    let web = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/src/pages/graph3d/live/LiveMetrics.tsx"
    ));
    let pane = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tui/live_calls.rs"
    ));
    let pane = pane.split("#[cfg(test)]\nmod tests").next().unwrap();
    for label in [
        "latency p50",
        "p95",
        "symbols returned",
        "across projects",
        "fallbacks",
        "capped",
        "files touched",
        "projects with hits",
    ] {
        assert!(web.contains(label), "the page shows `{label}`");
        assert!(pane.contains(label), "the TUI pane shows `{label}`");
    }
    for field in [
        "latency()",
        "t.symbols",
        "t.crossed",
        "t.fallbacks",
        "t.caps",
        "t.symbols_returned",
        "t.files_touched",
        "t.projects_hit",
    ] {
        assert!(pane.contains(field), "the pane reads `{field}` from Totals");
    }
}

/// T329.32 and T500 (D27): the live extras that fit a terminal are on both surfaces, the sparklines,
/// the `outside scope` mark and the folded-call counter. The series and the counter are the
/// store's (`CallsStore::spark`, `folded_counters`), so the pane only draws them.
#[test]
fn live_extras_are_on_the_page_and_in_the_tui() {
    let root = env!("CARGO_MANIFEST_DIR");
    let read = |rel: &str| std::fs::read_to_string(format!("{root}/{rel}")).expect(rel);
    let page = [
        "web/src/pages/graph3d/live/LiveMetrics.tsx",
        "web/src/pages/graph3d/live/CallFeed.tsx",
        "web/src/pages/graph3d/live/LiveGraph.tsx",
    ]
    .map(read)
    .join("\n");
    let pane = read("src/tui/live_calls.rs");
    let pane = pane.split("#[cfg(test)]\nmod tests").next().unwrap();
    let store = read("src/web/calls_store.rs");
    // The wording is shared: the TUI takes it from the store's constants and functions, so each
    // item is on the page as text and on the pane as a call into the same code.
    for (wording, shared, tui_call) in [
        ("over the last", "pub fn spark_label(", "spark_label("),
        (
            "outside scope",
            "pub const OUTSIDE_SCOPE: &str = \"outside scope\"",
            "OUTSIDE_SCOPE",
        ),
        (
            "folded calls",
            "pub fn folded_counters(",
            "folded_counters(",
        ),
    ] {
        assert!(page.contains(wording), "the page shows `{wording}`");
        assert!(store.contains(shared), "the store defines `{shared}`");
        assert!(
            pane.contains(tui_call),
            "the TUI pane uses `{tui_call}` for `{wording}`"
        );
    }
    assert!(
        pane.contains("folded calls"),
        "the TUI names the counter like the page"
    );
    assert!(pane.contains(".spark("), "the TUI draws the store's series");
    let view = read("src/web/calls_view.rs");
    assert!(
        view.contains("store.spark("),
        "the frame's sparklines are the store's series"
    );
}

/// T476 (D27): selecting a project and linking or unlinking a pair are writes on the Graph page
/// of both surfaces, each through the one `graph projects` function — the web by
/// `ClientMessage::Project`, the TUI by keys over the same `ProjectRequest` and
/// `web::project_write`. A write added to one without the other fails here by name.
#[test]
fn project_writes_exist_on_both_surfaces() {
    let Surfaces { app, .. } = SURFACES;
    let web = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/web/mod.rs"));
    let tui = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tui/projects.rs"));
    for (verb, key, desc) in [
        ("Select", "s", "select project"),
        ("Link", "l", "link to…"),
        ("Unlink", "u", "unlink from…"),
    ] {
        assert!(
            web.contains(&format!("R::{verb}")),
            "the web page does not write project {verb}"
        );
        assert!(
            tui.contains(&format!("ProjectRequest::{verb}")),
            "the TUI graph page does not write project {verb}"
        );
        assert!(
            app.contains(&format!("(\"graph\", \"{key}\", \"{desc}\")")),
            "the TUI's KEYS table has no graph key for project {verb}"
        );
    }
    assert!(
        app.contains("crate::web::project_write"),
        "the TUI writes through the web page's function, not a second one"
    );
}

/// T485 (D27): the compare view is on the Graph page of both surfaces, from the one `diff::page`
/// core (the page's `diff` request, the TUI's typed ref or export path), with the same change marks
/// and section names. A surface that grows its own diff, or whose marks drift, fails here.
#[test]
fn graph_compare_exists_on_both_surfaces() {
    let Surfaces { app, .. } = SURFACES;
    let panel = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/src/pages/graph3d/ComparePanel.tsx"
    ));
    let marks = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/src/pages/graph3d/compare.ts"
    ));
    let web = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/web/mod.rs"));
    let tui = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tui/compare.rs"));
    // The view's own tests build fixtures and call `diff::page_of` to check the answer; only the
    // code above them counts.
    let tui = tui.split("#[cfg(all(test").next().unwrap();
    assert!(
        web.contains("graph::diff::page(") && tui.contains("diff::page_of("),
        "both surfaces ask the one diff::page core"
    );
    assert!(
        !tui.contains("diff::report(") && !tui.contains("Store::open"),
        "the TUI view recomputes nothing"
    );
    for (name, mark) in [
        ("added", "+"),
        ("removed", "−"),
        ("changed", "~"),
        ("moved", "→"),
    ] {
        assert!(
            marks.contains(&format!("{name}: \"{mark}\"")),
            "the web panel has no `{mark}` mark for {name}"
        );
        assert!(
            tui.contains(&format!("const {}: &str = \"{mark}\"", name.to_uppercase())),
            "the TUI view has no `{mark}` mark for {name}"
        );
    }
    for section in [
        "changed",
        "removed",
        "renamed",
        "moved",
        "added",
        "edges added",
        "edges removed",
        "links added",
        "links removed",
        "changed, not analysed",
    ] {
        let title = format!("\"{section}\"");
        let web_title = format!("title=\"{section}\"");
        // `edges ${c}` and `links ${c}` are one template over "added" and "removed".
        let templated = section
            .split_once(' ')
            .is_some_and(|(head, _)| panel.contains(&format!("`{head} ${{c}}`")));
        assert!(
            panel.contains(&web_title) || templated,
            "the web panel has no `{section}` section"
        );
        assert!(
            tui.contains(&title),
            "the TUI view has no `{section}` section"
        );
    }
    assert!(
        app.contains("(\"graph\", \"c\", \"compare\")"),
        "the TUI's KEYS table documents the compare key"
    );
}

/// T329.40 (D27, web half; the TUI half is the test below): the Graph page's Export menu asks the
/// server for the file, which calls the one `render` behind `rtok graph export`, and opens a saved
/// file through `export::parse`. A page that draws or serialises the graph itself fails here.
#[test]
fn graph_export_page_asks_the_one_render() {
    let web = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/web/mod.rs"));
    let export = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/plugins/graph/export.rs"
    ));
    let cli = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/cli/graph.rs"));
    let menu = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/src/pages/graph3d/ExportMenu.tsx"
    ));
    assert!(
        web.contains("graph::export::page_file(") && web.contains("graph::export::parse("),
        "the websocket makes a file with page_file and opens one with parse"
    );
    assert!(
        export.contains("let bytes = render(") && cli.contains("export::render("),
        "page_file and the CLI both call render"
    );
    for needle in [
        "File names and symbol names are included",
        "api.exportGraph",
        "api.importGraph",
        "download(",
    ] {
        assert!(menu.contains(needle), "the Export menu lacks `{needle}`");
    }
    assert!(
        !menu.contains("<svg") && !menu.contains("toDataURL"),
        "the page saves the server's file; it draws no second picture"
    );
}

/// T329.41 (D27): the TUI exports the graph with the function `rtok graph export -o` ends in, and
/// views a saved file with the importer the CLI's `--from` uses, so the three cannot write or read
/// different things. The page's half is the test above.
#[test]
fn graph_export_tui_shares_the_cli_writer_and_reader() {
    let Surfaces { app, .. } = SURFACES;
    let cli = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/cli/graph.rs"));
    let tui = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tui/exporter.rs"));
    // The view's own tests call `export::render` to compute what the command writes.
    let tui = tui.split("#[cfg(all(test").next().unwrap();
    assert!(
        cli.contains("export::write(") && tui.contains("export::write("),
        "the command and the TUI write through export::write"
    );
    assert!(
        tui.contains("export::read(")
            && !tui.contains("fs::write(")
            && !tui.contains("Store::open"),
        "the saved-export view reads with export::read and writes nothing"
    );
    for key in [
        "(\"graph\", \"e\", \"export graph\")",
        "(\"graph\", \"v\", \"view saved export\")",
    ] {
        assert!(app.contains(key), "the TUI's KEYS table lacks {key}");
    }
}

/// T481 (D27): the graph health score is on the Graph page of both surfaces, from the one
/// `ProjectRow.health` and `scope_health` the registry carries. Both colour a project by the
/// server's `level`; only the scope's bare number is banded locally, the web by `levelOf` and the
/// TUI by `score::level_of`. A surface that drops the score, its breakdown or the scope fails here.
#[test]
fn graph_health_score_exists_on_both_surfaces() {
    let web = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/src/pages/graph3d/health.ts"
    ));
    let tui = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tui/health.rs"));
    let projects = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/tui/projects.rs"));
    assert!(
        web.contains("levelOf") && web.contains("s.level"),
        "the web page colours by the server's level"
    );
    for needle in [
        "p.health.level",
        "p.scope_health",
        "h.components",
        "h.reasons",
        "level_of",
    ] {
        assert!(tui.contains(needle), "the TUI graph page lacks {needle}");
    }
    assert!(
        projects.contains("p.health.clone()") && projects.contains("p.scope_health"),
        "the TUI reads the snapshot's rows, not a second store read"
    );
}

/// T231: both surfaces render the Hosts page — `agents list`'s blocks, kind,
/// version, installed surfaces, config path — from the same model accessor, so
/// `agents list`/`agents info` can leave EXEMPT for COMMAND_PAGES.
#[test]
fn hosts_page_exists_on_both_surfaces() {
    let Surfaces { model, tui, .. } = SURFACES;
    assert!(
        model.contains("(\"hosts\", \"hosts\")"),
        "pages() offers hosts"
    );
    assert!(
        model.contains("fn hosts_page_text"),
        "the one accessor lives on the model (D23)"
    );
    assert!(
        tui.contains("\"hosts\" =>"),
        "the TUI renders the hosts page"
    );
}

/// T228: both surfaces render the Config page — `config show`'s rows, key/value/D12
/// source — from the same model accessor, so `config show`/`config get` can leave
/// EXEMPT for COMMAND_PAGES.
#[test]
fn config_page_exists_on_both_surfaces() {
    let Surfaces { model, tui, .. } = SURFACES;
    assert!(
        model.contains("(\"config\", \"config\")"),
        "pages() offers config"
    );
    assert!(
        model.contains("fn config_page_text"),
        "the one accessor lives on the model (D23)"
    );
    assert!(
        tui.contains("\"config\" =>"),
        "the TUI renders the config page"
    );
}

/// T229: both surfaces render the Services page — `demon status`'s per-service rows
/// plus `otel status`'s exporter health — from the same model accessor, so `demon
/// status`/`otel status` can join `COMMAND_PAGES`.
#[test]
fn services_page_exists_on_both_surfaces() {
    let Surfaces { model, tui, .. } = SURFACES;
    assert!(
        model.contains("(\"services\", \"services\")"),
        "pages() offers services"
    );
    assert!(
        model.contains("fn services_page_text"),
        "the one accessor lives on the model (D23)"
    );
    assert!(
        tui.contains("\"services\" =>"),
        "the TUI renders the services page"
    );
}

/// T232: both surfaces render the Worktrees page — `worktree list`'s table (path,
/// branch, owner, state, age, `target/` size) — from the same model accessor, so
/// `worktree list` can leave EXEMPT for COMMAND_PAGES; `gc`/`clean` stay CLI-only.
#[test]
fn worktrees_page_exists_on_both_surfaces() {
    let Surfaces { model, tui, .. } = SURFACES;
    assert!(
        model.contains("(\"worktrees\", \"worktrees\")"),
        "pages() offers worktrees"
    );
    assert!(
        model.contains("fn worktrees_page_text"),
        "the one accessor lives on the model (D23)"
    );
    assert!(
        tui.contains("\"worktrees\" =>"),
        "the TUI renders the worktrees page"
    );
}

/// T358.5: both surfaces render the Usage page from the one report `rtok agents usage` builds
/// (`usage::report`): the tui shows its text, the SPA its rows as data.
#[test]
fn usage_page_exists_on_both_surfaces() {
    let Surfaces { model, tui, .. } = SURFACES;
    assert!(
        model.contains("(\"usage\", \"agent_usage\")"),
        "pages() offers usage"
    );
    assert!(
        model.contains("usage::report("),
        "the one report lives behind the model (D23)"
    );
    assert!(
        tui.contains("\"usage\" =>"),
        "the TUI renders the usage page"
    );
}

/// T441.13.7: both surfaces render the Tasks page from the one list `rtok task list --all` reads
/// (`Project::adapter().list`): the tui shows its table, the SPA the same rows as a board.
#[test]
fn tasks_page_exists_on_both_surfaces() {
    let tasks = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/model/tasks.rs"));
    let Surfaces { model, tui, .. } = SURFACES;
    assert!(
        model.contains("(\"tasks\", \"tasks\")"),
        "pages() offers tasks"
    );
    assert!(
        tasks.contains(".list(&filter(&[], true, None)?)"),
        "the one list lives behind the model (D23)"
    );
    assert!(
        tui.contains("\"tasks\" =>"),
        "the TUI renders the tasks page"
    );
}

/// Reading commands and the page of the model they render: (command path, page). The
/// page must be one `model::pages()` offers — a page both surfaces carry in the frame.
/// The mapping is many-to-one: several commands may render the same page.
const COMMAND_PAGES: &[(&str, &str)] = &[
    ("plugins", "plugins"),
    ("memory status", "plugins"),
    // the Sessions page rides the snapshot since T25.1, so the command renders a
    // real page, not an on-demand call
    ("agents sessions", "sessions"),
    // T284: one agent out of the same Sessions-page model (`model::agent_show`)
    ("agents show", "sessions"),
    // the Doctor page rides the snapshot since T15.6, so `rtok doctor` renders it
    ("doctor", "doctor"),
    // the Logs page rides the snapshot since T15.7, so `rtok logs` renders it
    ("logs", "logs"),
    // the Stats page rides the snapshot since T227, so `rtok stats` renders it
    ("stats", "stats"),
    // the Graph page rides the snapshot since T230, so both render it
    ("graph status", "graph"),
    ("graph dead", "graph"),
    // the Hosts page rides the snapshot since T231, so both render it
    ("agents list", "hosts"),
    ("agents info", "hosts"),
    // T330.1: the junk list is a section of the Hosts page text
    ("agents junk list", "hosts"),
    // the Config page rides the snapshot since T228, so both render it
    ("config show", "config"),
    ("config get", "config"),
    // the Services page rides the snapshot since T229, so both render it
    ("demon status", "services"),
    ("otel status", "services"),
    // the Worktrees page rides the snapshot since T232, so `worktree list` renders it
    ("worktree list", "worktrees"),
    ("agents usage", "usage"),
    // the Tasks page rides the snapshot since T441.13.7, so `task list` renders it
    ("task list", "tasks"),
];

/// The commands D27 exempts, each with its reason. Streaming commands print a stream,
/// not a state; writing commands mutate a host, a file or the store; surfaces render
/// the model rather than sit inside it; helpers answer a location or a verdict. The
/// last group is reading commands whose page is on-demand today — T15.11 moved the
/// query into the model, but the frame does not carry the page yet; each entry moves
/// to `COMMAND_PAGES` when it does.
const EXEMPT: &[(&str, &str)] = &[
    // streaming: the output is a stream, not a state (D27)
    (
        "hook",
        "reads the event JSON on stdin, writes JSON to stdout, exits",
    ),
    ("mcp", "serves MCP tools over stdio"),
    (
        "mcp ping",
        "proves a host's rtok MCP server answers (T275.1); not a page",
    ),
    ("run", "executes a command and filters its live output"),
    // `wrap` (T51.4) joins EXEMPT when the clap command lands — not before.
    ("filter", "filters stdin without executing"),
    ("expand", "prints one archived payload"),
    (
        "guard check",
        "prints the plugins::guard allow/deny verdict (T70.5)",
    ),
    (
        "archive rewrite",
        "rewrites a pi `context` array on stdin to stdout (T70.2)",
    ),
    // surfaces: renderers of the model, not pages of it (D23)
    ("web", "the web surface itself"),
    ("dashboard", "deprecated spelling of `rtok web`"),
    ("proxy", "serves the proxy; `--dry-run` echoes its settings"),
    // writing: each calls the provider's Batch API, not the operator model
    ("batch submit", "creates a provider Batch job"),
    ("batch status", "asks the provider, not the store"),
    ("batch fetch", "downloads provider results to a new file"),
    (
        "tui",
        "the terminal surface; its tabs are model::pages() (T15.1)",
    ),
    (
        "logs watch",
        "streams the log file as lines arrive (D27 exempts streaming)",
    ),
    (
        "agents sessions watch",
        "repaints the Sessions page as sessions change (D27 exempts streaming)",
    ),
    // writing: a surface that shows numbers is not one that mutates a tree (D27)
    (
        "agents install",
        "installs hooks, MCP and the proxy into a host",
    ),
    ("agents uninstall", "takes rtok back out of a host"),
    (
        "agents update",
        "refreshes or reinstalls what rtok installed in a host",
    ),
    (
        "agents outdated",
        "lists installed plugins older than this binary (T279.1); a one-shot CLI check, \
         not a shared model page",
    ),
    ("setup", "deprecated spelling of `rtok agents install`"),
    (
        "agents status",
        "writes the calling agent's own status text into the store (T284)",
    ),
    ("config init", "writes the annotated reference file"),
    ("config set", "edits one key in the user file"),
    ("bench", "runs the A/B schedule and writes Measurement rows"),
    ("memory import", "inserts note rows"),
    (
        "docs fetch",
        "downloads rustdoc JSON into the local cache; not a dashboard page",
    ),
    (
        "memory retire",
        "tombstones a note row; the Memory page renders the notes, not the verdict (T69.1)",
    ),
    ("memory pin", "flags a note row to lead recall (T69.1)"),
    ("memory unpin", "drops the recall lead flag (T69.1)"),
    (
        "memory history",
        "prints previous title and body of one note; recall and mem_get stay on the current body",
    ),
    ("memory revise", "replaces and retires note rows (T69.1)"),
    (
        "memory history",
        "prints earlier bodies of one note; recall and mem_get stay on the current body (T472)",
    ),
    (
        "memory sync",
        "writes a managed CLAUDE.md / AGENTS.md block (T69.6)",
    ),
    ("graph index", "walks a tree and inserts symbol rows"),
    ("demon start", "starts the supervisor"),
    ("demon stop", "asks the supervisor and its child to exit"),
    ("demon restart", "stop, then start"),
    ("demon kill", "SIGKILL and drop the state file"),
    ("demon supervise", "the detached half of `demon start`"),
    (
        "demon upgrade",
        "stops live surfaces, replaces the binary, starts the same set",
    ),
    ("otel flush", "posts rows past the watermarks"),
    (
        "worktree gc",
        "removes finished git worktrees and their merged branches (T153)",
    ),
    // T441.5: tasks live in the project's adapter (files, GitHub, GitLab), not the store's
    // snapshot the web and TUI pages render; MCP `task_*` (T441.6) is their second surface.
    (
        "task create",
        "allocates an id and writes a task through the adapter (T441.5)",
    ),
    (
        "task show",
        "reads the project's adapter, not the store snapshot (T441.5)",
    ),
    (
        "task status",
        "moves a task's status through the adapter (T441.5)",
    ),
    (
        "task next",
        "reads the project's adapter, not the store snapshot (T441.5)",
    ),
    (
        "task init",
        "writes [tasks] into the checkout's .rtok.toml (T441.5)",
    ),
    (
        "task sync",
        "raises the store's id counters to the adapter's highest ids; agents get that for free, since task_create seeds first (T441.12)",
    ),
    (
        "task export",
        "a human's one-way mirror of the disk tasks into an issue tracker; it writes to a third party, so no agent tool (T441.13.5)",
    ),
    (
        "task ready",
        "reads the project's adapter, not the store snapshot (T442)",
    ),
    (
        "task claim",
        "claims a task through the adapter; the store only records it for the hook (T442)",
    ),
    ("task release", "clears a claim through the adapter (T442)"),
    ("task dep", "writes a blocker through the adapter (T442)"),
    (
        "task priority",
        "writes a priority through the adapter (T442)",
    ),
    (
        "task conflict",
        "writes a symmetric conflict marker through the adapter (T441.13.2)",
    ),
    (
        "task parallel",
        "writes a parallel marker through the adapter (T441.13.2)",
    ),
    (
        "worktree add",
        "creates a locked git worktree and prints its path (T158)",
    ),
    (
        "worktree claim",
        "rewrites one git worktree lock and its claim row (T285)",
    ),
    (
        "worktree adopt",
        "binds the cwd's worktree to the session agent (T289.2 MCP worktree_adopt)",
    ),
    (
        "worktree remove",
        "removes one git worktree, its merged branch and its claim row (T286)",
    ),
    (
        "worktree clean",
        "deletes tagged build caches on the checkout's file system, not the store (T152)",
    ),
    (
        "agents junk clear",
        "deletes log siblings and archive payloads on rtok's own file system, not the store (T182)",
    ),
    // helpers: a location or a verdict, not model data
    ("config path", "prints where the config file is"),
    (
        "agents whoami",
        "prints this session's own rtok agent id from RTOK_AGENT_ID (T283); a one-row \
         identity call, not a shared model page",
    ),
    (
        "worktree whoami",
        "prints the caller's agent, worktree root and held worktrees (T411); a one-row \
         identity call like `agents whoami`, not a shared model page",
    ),
    (
        "agents send",
        "writes message rows for one agent or this project's live agents (T287)",
    ),
    (
        "agents inbox",
        "one agent's framed message queue, marked read when the agent reads its own (T287); \
         not a shared model page",
    ),
    (
        "info",
        "prints version, paths, disk usage, error count and proxy status",
    ),
    (
        "config validate",
        "checks a file and exits nonzero on issues",
    ),
    (
        "completions",
        "prints shell completions generated from the clap tree (T53.2)",
    ),
    (
        "man",
        "prints the man page generated from the clap tree (T53.2)",
    ),
    (
        "memory export",
        "dumps notes as the portable JSONL `memory import` reads; the Memory page is where they render (T66.2)",
    ),
    // reading, but on-demand today (T15.11); the frame does not carry the page yet
    (
        "report",
        "renders model::report_ledgers into a document (P22); no snapshot page",
    ),
    ("graph impact", "need a target; CLI/MCP only"),
    ("graph affected", "need a target; CLI/MCP only"),
    ("graph review", "need a diff; CLI only"),
    (
        "graph diff",
        "needs two revisions; CLI/MCP only, the page and the TUI have the compare view (T329.35, T485)",
    ),
    (
        "graph export",
        "needs a scope and a file; CLI/MCP, the TUI has the export key and the saved-export view (T329.41), and the Graph page's Export menu (T329.40) asks the same render over /ws",
    ),
    (
        "graph projects",
        "the registry's list and actions; the Graph page gets the selector in T329.12",
    ),
    (
        "graph projects add",
        "changes the registry; the selector is T329.12",
    ),
    (
        "graph projects select",
        "changes the registry; the selector is T329.12",
    ),
    (
        "graph projects remove",
        "changes the registry; the selector is T329.12",
    ),
    (
        "graph projects link",
        "changes the registry; the selector is T329.12",
    ),
    (
        "graph projects unlink",
        "changes the registry; the selector is T329.12",
    ),
    (
        "logs export",
        "the same Logs selection, unnumbered and uncoloured",
    ),
];

/// Every runnable command path, space-joined — the walk `config_coverage` already
/// does, minus the flags: a parent that needs a verb is navigation, not a command,
/// and clap's built-in `help` is not ours to classify.
fn walk(cmd: &Command, path: &mut Vec<String>, out: &mut Vec<String>) {
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" {
            continue;
        }
        path.push(sub.get_name().to_string());
        if !sub.has_subcommands() || !sub.is_subcommand_required_set() {
            out.push(path.join(" "));
        }
        walk(sub, path, out);
        path.pop();
    }
}

#[test]
fn every_command_is_exempt_or_renders_a_page_of_the_model() {
    let pages: Vec<&str> = model::pages().iter().map(|(page, _)| *page).collect();
    let mut commands = Vec::new();
    walk(&Cli::command(), &mut Vec::new(), &mut commands);
    assert!(!commands.is_empty(), "the walk found no commands");

    for cmd in &commands {
        if EXEMPT.iter().any(|(path, _)| *path == cmd.as_str()) {
            continue;
        }
        let Some((_, page)) = COMMAND_PAGES.iter().find(|(path, _)| *path == cmd.as_str()) else {
            panic!(
                "unclassified command `{cmd}`: a reading command renders a page of the model \
                 (COMMAND_PAGES); everything else needs a reason in EXEMPT (D27, T15.12)"
            );
        };
        assert!(
            pages.contains(page),
            "command `{cmd}` renders page `{page}`, which model::pages() does not offer"
        );
    }

    // The lists are data, extended as commands land (`report`, `tui`, `logs watch` at
    // integration); these hold them to the tree they claim to describe.
    for (path, reason) in EXEMPT {
        assert!(!reason.is_empty(), "exempt `{path}` carries no reason");
        assert!(
            commands.iter().any(|c| c.as_str() == *path),
            "`{path}` is exempt but is not a command"
        );
    }
    for (path, _) in COMMAND_PAGES {
        assert!(
            commands.iter().any(|c| c.as_str() == *path),
            "`{path}` is mapped to a page but is not a command"
        );
        assert!(
            !EXEMPT.iter().any(|(exempt, _)| exempt == path),
            "`{path}` is both mapped to a page and exempt"
        );
    }
}

/// Table-printing readers must accept `--json` and serialize the model page (T60.1).
const JSON_READERS: &[&str] = &[
    "stats",
    "info",
    "config show",
    "config get",
    "doctor",
    "plugins",
    "agents list",
    "agents info",
    "agents junk list",
    "agents sessions",
    "agents whoami",
    "agents show",
    "agents inbox",
    "logs",
    "demon status",
    "otel status",
    "memory status",
    "worktree list",
    "worktree whoami",
    "agents usage",
    "graph status",
    "graph dead",
    "task list",
    "task show",
    "task next",
    "task ready",
    "task claim",
    "task release",
    "task dep",
    "task priority",
    "task conflict",
    "task parallel",
];

fn command_at<'a>(root: &'a Command, path: &str) -> &'a Command {
    let mut cur = root;
    for part in path.split(' ') {
        cur = cur
            .find_subcommand(part)
            .unwrap_or_else(|| panic!("no command `{path}`"));
    }
    cur
}

#[test]
fn reading_commands_accept_json() {
    let root = Cli::command();
    for path in JSON_READERS {
        let ok = command_at(&root, path)
            .get_arguments()
            .any(|a| a.get_long() == Some("json"));
        assert!(ok, "reading command `{path}` has no --json (T60.1)");
    }
    for (path, _) in COMMAND_PAGES {
        assert!(
            JSON_READERS.contains(path),
            "reading command `{path}` renders a model page but is not gated for --json (T60.1)"
        );
    }
}

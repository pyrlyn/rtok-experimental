// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok doctor --project` (T977): what rtok could do for one checkout, with the next step for
//! each finding and, where the user's own `Measurement` rows allow, a rough token figure.
//!
//! Advice only: nothing here changes a file. Every figure comes from measurement rows (no row,
//! no number), and each probe fails open to an empty part so one broken piece never hides the
//! rest.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rtok_config::root::ProjectRoot;
use serde::Serialize;

use super::{Instructions, instruction_audit, instruction_lines, read_json};
use crate::config::{CATALOGUE, Config};
use crate::plugin::Runtime;
use crate::store::MeasurementTotal;

/// A walk over a huge tree must not stall the dashboard tick or the command.
const FILE_CAP: usize = 50_000;
const WALK_BUDGET: Duration = Duration::from_secs(2);

/// The default `[store] retain_calls_days`: ledger rows older than that may already be pruned,
/// so a longer window would average over a shrinking sample. No doctor audit has its own window.
const WINDOW_DAYS: u32 = 30;

/// Where the setup advice for one checkout comes from.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ProjectReport {
    /// The checkout analysed: the git checkout root of the path given, else the path itself.
    pub root: String,
    /// How many days back the saving figures look.
    pub window_days: u32,
    /// Languages found, most files first.
    pub languages: Vec<LanguageRow>,
    /// The walk hit its file or time cap, so file counts are a lower bound.
    pub walk_cut: bool,
    /// The graph index of this checkout; absent while the graph plugin is off or unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<GraphPart>,
    /// The `AGENTS.md` / `CLAUDE.md` audit scoped to this root.
    pub instructions: Instructions,
    /// Catalogue plugins that are off for this checkout.
    pub plugins: Vec<PluginAdvice>,
}

/// One language of the checkout and the server rtok would spawn for it.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct LanguageRow {
    pub language: String,
    pub files: u64,
    /// The marker file found at the root (`Cargo.toml`).
    pub marker: Option<String>,
    /// The language server rtok spawns for it; absent for a tags-only language.
    pub server: Option<String>,
    pub installed: bool,
    /// How to install the server, while it is missing.
    pub install: Option<String>,
    pub note: Option<String>,
    /// What the server would save, from graph `lsp.*` rows.
    pub saving: Option<Saving>,
}

/// An approximate saving, always from `Measurement` rows.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Saving {
    /// Rows behind the average, across all projects in the window.
    pub calls: u64,
    /// Average `est_before - est_after` per call, never negative.
    pub saved_per_call: u64,
    /// This project's rows in the window; absent when no session maps to the project.
    pub project_calls: Option<u64>,
    /// `saved_per_call` times `project_calls`.
    pub approx_tokens: Option<u64>,
}

impl Saving {
    fn text(&self, days: u32) -> String {
        let mut t = format!(
            "≈ {} tokens per call over {} calls (from your measurements)",
            self.saved_per_call, self.calls
        );
        if let (Some(calls), Some(tokens)) = (self.project_calls, self.approx_tokens) {
            t.push_str(&format!(
                "; here ≈ {tokens} tokens ({calls} calls in {days} days)"
            ));
        }
        t
    }
}

fn saving_text(saving: Option<&Saving>, days: u32) -> String {
    saving.map_or_else(|| "no measurements yet".into(), |s| s.text(days))
}

/// The graph index and backend of this checkout.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct GraphPart {
    pub registered: bool,
    /// The registry row's state (`ok`, `stale`, `not indexed`, ...); absent while unregistered.
    pub state: Option<String>,
    pub indexed: bool,
    pub files: Option<u64>,
    pub score: Option<u8>,
    pub reasons: Vec<GraphReason>,
    /// The `backend` value in effect for this checkout.
    pub backend: String,
    /// The mode the last check recorded (`lsp` or `tags`).
    pub served_by: Option<String>,
    /// Commands and config changes that are not a health reason.
    pub next: Vec<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct GraphReason {
    pub text: String,
    pub fix: String,
}

/// A catalogue plugin that is off for this checkout.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct PluginAdvice {
    pub plugin: String,
    /// Where to turn it on: `.rtok.toml`, or `user config` for keys a checkout may not set.
    #[serde(rename = "where")]
    pub set_in: String,
    pub saving: Option<Saving>,
}

/// The advice for the checkout holding `path`. Fails open: a part that cannot be read is empty.
pub fn audit(cfg: &Config, path: &Path) -> ProjectReport {
    let dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    let dir = crate::fs::canon(dir);
    let root = ProjectRoot::find(&dir).map_or(dir, |r| r.checkout);
    // The process's own checkout already has its config; any other one has its own `.rtok.toml`.
    let own = std::env::current_dir()
        .ok()
        .and_then(|d| ProjectRoot::find(&d))
        .is_some_and(|r| crate::fs::same_path(&crate::fs::canon(&r.checkout), &root));
    let eff = if own {
        cfg.clone()
    } else {
        Config::load_for(&root, cfg.loaded_from.0.as_deref()).unwrap_or_else(|_| cfg.clone())
    };
    let rt = Runtime::open(eff.clone(), "doctor").ok();
    audit_in(&eff, &root, rt.as_ref(), &on_path)
}

/// The checkout the process runs in, for the dashboard snapshots; `None` outside a checkout.
pub fn audit_cwd(cfg: &Config) -> Option<ProjectReport> {
    let cwd = std::env::current_dir().ok()?;
    ProjectRoot::find(&cwd)?;
    Some(audit(cfg, &cwd))
}

#[cfg(feature = "graph")]
fn on_path(bin: &str) -> bool {
    crate::plugins::graph::lsp::on_path(bin)
}

#[cfg(not(feature = "graph"))]
fn on_path(_: &str) -> bool {
    false
}

/// [`audit`] over a given config, runtime and "is this binary installed" check. `rt`, when
/// given, was opened with `cfg`.
pub(super) fn audit_in(
    cfg: &Config,
    root: &Path,
    rt: Option<&Runtime>,
    installed: &dyn Fn(&str) -> bool,
) -> ProjectReport {
    let (mut languages, walk_cut) = languages(cfg, root, installed);
    let ledger = Ledger::read(rt, root);
    for row in languages.iter_mut().filter(|r| r.server.is_some()) {
        row.saving = ledger.saving("graph", |kind| kind.starts_with("lsp."));
    }
    let graph = graph_part(cfg, root, rt, &languages);
    let settings = read_json(&cfg.doctor.settings_path);
    let claude = read_json(&cfg.doctor.claude_json);
    ProjectReport {
        root: root.display().to_string(),
        window_days: WINDOW_DAYS,
        languages,
        walk_cut,
        graph,
        instructions: instruction_audit(cfg, settings.as_ref(), claude.as_ref(), Some(root)),
        plugins: off_plugins(cfg, &ledger),
    }
}

/// Measurement totals of the window, split by the project of the session that wrote them.
struct Ledger {
    rows: Vec<(Option<String>, MeasurementTotal)>,
    project: Option<String>,
}

impl Ledger {
    fn read(rt: Option<&Runtime>, root: &Path) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let since = now - i64::from(WINDOW_DAYS) * 86_400;
        Self {
            rows: rt
                .and_then(|rt| rt.store.measurement_saved_by_project(since).ok())
                .unwrap_or_default(),
            project: crate::project::project_name(root),
        }
    }

    /// The average saving per call of `plugin`'s rows whose kind `pick` accepts. An `expand` row
    /// is a retrieval cost, not a call that saved, so it stays out of the average.
    fn saving(&self, plugin: &str, pick: impl Fn(&str) -> bool) -> Option<Saving> {
        let (mut calls, mut saved, mut here) = (0i64, 0i64, 0i64);
        for (project, t) in &self.rows {
            if t.plugin != plugin || t.kind == "expand" || !pick(&t.kind) {
                continue;
            }
            calls += t.rows;
            saved += t.est_before - t.est_after;
            if self.project.is_some() && *project == self.project {
                here += t.rows;
            }
        }
        if calls == 0 {
            return None;
        }
        let saved_per_call = u64::try_from(saved / calls).unwrap_or(0);
        // A plugin that never ran in this project has no call count to scale the average by.
        let project_calls = (here > 0).then_some(here as u64);
        Some(Saving {
            calls: calls as u64,
            saved_per_call,
            project_calls,
            approx_tokens: project_calls.map(|n| n * saved_per_call),
        })
    }
}

fn off_plugins(cfg: &Config, ledger: &Ledger) -> Vec<PluginAdvice> {
    CATALOGUE
        .iter()
        .filter(|(id, on)| !cfg.plugin_enabled(id, *on))
        .map(|(id, _)| PluginAdvice {
            plugin: (*id).into(),
            set_in: setting_file(&format!("plugins.{id}.enabled")).into(),
            saving: ledger.saving(id, |_| true),
        })
        .collect()
}

/// The file that may hold `key`: a checkout's `.rtok.toml` cannot set keys with authority over
/// credentials, execution or the filesystem.
fn setting_file(key: &str) -> &'static str {
    if crate::config::layers::project_allowed(key) {
        ".rtok.toml"
    } else {
        "user config"
    }
}

#[cfg(feature = "graph")]
fn languages(
    cfg: &Config,
    root: &Path,
    installed: &dyn Fn(&str) -> bool,
) -> (Vec<LanguageRow>, bool) {
    use crate::plugins::graph::lsp;
    use std::collections::BTreeMap;

    let (counts, cut) = count_files(cfg, root);
    let mut files: BTreeMap<&str, u64> = counts;
    // A server language with its marker but no source file yet still gets its install advice.
    for lang in lsp::served_languages() {
        if lsp::markers(lang).any(|m| root.join(m).is_file()) {
            files.entry(lang).or_insert(0);
        }
    }
    let mut rows: Vec<LanguageRow> = files
        .into_iter()
        .map(|(lang, n)| language_row(lang, n, root, installed))
        .collect();
    rows.sort_by(|a, b| {
        b.files
            .cmp(&a.files)
            .then_with(|| a.language.cmp(&b.language))
    });
    (rows, cut)
}

#[cfg(not(feature = "graph"))]
fn languages(_: &Config, _: &Path, _: &dyn Fn(&str) -> bool) -> (Vec<LanguageRow>, bool) {
    (Vec::new(), false)
}

/// Source files per language, honouring `.gitignore` and `[plugins.graph] exclude` like the
/// index does, and whether the walk was cut by [`FILE_CAP`] or [`WALK_BUDGET`].
#[cfg(feature = "graph")]
fn count_files(cfg: &Config, root: &Path) -> (std::collections::BTreeMap<&'static str, u64>, bool) {
    use crate::plugins::graph::{lsp, walk::Matcher};

    let started = std::time::Instant::now();
    let mut counts = std::collections::BTreeMap::new();
    let mut seen = 0usize;
    for entry in Matcher::new(root, &cfg.plugins.graph).walk() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        seen += 1;
        if let Some(lang) = entry
            .path()
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .and_then(lsp::language_of_ext)
        {
            *counts.entry(lang).or_insert(0) += 1;
        }
        if seen >= FILE_CAP || started.elapsed() > WALK_BUDGET {
            return (counts, true);
        }
    }
    (counts, false)
}

#[cfg(feature = "graph")]
fn language_row(
    lang: &str,
    files: u64,
    root: &Path,
    installed: &dyn Fn(&str) -> bool,
) -> LanguageRow {
    use crate::plugins::graph::lsp;

    let marker = lsp::markers(lang).find(|m| root.join(m).is_file());
    let bin = lsp::server_of(lang).map(|(bin, _)| bin);
    let present = bin.is_some_and(installed);
    let note = match (bin, marker) {
        (None, _) => Some(format!(
            "tags only — rtok has no language server for {lang}"
        )),
        (Some(_), None) if lang == "c" => Some(
            "no compile_commands.json: generate it with `cmake -DCMAKE_EXPORT_COMPILE_COMMANDS=ON` or `bear -- make`"
                .into(),
        ),
        (Some(bin), None) => lsp::marker_of(lang).map(|m| {
            format!("no {m} at the root: rtok starts {bin} only for files under a {m}")
        }),
        (Some(_), Some(_)) => None,
    };
    LanguageRow {
        language: lang.into(),
        files,
        marker: marker.map(Into::into),
        server: bin.map(Into::into),
        installed: present,
        install: bin
            .filter(|_| !present)
            .and_then(lsp::install_hint)
            .map(Into::into),
        note,
        saving: None,
    }
}

#[cfg(feature = "graph")]
fn graph_part(
    cfg: &Config,
    root: &Path,
    rt: Option<&Runtime>,
    languages: &[LanguageRow],
) -> Option<GraphPart> {
    use crate::plugins::graph::{backend_name, capability::Chosen, projects};

    if !cfg.plugin_enabled("graph", true) {
        return None;
    }
    let rt = rt?;
    let backend = backend_name(&rtok_plugin_sdk::Ctx::new(rt), root);
    let registered = rt.store.project_by_root(root).ok().flatten();
    let row = registered.clone().and_then(|p| projects::row(rt, p).ok());
    let index = row.as_ref().and_then(|r| r.index.as_ref());
    let indexed = index.is_some_and(|i| i.indexed_at.is_some() || i.rows > 0);
    let mut next = Vec::new();
    if !indexed {
        next.push(format!("rtok graph index {}", root.display()));
    }
    if matches!(backend.as_str(), "tags" | "text")
        && let Some(l) = languages.iter().find(|l| l.installed && l.marker.is_some())
    {
        let server = l.server.as_deref().unwrap_or("the language server");
        next.push(format!(
            "{server} is installed but `backend` is \"{backend}\": set `[plugins.graph] backend = \"auto\"` in {}",
            setting_file("plugins.graph.backend")
        ));
    }
    Some(GraphPart {
        registered: registered.is_some(),
        state: row.as_ref().map(|r| r.state.to_string()),
        indexed,
        files: index.and_then(|i| u64::try_from(i.files).ok()),
        score: row.as_ref().and_then(|r| r.health.score),
        reasons: row
            .as_ref()
            .map(|r| {
                r.health
                    .reasons
                    .iter()
                    .map(|x| GraphReason {
                        text: x.text.clone(),
                        fix: x.fix.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        backend,
        served_by: row.as_ref().and_then(|r| r.backend.as_ref()).map(|c| {
            match c.backend {
                Chosen::Lsp => "lsp",
                Chosen::Tags => "tags",
            }
            .to_string()
        }),
        next,
    })
}

#[cfg(not(feature = "graph"))]
fn graph_part(_: &Config, _: &Path, _: Option<&Runtime>, _: &[LanguageRow]) -> Option<GraphPart> {
    None
}

impl ProjectReport {
    /// The `project <root>` section of the doctor text; the TUI tab shows it through `to_text`.
    pub fn render(&self) -> String {
        let days = self.window_days;
        let mut out = format!("project {}\n  languages\n", self.root);
        if self.languages.is_empty() {
            out.push_str("    none found\n");
        }
        for l in &self.languages {
            let mut head = format!("    {} {} files", l.language, l.files);
            if let Some(m) = &l.marker {
                head.push_str(&format!(", {m}"));
            }
            match (&l.server, l.installed) {
                (Some(s), true) => head.push_str(&format!(", {s} installed")),
                (Some(s), false) => head.push_str(&format!(", {s} not on PATH")),
                (None, _) => {}
            }
            out.push_str(&head);
            out.push('\n');
            if let Some(install) = &l.install {
                out.push_str(&format!("      install: {install}\n"));
            }
            if let Some(note) = &l.note {
                out.push_str(&format!("      {note}\n"));
            }
            if l.server.is_some() {
                out.push_str(&format!(
                    "      language server saving: {}\n",
                    saving_text(l.saving.as_ref(), days)
                ));
            }
        }
        if self.walk_cut {
            out.push_str(&format!(
                "    counts are a lower bound: the walk stopped at {FILE_CAP} files or {}s\n",
                WALK_BUDGET.as_secs()
            ));
        }
        if let Some(g) = &self.graph {
            out.push_str("  graph\n");
            out.push_str(&format!(
                "    {}, {}{}\n",
                if g.registered {
                    "registered"
                } else {
                    "not registered"
                },
                g.state
                    .as_deref()
                    .unwrap_or(if g.indexed { "indexed" } else { "not indexed" }),
                g.files.map_or_else(String::new, |n| format!(", {n} files")),
            ));
            if let Some(score) = g.score {
                out.push_str(&format!("    health {score}/100\n"));
            }
            for r in &g.reasons {
                out.push_str(&format!("      {} (fix: {})\n", r.text, r.fix));
            }
            out.push_str(&format!(
                "    backend {}{}\n",
                g.backend,
                g.served_by
                    .as_ref()
                    .map_or_else(String::new, |m| format!(" (served by {m})"))
            ));
            for n in &g.next {
                out.push_str(&format!("    next: {n}\n"));
            }
        }
        out.push_str("  instructions\n");
        if self.instructions.rows.is_empty() {
            out.push_str("    no AGENTS.md or CLAUDE.md\n");
        }
        out.push_str(&instruction_lines(&self.instructions, "    "));
        out.push_str("  plugins off here\n");
        if self.plugins.is_empty() {
            out.push_str("    none\n");
        }
        for p in &self.plugins {
            out.push_str(&format!(
                "    {}: set `[plugins.{}] enabled = true` in {}; saving: {}\n",
                p.plugin,
                p.plugin,
                p.set_in,
                saving_text(p.saving.as_ref(), days)
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtok_plugin_sdk::Measurement;
    use std::path::PathBuf;

    fn fixture(tag: &str) -> PathBuf {
        let root = crate::testutil::tmp_dir(tag);
        std::fs::write(root.join(".git"), "").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        for file in ["src/a.rs", "src/b.rs", "tools/x.py", "ignored/y.rs"] {
            let p = root.join(file);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x\n").unwrap();
        }
        root
    }

    fn row<'a>(r: &'a ProjectReport, lang: &str) -> &'a LanguageRow {
        r.languages.iter().find(|l| l.language == lang).unwrap()
    }

    fn measurement(
        plugin: &'static str,
        kind: &'static str,
        before: u32,
        after: u32,
    ) -> Measurement {
        Measurement {
            plugin,
            kind,
            before_bytes: 1,
            after_bytes: 1,
            est_before: before,
            est_after: after,
            ref_id: None,
            call_id: None,
        }
    }

    #[test]
    fn counts_languages_without_the_gitignored_tree() {
        let root = fixture("t977-count");
        let report = audit_in(&Config::default(), &root, None, &|b| b == "rust-analyzer");
        let rust = row(&report, "rust");
        assert_eq!(
            (rust.files, rust.marker.as_deref(), rust.installed),
            (2, Some("Cargo.toml"), true)
        );
        assert_eq!(rust.server.as_deref(), Some("rust-analyzer"));
        assert!(rust.install.is_none() && rust.note.is_none());
        let python = row(&report, "python");
        assert_eq!(python.files, 1);
        assert!(python.server.is_none());
        assert!(python.note.as_deref().unwrap().starts_with("tags only"));
        assert_eq!(report.languages[0].language, "rust", "most files first");
        assert!(!report.walk_cut);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_server_carries_its_install_command() {
        let root = fixture("t977-missing");
        std::fs::write(root.join("src/native.c"), "int main;\n").unwrap();
        let report = audit_in(&Config::default(), &root, None, &|_| false);
        let rust = row(&report, "rust");
        assert!(!rust.installed);
        assert_eq!(
            rust.install.as_deref(),
            Some("rustup component add rust-analyzer")
        );
        let c = row(&report, "c");
        assert!(c.marker.is_none());
        assert!(c.install.as_deref().unwrap().contains("clangd"));
        assert!(c.note.as_deref().unwrap().contains("compile_commands.json"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn graph_part_asks_for_an_index_and_for_auto_once_a_server_is_installed() {
        let root = fixture("t977-graph");
        let rt = Runtime::in_memory("t977-graph").unwrap();
        let report = audit_in(&rt.config, &root, Some(&rt), &|b| b == "rust-analyzer");
        let g = report.graph.expect("the graph plugin is on");
        assert!(!g.registered && !g.indexed);
        assert_eq!(g.backend, "tags");
        assert!(g.next[0].starts_with("rtok graph index "));
        assert!(
            g.next[1].contains("backend = \"auto\"") && g.next[1].contains("user config"),
            "{:?}",
            g.next
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn off_plugins_name_the_file_that_may_turn_them_on() {
        let root = fixture("t977-plugins");
        let mut cfg = Config::default();
        cfg.plugins.read.enabled = false;
        cfg.plugins.graph.enabled = false;
        let report = audit_in(&cfg, &root, None, &|_| false);
        let at = |id: &str| {
            report
                .plugins
                .iter()
                .find(|p| p.plugin == id)
                .map(|p| p.set_in.as_str())
        };
        assert_eq!(at("read"), Some(".rtok.toml"));
        assert_eq!(at("graph"), Some("user config"));
        assert_eq!(at("cmd"), None, "an enabled plugin is not listed");
        assert!(
            report.graph.is_none(),
            "no graph part while the plugin is off"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn saving_comes_from_measurement_rows_only() {
        let root = fixture("t977-saving");
        let rt = Runtime::in_memory("t977-saving").unwrap();
        let none = audit_in(&rt.config, &root, Some(&rt), &|_| false);
        assert!(row(&none, "rust").saving.is_none(), "no rows, no figure");

        let name = crate::project::project_name(&root).unwrap();
        rt.store
            .upsert_session("here", None, Some(&name), None, None)
            .unwrap();
        rt.store
            .upsert_session("elsewhere", None, Some("other"), None, None)
            .unwrap();
        for (session, before, after) in [
            ("here", 300, 100),
            ("here", 200, 100),
            ("elsewhere", 400, 100),
        ] {
            rt.store
                .insert_measurement(session, &measurement("graph", "lsp.symbol", before, after))
                .unwrap();
        }
        // A cost, not a saving: it must not drag the average down.
        rt.store
            .insert_measurement("here", &measurement("graph", "expand", 0, 900))
            .unwrap();
        let report = audit_in(&rt.config, &root, Some(&rt), &|_| false);
        let s = row(&report, "rust").saving.clone().expect("rows exist");
        assert_eq!((s.calls, s.saved_per_call), (3, 200));
        assert_eq!((s.project_calls, s.approx_tokens), (Some(2), Some(400)));
        let text = report.render();
        assert!(
            text.contains("≈ 200 tokens per call over 3 calls (from your measurements); here ≈ 400 tokens (2 calls in 30 days)"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_section_reads_top_down_with_a_next_step_per_part() {
        let report = ProjectReport {
            root: "/work/app".into(),
            window_days: 30,
            languages: vec![LanguageRow {
                language: "rust".into(),
                files: 12,
                marker: Some("Cargo.toml".into()),
                server: Some("rust-analyzer".into()),
                installed: false,
                install: Some("rustup component add rust-analyzer".into()),
                note: None,
                saving: None,
            }],
            walk_cut: true,
            graph: Some(GraphPart {
                registered: false,
                state: None,
                indexed: false,
                files: None,
                score: None,
                reasons: vec![],
                backend: "tags".into(),
                served_by: None,
                next: vec!["rtok graph index /work/app".into()],
            }),
            instructions: Instructions::default(),
            plugins: vec![PluginAdvice {
                plugin: "read".into(),
                set_in: ".rtok.toml".into(),
                saving: None,
            }],
        };
        assert_eq!(
            report.render(),
            "project /work/app\n\
             \x20 languages\n\
             \x20   rust 12 files, Cargo.toml, rust-analyzer not on PATH\n\
             \x20     install: rustup component add rust-analyzer\n\
             \x20     language server saving: no measurements yet\n\
             \x20   counts are a lower bound: the walk stopped at 50000 files or 2s\n\
             \x20 graph\n\
             \x20   not registered, not indexed\n\
             \x20   backend tags\n\
             \x20   next: rtok graph index /work/app\n\
             \x20 instructions\n\
             \x20   no AGENTS.md or CLAUDE.md\n\
             \x20 plugins off here\n\
             \x20   read: set `[plugins.read] enabled = true` in .rtok.toml; saving: no measurements yet\n"
        );
    }
}

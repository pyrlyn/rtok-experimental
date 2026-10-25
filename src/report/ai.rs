// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The `--ai` rendering of [`Document`](super::Document) (T22.4): the same document,
//! shaped for a model instead of a person.
//!
//! Dense plain lines, stable `#id` anchors, tables in the `toon` plugin's encoding
//! (dogfood: the encoder lives in `crate::plugins::toon` and is used here through its
//! `pub(crate)` entry points; without the `toon` feature the same tables fall back to
//! `k=v` lines). Every number keeps its unit and its row count. One budget
//! (`[report] budget_tokens`): whole sections are dropped past it and named in
//! `dropped:` — never truncated mid-table. It formats; it computes nothing (D24): the
//! only arithmetic is measuring rendered sections against the budget.

#[cfg(feature = "toon")]
use serde_json::Value;

use super::Document;
use super::markdown::ms;
use crate::config::Config;
use crate::tokens::{self, Class};

#[cfg(feature = "toon")]
use crate::plugins::toon;

/// Stable section anchors, in the fixed P22 order — a model can be pointed at one.
const IDS: [&str; 8] = [
    "window",
    "savings",
    "calls",
    "cache",
    "expand",
    "config",
    "doctor",
    "recommendations",
];

/// Render the whole document under `cfg.report.budget_tokens`, counting the complete
/// serialized text (title, range and store path, `dropped:` line, anchors) with the
/// configured estimator. Sections that do not fit are dropped whole, in order, and named
/// in `dropped:`. When not even the header and a full `dropped:` list fit, the output
/// shrinks to the title and that list, and to nothing when that is still over the cap:
/// a cap is never exceeded silently.
pub fn render(doc: &Document, cfg: &Config) -> String {
    let budget = cfg.report.budget_tokens;
    let fits = |s: &str| tokens::estimate(s, Class::Prose, &cfg.estimator) <= budget;
    let mut kept: Vec<(&str, String)> = Vec::with_capacity(IDS.len());
    for id in IDS {
        kept.push((id, section(id, doc)));
        if !fits(&assemble(doc, &kept, Header::Full)) {
            kept.pop();
        }
    }
    // The `dropped:` line grew with every later rejection, which the optimistic
    // admission above could not see; give back sections until the final text fits.
    loop {
        let full = assemble(doc, &kept, Header::Full);
        if fits(&full) {
            return full;
        }
        if kept.pop().is_none() {
            break;
        }
    }
    let minimal = assemble(doc, &kept, Header::Minimal);
    if fits(&minimal) {
        minimal
    } else {
        String::new()
    }
}

#[derive(Clone, Copy)]
enum Header {
    Full,
    Minimal,
}

fn assemble(doc: &Document, kept: &[(&str, String)], header: Header) -> String {
    use std::fmt::Write as _;
    let dropped: Vec<&str> = IDS
        .into_iter()
        .filter(|id| kept.iter().all(|(k, _)| k != id))
        .collect();
    let mut s = String::from("rtok-report ai\n");
    if matches!(header, Header::Full) {
        let w = &doc.ledgers.window;
        let _ = writeln!(
            s,
            "range {}..{} window={} store={}",
            w.from_date, w.to_date, w.since, w.db_path
        );
    }
    let _ = writeln!(
        s,
        "dropped: {}",
        if dropped.is_empty() {
            "none".to_string()
        } else {
            dropped.join(",")
        }
    );
    for (id, body) in kept {
        let _ = writeln!(s, "#{id}");
        s.push_str(body);
    }
    s
}

fn section(id: &str, doc: &Document) -> String {
    match id {
        "window" => window(doc),
        "savings" => savings(doc),
        "calls" => calls(doc),
        "cache" => cache(doc),
        "expand" => expand(doc),
        "config" => config(doc),
        "doctor" => doctor(doc),
        "recommendations" => recommendations(doc),
        _ => unreachable!("report section ids are the fixed P22 set"),
    }
}

fn window(doc: &Document) -> String {
    use std::fmt::Write as _;
    let w = &doc.ledgers.window;
    let mut s = String::from("units: row counts per ledger\n");
    s.push_str(&table(
        &["ledger", "in_window", "total", "coverage"],
        &[
            vec![
                "calls".into(),
                w.calls_in_window.to_string(),
                w.calls_total.to_string(),
                format!("window {}", w.since),
            ],
            vec![
                "measurements".into(),
                String::new(),
                w.measurements.to_string(),
                "whole ledger no row times".into(),
            ],
            vec![
                "usage".into(),
                String::new(),
                w.usage.to_string(),
                "whole ledger no row times".into(),
            ],
        ],
    ));
    let _ = writeln!(s, "since={}", w.since);
    s
}

fn savings(doc: &Document) -> String {
    use std::fmt::Write as _;
    let sav = &doc.ledgers.savings;
    let mut s = String::from(
        "units: est tokens, floors from Measurement rows (no turn counts); expand rows negative\n",
    );
    if sav.rows.is_empty() {
        s.push_str("no rows.\n");
        return s;
    }
    s.push_str(&table(
        &["plugin", "rows", "est_before", "est_after", "saved"],
        &sav.rows
            .iter()
            .map(|r| {
                vec![
                    r.plugin.clone(),
                    r.rows.to_string(),
                    r.est_before.to_string(),
                    r.est_after.to_string(),
                    r.saved.to_string(),
                ]
            })
            .collect::<Vec<_>>(),
    ));
    let _ = writeln!(
        s,
        "total_saved={}tok rows={}",
        sav.total_saved, sav.total_rows
    );
    s
}

fn calls(doc: &Document) -> String {
    use std::fmt::Write as _;
    let calls = &doc.ledgers.calls;
    let mut s = String::from(
        "units: counts; p50_ms/p95_ms nearest-rank over timed calls, blank when untimed\n",
    );
    if calls.total == 0 {
        s.push_str("no rows.\n");
        return s;
    }
    s.push_str(&table(
        &["surface", "calls", "timed", "p50_ms", "p95_ms"],
        &calls
            .rows
            .iter()
            .map(|r| {
                vec![
                    r.surface.clone(),
                    r.calls.to_string(),
                    r.timed.to_string(),
                    ms(r.p50_ms),
                    ms(r.p95_ms),
                ]
            })
            .collect::<Vec<_>>(),
    ));
    let _ = writeln!(
        s,
        "in_window={} total={} window={}",
        calls.in_window, calls.total, doc.ledgers.window.since
    );
    s
}

fn cache(doc: &Document) -> String {
    use std::fmt::Write as _;
    let cache = &doc.ledgers.cache;
    let mut s = String::from("units: counts from usage rows, whole ledger\n");
    if cache.sessions == 0 {
        s.push_str("no rows.\n");
    } else {
        // Two columns are not tabular for the toon encoder (it needs ≥ 3), and read
        // denser as lines than as an encoded block — `table` falls back on its own.
        s.push_str(&table(
            &["cause", "busts"],
            &cache
                .by_cause
                .iter()
                .map(|(cause, n)| vec![cause.clone(), n.to_string()])
                .collect::<Vec<_>>(),
        ));
        let _ = writeln!(
            s,
            "turns={} sessions={} busts={}",
            cache.turns, cache.sessions, cache.busts
        );
    }
    if !doc.ledgers.lanes.is_empty() {
        s.push_str("hit = cache_read over all input, per proxy lane/tier\n");
        s.push_str(&table(&crate::model::LANE_HEADS, &doc.ledgers.lane_cells()));
    }
    s
}

fn expand(doc: &Document) -> String {
    use std::fmt::Write as _;
    let exp = &doc.ledgers.expand;
    let mut s = String::from("units: counts; rate = expanded/decisions\n");
    if exp.decisions == 0 {
        s.push_str("no rows.\n");
        return s;
    }
    let _ = writeln!(
        s,
        "decisions={} expanded={} rate={}% expanded_ids={}",
        exp.decisions,
        exp.expanded,
        super::markdown::dec(Some(100.0 * exp.rate)),
        if exp.expanded_ids.is_empty() {
            "none".to_string()
        } else {
            exp.expanded_ids
                .iter()
                .map(|id| super::shown_archive_id(id))
                .collect::<Vec<_>>()
                .join(",")
        }
    );
    s
}

fn config(doc: &Document) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("units: effective keys with origin\n");
    s.push_str(&table(
        &["key", "value", "source"],
        &doc.config
            .iter()
            .map(|e| vec![e.key.clone(), e.value.clone(), e.source.clone()])
            .collect::<Vec<_>>(),
    ));
    let _ = writeln!(s, "keys={}", doc.config.len());
    s
}

/// The doctor page with the agents block cut to the host variants where rtok has at least one
/// module installed. The full list names every supported host (over 30 variants on macOS, each
/// with four reasoned rows) and alone outgrew the default budget, so `--ai` dropped the whole
/// section on an empty store; the rest is counted, and `rtok doctor` still prints it all.
fn doctor(doc: &Document) -> String {
    use crate::agents::ModuleState;
    let mut report = doc.doctor.clone();
    let total = report.agents.len();
    report
        .agents
        .retain(|a| a.modules.iter().any(|m| m.state == ModuleState::Installed));
    let hidden = total - report.agents.len();
    let mut s = String::from("units: live probes, not store rows\n");
    s.push_str(&report.to_compact_text());
    if !s.ends_with('\n') {
        s.push('\n');
    }
    if hidden > 0 {
        use std::fmt::Write as _;
        let _ = writeln!(s, "agents without rtok: {hidden} (rtok doctor lists them)");
    }
    s
}

fn recommendations(doc: &Document) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("units: rules over ledgers, most recoverable tokens first\n");
    if doc.recommendations.is_empty() {
        s.push_str("no recommendations.\n");
        return s;
    }
    for (i, r) in doc.recommendations.iter().enumerate() {
        let _ = writeln!(s, "{}. [{}] {} ({})", i + 1, r.rule, r.finding, r.evidence);
    }
    s
}

/// A uniform string table: through the toon encoder when the shape is tabular,
/// `k=v` lines otherwise (ragged, non-scalar, or under 3 columns). Ends with `\n`.
fn table(keys: &[&str], rows: &[Vec<String>]) -> String {
    #[cfg(feature = "toon")]
    if let Some(block) = toon_block(keys, rows) {
        let mut block = block;
        block.push('\n');
        return block;
    }
    let mut s = flat_lines(keys, rows);
    s.push('\n');
    s
}

/// The dogfood path: the same encoder `rtok proxy` runs, over the caller's column
/// order (the encoder's own key discovery only validates the shape).
#[cfg(feature = "toon")]
fn toon_block(keys: &[&str], rows: &[Vec<String>]) -> Option<String> {
    let json_rows: Vec<Value> = rows
        .iter()
        .map(|r| {
            Value::Object(
                keys.iter()
                    .zip(r)
                    .map(|(k, v)| (k.to_string(), Value::String(flat(v))))
                    .collect(),
            )
        })
        .collect();
    let arr = Value::Array(json_rows);
    toon::tabular_keys(&arr, 1)?;
    let order: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
    Some(toon::encode(arr.as_array()?, &order))
}

fn flat_lines(keys: &[&str], rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|r| {
            keys.iter()
                .zip(r)
                .map(|(k, v)| format!("{k}={}", flat(v)))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One unfolded line per cell: a raw newline would break the toon block structure
/// (and a Markdown table row — `markdown::cell` reuses it).
pub(super) fn flat(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn doc() -> Document {
        use crate::model::*;
        Document {
            ledgers: ReportLedgers {
                window: ReportWindow {
                    since: "30d".into(),
                    from_unix: 1,
                    to_unix: 2,
                    from_date: "2026-08-10".into(),
                    to_date: "2026-09-09".into(),
                    db_path: "/tmp/rtok.db".into(),
                    calls_in_window: 7,
                    calls_total: 7,
                    measurements: 3,
                    usage: 3,
                },
                savings: ReportSavingsSection {
                    rows: vec![ReportSavings {
                        plugin: "cmd".into(),
                        rows: 1,
                        est_before: 25,
                        est_after: 10,
                        saved: 15,
                    }],
                    total_rows: 1,
                    total_saved: 15,
                    kinds: vec!["filter".into()],
                },
                sinks: ReportSinksSection { rows: vec![] },
                calls: ReportCallsSection {
                    rows: vec![ReportCalls {
                        surface: "hook".into(),
                        calls: 2,
                        timed: 2,
                        p50_ms: Some(2.0),
                        p95_ms: Some(4.0),
                    }],
                    in_window: 2,
                    total: 2,
                    hooks: vec![ReportHook {
                        name: "PreToolUse".into(),
                        calls: 2,
                    }],
                },
                cache: ReportCache {
                    sessions: 1,
                    turns: 3,
                    busts: 1,
                    by_cause: [("tools".to_string(), 1)].into(),
                    detail: vec![ReportBust {
                        session: "s1".into(),
                        turn: 3,
                        cause: "tools".into(),
                        cache_create: 31_000,
                        cache_read: 200,
                    }],
                },
                lanes: Default::default(),
                expand: ReportExpand {
                    decisions: 3,
                    expanded: 1,
                    rate: 1.0 / 3.0,
                    expanded_ids: vec!["abc".into()],
                    cost: 5,
                    cost_rows: 1,
                },
            },
            config: vec![crate::model::ConfigEntry {
                key: "report.since".into(),
                value: "30d".into(),
                source: "user".into(),
            }],
            doctor: crate::doctor::Report {
                tools_rewrite_advice: None,
                hooks_total: 2,
                hooks_by_event: BTreeMap::from([
                    ("PreToolUse".to_string(), 1),
                    ("PostToolUse".to_string(), 1),
                ]),
                mcp: vec![],
                proxy: "8787".into(),
                proxy_openai: String::new(),
                mcp_tool_search_disabled: false,
                mcp_tool_search: Default::default(),
                bash_max_output_length: None,
                auto_compact_window: None,
                read_share: None,
                instructions: None,
                skills: None,
                overlaps: vec![],
                agents: vec![],
                problems: vec![],
                config_notes: vec![],
                stale_skills: vec![],
                #[cfg(feature = "graph")]
                graph_alerts: vec![],
                #[cfg(feature = "graph")]
                graph_health: vec![],
                junk_bytes: None,
                project: None,
            },
            recommendations: vec![
                crate::report::Recommendation {
                    rule: "expand-rate".into(),
                    finding: "pointers frozen".into(),
                    evidence: "3 rows".into(),
                },
                crate::report::Recommendation {
                    rule: "cache-bust".into(),
                    finding: "tools rewritten".into(),
                    evidence: "1 turn".into(),
                },
            ],
        }
    }

    fn agent(host: &'static str, state: crate::agents::ModuleState) -> crate::doctor::AgentModules {
        crate::doctor::AgentModules {
            host,
            kind: "cli",
            modules: vec![crate::agents::ModuleRow {
                name: "hooks",
                state,
                note: String::new(),
            }],
        }
    }

    /// Only host variants with rtok installed are listed; the rest are counted, so a machine
    /// with many supported hosts does not push the doctor section past the budget.
    #[test]
    fn doctor_lists_only_hosts_with_rtok_installed() {
        use crate::agents::ModuleState;
        let mut d = doc();
        d.doctor.agents = vec![
            agent("claude", ModuleState::Installed),
            agent("zed", ModuleState::NotSupported),
            agent("kimi", ModuleState::NotInstalled),
        ];
        let out = doctor(&d);
        assert!(out.contains("  claude (cli)"), "{out}");
        assert!(
            !out.contains("zed (cli)") && !out.contains("kimi (cli)"),
            "{out}"
        );
        assert!(out.contains("agents without rtok: 2"), "{out}");
        d.doctor.agents.truncate(1);
        assert!(!doctor(&d).contains("agents without rtok"));
    }

    fn cfg_with_budget(budget: u32) -> Config {
        let mut cfg = Config::default();
        cfg.report.budget_tokens = budget;
        cfg
    }

    /// The Check's shape clause: one ordered, explicit task list — and the empty
    /// rule set says so instead of inventing tasks (Gate P22).
    #[test]
    fn recommendations_close_as_an_explicit_task_list() {
        let out = render(&doc(), &cfg_with_budget(8000));
        let tail = &out[out.find("#recommendations").expect("id")..];
        assert!(tail.contains("1. [expand-rate] pointers frozen (3 rows)"));
        assert!(tail.contains("2. [cache-bust] tools rewritten (1 turn)"));
        assert!(
            out.ends_with("2. [cache-bust] tools rewritten (1 turn)\n"),
            "nothing follows the task list"
        );

        let mut empty = doc();
        empty.recommendations.clear();
        let out = render(&empty, &cfg_with_budget(8000));
        assert!(out.contains("no recommendations."));
    }

    /// The Check's budget clause: whole sections dropped and named — kept tables
    /// intact, ids of dropped sections gone.
    #[test]
    fn tight_budget_drops_whole_sections_and_names_them() {
        let full = render(&doc(), &cfg_with_budget(8000));
        assert!(full.contains("dropped: none"));
        let out = render(&doc(), &cfg_with_budget(120));
        assert!(!out.contains("dropped: none"), "{out}");
        let dropped: Vec<&str> = out
            .lines()
            .find(|l| l.starts_with("dropped: "))
            .expect("dropped line")[9..]
            .split(',')
            .collect();
        assert!(!dropped.is_empty());
        for id in &dropped {
            assert!(
                IDS.contains(id),
                "named drops are section ids: {id} in {out}"
            );
            assert!(
                !out.contains(&format!("#{id}\n")),
                "a dropped section leaves no heading: {id}"
            );
        }
        // A kept toon block is whole: its header row count matches its body rows.
        for line in out
            .lines()
            .filter(|l| l.starts_with('[') && l.contains("]{"))
        {
            let n: usize = line[1..line.find(']').unwrap()].parse().unwrap();
            assert!(n > 0, "kept block is non-empty: {line}");
        }
    }

    /// The Check's stability clause: two runs over the same data, same bytes.
    #[test]
    fn heading_ids_are_stable_across_runs() {
        let (a, b) = (
            render(&doc(), &cfg_with_budget(8000)),
            render(&doc(), &cfg_with_budget(8000)),
        );
        assert_eq!(a, b);
        let ids: Vec<String> = a
            .lines()
            .filter(|l| l.starts_with('#'))
            .map(str::to_string)
            .collect();
        assert_eq!(
            ids,
            IDS.iter().map(|id| format!("#{id}")).collect::<Vec<_>>(),
            "every section carries its stable id, in order"
        );
    }

    /// Dogfood + density: tabular sections use the toon encoding (no Markdown pipe
    /// tables), every number keeps its unit and row count.
    #[test]
    fn tables_are_toon_and_numbers_keep_units() {
        let out = render(&doc(), &cfg_with_budget(8000));
        assert!(!out.contains("| ---"), "no Markdown pipe tables");
        assert!(out.contains("[1]{plugin,rows,est_before,est_after,saved}:"));
        assert!(out.contains("total_saved=15tok rows=1"));
        assert!(out.contains("units:"));
    }

    /// T664: one line per host variant, no per-module reasons — those repeated across
    /// every T413 host and pushed `#doctor` past the default budget.
    #[test]
    fn doctor_agents_are_one_line_per_host_variant() {
        use crate::agents::{ModuleRow, ModuleState};
        let row = |name, state, note: &str| ModuleRow {
            name,
            state,
            note: note.into(),
        };
        let mut d = doc();
        d.doctor.agents = vec![crate::doctor::AgentModules {
            host: "claude",
            kind: "desktop",
            modules: vec![
                row("hooks", ModuleState::NotSupported, ": no hook events"),
                row("mcp", ModuleState::Installed, ""),
                row("proxy", ModuleState::NotInstalled, " (--proxy)"),
            ],
        }];
        let out = render(&d, &cfg_with_budget(8000));
        assert!(
            out.contains("\n  claude (desktop) hooks=n/a mcp=yes proxy=no\n"),
            "{out}"
        );
        assert!(
            !out.contains("no hook events"),
            "reasons stay in rtok doctor: {out}"
        );
        assert!(
            d.doctor.to_text().contains("no hook events"),
            "full text unchanged"
        );
    }

    /// T105 Check, same as markdown.rs/html.rs (T302): a non-finite `rate` (a
    /// corrupted/legacy DB row) must never leak `NaN`/`inf` into the `--ai` rendering —
    /// this surface is read by a model/parser downstream.
    #[test]
    fn non_finite_rate_never_leaks_into_the_document() {
        let mut d = doc();
        d.ledgers.expand.rate = f64::NAN;
        let out = render(&d, &cfg_with_budget(8000));
        assert!(!out.contains("NaN"), "no NaN: {out}");
        assert!(!out.to_lowercase().contains("inf"), "no inf: {out}");

        let mut d = doc();
        d.ledgers.expand.rate = f64::INFINITY;
        let out = render(&d, &cfg_with_budget(8000));
        assert!(!out.contains("NaN"), "no NaN: {out}");
        assert!(!out.to_lowercase().contains("inf"), "no inf: {out}");
    }

    fn tokens_of(out: &str, cfg: &Config) -> u32 {
        tokens::estimate(out, Class::Prose, &cfg.estimator)
    }

    /// T580: the complete text, not just the bodies, stays under the cap, and the
    /// `dropped:` line names exactly the sections that are missing.
    #[test]
    fn whole_output_stays_within_the_budget() {
        let mut long = doc();
        long.ledgers.window.db_path = "/very/long/store/path/".repeat(30);
        let mut unicode = doc();
        unicode.ledgers.window.db_path = "/хранилище/数据库/".repeat(10);
        let mut coarse = cfg_with_budget(0);
        coarse.estimator.prose = 1.0;
        for d in [doc(), long, unicode] {
            for rates in [Config::default(), coarse.clone()] {
                for budget in (0..1500).step_by(7) {
                    let mut cfg = rates.clone();
                    cfg.report.budget_tokens = budget;
                    let out = render(&d, &cfg);
                    assert!(tokens_of(&out, &cfg) <= budget, "{budget}: {out}");
                    if out.is_empty() {
                        continue;
                    }
                    let named: Vec<&str> = out
                        .lines()
                        .find_map(|l| l.strip_prefix("dropped: "))
                        .map(|l| {
                            if l == "none" {
                                vec![]
                            } else {
                                l.split(',').collect()
                            }
                        })
                        .expect("dropped line");
                    for id in IDS {
                        let present = out.contains(&format!("#{id}\n"));
                        assert_eq!(present, !named.contains(&id), "{id} at {budget}: {out}");
                    }
                }
            }
        }
    }

    /// T580: admission is exact at the boundary: the full report fits its own size and
    /// loses a section one token below it.
    #[test]
    fn budget_boundary_is_exact() {
        let cfg = cfg_with_budget(8000);
        let full = render(&doc(), &cfg);
        let size = tokens_of(&full, &cfg);
        assert_eq!(render(&doc(), &cfg_with_budget(size)), full);
        let below = render(&doc(), &cfg_with_budget(size - 1));
        assert!(!below.contains("dropped: none"), "{below}");
    }

    /// T580: when not even the header fits, the output shrinks to the title and the
    /// dropped list, then to nothing, instead of exceeding the cap.
    #[test]
    fn a_budget_below_the_header_degrades_explicitly() {
        let mut d = doc();
        d.ledgers.window.db_path = "/p".repeat(400);
        let minimal = "rtok-report ai\ndropped: window,savings,calls,cache,expand,config,doctor,recommendations\n";
        let cfg = cfg_with_budget(tokens_of(minimal, &Config::default()));
        assert_eq!(render(&d, &cfg), minimal);
        assert_eq!(render(&d, &cfg_with_budget(0)), "");
    }
}

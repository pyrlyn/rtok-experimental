// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The item breakdown (T330.6): every concrete item `agents junk list` reports and `clear`
//! would remove, with its size, when it was last used and why it is junk. `list` and the
//! `clear` dry run print through the same lines, so what the user reviewed in one is what the
//! other plans. The row is the JSON shape too.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use super::junk::Report;
use super::junk_cache::{Item, RTOK_OWN};
use super::junk_clear::{self, Planned};
use crate::config::Config;
use change_preview::human_bytes;

/// How many items a kind shows before "+N more" (T330: 10 largest).
pub const DEFAULT_SHOWN: usize = 10;
pub const SORTS: [&str; 3] = ["size", "last-used", "path"];

/// `--items`: how many items each kind prints; `0` prints totals only.
pub fn shown_arg(s: &str) -> Result<usize, String> {
    match s {
        "all" => Ok(usize::MAX),
        n => n.parse().map_err(|_| format!("not a count or `all`: {s}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    /// Largest first.
    Size,
    /// Longest unused first: the items worth reviewing. An unknown time sorts last, as the
    /// recent item it is treated as.
    LastUsed,
    Path,
}

impl Sort {
    pub fn parse(name: &str) -> Self {
        match name {
            "last-used" => Self::LastUsed,
            "path" => Self::Path,
            _ => Self::Size,
        }
    }
}

/// What a printed breakdown looks like: one run's flags plus the config it needs.
pub struct View {
    pub shown: usize,
    pub sort: Sort,
    pub min_size: u64,
    /// Raw byte counts instead of KB/MB/GB.
    pub exact: bool,
    /// Wrap each path in an OSC 8 link; only for a terminal.
    pub links: bool,
    /// `agents.junk.stale_session_days`, named in a session's reason.
    pub days: u32,
    pub now: SystemTime,
    /// Paths under it print with `~`.
    pub home: Option<PathBuf>,
}

impl View {
    pub fn new(cfg: &Config) -> Self {
        Self {
            days: cfg.agents.junk.stale_session_days,
            home: Some(super::home_dir()),
            ..Self::at(SystemTime::now())
        }
    }

    #[cfg(test)]
    pub fn test(exact: bool, links: bool) -> Self {
        Self {
            exact,
            links,
            ..Self::at(SystemTime::now())
        }
    }

    fn at(now: SystemTime) -> Self {
        Self {
            shown: DEFAULT_SHOWN,
            sort: Sort::Size,
            min_size: 0,
            exact: false,
            links: false,
            days: 30,
            now,
            home: None,
        }
    }

    fn size(&self, n: u64) -> String {
        if self.exact {
            n.to_string()
        } else {
            human_bytes(n)
        }
    }

    fn secs(&self, t: SystemTime) -> i64 {
        match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        }
    }
}

/// One item of the breakdown; the `--json` shape is its serialized form.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    #[serde(skip)]
    pub agent: &'static str,
    pub kind: &'static str,
    pub class: &'static str,
    pub path: String,
    pub size_bytes: u64,
    /// Unix seconds; `None` when it could not be read, which counts as recent.
    pub last_used: Option<i64>,
    pub reason: String,
    pub will_clear: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    /// What a `clear --yes` did to it ("removed"), for the text.
    #[serde(skip)]
    result: String,
    #[serde(skip)]
    evidence: &'static str,
    /// Part of what the kind line sums, so the text lists it.
    #[serde(skip)]
    counted: bool,
    #[serde(skip)]
    stamped: bool,
}

impl Row {
    fn of_item(agent: &'static str, i: &Item) -> Self {
        let skip_reason = match (&i.kept, i.class) {
            (Some(why), _) => Some(why.clone()),
            (None, "review") => Some("review kind: add --include review".to_string()),
            (None, "explicit") => Some(format!("explicit kind: name it with --kind {}", i.kind)),
            _ => None,
        };
        Self {
            agent,
            kind: i.kind,
            class: i.class,
            path: i.path.clone(),
            size_bytes: i.bytes,
            last_used: None,
            reason: String::new(),
            will_clear: skip_reason.is_none(),
            skip_reason,
            result: String::new(),
            evidence: i.evidence,
            counted: i.counted(),
            stamped: false,
        }
    }

    /// rtok's own log and archive files: `scan` already says why each is junk.
    fn of_own(o: &super::junk::Outcome) -> Self {
        Self {
            agent: "rtok",
            kind: o.kind,
            class: "safe",
            path: o.path.clone(),
            size_bytes: o.bytes,
            last_used: None,
            reason: o.note.clone(),
            will_clear: true,
            skip_reason: None,
            result: String::new(),
            evidence: RTOK_OWN,
            counted: true,
            stamped: false,
        }
    }

    /// The last-used time and the reason, read when a row is about to be shown: the walk costs
    /// as much as sizing the item, so a row that is hidden behind "+N more" never pays it.
    fn stamp(&mut self, view: &View) {
        if self.stamped {
            return;
        }
        self.stamped = true;
        let t = junk_clear::newest(Path::new(&self.path)).flatten();
        self.last_used = t.map(|t| view.secs(t));
        if self.reason.is_empty() {
            self.reason = reason(self.kind, self.evidence, self.last_used, view);
        }
    }
}

impl From<&Planned> for Row {
    fn from(p: &Planned) -> Self {
        let skipped = p.action == "skip";
        Self {
            agent: p.agent,
            kind: p.kind,
            class: "",
            path: p.path.clone(),
            size_bytes: p.bytes,
            last_used: p.last_used,
            reason: p.reason.clone(),
            will_clear: p.planned,
            skip_reason: skipped.then(|| p.note.clone()),
            result: if skipped {
                String::new()
            } else {
                p.note.clone()
            },
            evidence: "",
            counted: true,
            stamped: true,
        }
    }
}

/// Why an item is junk, in the words of the T330 card.
pub fn reason(kind: &str, evidence: &str, last_used: Option<i64>, view: &View) -> String {
    let days = last_used.map(|t| (view.secs(view.now) - t).max(0) / 86_400);
    let future = last_used.is_some_and(|t| t > view.secs(view.now));
    let base = match (kind, days) {
        ("sessions", Some(d)) => format!("not touched for {d} days (threshold {})", view.days),
        ("crash-dumps", Some(d)) => format!("crash dump {d} days old"),
        _ => format!("{kind} ({evidence})"),
    };
    if future {
        format!("{base}, future timestamp")
    } else {
        base
    }
}

/// Every item of the report, counted or not, with the reason `clear` leaves it.
pub fn rows(report: &Report) -> Vec<Row> {
    let own = report.own.iter().map(Row::of_own);
    let items = report
        .agents
        .iter()
        .flat_map(|a| a.items.iter().map(|i| Row::of_item(a.name, i)));
    own.chain(items).collect()
}

fn order(rows: &mut [Row], view: &View) {
    match view.sort {
        Sort::Size => {
            rows.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.path.cmp(&b.path)))
        }
        Sort::Path => rows.sort_by(|a, b| a.path.cmp(&b.path)),
        Sort::LastUsed => {
            rows.iter_mut().for_each(|r| r.stamp(view));
            // `None` (unknown) after every known time; `Reverse` would put it first.
            rows.sort_by(|a, b| match (a.last_used, b.last_used) {
                (Some(x), Some(y)) => x.cmp(&y).then(a.path.cmp(&b.path)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.path.cmp(&b.path),
            });
        }
    }
}

/// The rows of one kind under one agent, cut to what the text prints.
pub struct Group {
    pub agent: &'static str,
    pub kind: &'static str,
    shown: Vec<Row>,
    more: usize,
    more_bytes: u64,
}

impl Group {
    pub fn lines(&self, view: &View, indent: &str) -> String {
        let mut out = String::new();
        if view.shown == 0 {
            return out;
        }
        for r in &self.shown {
            let path = tilde(&r.path, view.home.as_deref());
            let path = if view.links {
                super::junk::folder_link(&r.path, &path)
            } else {
                path
            };
            out.push_str(&format!(
                "{indent}{path}  {}  last used {}  {}",
                view.size(r.size_bytes),
                when(r.last_used, view),
                r.reason
            ));
            if let Some(why) = &r.skip_reason {
                out.push_str(&format!("  (not cleared: {why})"));
            } else if !r.result.is_empty() {
                out.push_str(&format!("  ({})", r.result));
            }
            out.push('\n');
        }
        if self.more > 0 {
            out.push_str(&format!(
                "{indent}+{} more ({})\n",
                self.more,
                view.size(self.more_bytes)
            ));
        }
        out
    }
}

/// The text groups, in the order their first row came. Only counted rows: what the kind line
/// sums. The ones `clear` leaves for a stated reason are the lines `to_list` already prints.
pub fn groups(rows: Vec<Row>, view: &View) -> Vec<Group> {
    let mut by: Vec<((&'static str, &'static str), Vec<Row>)> = Vec::new();
    for r in rows
        .into_iter()
        .filter(|r| r.counted && r.size_bytes >= view.min_size)
    {
        let key = (r.agent, r.kind);
        match by.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.1.push(r),
            None => by.push((key, vec![r])),
        }
    }
    by.into_iter()
        .map(|((agent, kind), mut rows)| {
            order(&mut rows, view);
            let hidden = rows.split_off(rows.len().min(view.shown));
            rows.iter_mut().for_each(|r| r.stamp(view));
            Group {
                agent,
                kind,
                shown: rows,
                more: hidden.len(),
                more_bytes: hidden.iter().map(|r| r.size_bytes).sum(),
            }
        })
        .collect()
}

/// `report` as JSON with every item of each agent under `items`: the shape of [`Row`].
pub fn json(report: &Report, view: &View) -> Value {
    let mut out = serde_json::to_value(report).unwrap_or(Value::Null);
    let mut all = rows(report);
    all.retain(|r| r.size_bytes >= view.min_size);
    all.iter_mut().for_each(|r| r.stamp(view));
    order(&mut all, view);
    if let Some(agents) = out.get_mut("agents").and_then(Value::as_array_mut) {
        for a in agents {
            let name = a.get("name").and_then(Value::as_str).unwrap_or_default();
            let mine: Vec<&Row> = all.iter().filter(|r| r.agent == name).collect();
            a["items"] = serde_json::to_value(mine).unwrap_or(Value::Null);
        }
    }
    out
}

/// What `plan` stores on a [`Planned`]: the time and the reason the breakdown prints.
pub fn stamp_planned(
    kind: &str,
    evidence: &str,
    path: &str,
    note: &str,
    view: &View,
) -> (Option<i64>, String) {
    let last = junk_clear::newest(Path::new(path))
        .flatten()
        .map(|t| view.secs(t));
    let why = if note.is_empty() {
        reason(kind, evidence, last, view)
    } else {
        note.to_string()
    };
    (last, why)
}

fn tilde(path: &str, home: Option<&Path>) -> String {
    match home.and_then(|h| Path::new(path).strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.to_string(),
    }
}

/// `3 weeks ago (2026-09-09 14:02)`, in the local time zone.
fn when(last: Option<i64>, view: &View) -> String {
    let Some(t) = last else {
        return "unknown".to_string();
    };
    let age = (view.secs(view.now) - t).max(0);
    let unit = |n: i64, name: &str| format!("{n} {name}{} ago", if n == 1 { "" } else { "s" });
    let rel = match age {
        0..=59 => "just now".to_string(),
        60..=3_599 => unit(age / 60, "minute"),
        3_600..=172_799 => unit(age / 3_600, "hour"),
        172_800..=1_209_599 => unit(age / 86_400, "day"),
        1_209_600..=5_183_999 => unit(age / 604_800, "week"),
        _ => unit(age / 2_592_000, "month"),
    };
    let abs = jiff::Timestamp::from_second(t)
        .map(|ts| {
            ts.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    format!("{rel} ({abs})")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        View {
            shown: 2,
            sort: Sort::Size,
            min_size: 0,
            exact: true,
            links: false,
            days: 30,
            now: UNIX_EPOCH + std::time::Duration::from_secs(100 * 86_400),
            home: Some(PathBuf::from("/home/u")),
        }
    }

    fn row(path: &str, size: u64, last: Option<i64>) -> Row {
        Row {
            agent: "claude",
            kind: "cache",
            class: "safe",
            path: path.into(),
            size_bytes: size,
            last_used: last,
            reason: "cache (research.md §22)".into(),
            will_clear: true,
            skip_reason: None,
            result: String::new(),
            evidence: "research.md §22",
            counted: true,
            stamped: true,
        }
    }

    #[test]
    fn a_kind_shows_its_largest_items_then_the_rest_as_one_line() {
        let rows = vec![
            row("/a", 10, None),
            row("/b", 30, None),
            row("/c", 20, None),
        ];
        let g = &groups(rows, &view())[0];
        let text = g.lines(&view(), "  ");
        assert!(text.starts_with("  /b  30  last used unknown"), "{text}");
        assert_eq!(text.lines().count(), 3, "{text}");
        assert!(text.ends_with("  +1 more (10)\n"), "{text}");
    }

    #[test]
    fn items_zero_prints_no_item_line_and_all_prints_every_item() {
        let rows = || vec![row("/a", 10, None), row("/b", 30, None)];
        let none = View { shown: 0, ..view() };
        assert_eq!(groups(rows(), &none)[0].lines(&none, ""), "");
        let all = View {
            shown: shown_arg("all").unwrap(),
            ..view()
        };
        assert_eq!(groups(rows(), &all)[0].lines(&all, "").lines().count(), 2);
        assert!(shown_arg("x").is_err());
    }

    #[test]
    fn min_size_hides_smaller_items_and_sort_orders_by_time_or_path() {
        let v = View {
            min_size: 15,
            shown: 9,
            ..view()
        };
        let rows = vec![
            row("/a", 10, None),
            row("/b", 30, None),
            row("/c", 20, None),
        ];
        assert_eq!(groups(rows.clone(), &v)[0].shown.len(), 2);
        let by_time = View {
            sort: Sort::LastUsed,
            shown: 9,
            ..view()
        };
        let rows = vec![
            row("/a", 1, Some(500)),
            row("/b", 1, None),
            row("/c", 1, Some(100)),
        ];
        let order: Vec<_> = groups(rows, &by_time)[0]
            .shown
            .iter()
            .map(|r| r.path.clone())
            .collect();
        assert_eq!(order, ["/c", "/a", "/b"], "oldest first, unknown last");
        let by_path = View {
            sort: Sort::Path,
            shown: 9,
            ..view()
        };
        let rows = vec![row("/b", 1, None), row("/a", 1, None)];
        assert_eq!(groups(rows, &by_path)[0].shown[0].path, "/a");
    }

    #[test]
    fn reasons_name_the_threshold_and_a_future_time() {
        let v = view();
        let old = v.secs(v.now) - 35 * 86_400;
        assert_eq!(
            reason("sessions", "x", Some(old), &v),
            "not touched for 35 days (threshold 30)"
        );
        assert_eq!(
            reason("crash-dumps", "x", Some(v.secs(v.now) - 12 * 86_400), &v),
            "crash dump 12 days old"
        );
        assert_eq!(
            reason("cache", "research.md §22", None, &v),
            "cache (research.md §22)"
        );
        assert!(
            reason("sessions", "x", Some(v.secs(v.now) + 60), &v).ends_with("future timestamp")
        );
    }

    #[test]
    fn times_and_paths_read_like_the_card() {
        let v = view();
        let t = v.secs(v.now);
        assert!(when(Some(t - 21 * 86_400), &v).starts_with("3 weeks ago ("));
        assert!(when(Some(t - 3_600), &v).starts_with("1 hour ago ("));
        assert_eq!(when(None, &v), "unknown");
        assert_eq!(
            tilde("/home/u/.claude/cache", v.home.as_deref()),
            "~/.claude/cache"
        );
        assert_eq!(tilde("/var/x", v.home.as_deref()), "/var/x");
    }

    #[test]
    fn a_review_item_is_listed_with_why_it_is_not_cleared() {
        let item = Item {
            kind: "index",
            class: "review",
            path: "/x".into(),
            bytes: 5,
            evidence: "research.md §22",
            kept: None,
        };
        let r = Row::of_item("claude", &item);
        assert!(!r.will_clear);
        assert_eq!(
            r.skip_reason.as_deref(),
            Some("review kind: add --include review")
        );
        let g = &groups(vec![r], &View { shown: 9, ..view() })[0];
        assert!(g.lines(&view(), "").contains("(not cleared: review kind"));
    }
}

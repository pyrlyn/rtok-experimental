// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok agents junk clear` with filters (T330.4): the plan is the `list` report's own items
//! (no second scanner) cut by `--agent`, `--kind`, `--include review` and `--older-than`, and
//! each item is checked again right before it goes. With no filter, or `--agent rtok` alone,
//! the command is T182's run unchanged (T344, read conservatively): new deletions need a flag.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use schemars::JsonSchema;
use serde::Serialize;

use super::junk::{self, AGENT_SCAN_LIMIT, Report};
use super::junk_cache::{GC_VERDICT, RTOK_OWN, SECTION_22, TAG, real};
use super::junk_items::{self, Row, View};
use super::junk_kinds::{PACKAGE_LOCKS, held_reason};
use super::junk_review::EXTRA;
use super::junk_worktrees::{self, Remover};
use super::restart::{RealProcs, host_running};
use super::{HOSTS, host};
use crate::config::Config;
use crate::store::Store;
use crate::worktree::list::{is_cache_dir, usage_until};
use change_preview::human_bytes;

/// What `--kind` takes today: rtok's own T182 junk, then the T330.3, T330.5.1, T330.5.2 and
/// T330.5.4 kinds. `snapshots` is accepted and never clears anything: its class is `never`.
pub const KINDS: [&str; 15] = [
    "log",
    "archive",
    "cache",
    "temp",
    "logs",
    "build",
    "deps",
    "locks",
    "backups",
    "swap",
    "snapshots",
    "index",
    "crash-dumps",
    "sessions",
    "stale-worktrees",
];

/// Kinds a running agent may be writing right now. A §22 cache joins them; a tagged cache is
/// judged by T152's idle rule whether or not its agent runs (T342).
const LIVE_KINDS: [&str; 4] = ["temp", "locks", "swap", "index"];

/// An item touched this recently is in use, whatever the scan said.
const SETTLE: Duration = Duration::from_secs(60);

const RUNNING: &str = "agent running";

/// The `clear` flags that choose what goes.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub agents: Vec<String>,
    pub kinds: Vec<String>,
    pub include_review: bool,
    pub older_than: Option<Duration>,
    pub trash: bool,
    /// Only items at least this big: what the dry run shows is what `--yes` removes (T330.6).
    pub min_size: u64,
}

impl Filter {
    /// No selecting flag, or `--agent rtok` alone: exactly T182's junk.
    pub fn is_t182(&self) -> bool {
        self.kinds.is_empty()
            && !self.include_review
            && self.older_than.is_none()
            && self.min_size == 0
            && self.agents.iter().all(|a| a == "rtok")
    }

    fn wants_agent(&self, name: &str) -> bool {
        self.agents.is_empty() || self.agents.iter().any(|a| a == name)
    }

    /// The T330 class gate: `safe` by default, `review` when named or with `--include review`,
    /// `explicit` only when named, `never` never.
    fn wants_kind(&self, kind: &str, class: &str) -> bool {
        let named = self.kinds.iter().any(|k| k == kind);
        if !self.kinds.is_empty() && !named {
            return false;
        }
        match class {
            "safe" => true,
            "review" => named || self.include_review,
            "explicit" => named,
            _ => false,
        }
    }
}

/// `--agent`'s value: `rtok` or a host id. An error here is a usage error (exit 2).
pub fn agent_arg(s: &str) -> Result<String, String> {
    if s == "rtok" || HOSTS.contains(&s) {
        Ok(s.to_string())
    } else {
        Err(format!(
            "not an agent: {s} (rtok or a host id; see `rtok agents list`)"
        ))
    }
}

/// One item of the plan and what happened to it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Planned {
    pub agent: &'static str,
    pub kind: &'static str,
    pub path: String,
    pub bytes: u64,
    /// Files in it, counted for a planned item before anything is removed.
    pub files: u64,
    /// Unix seconds of the newest file in it; `None` when unreadable (T330.6).
    pub last_used: Option<i64>,
    /// Why it is junk, as `list` says it.
    pub reason: String,
    /// In the plan: false for an item left at planning time (a running agent's).
    pub planned: bool,
    /// `clear` (removed, or would be on a dry run) or `skip`.
    pub action: &'static str,
    pub note: String,
    /// Planned and not removed: the re-check refused it or the removal failed.
    pub failed: bool,
    #[serde(skip)]
    evidence: &'static str,
    /// The real path at planning time: another one at removal means a link was swapped in.
    #[serde(skip)]
    real: PathBuf,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Cleared {
    pub yes: bool,
    pub items: Vec<Planned>,
    pub planned_bytes: u64,
    pub freed_bytes: u64,
}

impl Cleared {
    pub fn failed(&self) -> bool {
        self.items.iter().any(|p| p.failed)
    }
}

/// The newest file mtime under `path` (T152's measure: a folder's own mtime is not use), inner
/// `None` for a folder without files; `None` when the walk did not finish, so an unknown age
/// never reads as old.
pub(super) fn newest(path: &Path) -> Option<Option<SystemTime>> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() {
        return Some(meta.modified().ok());
    }
    let (u, cut) = usage_until(path, Instant::now() + AGENT_SCAN_LIMIT);
    (!cut).then_some(u.modified)
}

fn age(now: SystemTime, t: SystemTime) -> Duration {
    now.duration_since(t).unwrap_or_default()
}

/// Archive retention: `--older-than` may only widen `core.retain_calls_days`, and 0 (keep
/// forever) stays 0.
fn retention_days(cfg: &Config, f: &Filter) -> u32 {
    let days = cfg.core.retain_calls_days;
    let floor = f.older_than.map_or(0, |d| d.as_secs().div_ceil(86_400));
    match days {
        0 => 0,
        d => d.max(u32::try_from(floor).unwrap_or(u32::MAX)),
    }
}

/// The plan: every counted item of `report` the filter wants, each path once (a shared folder
/// goes once). `running` answers per host; it is asked only when an item depends on it.
pub fn plan(
    cfg: &Config,
    report: &Report,
    f: &Filter,
    now: SystemTime,
    running: &dyn Fn(&str) -> bool,
) -> Vec<Planned> {
    let t182 = Filter {
        agents: vec!["rtok".into()],
        kinds: vec!["log".into(), "archive".into()],
        ..f.clone()
    };
    let f = if f.is_t182() { &t182 } else { f };
    let old = |p: &Path| {
        let old_enough = |o| newest(p).is_some_and(|t| t.is_none_or(|t| age(now, t) >= o));
        f.older_than.is_none_or(old_enough)
    };
    let mut out: Vec<Planned> = Vec::new();
    let view = View {
        now,
        ..View::new(cfg)
    };
    let item = |agent, kind, path: &str, bytes, evidence, note: Option<&str>, why: &str| {
        let (last_used, reason) = junk_items::stamp_planned(kind, evidence, path, why, &view);
        Planned {
            agent,
            kind,
            path: path.to_string(),
            bytes,
            files: 0,
            last_used,
            reason,
            planned: note.is_none(),
            action: if note.is_none() { "clear" } else { "skip" },
            note: note.unwrap_or_default().to_string(),
            failed: false,
            evidence,
            real: real(Path::new(path)),
        }
    };
    for a in report.agents.iter().filter(|a| f.wants_agent(a.name)) {
        if !a.host {
            let own = junk::scan_with(cfg, retention_days(cfg, f));
            let own = own.iter().filter(|o| f.wants_kind(o.kind, "safe"));
            let own = own.filter(|o| o.bytes >= f.min_size);
            for o in own.filter(|o| o.kind == "archive" || old(Path::new(&o.path))) {
                out.push(item(
                    a.name, o.kind, &o.path, o.bytes, RTOK_OWN, None, &o.note,
                ));
            }
        }
        let mut live: Option<bool> = None;
        for i in a.items.iter().filter(|i| i.counted()) {
            // D36 evidence only, whatever else marked the item counted.
            let documented = matches!(i.evidence, SECTION_22 | TAG | RTOK_OWN | EXTRA | GC_VERDICT);
            if !documented
                || i.bytes < f.min_size
                || !f.wants_kind(i.kind, i.class)
                || !old(Path::new(&i.path))
            {
                continue;
            }
            // A path the user named is no more known to tolerate a running agent than §22's.
            let sensitive =
                LIVE_KINDS.contains(&i.kind) || matches!(i.evidence, SECTION_22 | EXTRA);
            let busy = a.host && sensitive && *live.get_or_insert_with(|| running(a.name));
            let note = busy.then_some(RUNNING);
            out.push(item(a.name, i.kind, &i.path, i.bytes, i.evidence, note, ""));
        }
    }
    // A folder shared with a running agent stays, whichever owner was listed first.
    let busy: Vec<String> = out
        .iter()
        .filter(|p| !p.planned)
        .map(|p| p.path.clone())
        .collect();
    out.retain(|p| !p.planned || !busy.contains(&p.path));
    // An emptied folder keeps its own block on most filesystems (4 KiB on ext4), so its size
    // never reaches 0; with nothing left inside it is not worth planning again.
    out.retain(|p| !std::fs::read_dir(&p.path).is_ok_and(|mut d| d.next().is_none()));
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.path.clone()));
    out
}

/// Paths `clear` never removes nor empties a folder around: the store, every host's settings
/// files, and (by name) package-manager lockfiles.
fn protected(cfg: &Config) -> Vec<PathBuf> {
    let db = cfg.core.db_path.display().to_string();
    let mut out: Vec<PathBuf> = ["", "-wal", "-shm"]
        .map(|s| PathBuf::from(format!("{db}{s}")))
        .to_vec();
    for a in HOSTS.iter().filter_map(|id| host(id)) {
        out.extend(a.variants().iter().flat_map(|v| a.markers(cfg, v.kind)));
    }
    out.iter().map(|p| real(p)).collect()
}

/// Why `p` must stay now, and whether that is a failure ("already gone" is not).
fn recheck(p: &Planned, keep: &[PathBuf], now: SystemTime) -> Option<(String, bool)> {
    let path = Path::new(&p.path);
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Some(("already gone".into(), false));
        }
        Err(e) => return Some((format!("cannot check it: {e}"), true)),
    };
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let here = real(path);
    let why = if meta.file_type().is_symlink() {
        Some("a symlink: never followed".to_string())
    } else if here != p.real {
        Some("changed since plan: its real path moved".into())
    } else if PACKAGE_LOCKS.contains(&name) || keep.iter().any(|k| k.starts_with(&here)) {
        Some("is or holds a protected file (store, settings, lockfile)".into())
    } else if p.evidence == TAG && !is_cache_dir(path) {
        Some("no longer a tagged cache".into())
    } else {
        match newest(path) {
            None => Some("scan stopped: not cleared".into()),
            Some(Some(t)) if t > now || age(now, t) < SETTLE => {
                Some("modified in the last minute".into())
            }
            Some(_) => meta.is_file().then(|| held_reason(path)).flatten(),
        }
    };
    why.map(|w| (w, true))
}

fn discard(path: &Path, trash: bool) -> std::io::Result<()> {
    if trash {
        // Only macOS sets a delete method below; elsewhere the binding is never mutated.
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut ctx = trash::TrashContext::default();
        // Finder would ask for Automation permission and play a sound; a CLI wants neither.
        #[cfg(target_os = "macos")]
        trash::macos::TrashContextExtMacos::set_delete_method(
            &mut ctx,
            trash::macos::DeleteMethod::NsFileManager,
        );
        return ctx.delete(path).map_err(std::io::Error::other);
    }
    // `remove_dir_all` removes a symlink inside, never what it points to.
    if std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// A cache keeps its top folder (an app may expect it) and its `CACHEDIR.TAG` (T330).
fn remove(p: &Planned, trash: bool) -> std::io::Result<()> {
    let path = Path::new(&p.path);
    if !matches!(p.kind, "cache" | "build" | "index") {
        return discard(path, trash);
    }
    for e in std::fs::read_dir(path)? {
        let e = e?;
        if e.file_name() != super::junk_cache::TAG {
            discard(&e.path(), trash)?;
        }
    }
    Ok(())
}

/// Re-checks and removes every planned item. Archives go through the store's retention with
/// their rows, as T182 does, never one file at a time and never to the trash.
pub fn apply(cfg: &Config, items: &mut [Planned], f: &Filter, now: SystemTime) {
    let keep = protected(cfg);
    let todo = items.iter_mut().filter(|p| p.action == "clear");
    let mut archives = Vec::new();
    let mut worktrees = Remover::default();
    for p in todo {
        if p.kind == "archive" {
            archives.push(p);
            continue;
        }
        let res = match recheck(p, &keep, now) {
            Some((why, failed)) => Err((why, failed)),
            // git unregisters a worktree, so there is no folder to move to the trash.
            None if p.kind == junk_worktrees::KIND && f.trash => {
                Err(("git removes a worktree: not moved to trash".into(), true))
            }
            None if p.kind == junk_worktrees::KIND => worktrees
                .remove(cfg, now, Path::new(&p.path))
                .map(Some)
                .map_err(|e| (e, true)),
            None => remove(p, f.trash)
                .map(|()| None)
                .map_err(|e| (format!("failed, kept: {e}"), true)),
        };
        match res {
            Ok(note) => {
                let done = if f.trash { "moved to trash" } else { "removed" };
                p.note = note.unwrap_or_else(|| done.into());
            }
            Err((why, failed)) => (p.action, p.note, p.failed) = ("skip", why, failed),
        }
    }
    if archives.is_empty() {
        return;
    }
    let days = retention_days(cfg, f);
    let store = Store::open(&cfg.core.db_path);
    let done = store.and_then(|s| s.run_retention(days, cfg.core.retain_hook_bodies_days));
    for p in archives {
        match &done {
            Ok(_) => p.note = "removed".into(),
            Err(e) => (p.action, p.note, p.failed) = ("skip", format!("failed, kept: {e}"), true),
        }
    }
}

/// A host runs when a live rtok session names it (T284) or its binary or app is up. The
/// process probe is off under the test harness: tests fake a running agent through the store.
fn running(cfg: &Config) -> impl Fn(&str) -> bool {
    let store = Store::open(&cfg.core.db_path).ok();
    let live = store
        .as_ref()
        .and_then(|s| s.live_agents(&cfg.agents.idle).ok())
        .unwrap_or_default();
    let probe = !super::host_sandboxed();
    move |id: &str| {
        let hid = store.as_ref().and_then(|s| s.host_id(id).ok().flatten());
        let by_session = hid.is_some_and(|h| live.iter().any(|r| r.host_id == h));
        by_session || (probe && host(id).is_some_and(|a| host_running(a, &RealProcs)))
    }
}

/// `clear` beyond T182: scan as `list` does, plan, and with `yes` apply.
pub fn run(cfg: &Config, f: &Filter, yes: bool) -> Result<Cleared> {
    Ok(run_in(cfg, &junk::report(cfg), f, yes, None))
}

/// [`run`] over a report already taken, cut to `only`: the paths a user was shown and
/// confirmed, so an item that appeared since their plan is never removed unseen. `None` takes
/// the whole plan.
pub fn run_in(
    cfg: &Config,
    report: &Report,
    f: &Filter,
    yes: bool,
    only: Option<&[String]>,
) -> Cleared {
    let now = SystemTime::now();
    let mut items = plan(cfg, report, f, now, &running(cfg));
    if let Some(only) = only {
        items.retain(|p| only.contains(&p.path));
    }
    for p in items.iter_mut().filter(|p| p.planned) {
        p.files = crate::removal_preview::size_of(Path::new(&p.path)).1;
    }
    if yes {
        apply(cfg, &mut items, f, now);
    }
    summed(items, yes)
}

pub fn summed(items: Vec<Planned>, yes: bool) -> Cleared {
    let planned_bytes = items.iter().filter(|p| p.planned).map(|p| p.bytes).sum();
    let freed = items.iter().filter(|p| yes && p.action == "clear");
    Cleared {
        yes,
        freed_bytes: freed.map(|p| p.bytes).sum(),
        planned_bytes,
        items,
    }
}

/// Each kind's items as `list` prints them, then planned and freed bytes per agent and kind,
/// then the total.
pub fn to_text(c: &Cleared, view: &View) -> String {
    let rows: Vec<Row> = c.items.iter().map(Row::from).collect();
    let mut out = String::new();
    for g in junk_items::groups(rows, view) {
        out.push_str(&g.lines(view, "  "));
        let of = c
            .items
            .iter()
            .filter(|p| (p.agent, p.kind) == (g.agent, g.kind));
        let planned: u64 = of.clone().filter(|p| p.planned).map(|p| p.bytes).sum();
        let freed: u64 = of
            .filter(|p| c.yes && p.action == "clear")
            .map(|p| p.bytes)
            .sum();
        let what = if c.yes {
            format!(
                "freed {} of {} planned",
                human_bytes(freed),
                human_bytes(planned)
            )
        } else {
            format!("{} planned", human_bytes(planned))
        };
        out.push_str(&format!("{} {}: {what}\n", g.agent, g.kind));
    }
    let n = c.items.iter().filter(|p| p.planned).count();
    if !c.yes && n > 0 {
        let planned = c.items.iter().filter(|p| p.planned);
        let planned = planned.map(|p| (p.path.as_str(), p.bytes, p.files));
        out.push_str(&crate::removal_preview::render(planned));
    }
    out.push_str(&match (c.yes, n) {
        (_, 0) => "nothing to clear\n".to_string(),
        (false, n) => format!(
            "dry run: {n} items, {} to free, nothing changed; rerun with --yes\n",
            human_bytes(c.planned_bytes)
        ),
        (true, _) => format!(
            "Freed {} of {} planned\n",
            human_bytes(c.freed_bytes),
            human_bytes(c.planned_bytes)
        ),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::junk::AgentJunk;
    use crate::agents::junk_cache::{Item, age_files, tagged};

    const DAY: u64 = 86_400;

    fn put(path: &Path, n: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; n]).unwrap();
    }

    fn item(kind: &'static str, path: &Path, evidence: &'static str) -> Item {
        Item {
            kind,
            class: "safe",
            path: path.display().to_string(),
            bytes: 10,
            evidence,
            kept: None,
        }
    }

    fn row(name: &'static str, items: Vec<Item>) -> AgentJunk {
        AgentJunk {
            name,
            host: name != "rtok",
            installed: true,
            folders: Vec::new(),
            total_bytes: 0,
            kinds: Vec::new(),
            items,
            notes: Vec::new(),
            freed_default_bytes: 0,
            freed_review_bytes: 0,
        }
    }

    fn report(agents: Vec<AgentJunk>) -> Report {
        Report {
            agents,
            own: Vec::new(),
            total_bytes: 0,
            freed_default_bytes: 0,
            freed_review_bytes: 0,
        }
    }

    fn filter(agents: &[&str], kinds: &[&str]) -> Filter {
        Filter {
            agents: agents.iter().map(|s| s.to_string()).collect(),
            kinds: kinds.iter().map(|s| s.to_string()).collect(),
            ..Filter::default()
        }
    }

    fn name(p: &Planned) -> &str {
        Path::new(&p.path).file_name().unwrap().to_str().unwrap()
    }

    fn rows(items: &[Planned]) -> Vec<(&str, &str, &str, &str)> {
        let rows = items.iter().map(|p| (p.agent, p.kind, p.action, name(p)));
        rows.collect::<Vec<_>>()
    }

    fn idle(_: &str) -> bool {
        false
    }

    fn classed(kind: &'static str, class: &'static str, path: &Path, ev: &'static str) -> Item {
        Item {
            class,
            ..item(kind, path, ev)
        }
    }

    /// T330.5.1: a §22 log goes only with `--include review` or `--kind logs`, an `extra` temp
    /// file by default, a snapshot never (not even named), and a running host keeps its `extra`
    /// paths like a §22 one.
    #[test]
    fn review_kinds_need_the_flag_extra_paths_plan_and_snapshots_never_do() {
        let (cfg, dir) = crate::testutil::config("junk-clear-review");
        let p = |n: &str| dir.join(n);
        let mut snap = classed("snapshots", "never", &p("history"), SECTION_22);
        snap.kept = Some("snapshot: never cleared".into());
        let r = report(vec![row(
            "claude",
            vec![
                classed("logs", "review", &p("old.log"), SECTION_22),
                classed("temp", "safe", &p("old.tmp"), EXTRA),
                classed("logs", "review", &p("app.log"), EXTRA),
                classed("backups", "review", &p("s.json.bak-1"), RTOK_OWN),
                snap,
            ],
        )]);
        let now = SystemTime::now();
        let names = |f: &Filter, running: &dyn Fn(&str) -> bool| {
            let planned = plan(&cfg, &r, f, now, running);
            let names = planned
                .iter()
                .filter(|p| p.planned)
                .map(|p| name(p).to_owned());
            names.collect::<Vec<_>>()
        };
        assert_eq!(names(&filter(&["claude"], &[]), &idle), ["old.tmp"]);
        let review = Filter {
            include_review: true,
            ..filter(&["claude"], &[])
        };
        let all = ["old.log", "old.tmp", "app.log", "s.json.bak-1"];
        assert_eq!(names(&review, &idle), all);
        let logs = names(&filter(&[], &["logs"]), &idle);
        assert_eq!(logs, ["old.log", "app.log"]);
        assert!(names(&filter(&[], &["snapshots"]), &idle).is_empty());
        // Running: the §22 log and the `extra` paths stay, rtok's own backup does not.
        assert_eq!(names(&review, &|_| true), ["s.json.bak-1"]);
    }

    #[test]
    fn the_class_gate_and_the_t182_selection() {
        let f = filter(&[], &[]);
        assert!(f.wants_kind("cache", "safe") && !f.wants_kind("logs", "review"));
        assert!(!f.wants_kind("sessions", "explicit") && !f.wants_kind("snapshots", "never"));
        let review = Filter {
            include_review: true,
            ..f.clone()
        };
        assert!(review.wants_kind("logs", "review") && !review.wants_kind("sessions", "explicit"));
        let named = filter(&[], &["sessions", "logs"]);
        assert!(named.wants_kind("sessions", "explicit") && named.wants_kind("logs", "review"));
        assert!(!named.wants_kind("cache", "safe") && !named.wants_kind("snapshots", "never"));
        assert!(f.is_t182() && filter(&["rtok"], &[]).is_t182());
        assert!(!filter(&["claude"], &[]).is_t182() && !filter(&[], &["cache"]).is_t182());
        assert!(agent_arg("rtok").is_ok() && agent_arg("claude").is_ok());
        assert!(agent_arg("nope").is_err());
    }

    /// `--agent` and `--kind` cut the report; `--agent rtok` alone stays T182's (its cache is
    /// left); a running host keeps its temp and §22 cache but not its tagged cache; a find
    /// without D36 evidence, a kept item and a second path of a shared folder never plan.
    #[test]
    fn the_plan_follows_the_filters_running_hosts_and_evidence() {
        let (cfg, dir) = crate::testutil::config("junk-clear-plan");
        let p = |n: &str| dir.join(n);
        let mut kept = item("cache", &p("fresh"), TAG);
        kept.kept = Some("modified within the idle window".into());
        let r = report(vec![
            row("rtok", vec![item("cache", &p("rtok-cache"), RTOK_OWN)]),
            row(
                "claude",
                vec![
                    item("temp", &p("snap"), SECTION_22),
                    item("cache", &p("cc-cache"), SECTION_22),
                    item("build", &p("target"), TAG),
                    item("locks", &p("x.lock"), "none"),
                    kept,
                ],
            ),
            row("codex", vec![item("cache", &p("cc-cache"), SECTION_22)]),
        ]);
        let now = SystemTime::now();
        let every = Filter {
            include_review: true,
            ..Filter::default()
        };
        let all = plan(&cfg, &r, &every, now, &idle);
        assert_eq!(
            rows(&all)[..1],
            [("rtok", "cache", "clear", "rtok-cache")],
            "{all:?}"
        );
        assert_eq!(
            all.len(),
            4,
            "shared, kept and undocumented ones drop: {all:?}"
        );
        let t182 = plan(&cfg, &r, &filter(&["rtok"], &[]), now, &idle);
        assert!(
            t182.is_empty(),
            "no log or archive in the fixture: {t182:?}"
        );
        let cache = plan(&cfg, &r, &filter(&["claude"], &["cache"]), now, &idle);
        assert_eq!(rows(&cache), [("claude", "cache", "clear", "cc-cache")]);
        let busy = plan(&cfg, &r, &filter(&["claude"], &[]), now, &|h| h == "claude");
        assert_eq!(
            rows(&busy),
            [
                ("claude", "temp", "skip", "snap"),
                ("claude", "cache", "skip", "cc-cache"),
                ("claude", "build", "clear", "target"),
            ]
        );
        assert_eq!(busy[0].note, RUNNING);
        assert!(!busy[0].planned && !busy[0].failed);
        let shared = plan(&cfg, &r, &every, now, &|h| h == "claude");
        let cc = shared.iter().filter(|x| x.path.ends_with("cc-cache"));
        let cc: Vec<_> = cc.map(|x| (x.agent, x.action)).collect();
        assert_eq!(
            cc,
            [("claude", "skip")],
            "a shared folder of a running agent stays"
        );
    }

    #[test]
    fn older_than_takes_only_what_was_not_touched_for_that_long() {
        let (cfg, dir) = crate::testutil::config("junk-clear-older");
        put(&dir.join("old/a"), 3);
        put(&dir.join("new/a"), 3);
        age_files(&dir.join("old"), 10 * DAY);
        age_files(&dir.join("new"), 2 * DAY);
        let r = report(vec![row(
            "claude",
            vec![
                item("temp", &dir.join("old"), SECTION_22),
                item("temp", &dir.join("new"), SECTION_22),
            ],
        )]);
        let f = Filter {
            older_than: Some(Duration::from_secs(7 * DAY)),
            ..Filter::default()
        };
        let planned = plan(&cfg, &r, &f, SystemTime::now(), &idle);
        assert_eq!(rows(&planned), [("claude", "temp", "clear", "old")]);
        assert_eq!(retention_days(&cfg, &f), cfg.core.retain_calls_days.max(7));
    }

    /// `--yes` re-checks each item: a cache is emptied with its top folder and tag kept, an idle
    /// temp entry goes; a fresh file, a symlink, a folder holding the store, a lockfile and a
    /// path already gone stay, and only "already gone" is not a failure.
    #[test]
    fn apply_rechecks_every_item_and_keeps_what_changed_or_is_protected() {
        let (cfg, dir) = crate::testutil::config("junk-clear-apply");
        let p = |n: &str| dir.join(n);
        put(&p("cache/blob"), 100);
        put(&p("cache/sub/blob"), 100);
        tagged(&p("cache"));
        put(&p("temp/old"), 10);
        put(&p("busy/f"), 10);
        put(&p("lock/Cargo.lock"), 10);
        put(&cfg.core.db_path, 10);
        put(&p("outside/important"), 10);
        for d in ["cache", "temp", "lock"] {
            age_files(&p(d), 2 * DAY);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(p("outside"), p("link")).unwrap();
        let db_dir = cfg.core.db_path.parent().unwrap().to_path_buf();
        let mut items = vec![
            item("cache", &p("cache"), TAG),
            item("temp", &p("temp/old"), SECTION_22),
            item("temp", &p("busy/f"), SECTION_22),
            item("temp", &p("lock/Cargo.lock"), SECTION_22),
            item("cache", &db_dir, RTOK_OWN),
            item("temp", &p("gone"), SECTION_22),
        ];
        if cfg!(unix) {
            items.push(item("temp", &p("link"), SECTION_22));
        }
        let r = report(vec![row("claude", items)]);
        let f = Filter::default();
        let mut planned = plan(
            &cfg,
            &r,
            &filter(&["claude"], &[]),
            SystemTime::now(),
            &idle,
        );
        let before = rows(&planned).len();
        apply(&cfg, &mut planned, &f, SystemTime::now());
        assert_eq!(rows(&planned).len(), before);
        let note = |name: &str| {
            let hit = planned.iter().find(|x| x.path.ends_with(name)).unwrap();
            (hit.action, hit.note.as_str(), hit.failed)
        };
        assert_eq!(note("cache"), ("clear", "removed", false));
        assert!(p("cache").is_dir() && p("cache/CACHEDIR.TAG").is_file());
        assert!(!p("cache/blob").exists() && !p("cache/sub").exists());
        assert_eq!(note("old"), ("clear", "removed", false));
        assert!(!p("temp/old").exists());
        assert_eq!(note("f"), ("skip", "modified in the last minute", true));
        let held = "is or holds a protected file (store, settings, lockfile)";
        assert_eq!(note("Cargo.lock"), ("skip", held, true));
        assert_eq!(
            note(db_dir.file_name().unwrap().to_str().unwrap()).0,
            "skip"
        );
        assert!(cfg.core.db_path.is_file() && p("lock/Cargo.lock").is_file());
        assert_eq!(note("gone"), ("skip", "already gone", false));
        if cfg!(unix) {
            assert_eq!(note("link"), ("skip", "a symlink: never followed", true));
            assert!(p("outside/important").is_file());
        }
        let c = summed(planned, true);
        assert!(c.failed());
        assert_eq!(c.freed_bytes, 20);
        let text = to_text(&c, &View::test(false, false));
        assert!(
            text.contains("claude cache: freed 10 B of 20 B planned"),
            "{text}"
        );
        // The symlink item (10 B) is only planned where the test can make one.
        let planned = if cfg!(unix) { 70 } else { 60 };
        assert!(
            text.ends_with(&format!("Freed 20 B of {planned} B planned\n")),
            "{text}"
        );
    }

    /// A live rtok session of a host is how a test fakes a running agent: no process probe.
    #[test]
    fn a_live_rtok_session_marks_its_host_running() {
        let (cfg, _dir) = crate::testutil::config("junk-clear-live");
        let store = Store::open(&cfg.core.db_path).unwrap();
        let claude = store.host_id("claude").unwrap().unwrap();
        store.register_agent(claude, "s", None, None, None).unwrap();
        assert!(running(&cfg)("claude"));
    }

    #[test]
    fn a_dry_run_changes_nothing_and_says_so() {
        let (cfg, dir) = crate::testutil::config("junk-clear-dry");
        put(&dir.join("cache/blob"), 10);
        let r = report(vec![row(
            "claude",
            vec![item("cache", &dir.join("cache"), SECTION_22)],
        )]);
        let planned = plan(
            &cfg,
            &r,
            &filter(&["claude"], &[]),
            SystemTime::now(),
            &idle,
        );
        let text = to_text(&summed(planned, false), &View::test(false, false));
        assert!(dir.join("cache/blob").is_file());
        assert!(text.contains("claude cache: 10 B planned"), "{text}");
        assert!(
            text.ends_with("dry run: 1 items, 10 B to free, nothing changed; rerun with --yes\n")
        );
        let none = summed(Vec::new(), true);
        assert_eq!(
            to_text(&none, &View::test(false, false)).lines().last(),
            Some("nothing to clear")
        );
    }
}

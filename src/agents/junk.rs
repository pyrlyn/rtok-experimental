// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok agents junk clear` (T182): remove junk rtok owns under its own home — log siblings
//! left behind past `[log] files` and archive payloads past `core.retain_calls_days`. With
//! `--yes` it also clears old hook bodies (`core.retain_hook_bodies_days`) and converts the
//! store to incremental vacuum if it is not already (T352; `mcp`/`proxy` do the same in the
//! background at session start). Dry run by default; `--yes` applies. Bare `clear` never
//! touches a host directory; with filters it is `junk_clear` (T330.4). `list` (T330.1, T330.2) also shows every installed host's folders from
//! the `junk_map` of `research.md` §22, read-only, sized with a per-agent time limit.
//!
//! Inventory for T182 found nothing else unbounded in `~/.rtok`: log rotation (`src/log.rs`)
//! already caps generations on every write, archive retention (`Store::run_retention`) already
//! runs at proxy/MCP session start, and the semantic cache and graph index live inside
//! `rtok.db`, not as loose files — this command makes the first two runnable on demand and
//! reports what they would do before anyone applies them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;

use super::junk_cache::{self, Ctx, Item, Owned};
use super::junk_items::{self, View};
use super::junk_kinds;
use super::junk_map::{Role, Roots, specs};
use super::junk_review;
use super::junk_sessions;
use super::junk_worktrees;
use super::{Agent, HOSTS, host, present};
use crate::config::Config;
use crate::store::Store;
use change_preview::human_bytes;

#[derive(Debug, Serialize)]
pub struct Outcome {
    /// `log` or `archive`.
    pub kind: &'static str,
    pub path: String,
    pub bytes: u64,
    /// A log sibling or archive is one file.
    pub files: u64,
    /// `clear` or `keep` (only after `--yes` failed to remove it).
    pub action: &'static str,
    pub note: String,
    pub failed: bool,
}

/// `rtok.log.<N>` siblings beyond `[log] files`. `rotate()` (`src/log.rs`) only ever drops the
/// single generation past the *current* cap on each write, so lowering `files` after some ran
/// leaves the older generations behind forever — the one real unbounded case this inventory
/// found in rtok's own home.
fn stale_log_siblings(cfg: &Config) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (
        cfg.log.path.parent(),
        cfg.log.path.file_name().and_then(|n| n.to_str()),
    ) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let fname = e.file_name();
            let suffix = fname.to_str()?.strip_prefix(name)?.strip_prefix('.')?;
            (suffix.parse::<u32>().ok()? > cfg.log.files).then(|| e.path())
        })
        .collect()
}

fn outcome(kind: &'static str, path: PathBuf, note: String) -> Outcome {
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Outcome {
        kind,
        path: path.display().to_string(),
        bytes,
        files: 1,
        action: "clear",
        note,
        failed: false,
    }
}

/// Everything `clear` would touch. Read-only; an unreadable path is skipped, not fatal
/// (fail open).
pub fn scan(cfg: &Config) -> Vec<Outcome> {
    scan_with(cfg, cfg.core.retain_calls_days)
}

/// [`scan`] with the archive retention window `days` (`clear --older-than` may only widen it).
pub fn scan_with(cfg: &Config, days: u32) -> Vec<Outcome> {
    let mut out: Vec<Outcome> = stale_log_siblings(cfg)
        .into_iter()
        .map(|p| outcome("log", p, format!("past `[log] files` = {}", cfg.log.files)))
        .collect();
    if let Ok(store) = Store::open(&cfg.core.db_path)
        && let Ok(paths) = store.archives_pending_retention(days)
    {
        out.extend(paths.into_iter().map(|p| {
            outcome(
                "archive",
                p,
                format!("past `core.retain_calls_days` = {days}"),
            )
        }));
    }
    out
}

/// Dry run without `yes`. With it: removes the stray log siblings directly, then runs the
/// same store retention `proxy`/`mcp` already run at session start — same rows, same files,
/// just on demand. Never touches `rtok.db` itself or an archive still referenced by a call.
pub fn run(cfg: &Config, yes: bool) -> Vec<Outcome> {
    let mut outcomes = scan(cfg);
    if !yes {
        return outcomes;
    }
    for o in outcomes.iter_mut().filter(|o| o.kind == "log") {
        match std::fs::remove_file(&o.path) {
            Ok(()) => o.note = "removed".into(),
            Err(e) => {
                o.failed = true;
                o.action = "keep";
                o.note = format!("failed, kept: {e}");
            }
        }
    }
    if let Ok(store) = Store::open(&cfg.core.db_path) {
        // Fire-and-forget on disk like `run_retention` itself (T75/T182): a file it could not
        // remove is simply left for the next run, never fatal here.
        let _ = store.run_retention(cfg.core.retain_calls_days, cfg.core.retain_hook_bodies_days);
        // T352: the same one-time conversion `mcp`/`proxy` session start runs in the background,
        // here on demand (a no-op once the store is incremental).
        let _ = store.convert_to_incremental_vacuum();
        for o in outcomes.iter_mut().filter(|o| o.kind == "archive") {
            o.note = "removed".into();
        }
    }
    outcomes
}

pub fn to_table(outcomes: &[Outcome], yes: bool) -> String {
    let head = ["action", "kind", "path", "bytes"]
        .map(String::from)
        .to_vec();
    let mut lines = vec![head];
    lines.extend(outcomes.iter().map(|o| {
        vec![
            o.action.into(),
            o.kind.into(),
            o.path.clone(),
            human_bytes(o.bytes),
        ]
    }));
    let cols = [
        crate::render::Col::left(0),
        crate::render::Col::left(0),
        crate::render::Col::left(0),
        crate::render::Col::right(0),
    ];
    let notes = outcomes.iter().map(|o| o.note.as_str());
    let mut out = crate::worktree::noted_table(&cols, &lines, notes);
    let cleared = outcomes.iter().filter(|o| o.action == "clear" && !o.failed);
    let (n, bytes) = cleared.fold((0, 0), |(n, b), o| (n + 1, b + o.bytes));
    if !yes && n > 0 {
        let planned = outcomes.iter().filter(|o| o.action == "clear");
        let planned = planned.map(|o| (o.path.as_str(), o.bytes, o.files));
        out.push('\n');
        out.push_str(&crate::removal_preview::render(planned));
    }
    out.push_str(&match (yes, n) {
        (false, 0) => "\nnothing to clear\n".to_string(),
        (false, n) => format!(
            "\ndry run: {n} items, {} to free, nothing changed; rerun with --yes\n",
            human_bytes(bytes)
        ),
        (true, n) => format!("\nfreed {} in {n} items\n", human_bytes(bytes)),
    });
    out
}

/// One folder an agent writes to, with its disk usage.
#[derive(Debug, Serialize)]
pub struct Folder {
    pub path: String,
    /// `data`, `temp`, `logs` or `cache` (`junk_map::Role`); rtok's own folders are `data`.
    pub role: &'static str,
    pub size_bytes: u64,
    /// A row of `research.md` §22 backs this path (D36). Anything else is listed read-only:
    /// "not documented: not cleared".
    pub documented: bool,
    /// The other agents whose folder resolves to this same directory. It is sized once.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shared_with: Vec<&'static str>,
    /// Why the size may be low or the path is not what it looks like: permission denied, scan
    /// timeout, or a symlink to the real folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One junk kind under an agent: what `clear` would remove, summed.
#[derive(Debug, Serialize)]
pub struct KindRow {
    pub kind: &'static str,
    /// `safe`: always regenerated, cleared by default (T330 classes).
    pub class: &'static str,
    pub items: usize,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct AgentJunk {
    pub name: &'static str,
    /// A host from `agents::HOSTS`; `rtok` itself is the one row that is not.
    pub host: bool,
    pub installed: bool,
    pub folders: Vec<Folder>,
    /// Disk usage of the folders, a folder nested in another one counted once.
    pub total_bytes: u64,
    pub kinds: Vec<KindRow>,
    /// Every directory `clear` may empty (T330.3), with the evidence and, for one it would
    /// leave alone, the reason. Only an item without a reason is in `kinds` and the totals.
    pub items: Vec<Item>,
    /// Lines `list` prints under the kinds: the session threshold in use and the host's own
    /// session retention (T330.5.2).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub freed_default_bytes: u64,
    /// What `--include review` would free on top of the `safe` kinds (T330.5.1).
    pub freed_review_bytes: u64,
}

/// `rtok agents junk list` (T330.1): per-agent folders, junk kinds and space `clear` frees.
#[derive(Debug, Serialize)]
pub struct Report {
    pub agents: Vec<AgentJunk>,
    /// rtok's own log and archive files `clear` would remove: no `Item`, so the totals keep
    /// counting them through `kinds`; the breakdown lists them from here.
    #[serde(skip)]
    pub own: Vec<Outcome>,
    /// Every folder once, however many agents share it.
    pub total_bytes: u64,
    pub freed_default_bytes: u64,
    pub freed_review_bytes: u64,
}

pub use crate::disk_usage::{AGENT_SCAN_LIMIT, Usage, disk_usage, disk_usage_until};

/// The directories rtok's junk lives in: the log directory and the archive directory. One
/// nested inside the other is folded into the outer so a byte is shown once.
fn rtok_folders(cfg: &Config, cache: &Path) -> Vec<Folder> {
    let dirs: Vec<PathBuf> = [
        cfg.log.path.parent(),
        Some(cfg.core.archive_dir.as_path()),
        Some(cache),
    ]
    .into_iter()
    .flatten()
    .filter(|d| d.is_dir())
    .map(Path::to_path_buf)
    .collect();
    outermost(dirs)
        .into_iter()
        .map(|d| Folder {
            size_bytes: disk_usage(&d),
            path: d.display().to_string(),
            role: if d == cache { "cache" } else { "data" },
            documented: true,
            shared_with: Vec::new(),
            note: None,
        })
        .collect()
}

/// The cache directories rtok writes: the language-server state it confines to each registered
/// project (`lsp::lsp_state_root`, a tool may be using it, so T152's idle rule applies), its
/// own platform cache dir and the plugin copies Claude Code no longer uses (T279).
fn rtok_owned(cfg: &Config, roots: &Roots, cache: &Path) -> Vec<Owned> {
    let mut owned = vec![Owned {
        path: cache.to_path_buf(),
        evidence: junk_cache::RTOK_OWN,
        idle_rule: false,
    }];
    owned.extend(project_lsp_dirs(cfg, &["cache", "pub-cache"]));
    owned.extend(junk_kinds::staging_caches(&roots.resolve("{claude}")));
    owned
}

/// The `subdirs` (`cache`, `pub-cache`, `data`, `state`) of the language-server state rtok
/// confines to each registered project. Absent from the `measure`-only build: that binary has
/// no graph plugin, so it never writes these dirs.
#[cfg(feature = "graph")]
fn project_lsp_dirs(cfg: &Config, subdirs: &[&str]) -> Vec<Owned> {
    let projects = Store::open(&cfg.core.db_path).and_then(|s| s.projects());
    projects
        .unwrap_or_default()
        .into_iter()
        .flat_map(|p| {
            let state = crate::plugins::graph::lsp::lsp_state_root(Path::new(&p.root));
            subdirs.iter().map(move |d| Owned {
                path: state.join(d),
                evidence: junk_cache::RTOK_OWN,
                idle_rule: true,
            })
        })
        .collect()
}

#[cfg(not(feature = "graph"))]
fn project_lsp_dirs(_cfg: &Config, _subdirs: &[&str]) -> Vec<Owned> {
    Vec::new()
}

/// `paths` without any that sits inside another one, so a directory is walked once.
fn outermost(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort();
    paths.dedup();
    let all = paths.clone();
    paths.retain(|d| !all.iter().any(|o| o != d && d.starts_with(o)));
    paths
}

/// The junk kinds in the order the T330 tables list them.
const KINDS: [&str; 13] = [
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

/// The `stale-worktrees` items that belong to agent row `name`.
fn stale_for<'a>(
    stale: &'a [(&'static str, Item)],
    name: &'a str,
) -> impl Iterator<Item = Item> + 'a {
    stale
        .iter()
        .filter(move |(n, _)| *n == name)
        .map(|(_, i)| i.clone())
}

/// What `list` says about the session threshold in use (T330 "Old sessions: time only").
fn session_note(days: u32) -> String {
    let note = format!("old sessions: not touched for more than {days} days");
    match days {
        0 => format!("{note} (threshold 0: every closed session is junk)"),
        _ => note,
    }
}

/// One row per kind and class of `items`: only an item `clear` would take now counts. A kind
/// row says its class, so `review` rows are counted apart from what a plain `clear` frees, and
/// an `explicit` row (sessions) is shown but belongs to neither "Freed" total.
fn kind_rows(items: &[Item]) -> Vec<KindRow> {
    let mut rows = Vec::new();
    for kind in KINDS {
        for class in ["safe", "review", "explicit"] {
            let counted = items.iter().filter(|i| i.kind == kind && i.class == class);
            let counted = counted.filter(|i| i.counted());
            let (n, bytes) = counted.fold((0, 0), |(n, b), i| (n + 1, b + i.bytes));
            if n > 0 {
                rows.push(KindRow {
                    kind,
                    class,
                    items: n,
                    size_bytes: bytes,
                });
            }
        }
    }
    rows
}

/// What `clear` frees: the `safe` rows, plus the `review` ones with `review`.
fn freed(rows: &[KindRow], review: bool) -> u64 {
    let wanted = rows
        .iter()
        .filter(|k| k.class == "safe" || (review && k.class == "review"));
    wanted.map(|k| k.size_bytes).sum()
}

/// The folders a host's own config files sit in, under the home `Roots` names only: a marker
/// such as Claude Desktop's config has a fixed platform path no fixture can redirect, and a test
/// must not size the real one. rtok keeps its `_backup` copies of those files there (T249).
fn marker_dirs(a: &dyn Agent, cfg: &Config, roots: &Roots) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = a
        .variants()
        .iter()
        .flat_map(|v| a.markers(cfg, v.kind))
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .filter(|d| {
            d.parent().is_some() && d != roots.home() && d.starts_with(roots.home()) && d.is_dir()
        })
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

/// A folder a host writes, before it is sized.
struct Found {
    path: PathBuf,
    role: Role,
    documented: bool,
    per_commit: bool,
}

/// The folders of one host that exist: the §22 map, then the folders its own config files sit
/// in (a host with no §22 row, Cursor, still shows where its settings live, read-only). A
/// config-file folder inside a folder already found is dropped so a byte is not listed twice,
/// and the home directory itself is never a folder (Aider's files sit directly in it).
fn host_folders(a: &dyn Agent, cfg: &Config, roots: &Roots) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    for s in specs(a.id()) {
        let path = roots.resolve(&s.path);
        let known = found.iter().any(|f| f.path == path);
        if !known && std::fs::symlink_metadata(&path).is_ok() {
            found.push(Found {
                path,
                role: s.role,
                documented: s.documented,
                per_commit: s.per_commit,
            });
        }
    }
    let mut parents = marker_dirs(a, cfg, roots);
    parents.sort_by_key(|d| d.components().count());
    for d in parents {
        if !found.iter().any(|f| d.starts_with(&f.path)) {
            found.push(Found {
                path: d,
                role: Role::Data,
                documented: false,
                per_commit: false,
            });
        }
    }
    found.sort_by(|x, y| x.path.cmp(&y.path));
    found
}

/// What the real path of `path` is, when it is a symlink: the folder is sized at its target
/// (the link itself costs nothing) and the listing says so.
fn symlink_target(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    meta.file_type()
        .is_symlink()
        .then(|| dunce::canonicalize(path).ok())
        .flatten()
}

/// Every host of `HOSTS` as an agent row: installed ones, or all of them with `all`. The
/// folders are sized once however many agents share them, each agent within `limit`.
fn host_rows(
    cfg: &Config,
    roots: &Roots,
    opts: &Options,
    limit: Duration,
    cx: &Ctx,
) -> (Vec<AgentJunk>, BTreeMap<PathBuf, u64>) {
    let mut rows: Vec<AgentJunk> = Vec::new();
    let mut sized: HashMap<PathBuf, Usage> = HashMap::new();
    // Canonical folder -> the agents that own it, for the "shared with" note.
    let mut owners: HashMap<PathBuf, Vec<&'static str>> = HashMap::new();
    let mut keys: Vec<Vec<PathBuf>> = Vec::new();
    for &id in HOSTS {
        let Some(a) = host(id) else { continue };
        let installed = a.variants().iter().any(present);
        if !installed && !opts.all {
            continue;
        }
        let deadline = Instant::now() + limit;
        let mut folders = Vec::new();
        let mut agent_keys = Vec::new();
        let mut owned = Vec::new();
        let mut temp = Vec::new();
        let mut logs = Vec::new();
        for f in host_folders(a, cfg, roots) {
            let target = symlink_target(&f.path);
            let key = target.clone().unwrap_or_else(|| f.path.clone());
            if f.per_commit {
                owned.extend(junk_kinds::stale_code_caches(&f.path));
            } else if f.role == Role::Cache && f.documented {
                owned.push(Owned {
                    path: f.path.clone(),
                    evidence: junk_cache::SECTION_22,
                    idle_rule: false,
                });
            }
            if f.role == Role::Temp && f.documented {
                temp.push(f.path.clone());
            }
            if f.role == Role::Logs && f.documented {
                logs.push(f.path.clone());
            }
            let usage = *sized
                .entry(key.clone())
                .or_insert_with(|| disk_usage_until(&key, Some(deadline)));
            let mut notes = Vec::new();
            if let Some(t) = &target {
                notes.push(format!("symlink to {}", t.display()));
            }
            if usage.denied {
                notes.push("permission denied: size is a lower bound".into());
            }
            if usage.timed_out {
                notes.push(format!(
                    "scan stopped after {}s: size is a lower bound",
                    limit.as_secs()
                ));
            }
            owners.entry(key.clone()).or_default().push(a.id());
            agent_keys.push(key);
            folders.push(Folder {
                path: f.path.display().to_string(),
                role: f.role.as_str(),
                size_bytes: usage.bytes,
                documented: f.documented,
                shared_with: Vec::new(),
                note: (!notes.is_empty()).then(|| notes.join("; ")),
            });
        }
        let junk = &cfg.agents.junk;
        let temp_age = junk_review::hours(junk.temp_min_age_hours);
        let (extra_owned, extra) =
            junk_review::extra_items(junk, a.id(), &cfg.home, roots.home(), cx, limit);
        owned.extend(extra_owned);
        let tag_roots = outermost(agent_keys.clone());
        let mut items = junk_cache::cache_items(&owned, &tag_roots, cx, limit);
        items.extend(extra);
        items.extend(junk_kinds::temp_items(&temp, temp_age, cx, limit));
        items.extend(junk_review::log_items(&logs, junk, cx, limit));
        items.extend(junk_kinds::found_items(&tag_roots, limit));
        let backups = marker_dirs(a, cfg, roots);
        items.extend(junk_review::backup_items(
            &backups,
            cfg.setup.backup_files,
            limit,
        ));
        if a.id() == "gemini" {
            items.extend(junk_review::snapshot_items(roots, limit));
        }
        items.extend(junk_review::diagnostic_reports(
            &junk_review::process_names(a),
            roots,
            limit,
        ));
        let mut notes = Vec::new();
        if a.id() == "claude" {
            let old = junk_sessions::session_items(roots, junk.stale_session_days, cx, limit);
            if !old.is_empty() {
                notes.push(session_note(junk.stale_session_days));
            }
            items.extend(old);
        }
        if !folders.is_empty() {
            notes.extend(junk_sessions::retention(roots, a.id()));
        }
        let own: Vec<PathBuf> = opts
            .worktrees
            .iter()
            .filter(|(h, _)| *h == a.id())
            .map(|(_, p)| p.clone())
            .collect();
        items.extend(junk_kinds::build_items(&own, cx, limit));
        items.extend(stale_for(&opts.stale_worktrees, a.id()));
        junk_review::exclude(&mut items, &junk.exclude, &cfg.home, roots.home(), limit);
        let items = junk_cache::drop_nested(items);
        let kinds = kind_rows(&items);
        keys.push(agent_keys);
        rows.push(AgentJunk {
            name: a.id(),
            host: true,
            installed,
            folders,
            total_bytes: 0,
            freed_default_bytes: freed(&kinds, false),
            freed_review_bytes: freed(&kinds, true),
            kinds,
            items,
            notes,
        });
    }
    for (row, agent_keys) in rows.iter_mut().zip(&keys) {
        for (folder, key) in row.folders.iter_mut().zip(agent_keys) {
            folder.shared_with = owners[key]
                .iter()
                .copied()
                .filter(|n| *n != row.name)
                .collect();
        }
        row.total_bytes = top_level_bytes(
            row.folders
                .iter()
                .zip(agent_keys)
                .map(|(f, k)| (k.as_path(), f.size_bytes)),
        );
    }
    let by_key = sized.into_iter().map(|(k, u)| (k, u.bytes)).collect();
    (rows, by_key)
}

/// The sum of the folders that are not inside another listed folder, so a nested log folder
/// is not added on top of its data home.
fn top_level_bytes<'a>(folders: impl Iterator<Item = (&'a Path, u64)> + Clone) -> u64 {
    let all: Vec<&Path> = folders.clone().map(|(p, _)| p).collect();
    folders
        .filter(|(p, _)| !all.iter().any(|o| o != p && p.starts_with(o)))
        .map(|(_, b)| b)
        .sum()
}

/// Which hosts a [`report`] covers and the T152 rules a cache is judged by.
#[derive(Debug, Clone)]
pub struct Options {
    /// Also hosts that are not installed (their rows have no folders unless one is left over).
    pub all: bool,
    /// A tagged or language-server cache modified within this window stays (T152, T342).
    pub idle: Duration,
    /// Where the command runs: a tagged cache of that worktree stays.
    pub cwd: PathBuf,
    pub now: SystemTime,
    /// The agent worktrees `build` looks in, as `(host, path)` ([`junk_kinds::agent_worktrees`]).
    /// Empty unless the caller lists them: finding them runs git.
    pub worktrees: Vec<(&'static str, PathBuf)>,
    /// The `stale-worktrees` items (T330.5.3) with the agent row each belongs to; `rtok` for a
    /// worktree no host is known to have made. Empty unless the caller judges them: it runs git.
    pub stale_worktrees: Vec<(&'static str, Item)>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            all: false,
            idle: junk_cache::DEFAULT_IDLE,
            cwd: std::env::current_dir().unwrap_or_default(),
            now: SystemTime::now(),
            worktrees: Vec::new(),
            stale_worktrees: Vec::new(),
        }
    }
}

/// Read-only: the same [`scan`] `clear` runs, summed per kind for rtok, plus every installed
/// host's folders (T330.2) and the cache each may clear (T330.3.1). `clear` with filters
/// plans from this report (`junk_clear`, T330.4).
pub fn report(cfg: &Config) -> Report {
    report_in_repo(cfg, false)
}

/// [`report`] over the repository the command runs in, for `all` hosts or only the installed
/// ones: the agent worktrees `build` looks in and the `stale-worktrees` gc would remove.
pub fn report_in_repo(cfg: &Config, all: bool) -> Report {
    let opts = Options {
        all,
        ..Options::default()
    };
    let found = junk_worktrees::worktrees(cfg, &opts.cwd);
    let stale = junk_worktrees::items(cfg, &opts.cwd, &found, opts.now, AGENT_SCAN_LIMIT);
    let options = Options {
        worktrees: junk_kinds::in_hosts(found),
        stale_worktrees: stale,
        ..opts
    };
    report_with(cfg, &Roots::from_env(), options, AGENT_SCAN_LIMIT)
}

pub fn report_with(cfg: &Config, roots: &Roots, opts: Options, limit: Duration) -> Report {
    let cx = Ctx::new(&opts.cwd, opts.idle, opts.now);
    let outcomes = scan(cfg);
    let mut kinds: Vec<KindRow> = ["log", "archive"]
        .into_iter()
        .filter_map(|kind| {
            let rows = outcomes.iter().filter(|o| o.kind == kind);
            let (items, size_bytes) = rows.fold((0, 0), |(n, b), o| (n + 1, b + o.bytes));
            (items > 0).then_some(KindRow {
                kind,
                class: "safe",
                items,
                size_bytes,
            })
        })
        .collect();
    let rtok_cache = roots.resolve("{rtok_cache}");
    let rtok_folders = rtok_folders(cfg, &rtok_cache);
    let tag_roots = rtok_folders
        .iter()
        .map(|f| PathBuf::from(&f.path))
        .collect();
    let junk = &cfg.agents.junk;
    let (extra_owned, extra) =
        junk_review::extra_items(junk, "rtok", &cfg.home, roots.home(), &cx, limit);
    let mut owned = rtok_owned(cfg, roots, &rtok_cache);
    owned.extend(extra_owned);
    let mut cache = junk_cache::cache_items(&owned, &outermost(tag_roots), &cx, limit);
    cache.extend(extra);
    // The language servers' data and state hold their index: review, and the project folder
    // stays like a cache's (T152's idle rule applies as for the caches).
    let index =
        junk_cache::cache_items(&project_lsp_dirs(cfg, &["data", "state"]), &[], &cx, limit);
    cache.extend(index.into_iter().map(|i| Item {
        kind: "index",
        class: "review",
        ..i
    }));
    cache.extend(stale_for(&opts.stale_worktrees, "rtok"));
    junk_review::exclude(&mut cache, &junk.exclude, &cfg.home, roots.home(), limit);
    let cache = junk_cache::drop_nested(cache);
    kinds.extend(kind_rows(&cache));
    let rtok_total = rtok_folders.iter().map(|f| f.size_bytes).sum::<u64>();
    let mut agents = vec![AgentJunk {
        name: "rtok",
        host: false,
        installed: true,
        folders: rtok_folders,
        total_bytes: rtok_total,
        freed_default_bytes: freed(&kinds, false),
        freed_review_bytes: freed(&kinds, true),
        kinds,
        items: cache,
        notes: Vec::new(),
    }];
    let (hosts, mut by_key) = host_rows(cfg, roots, &opts, limit, &cx);
    agents.extend(hosts);
    for f in &agents[0].folders {
        by_key.insert(PathBuf::from(&f.path), f.size_bytes);
    }
    // Each directory once, however many agents list it, and a nested one inside its parent.
    let total_bytes = top_level_bytes(by_key.iter().map(|(p, b)| (p.as_path(), *b)));
    let log_archive: u64 = agents[0]
        .kinds
        .iter()
        .filter(|k| !KINDS.contains(&k.kind))
        .map(|k| k.size_bytes)
        .sum();
    // Each path once however many agents list it, and a nested one inside its parent.
    let freed = |review: bool| {
        let wanted = |c: &str| c == "safe" || (review && c == "review");
        let items = agents.iter().flat_map(|a| &a.items);
        let items = items.filter(|i| i.counted() && wanted(i.class));
        let by_path: BTreeMap<&Path, u64> = items.map(|i| (Path::new(&i.path), i.bytes)).collect();
        log_archive + top_level_bytes(by_path.iter().map(|(p, b)| (*p, *b)))
    };
    // T343: agents always come by space freed, largest first; `--sort` orders the items.
    // Stable, so rtok keeps the lead on a tie.
    let (default_bytes, review_bytes) = (freed(false), freed(true));
    agents.sort_by_key(|a| std::cmp::Reverse(a.freed_default_bytes));
    Report {
        freed_default_bytes: default_bytes,
        freed_review_bytes: review_bytes,
        agents,
        total_bytes,
        own: outcomes,
    }
}

/// `text` as an OSC 8 hyperlink to the folder, for a terminal that renders it. A path that is
/// not absolute has no `file://` URL and stays plain.
pub(super) fn folder_link(path: &str, text: &str) -> String {
    match url::Url::from_file_path(path) {
        Ok(u) => format!("\x1b]8;;{u}\x1b\\{text}\x1b]8;;\x1b\\"),
        Err(()) => text.to_string(),
    }
}

/// Text of `agents junk list` and of the Hosts page's junk section; `view` says how sizes
/// print, whether paths are OSC 8 links (only for a terminal) and how many items each kind lists.
pub fn to_list(report: &Report, view: &View) -> String {
    let size = |n: u64| {
        if view.exact {
            n.to_string()
        } else {
            human_bytes(n)
        }
    };
    let groups = junk_items::groups(junk_items::rows(report), view);
    let mut out = String::new();
    for a in &report.agents {
        out.push_str(a.name);
        if a.host && !a.installed {
            out.push_str(" (not installed)");
        }
        out.push('\n');
        for f in &a.folders {
            let path = if view.links {
                folder_link(&f.path, &f.path)
            } else {
                f.path.clone()
            };
            out.push_str(&format!("  {path}  {}  {}", f.role, size(f.size_bytes)));
            if !f.documented {
                out.push_str("  not documented: not cleared");
            }
            if !f.shared_with.is_empty() {
                out.push_str(&format!("  shared with {}", f.shared_with.join(", ")));
            }
            if let Some(note) = &f.note {
                out.push_str(&format!("  ({note})"));
            }
            out.push('\n');
        }
        if a.host && a.folders.is_empty() {
            out.push_str("  no folders found\n");
        }
        for k in &a.kinds {
            out.push_str(&format!(
                "    {} ({}): {} items, {}\n",
                k.kind,
                k.class,
                k.items,
                size(k.size_bytes)
            ));
            if let Some(g) = groups
                .iter()
                .find(|g| (g.agent, g.kind) == (a.name, k.kind))
            {
                out.push_str(&g.lines(view, "      "));
            }
        }
        for note in &a.notes {
            out.push_str(&format!("  {note}\n"));
        }
        // A find nothing documents is one line per kind: a worktree can hold hundreds.
        let (listed, kept): (Vec<&Item>, Vec<&Item>) = a
            .items
            .iter()
            .filter(|i| !i.counted())
            .partition(|i| i.kept.as_deref() == Some(junk_kinds::NOT_DOCUMENTED));
        for kind in KINDS {
            let of_kind = listed.iter().filter(|i| i.kind == kind);
            let (n, bytes) = of_kind.fold((0, 0), |(n, b), i| (n + 1, b + i.bytes));
            if n > 0 {
                out.push_str(&format!(
                    "    {kind}: {n} listed, {}  {}\n",
                    size(bytes),
                    junk_kinds::NOT_DOCUMENTED
                ));
            }
        }
        // One line per reason when several items share it: a log folder keeps hundreds of
        // fresh files, and each would otherwise be its own line.
        let mut groups: Vec<(&str, &str, Vec<&Item>)> = Vec::new();
        for i in kept {
            let why = i.kept.as_deref().unwrap_or_default();
            match groups.iter_mut().find(|g| (g.0, g.1) == (i.kind, why)) {
                Some(g) => g.2.push(i),
                None => groups.push((i.kind, why, vec![i])),
            }
        }
        for (kind, why, items) in groups {
            let bytes = size(items.iter().map(|i| i.bytes).sum());
            match items.as_slice() {
                [one] => out.push_str(&format!("    kept {}  {bytes}  ({why})\n", one.path)),
                many => out.push_str(&format!(
                    "    kept {kind}: {} items, {bytes}  ({why})\n",
                    many.len()
                )),
            }
        }
        if !a.host || !a.kinds.is_empty() {
            out.push_str(&format!(
                "  Freed by `clear`: {}\n  Freed with `--include review`: {}\n",
                size(a.freed_default_bytes),
                size(a.freed_review_bytes)
            ));
        }
    }
    out.push_str(&format!(
        "total\n  Folders: {}\n  Freed by `clear`: {}\n  Freed with `--include review`: {}\n",
        size(report.total_bytes),
        size(report.freed_default_bytes),
        size(report.freed_review_bytes)
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_dir(cfg: &Config) -> std::path::PathBuf {
        let dir = cfg.log.path.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn stale_log_siblings_are_found_past_the_cap_and_current_ones_are_not() {
        let (mut cfg, _dir) = crate::testutil::config("junk-log-siblings");
        cfg.log.files = 2;
        let logs = log_dir(&cfg);
        std::fs::write(logs.join("rtok.log.1"), b"a").unwrap();
        std::fs::write(logs.join("rtok.log.2"), b"bb").unwrap();
        std::fs::write(logs.join("rtok.log.6"), b"stale").unwrap();
        assert_eq!(stale_log_siblings(&cfg), vec![logs.join("rtok.log.6")]);
    }

    #[test]
    fn dry_run_lists_without_deleting_and_yes_removes() {
        let (mut cfg, _dir) = crate::testutil::config("junk-dry-run");
        cfg.log.files = 1;
        let logs = log_dir(&cfg);
        let stale = logs.join("rtok.log.5");
        std::fs::write(&stale, b"junk").unwrap();

        let preview = run(&cfg, false);
        assert_eq!(preview.len(), 1);
        assert!(stale.exists(), "dry run must not delete anything");

        let applied = run(&cfg, true);
        assert_eq!(applied[0].note, "removed");
        assert!(!stale.exists());
    }

    #[test]
    fn no_db_at_the_configured_path_is_skipped_not_fatal() {
        let (cfg, _dir) = crate::testutil::config("junk-no-db");
        assert_eq!(scan(&cfg).len(), 0);
    }

    /// The fixture T182's card asks for: a call the retention window still covers (its
    /// archive must survive `clear`) and one old enough to purge (its archive is the one
    /// `clear` removes). Mirrors `Store::run_retention_purges_old_call_and_archive`.
    #[test]
    fn archive_retention_keeps_the_referenced_one_and_clears_the_orphan() {
        let (cfg, _dir) = crate::testutil::config("junk-archive-retention");
        let store = crate::store::Store::open(&cfg.core.db_path).unwrap();
        store
            .upsert_session("s1", Some(1), None, None, Some("proxy"))
            .unwrap();
        let spill = |byte: u8, ts: Option<i64>| {
            let call = store
                .insert_call(
                    "s1",
                    "proxy",
                    "api_request",
                    Some(1),
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            // Distinct bodies: archives are content-addressed, so two identical bodies would
            // dedupe into one archive still "referenced" by the live call.
            let body = vec![byte; 70 * 1024];
            store
                .insert_call_io(
                    call,
                    Some(&body),
                    None,
                    64 * 1024,
                    Some(&cfg.core.archive_dir),
                )
                .unwrap();
            if let Some(ts) = ts {
                store.set_call_ts(call, ts).unwrap();
            }
        };
        spill(b'a', None); // live: stays inside `core.retain_calls_days`
        spill(b'b', Some(0)); // old enough to purge

        let outcomes = run(&cfg, true);
        let archives: Vec<_> = outcomes.iter().filter(|o| o.kind == "archive").collect();
        assert_eq!(archives.len(), 1, "only the orphan is reported");
        assert_eq!(archives[0].note, "removed");

        let left: Vec<_> = std::fs::read_dir(&cfg.core.archive_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(left.len(), 1, "the referenced archive survives");
    }

    /// Unix only: elsewhere `disk_usage` has no file id and counts each hard link.
    #[cfg(unix)]
    #[test]
    fn disk_usage_counts_a_hard_link_once_and_never_follows_a_symlink() {
        let (_cfg, dir) = crate::testutil::config("junk-disk-usage");
        let root = dir.join("agent");
        std::fs::create_dir_all(&root).unwrap();
        let outside = dir.join("outside.bin");
        std::fs::write(&outside, vec![1u8; 64 * 1024]).unwrap();
        let a = root.join("a.bin");
        std::fs::write(&a, vec![2u8; 8 * 1024]).unwrap();
        let alone = disk_usage(&root);
        std::fs::hard_link(&a, root.join("b.bin")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        let with_links = disk_usage(&root);
        assert!(
            alone >= 8 * 1024,
            "allocated blocks cover the file: {alone}"
        );
        // The extra entries cost directory/inode metadata at most, never another 8 KB or the
        // 64 KB the link points at.
        assert!(with_links < alone + 8 * 1024, "{alone} -> {with_links}");
    }

    #[test]
    fn report_sums_the_kinds_the_clear_plan_lists_and_the_text_prints_them() {
        let (mut cfg, dir) = crate::testutil::config("junk-report");
        cfg.log.files = 1;
        let logs = log_dir(&cfg);
        std::fs::write(logs.join("rtok.log.4"), b"1234").unwrap();
        std::fs::write(logs.join("rtok.log.5"), b"12345678").unwrap();

        // Fixture roots, not `report`'s environment: that would size this machine's real caches.
        let report = report_with(
            &cfg,
            &roots(&dir, &[]),
            Options::default(),
            AGENT_SCAN_LIMIT,
        );
        let planned: u64 = scan(&cfg).iter().map(|o| o.bytes).sum();
        assert_eq!(planned, 12);
        assert_eq!(report.freed_default_bytes, planned);
        let freed: Vec<u64> = report
            .agents
            .iter()
            .map(|a| a.freed_default_bytes)
            .collect();
        assert!(freed.windows(2).all(|w| w[0] >= w[1]), "{freed:?}");
        let rtok = report.agents.iter().find(|a| a.name == "rtok").unwrap();
        assert_eq!((rtok.kinds[0].kind, rtok.kinds[0].items), ("log", 2));
        assert!(
            rtok.folders
                .iter()
                .any(|f| f.path == logs.display().to_string())
        );

        let text = to_list(&report, &View::test(true, false));
        assert!(text.contains("log (safe): 2 items, 12"), "{text}");
        assert!(text.contains("Freed by `clear`: 12"), "{text}");
    }
    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    /// Roots over a fixture home with `env` as the only environment.
    fn roots(home: &Path, env: &[(&str, PathBuf)]) -> Roots {
        let env: Vec<(String, PathBuf)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        Roots::new(home.to_path_buf(), move |k| {
            env.iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone().into())
        })
    }

    fn agent<'a>(r: &'a Report, name: &str) -> &'a AgentJunk {
        r.agents
            .iter()
            .find(|a| a.name == name)
            .unwrap_or_else(|| panic!("no `{name}` row"))
    }

    fn folder<'a>(a: &'a AgentJunk, path: &Path) -> &'a Folder {
        let want = path.display().to_string();
        a.folders
            .iter()
            .find(|f| f.path == want)
            .unwrap_or_else(|| panic!("`{}` has no folder {want}: {:?}", a.name, a.folders))
    }

    #[cfg(unix)]
    /// T330.2: Claude Code, Cursor and Codex under a fixture home. §22's rows are documented,
    /// Cursor's folders are not, and a symlink out of a folder is not followed.
    #[test]
    fn host_rows_list_their_folders_with_sizes_and_the_documented_mark() {
        let (cfg, dir) = crate::testutil::config("junk-hosts");
        let outside = dir.join("outside");
        write(&outside.join("big"), 1 << 20);
        let claude = dir.join(".claude");
        write(&claude.join("settings.json"), 10);
        write(&claude.join("debug/a.log"), 100);
        std::os::unix::fs::symlink(&outside, claude.join("link")).unwrap();
        write(&dir.join(".cursor/hooks.json"), 10);
        let cursor_data = dir.join("Library/Application Support/Cursor");
        write(&cursor_data.join("state"), 10);
        write(&dir.join(".codex/log/codex.log"), 10);
        write(&dir.join(".codex/config.toml"), 10);

        let report = report_with(
            &cfg,
            &roots(&dir, &[]),
            // The hosts here are folders only, not installed apps (T426).
            Options {
                all: true,
                ..Options::default()
            },
            AGENT_SCAN_LIMIT,
        );

        let c = agent(&report, "claude");
        let home = folder(c, &claude);
        assert_eq!((home.role, home.documented), ("data", true));
        assert_eq!(home.size_bytes, disk_usage(&claude));
        assert!(
            home.size_bytes < 512 * 1024,
            "the link out of the folder was followed: {}",
            home.size_bytes
        );
        let logs = folder(c, &claude.join("debug"));
        assert_eq!((logs.role, logs.documented), ("logs", true));
        assert_eq!(logs.size_bytes, disk_usage(&claude.join("debug")));
        // The debug log is inside the data home, so the agent total does not add it twice.
        assert_eq!(c.total_bytes, home.size_bytes);

        let cur = agent(&report, "cursor");
        assert!(cur.folders.len() >= 2, "{:?}", cur.folders);
        assert!(
            cur.folders.iter().all(|f| !f.documented),
            "{:?}",
            cur.folders
        );
        let app = folder(cur, &cursor_data);
        assert_eq!(app.size_bytes, disk_usage(&cursor_data));
        assert!(folder(cur, &dir.join(".cursor")).size_bytes > 0);

        let x = agent(&report, "codex");
        assert_eq!(folder(x, &dir.join(".codex/log")).role, "logs");
        assert!(folder(x, &dir.join(".codex")).documented);

        let text = to_list(&report, &View::test(true, false));
        assert!(
            text.contains(&format!(
                "{}  data  {}  not documented: not cleared",
                cursor_data.display(),
                app.size_bytes
            )),
            "{text}"
        );
        assert!(!text.contains("\x1b]8"), "no link off a terminal: {text}");
        let linked = to_list(&report, &View::test(true, true));
        assert!(linked.contains("\x1b]8;;file://"), "{linked}");
    }

    /// T391: only the commit folders of `CachedData` that are not the newest are VS Code cache,
    /// and only after the idle window; Cursor's `CachedData` stays a read-only folder.
    #[cfg(unix)]
    #[test]
    fn vscode_code_cache_offers_old_commit_folders_and_cursor_none() {
        let (mut cfg, dir) = crate::testutil::config("junk-code-cache");
        cfg.log.path = dir.join("none/rtok.log");
        cfg.core.archive_dir = dir.join("none/archive");
        let code = dir.join(".config/Code/CachedData");
        for (name, secs) in [("new", 60), ("old", 40 * 86_400)] {
            let f = code.join(name).join("f.cache");
            write(&f, 100);
            let when = SystemTime::now() - Duration::from_secs(secs);
            for p in [&f, &code.join(name)] {
                std::fs::File::open(p).unwrap().set_modified(when).unwrap();
            }
        }
        write(&dir.join(".config/Cursor/CachedData/x/f"), 100);

        let opts = Options {
            all: true,
            cwd: std::env::temp_dir(),
            ..Options::default()
        };
        let report = report_with(&cfg, &roots(&dir, &[]), opts, AGENT_SCAN_LIMIT);

        let v = agent(&report, "vscode");
        let paths: Vec<_> = v.items.iter().map(|i| i.path.clone()).collect();
        assert_eq!(paths, [code.join("old").display().to_string()]);
        assert!(v.items[0].kept.is_none(), "{:?}", v.items);
        assert!(folder(v, &code).documented);
        assert!(agent(&report, "cursor").items.is_empty());
    }

    /// An override that points two hosts at one directory: it is listed under both with a
    /// "shared with" note and counted once in the report total.
    #[test]
    fn a_folder_two_agents_share_is_sized_once_and_marked_shared() {
        let (mut cfg, dir) = crate::testutil::config("junk-shared");
        // rtok's own folders must not be the fixture home, which holds the shared folder.
        cfg.log.path = dir.join("none/rtok.log");
        cfg.core.archive_dir = dir.join("none/archive");
        let shared = dir.join("shared");
        write(&shared.join("blob"), 64 * 1024);
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::create_dir_all(dir.join(".codex")).unwrap();
        let env = [
            ("CLAUDE_CONFIG_DIR", shared.clone()),
            ("CODEX_HOME", shared.clone()),
        ];

        let report = report_with(
            &cfg,
            &roots(&dir, &env),
            // The hosts here are folders only, not installed apps (T426).
            Options {
                all: true,
                ..Options::default()
            },
            AGENT_SCAN_LIMIT,
        );

        let c = folder(agent(&report, "claude"), &shared);
        let x = folder(agent(&report, "codex"), &shared);
        assert_eq!(c.shared_with, vec!["codex"]);
        assert_eq!(x.shared_with, vec!["claude"]);
        assert_eq!(c.size_bytes, x.size_bytes);
        let both = agent(&report, "claude").total_bytes + agent(&report, "codex").total_bytes;
        assert!(
            report.total_bytes < both,
            "{} !< {both}",
            report.total_bytes
        );
        assert!(to_list(&report, &View::test(false, false)).contains("shared with codex"));
    }

    #[test]
    fn all_adds_hosts_that_are_not_installed_and_the_default_skips_them() {
        let (cfg, dir) = crate::testutil::config("junk-all");
        let r = roots(&dir, &[]);
        let names = |report: &Report| report.agents.len();

        let some = report_with(&cfg, &r, Options::default(), AGENT_SCAN_LIMIT);
        let every = report_with(
            &cfg,
            &r,
            Options {
                all: true,
                ..Options::default()
            },
            AGENT_SCAN_LIMIT,
        );

        assert_eq!(every.agents.len(), HOSTS.len() + 1, "rtok plus every host");
        assert!(names(&some) < names(&every));
        let absent = every
            .agents
            .iter()
            .find(|a| a.host && !a.installed)
            .unwrap();
        assert!(absent.folders.is_empty());
        assert!(
            to_list(&every, &View::test(false, false))
                .contains(&format!("{} (not installed)", absent.name))
        );
    }

    #[test]
    fn a_scan_past_its_limit_is_reported_not_fatal() {
        let (cfg, dir) = crate::testutil::config("junk-timeout");
        write(&dir.join(".claude/settings.json"), 10);

        let report = report_with(
            &cfg,
            &roots(&dir, &[]),
            Options {
                all: true,
                ..Options::default()
            },
            Duration::ZERO,
        );

        let note = folder(agent(&report, "claude"), &dir.join(".claude"))
            .note
            .clone();
        assert!(note.is_some_and(|n| n.contains("scan stopped")));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_is_reported_not_fatal() {
        use std::os::unix::fs::PermissionsExt;
        let (_cfg, dir) = crate::testutil::config("junk-denied");
        let locked = dir.join("locked");
        write(&locked.join("f"), 10);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let usage = disk_usage_until(&locked, None);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A superuser reads it anyway, which is not the case under test.
        if !usage.denied {
            assert!(usage.bytes > 0);
        }
        assert!(usage.denied || usage.bytes > 4096);
    }

    /// The fixture of T330.3.1's Check: every cache kind of evidence, under the right agent.
    #[cfg(unix)]
    #[test]
    fn caches_sit_under_their_agent_and_only_evidence_counts_as_freed() {
        use super::junk_cache::{age_files, tagged};
        let (mut cfg, dir) = crate::testutil::config("junk-cache");
        // rtok's own folders must not be the fixture home, which holds the agents'.
        cfg.log.path = dir.join("none/rtok.log");
        cfg.core.archive_dir = dir.join("none/archive");
        let r = roots(&dir, &[]);
        let claude = dir.join(".claude");
        write(&claude.join("settings.json"), 10);
        write(&claude.join("paste-cache/p"), 100);
        write(&claude.join("projects/p/target/o"), 1000);
        tagged(&claude.join("projects/p/target"));
        write(&claude.join("projects/q/bad/o"), 500);
        std::fs::write(claude.join("projects/q/bad/CACHEDIR.TAG"), "Signature: no").unwrap();
        let cursor = dir.join("Library/Application Support/Cursor");
        write(&cursor.join("Code Cache/blob"), 300);
        write(&dir.join("Library/Caches/Cursor/y"), 400);
        let copilot = r.resolve("{copilot_cache}");
        write(&copilot.join("c"), 50);
        let rtok_cache = r.resolve("{rtok_cache}");
        write(&rtok_cache.join("r"), 20);
        let project = dir.join("proj");
        write(&project.join(".rtok-lsp-xdg/cache/lsp"), 70);
        let store = Store::open(&cfg.core.db_path).unwrap();
        store
            .register_project(&project, crate::store::Origin::Manual)
            .unwrap();
        for d in [&claude, &cursor, &copilot, &rtok_cache, &project] {
            age_files(d, 3 * 86_400);
        }
        let before: Vec<_> = walk_names(&dir);

        let opts = Options {
            all: true,
            // Not inside the fixture, so no tagged cache is "the worktree this runs from".
            cwd: std::env::temp_dir(),
            ..Options::default()
        };
        let report = report_with(&cfg, &r, opts, AGENT_SCAN_LIMIT);

        let c = agent(&report, "claude");
        let paths: Vec<_> = c.items.iter().map(|i| i.path.clone()).collect();
        let want = [claude.join("paste-cache"), claude.join("projects/p/target")];
        assert_eq!(
            paths,
            want.iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>(),
            "the bad-signature dir is not cache"
        );
        assert_eq!(
            (c.items[0].evidence, c.items[1].evidence),
            ("research.md §22", "CACHEDIR.TAG")
        );
        let cache_bytes: u64 = want.iter().map(|p| disk_usage(p)).sum();
        assert_eq!((c.kinds[0].kind, c.kinds[0].items), ("cache", 2));
        assert_eq!(c.freed_default_bytes, cache_bytes);

        let cur = agent(&report, "cursor");
        assert!(cur.items.is_empty() && cur.kinds.is_empty());
        assert_eq!(cur.freed_default_bytes, 0);
        let code_cache = folder(cur, &cursor.join("Code Cache"));
        assert_eq!(
            (
                code_cache.role,
                code_cache.documented,
                code_cache.size_bytes
            ),
            ("cache", false, disk_usage(&cursor.join("Code Cache")))
        );
        assert!(!folder(cur, &dir.join("Library/Caches/Cursor")).documented);

        let cp = agent(&report, "copilot");
        assert_eq!(cp.items[0].path, copilot.display().to_string());
        assert_eq!(cp.freed_default_bytes, disk_usage(&copilot));

        let rtok = agent(&report, "rtok");
        let own: Vec<_> = rtok
            .items
            .iter()
            .map(|i| (i.path.clone(), i.evidence))
            .collect();
        assert!(
            own.contains(&(rtok_cache.display().to_string(), "rtok-owned")),
            "{own:?}"
        );
        // The registry stores a canonical root, so `/var` and `/private/var` are one cache.
        assert!(
            own.iter()
                .any(|(p, e)| Path::new(p).ends_with(".rtok-lsp-xdg/cache") && *e == "rtok-owned"),
            "{own:?}"
        );
        assert_eq!(folder(rtok, &rtok_cache).role, "cache");

        let counted: u64 = [c, cp, rtok].iter().map(|a| a.freed_default_bytes).sum();
        assert_eq!(
            report.freed_default_bytes, counted,
            "Cursor's caches add nothing"
        );
        let text = to_list(&report, &View::test(true, false));
        assert!(text.contains("cache (safe): 2 items"), "{text}");
        assert!(
            text.contains(&format!(
                "{}  cache  {}  not documented: not cleared",
                cursor.join("Code Cache").display(),
                code_cache.size_bytes
            )),
            "{text}"
        );
        assert_eq!(walk_names(&dir), before, "a report changes no file");
    }

    #[cfg(unix)]
    fn walk_names(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                out.push(e.path());
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(e.path());
                }
            }
        }
        out.sort();
        out
    }

    /// A tagged dir two agents reach through one shared folder is listed under both and freed
    /// once; a symlinked documented cache is never an item.
    #[cfg(unix)]
    #[test]
    fn a_shared_tagged_cache_is_freed_once_and_a_symlinked_cache_is_skipped() {
        use super::junk_cache::{age_files, tagged};
        let (mut cfg, dir) = crate::testutil::config("junk-cache-shared");
        cfg.log.path = dir.join("none/rtok.log");
        cfg.core.archive_dir = dir.join("none/archive");
        let shared = dir.join("shared");
        write(&shared.join("t/o"), 4096);
        tagged(&shared.join("t"));
        age_files(&shared, 3 * 86_400);
        let outside = dir.join("outside");
        write(&outside.join("big"), 1 << 20);
        std::fs::create_dir_all(dir.join("c")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("c/paste-cache")).unwrap();
        let env = [
            ("CLAUDE_CONFIG_DIR", shared.clone()),
            ("CODEX_HOME", shared.clone()),
        ];
        let opts = Options {
            all: true,
            cwd: std::env::temp_dir(),
            ..Options::default()
        };
        let report = report_with(&cfg, &roots(&dir, &env), opts, AGENT_SCAN_LIMIT);

        let t = shared.join("t").display().to_string();
        for name in ["claude", "codex"] {
            let a = agent(&report, name);
            assert_eq!(
                a.items.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(),
                [t.as_str()]
            );
        }
        assert_eq!(report.freed_default_bytes, disk_usage(&shared.join("t")));

        let env = [("CLAUDE_CONFIG_DIR", dir.join("c"))];
        let report = report_with(
            &cfg,
            &roots(&dir, &env),
            Options {
                all: true,
                ..Options::default()
            },
            AGENT_SCAN_LIMIT,
        );
        assert!(
            agent(&report, "claude").items.is_empty(),
            "no link is followed out"
        );
    }

    /// T330.3.2's fixture: temp, a worktree's tagged `target/` and a stale plugin copy count;
    /// a lock, a swap file and `dist/` are listed and count for nothing (D36).
    #[cfg(unix)]
    #[test]
    fn temp_build_and_staging_count_while_locks_and_swap_are_listed_only() {
        use super::junk_cache::{age_files, tagged};
        let (mut cfg, dir) = crate::testutil::config("junk-kinds");
        cfg.log.path = dir.join("none/rtok.log");
        cfg.core.archive_dir = dir.join("none/archive");
        let r = roots(&dir, &[]);
        let claude = dir.join(".claude");
        write(&claude.join("settings.json"), 10);
        write(&claude.join("shell-snapshots/old.sh"), 300);
        write(&claude.join("shell-snapshots/new.sh"), 5);
        write(&claude.join("session.lock"), 1);
        write(&claude.join("Cargo.lock"), 1);
        write(&claude.join("x/.edit.swp"), 1);
        write(&claude.join("plugins/cache/rtok/rtok/0.1.0/p"), 40);
        write(&claude.join("plugins/cache/rtok/rtok/0.2.0/p"), 40);
        let used = claude.join("plugins/cache/rtok/rtok/0.2.0");
        std::fs::write(
            claude.join("plugins/installed_plugins.json"),
            format!(
                r#"{{"plugins":{{"rtok@rtok":[{{"installPath":"{}"}}]}}}}"#,
                used.display()
            ),
        )
        .unwrap();
        let wt = dir.join("wt");
        write(&wt.join("target/o"), 1000);
        tagged(&wt.join("target"));
        write(&wt.join("dist/app.js"), 10);
        age_files(&claude.join("shell-snapshots"), 3 * 86_400);
        age_files(&claude.join("plugins"), 3 * 86_400);
        age_files(&wt, 3 * 86_400);
        // Fresh again: only the old one may go.
        write(&claude.join("shell-snapshots/new.sh"), 5);
        // The scan opens the store, as the T330.3.1 fixture does by registering a project.
        Store::open(&cfg.core.db_path).unwrap();
        let before = walk_names(&dir);

        let opts = Options {
            cwd: std::env::temp_dir(),
            worktrees: vec![("claude", wt.clone())],
            ..Options::default()
        };
        let report = report_with(&cfg, &r, Options { all: true, ..opts }, AGENT_SCAN_LIMIT);

        let c = agent(&report, "claude");
        let rows: Vec<_> = c.kinds.iter().map(|k| (k.kind, k.items)).collect();
        assert_eq!(rows, [("temp", 1), ("build", 1)]);
        let want =
            disk_usage(&claude.join("shell-snapshots/old.sh")) + disk_usage(&wt.join("target"));
        assert_eq!(c.freed_default_bytes, want);
        let listed: Vec<_> = c
            .items
            .iter()
            .filter(|i| !i.counted())
            .map(|i| (i.kind, i.path.rsplit('/').next().unwrap().to_owned()))
            .collect();
        for (kind, name) in [
            ("locks", "session.lock"),
            ("swap", ".edit.swp"),
            ("build", "dist"),
        ] {
            assert!(listed.contains(&(kind, name.to_owned())), "{listed:?}");
        }
        assert!(!c.items.iter().any(|i| i.path.ends_with("Cargo.lock")));

        let own = agent(&report, "rtok");
        let stale = claude.join("plugins/cache/rtok/rtok/0.1.0");
        let paths: Vec<_> = own
            .items
            .iter()
            .filter(|i| i.counted())
            .map(|i| &i.path)
            .collect();
        assert!(paths.contains(&&stale.display().to_string()), "{paths:?}");
        assert!(!paths.iter().any(|p| p.ends_with("0.2.0")));
        assert_eq!(report.freed_default_bytes, want + disk_usage(&stale));

        let text = to_list(&report, &View::test(true, false));
        assert!(text.contains("temp (safe): 1 items"), "{text}");
        assert!(text.contains("locks: 1 listed"), "{text}");
        assert!(
            text.contains(".edit.swp") && text.contains("(owner unknown)"),
            "{text}"
        );
        assert_eq!(walk_names(&dir), before, "a report changes no file");
    }

    /// T330.5.2: sessions are an `explicit` row outside both "Freed" totals, rtok's language-server
    /// data and state are `review` index items, and `.rust-analyzer/` and a `-wal` are listed only.
    #[cfg(unix)]
    #[test]
    fn sessions_and_index_rows_follow_their_classes() {
        use super::junk_cache::age_files;
        let (mut cfg, dir) = crate::testutil::config("junk-sessions-index");
        cfg.log.path = dir.join("none/rtok.log");
        cfg.core.archive_dir = dir.join("none/archive");
        let r = roots(&dir, &[]);
        let id = "0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0";
        let claude = dir.join(".claude");
        write(&claude.join(format!("projects/p/{id}.jsonl")), 100);
        write(&claude.join("projects/p/state.db-wal"), 10);
        write(&claude.join("projects/p/.rust-analyzer/db"), 10);
        let project = dir.join("proj");
        write(&project.join(".rtok-lsp-xdg/data/idx"), 70);
        write(&project.join(".rtok-lsp-xdg/state/s"), 30);
        let store = Store::open(&cfg.core.db_path).unwrap();
        store
            .register_project(&project, crate::store::Origin::Manual)
            .unwrap();
        for d in [&claude, &project] {
            age_files(d, 40 * 86_400);
        }
        let opts = Options {
            all: true,
            cwd: std::env::temp_dir(),
            ..Options::default()
        };

        let report = report_with(&cfg, &r, opts, AGENT_SCAN_LIMIT);

        let c = agent(&report, "claude");
        let sessions = c.kinds.iter().find(|k| k.kind == "sessions").unwrap();
        assert_eq!((sessions.class, sessions.items), ("explicit", 1));
        assert_eq!(c.freed_review_bytes, 0, "an explicit kind is in no total");
        let listed = |kind: &'static str| {
            c.items
                .iter()
                .filter(move |i| i.kind == kind && !i.counted())
        };
        assert_eq!(listed("index").count(), 2, "{:?}", c.items);
        assert!(listed("index").all(|i| i.kept.as_deref() == Some(junk_kinds::NOT_DOCUMENTED)));
        assert_eq!(
            c.notes[0],
            "old sessions: not touched for more than 30 days"
        );
        assert!(c.notes[1].starts_with("host retention: cleanupPeriodDays"));

        let own = agent(&report, "rtok");
        let index = own.kinds.iter().find(|k| k.kind == "index").unwrap();
        assert_eq!((index.class, index.items), ("review", 2));
        assert_eq!(own.freed_default_bytes, 0);
        assert!(own.freed_review_bytes >= 100);
        let text = to_list(&report, &View::test(true, false));
        assert!(text.contains("sessions (explicit): 1 items"), "{text}");
        assert!(text.contains("index (review): 2 items"), "{text}");
    }
}

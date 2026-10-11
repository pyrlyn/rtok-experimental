// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Pure TUI state: which tab, what data, when it last refreshed — and, on the Plugins
//! tab, the row cursor and the toggle that writes `plugins.<id>.enabled` through `rtok
//! config set`'s writer (T15.4). No terminal and no clock reach this module from
//! outside a call — both arrive through [`super`]'s loop, which is what keeps the state
//! transitions unit-testable without a tty.

use std::sync::mpsc;

use crossterm::event::{KeyCode, KeyModifiers};

use super::compare::Compare;
use super::doctor_fix::{DoctorFix, Engine, Keyed};
use super::exporter::Exporter;
use super::junk_clear::{self, JunkClear};
use super::live_calls::LiveCalls;
use super::projects::{self, Key};
use crate::config::{Config, validate};
use crate::model::{self, Snapshot};

/// The one key table (T60.8): the footer and the `?` overlay render from it, so the
/// key hints are never hand-written twice. `(page, key, description)`; page `""` is
/// global. The handler in [`App::key`] stays behavioural code; every hint the UI
/// prints is generated from here.
pub(crate) const KEYS: &[(&str, &str, &str)] = &[
    ("", "q/Esc", "quit"),
    ("", "Left/Right", "switch tab"),
    ("", "1..9", "jump to tab"),
    ("", "?", "help"),
    ("", "r", "refresh now"),
    ("plugins", "↑/↓", "move cursor"),
    ("plugins", "Space/Enter", "toggle plugin"),
    ("calls", "↑/↓", "move selection"),
    ("calls", "Enter/z", "detail pane"),
    ("calls", "e", "expand archive"),
    ("calls", "/", "filter expand"),
    ("graph", "f", "freeze/unfreeze calls"),
    ("graph", "w", "calls window"),
    ("graph", "a", "filter calls by caller"),
    ("graph", "t", "filter calls by tool"),
    ("graph", "o", "filter calls by project"),
    ("sessions", "↑/↓", "move selection"),
    ("sessions", "Enter", "detail pane"),
    ("sessions", "l", "live-only filter"),
    ("skills", "↑/↓", "move selection"),
    ("skills", "n", "never-invoked only"),
    ("doctor", "f", "fix checklist"),
    ("doctor", "Space", "select entry"),
    ("doctor", "k", "keep this copy"),
    ("doctor", "Enter/y", "apply selected (confirm)"),
    ("hosts", "c", "clear safe junk (plan)"),
    ("hosts", "y", "remove planned (confirm)"),
    ("hosts", "n/Esc", "decline"),
    ("config", "/", "filter entries"),
    ("graph", "↑/↓", "move cursor"),
    ("graph", "s", "select project"),
    ("graph", "l", "link to…"),
    ("graph", "u", "unlink from…"),
    ("graph", "b", "both ways"),
    ("graph", "Enter", "pick target"),
    ("graph", "y", "confirm plan"),
    ("graph", "n/Esc", "decline"),
    ("graph", "c", "compare"),
    ("compare", "type", "ref, PROJECT:REF or export path"),
    ("compare", "Enter", "run compare"),
    ("compare", "↑/↓ PgUp/PgDn", "scroll"),
    ("compare", "c", "compare again"),
    ("compare", "Esc", "close"),
    ("graph", "e", "export graph"),
    ("graph", "v", "view saved export"),
    ("export", "↑/↓", "choose field"),
    ("export", "Space", "change format, level, redact"),
    ("export", "type", "file to write, or export to view"),
    ("export", "Enter", "write / view"),
    ("export", "↑/↓ PgUp/PgDn", "scroll result"),
    ("export", "Esc", "close"),
];

/// The key rows for one page: globals first, then the page's own.
pub(crate) fn keys_for(page: &str) -> Vec<(&'static str, &'static str)> {
    KEYS.iter()
        .filter(|(p, _, _)| p.is_empty() || *p == page)
        .map(|(_, k, d)| (*k, *d))
        .collect()
}

/// The TUI's whole state. The tabs are [`model::pages`] by reference — there is no
/// second list to let drift (D23); a page the model adds is a tab at the next `App::new`.
pub struct App {
    tabs: &'static [(&'static str, &'static str)],
    selected: usize,
    snapshot: Snapshot,
    /// Unix seconds of the last model re-read; the screen stamps it with `log::stamp`.
    updated: u64,
    /// The config the App reads the model through, so a toggle's re-read and every
    /// later tick serve the same copy (T15.4) — the loop no longer holds its own.
    cfg: Config,
    /// The Plugins tab's row cursor: which plugin a toggle would hit (T15.4).
    plugin_cursor: usize,
    /// The Plugins tab's status line: the last toggle's outcome (T15.4).
    plugin_status: String,
    /// The Calls page's own state (T15.5): which row is selected and whether its detail
    /// pane is open.
    calls: CallsState,
    /// The Sessions page's own state (T60.10): which row is selected and whether the
    /// live-only filter is on.
    sessions: SessionsState,
    skills: SkillsState,
    /// The Doctor tab's `doctor --fix` checklist (T478).
    doctor_fix: DoctorFix,
    /// The Hosts tab's clear-safe-junk plan and confirm (T479).
    junk: JunkClear,
    /// The Graph page's live calls pane (T480).
    live: LiveCalls,
    /// The Config page's own state (T228): `/` filter text and whether it is capturing.
    config: ConfigState,
    /// The Graph page's registry cursor and the plan being made (T476).
    projects: projects::ProjectsState,
    /// The Graph page's compare view (T485).
    compare: Compare,
    /// The Graph page's export form and saved-export view (T329.41).
    exporter: Exporter,
    /// Whether the `?` help overlay is up (T60.8).
    help: bool,
    /// The running TUI's model reader: a snapshot parses transcripts and probes the
    /// doctor (seconds on a busy machine), so reading it on the key loop froze the
    /// screen at start and on every tick. `None` reads inline — what unit tests drive.
    worker: Option<Worker>,
    /// Generation of the last requested re-read; `shown` is the one on screen.
    requested: u64,
    shown: u64,
}

/// One background thread that turns a config into a snapshot. Requests queued while a
/// read runs collapse into the newest, so a slow model never builds a backlog.
struct Worker {
    tx: mpsc::Sender<(u64, Config)>,
    rx: mpsc::Receiver<(u64, Snapshot)>,
}

impl Worker {
    /// `None` when the thread will not start; the App then reads inline (fail open, D1).
    fn spawn() -> Option<Self> {
        let (tx, jobs) = mpsc::channel::<(u64, Config)>();
        let (done, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("rtok-tui-model".into())
            .spawn(move || {
                while let Ok(mut job) = jobs.recv() {
                    while let Ok(newer) = jobs.try_recv() {
                        job = newer;
                    }
                    if done.send((job.0, model::snapshot(&job.1))).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { tx, rx })
    }
}

/// Selection and detail state of the Calls page (T15.5). The row list lives in the
/// snapshot (D23); this only remembers where the cursor sits on it.
#[derive(Default)]
struct CallsState {
    selected: usize,
    detail: bool,
    /// Open archive pane (T60.4), if `e` fetched a payload for this row.
    expand: Option<ExpandPane>,
}

/// Fetched archive body for the Calls expand pane. `text` is already `[expand]
/// max_lines`-capped; `/` filters it through `filter_lines` without re-fetching.
struct ExpandPane {
    id: String,
    text: String,
    filter: String,
    filtering: bool,
    scroll: u16,
}

/// Selection and filter state of the Sessions page (T60.10 / T60.3). The rows live
/// in the snapshot (D23); this remembers the cursor, whether `l` narrowed the page
/// to sessions that are still running, and whether Enter opened the detail pane.
#[derive(Default)]
struct SessionsState {
    selected: usize,
    live_only: bool,
    detail: bool,
}

/// The Config page's `/` filter state (T228): same capture shape as the Calls expand
/// pane's filter (T60.4), applied to the page itself since there is no sub-pane here.
#[derive(Default)]
struct ConfigState {
    filter: String,
    filtering: bool,
}

#[derive(Default)]
struct SkillsState {
    selected: usize,
    never_only: bool,
}

impl App {
    /// An App that reads the model inline: the first snapshot is on hand when this
    /// returns (unit tests).
    #[cfg(test)]
    pub fn new(cfg: &Config) -> Self {
        let mut app = Self::empty(cfg);
        app.tick();
        app
    }

    /// The running TUI's App: the screen paints at once over an empty snapshot and
    /// every read runs on a [`Worker`]; [`Self::poll`] takes the results.
    pub fn background(cfg: &Config) -> Self {
        let mut app = Self::empty(cfg);
        app.worker = Worker::spawn();
        app.tick();
        app
    }

    fn empty(cfg: &Config) -> Self {
        let tabs = model::pages();
        // `[tui] tab` names the opening tab; empty or unknown falls back to the first
        // page rather than failing the surface (fail open, D1).
        let selected = tabs
            .iter()
            .position(|(page, _)| *page == cfg.tui.tab)
            .unwrap_or(0);
        Self {
            tabs,
            selected,
            snapshot: Snapshot::default(),
            updated: 0,
            cfg: cfg.clone(),
            plugin_cursor: 0,
            plugin_status: String::new(),
            calls: CallsState::default(),
            sessions: SessionsState::default(),
            skills: SkillsState::default(),
            doctor_fix: DoctorFix::new(Engine::machine()),
            junk: JunkClear::new(junk_clear::Engine::machine()),
            live: LiveCalls::new(cfg),
            config: ConfigState::default(),
            projects: projects::ProjectsState::default(),
            compare: Compare::default(),
            exporter: Exporter::default(),
            help: false,
            worker: None,
            requested: 0,
            shown: 0,
        }
    }

    /// Tab titles straight off the model's page list.
    pub fn tab_names(&self) -> Vec<&'static str> {
        self.tabs.iter().map(|(page, _)| *page).collect()
    }

    /// The selected page's name.
    pub fn page(&self) -> &'static str {
        self.tabs[self.selected].0
    }

    /// The selected tab's index — where the highlight sits.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The Plugins tab's row cursor, clamped to the rows the model serves (T15.4).
    pub fn plugin_cursor(&self) -> usize {
        self.plugin_cursor
            .min(self.snapshot.plugins.len().saturating_sub(1))
    }

    /// The Plugins tab's status line: the last toggle's outcome, `""` until one (T15.4).
    pub fn plugin_status(&self) -> &str {
        &self.plugin_status
    }

    /// The last snapshot the model served.
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Unix seconds of the last tick; the screen stamps it with `log::stamp`.
    pub fn updated(&self) -> u64 {
        self.updated
    }

    /// A tick's new data: one model snapshot, timestamped now.
    pub fn refresh(&mut self, snapshot: Snapshot) {
        // The cursor stays on a row that still exists — a shorter page is not a panic.
        self.plugin_cursor = self
            .plugin_cursor
            .min(snapshot.plugins.len().saturating_sub(1));
        self.sessions.selected = self
            .sessions
            .selected
            .min(self.visible_sessions(&snapshot).saturating_sub(1));
        self.skills.selected = self
            .skills
            .selected
            .min(self.visible_skills(&snapshot).saturating_sub(1));
        self.snapshot = snapshot;
        self.updated = crate::log::now();
    }

    /// How many rows the Sessions page shows right now (T60.10): the live-only
    /// filter narrows the snapshot's list before the cursor is clamped against it.
    fn visible_sessions(&self, snapshot: &Snapshot) -> usize {
        if self.sessions.live_only {
            snapshot
                .sessions
                .iter()
                .filter(|s| s.ended_at.is_none())
                .count()
        } else {
            snapshot.sessions.len()
        }
    }

    /// The loop's tick: re-read the model through the config the App holds, so a
    /// toggle's re-read and the next tick cannot disagree (T15.4).
    /// A timer tick skips while a read is still running, so a slow model is not
    /// re-read back to back.
    pub fn tick(&mut self) {
        if !self.loading() {
            self.request();
        }
    }

    /// Re-read now: on the worker when there is one, inline otherwise (or when the
    /// worker is gone — fail open, D1).
    fn request(&mut self) {
        self.requested += 1;
        if let Some(w) = &self.worker {
            if w.tx.send((self.requested, self.cfg.clone())).is_ok() {
                return;
            }
            self.worker = None;
        }
        self.refresh(model::snapshot(&self.cfg));
        self.shown = self.requested;
    }

    /// Take the worker's newest finished read, if any. Called by the loop between keys.
    pub fn poll(&mut self) {
        self.live.poll(self.page() == "graph");
        self.compare.poll();
        self.exporter.poll();
        let Some(latest) = self.worker.as_ref().and_then(|w| w.rx.try_iter().last()) else {
            return;
        };
        self.refresh(latest.1);
        self.shown = latest.0;
    }

    /// The Graph page's live calls pane (T480).
    pub(super) fn live(&self) -> &LiveCalls {
        &self.live
    }

    #[cfg(test)]
    pub(super) fn live_mut(&mut self) -> &mut LiveCalls {
        &mut self.live
    }

    /// Whether a requested re-read has not reached the screen yet.
    pub fn loading(&self) -> bool {
        self.shown < self.requested
    }

    /// The Calls page's selected row (T15.5), clamped to the rows it holds — a refresh
    /// that shrinks the page can move the selection, never past its end.
    pub fn calls_selected(&self) -> usize {
        self.calls
            .selected
            .min(self.snapshot.calls.len().saturating_sub(1))
    }

    /// Whether the Calls page's detail pane is open (T15.5).
    pub fn calls_detail(&self) -> bool {
        self.calls.detail
    }

    /// Open expand pane (T60.4): archive id, rendered body (after `/` filter),
    /// whether `/` is capturing keys, the filter string, and vertical scroll.
    pub fn calls_expand(&self) -> Option<(&str, String, bool, &str, u16)> {
        let pane = self.calls.expand.as_ref()?;
        let text = if pane.filter.is_empty() {
            pane.text.clone()
        } else {
            crate::expand::filter_lines(&pane.text, None, Some(&pane.filter), 0)
                .ok()
                .map(|v| v.join("\n"))
                .unwrap_or_else(|| pane.text.clone())
        };
        Some((
            pane.id.as_str(),
            text,
            pane.filtering,
            pane.filter.as_str(),
            pane.scroll,
        ))
    }

    /// The Sessions page's selected row (T60.10), clamped to the rows it shows —
    /// the filtered list when `l` narrowed it, the snapshot's list otherwise.
    pub fn sessions_selected(&self) -> usize {
        self.sessions
            .selected
            .min(self.visible_sessions(&self.snapshot).saturating_sub(1))
    }

    /// Whether the Sessions page shows only sessions that are still running (T60.10).
    pub fn sessions_live_only(&self) -> bool {
        self.sessions.live_only
    }

    /// Whether the Sessions page's detail pane is open (T60.3).
    pub fn sessions_detail(&self) -> bool {
        self.sessions.detail
    }

    /// Whether the `?` help overlay is up (T60.8).
    pub fn help_open(&self) -> bool {
        self.help
    }

    fn visible_skills(&self, snapshot: &Snapshot) -> usize {
        snapshot
            .skills
            .rows
            .iter()
            .filter(|r| !self.skills.never_only || r.never)
            .count()
    }

    pub fn skills_selected(&self) -> usize {
        self.skills
            .selected
            .min(self.visible_skills(&self.snapshot).saturating_sub(1))
    }

    pub fn skills_never_only(&self) -> bool {
        self.skills.never_only
    }

    fn skills_key(&mut self, code: KeyCode) -> bool {
        let last = self.visible_skills(&self.snapshot).saturating_sub(1);
        match code {
            KeyCode::Up => {
                self.skills.selected = self.skills.selected.saturating_sub(1);
                true
            }
            KeyCode::Down => {
                self.skills.selected = (self.skills.selected + 1).min(last);
                true
            }
            KeyCode::Char('n') => {
                self.skills.never_only = !self.skills.never_only;
                self.skills.selected = self
                    .skills
                    .selected
                    .min(self.visible_skills(&self.snapshot).saturating_sub(1));
                true
            }
            _ => false,
        }
    }

    /// The Config page's `/` filter (T228): `Char('/')` starts capturing, `Esc`/`Enter`
    /// stops, `Backspace` edits — the same capture shape as the Calls expand pane's
    /// filter (T60.4). Returns `true` when the key was consumed.
    fn config_key(&mut self, code: KeyCode) -> bool {
        if self.config.filtering {
            match code {
                KeyCode::Esc | KeyCode::Enter => self.config.filtering = false,
                KeyCode::Backspace => {
                    self.config.filter.pop();
                }
                KeyCode::Char(c) => self.config.filter.push(c),
                _ => {}
            }
            return true;
        }
        if code == KeyCode::Char('/') {
            self.config.filtering = true;
            return true;
        }
        false
    }

    /// The Graph page's project keys (T476). A confirmed plan is written through the web page's
    /// own function, then the model is re-read so the table shows what the registry now holds.
    fn graph_key(&mut self, code: KeyCode) -> bool {
        let rows = projects::entries(&self.snapshot);
        match self.projects.key(code, &rows) {
            Key::Ignored => return false,
            Key::Handled => {}
            Key::Write(req, plan) => {
                let status = match crate::web::project_write(&self.cfg, req) {
                    // `select` answers with the registry table, which the page already shows.
                    Ok(_) if plan.starts_with("select ") => format!("{plan}: done"),
                    Ok(out) => out.trim().lines().collect::<Vec<_>>().join("; "),
                    Err(e) => format!("{e:#}"),
                };
                self.projects.set_status(status);
                self.request();
            }
        }
        true
    }

    /// The Graph page's compare view on the registry row under the cursor (T485). Typing is
    /// routed before the global keys, so a `?` or an `r` in a ref reaches the input.
    fn compare_key(&mut self, code: KeyCode) -> bool {
        let rows = projects::entries(&self.snapshot);
        let cursor = self.projects.cursor(&rows);
        let target = rows.get(cursor).map(|e| (e.id, e.name.as_str()));
        self.compare
            .key(code, target, &self.cfg, self.worker.is_some())
    }

    pub(super) fn compare(&self) -> &Compare {
        &self.compare
    }

    /// The Graph page's export form (T329.41); one panel is open at a time, so a key goes to
    /// compare or to this, never both.
    fn exporter_key(&mut self, code: KeyCode) -> bool {
        if self.compare.is_open() {
            return false;
        }
        let rows = projects::entries(&self.snapshot);
        let cursor = self.projects.cursor(&rows);
        let target = rows.get(cursor).map(|e| (e.id, e.name.as_str()));
        self.exporter
            .key(code, target, &self.cfg, self.worker.is_some())
    }

    pub(super) fn exporter(&self) -> &Exporter {
        &self.exporter
    }

    /// The Graph page's registry rows and the state its keys keep (T476).
    pub(super) fn projects(&self) -> (Vec<projects::Entry>, &projects::ProjectsState) {
        (projects::entries(&self.snapshot), &self.projects)
    }

    pub(super) fn doctor_fix(&self) -> &DoctorFix {
        &self.doctor_fix
    }

    pub(super) fn junk(&self) -> &JunkClear {
        &self.junk
    }

    pub(super) fn cfg(&self) -> &Config {
        &self.cfg
    }

    /// The Config page's filter state: whether `/` is capturing keys, and the filter text.
    pub fn config_filter(&self) -> (bool, &str) {
        (self.config.filtering, self.config.filter.as_str())
    }

    #[cfg(test)]
    pub(in crate::tui) fn set_doctor_engine(&mut self, engine: Engine) {
        self.doctor_fix = DoctorFix::new(engine);
    }

    #[cfg(test)]
    pub(in crate::tui) fn set_junk_engine(&mut self, engine: junk_clear::Engine) {
        self.junk = JunkClear::new(engine);
    }

    #[cfg(test)]
    pub(in crate::tui) fn set_skills(&mut self, skills: model::SkillsPage) {
        self.snapshot.skills = skills;
        self.skills.selected = 0;
    }

    /// The Sessions page's keys (T60.10 / T60.3): `Up`/`Down` walk the visible rows,
    /// `Enter` expands the selected one, `l` toggles the live-only filter. Returns
    /// `true` when the key was consumed.
    fn sessions_key(&mut self, code: KeyCode) -> bool {
        let last = self.visible_sessions(&self.snapshot).saturating_sub(1);
        match code {
            KeyCode::Up => {
                self.sessions.selected = self.sessions.selected.saturating_sub(1);
                true
            }
            KeyCode::Down => {
                self.sessions.selected = (self.sessions.selected + 1).min(last);
                true
            }
            KeyCode::Enter if self.visible_sessions(&self.snapshot) > 0 => {
                self.sessions.detail = !self.sessions.detail;
                true
            }
            KeyCode::Char('l') => {
                self.sessions.live_only = !self.sessions.live_only;
                self.sessions.selected = self
                    .sessions
                    .selected
                    .min(self.visible_sessions(&self.snapshot).saturating_sub(1));
                true
            }
            _ => false,
        }
    }

    /// The Calls page's keys (T15.5): `Up`/`Down` walk the rows, `Enter`/`z` expand the
    /// selected one. Returns `true` when the key was consumed; the shell's keys
    /// (`q`, arrows, digits) are never reached here.
    fn calls_key(&mut self, code: KeyCode) -> bool {
        let filtering = self
            .calls
            .expand
            .as_ref()
            .is_some_and(|pane| pane.filtering);
        if filtering {
            let pane = self.calls.expand.as_mut().expect("filtering implies pane");
            match code {
                KeyCode::Esc | KeyCode::Enter => pane.filtering = false,
                KeyCode::Backspace => {
                    pane.filter.pop();
                }
                KeyCode::Char(c) => pane.filter.push(c),
                _ => {}
            }
            return true;
        }
        if self.calls.expand.is_some() {
            match code {
                KeyCode::Esc | KeyCode::Char('e') => {
                    self.calls.expand = None;
                    return true;
                }
                KeyCode::Char('/') => {
                    if let Some(pane) = self.calls.expand.as_mut() {
                        pane.filtering = true;
                    }
                    return true;
                }
                KeyCode::Up => {
                    if let Some(pane) = self.calls.expand.as_mut() {
                        pane.scroll = pane.scroll.saturating_sub(1);
                    }
                    return true;
                }
                KeyCode::Down => {
                    if let Some(pane) = self.calls.expand.as_mut() {
                        pane.scroll = pane.scroll.saturating_add(1);
                    }
                    return true;
                }
                _ => {}
            }
        }
        let last = self.snapshot.calls.len().saturating_sub(1);
        match code {
            KeyCode::Up => {
                self.calls.selected = self.calls.selected.saturating_sub(1);
                true
            }
            KeyCode::Down => {
                self.calls.selected = (self.calls.selected + 1).min(last);
                true
            }
            KeyCode::Enter | KeyCode::Char('z') if !self.snapshot.calls.is_empty() => {
                self.calls.detail = !self.calls.detail;
                true
            }
            KeyCode::Char('e') => {
                self.open_expand();
                true
            }
            _ => false,
        }
    }

    fn open_expand(&mut self) {
        let Some(row) = self.snapshot.calls.get(self.calls_selected()) else {
            return;
        };
        let Some(id) = self.snapshot.ref_ids.get(&row.id).cloned() else {
            return;
        };
        let Some(text) = model::expand_payload(&self.cfg, &id, None) else {
            return;
        };
        self.calls.expand = Some(ExpandPane {
            id,
            text,
            filter: String::new(),
            filtering: false,
            scroll: 0,
        });
    }

    /// One key press; returns `true` when the loop should stop. `Left`/`Right` wrap,
    /// `1..=9` jump; the Calls page claims `Up`/`Down`/`Enter`/`z`/`e`/`/` (T15.5 / T60.4); the Sessions
    /// page claims `Up`/`Down`/`Enter`/`l` (T60.10 / T60.3); the Plugins page claims the row keys
    /// (T15.4); everything else is the next page's to claim.
    pub fn key(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            return true;
        }
        if self.page() == "graph" && self.compare.typing() && self.compare_key(code) {
            return false;
        }
        if self.page() == "graph" && self.exporter.typing() && self.exporter_key(code) {
            return false;
        }
        // T60.8: `?` toggles the help overlay and `r` re-reads the model before the
        // next tick; both are global.
        if code == KeyCode::Char('?') {
            self.help = !self.help;
            return false;
        }
        if code == KeyCode::Char('r') {
            self.request();
            return false;
        }
        if self.page() == "calls" && self.calls_key(code) {
            return false;
        }
        if self.page() == "sessions" && self.sessions_key(code) {
            return false;
        }
        if self.page() == "skills" && self.skills_key(code) {
            return false;
        }
        if self.page() == "config" && self.config_key(code) {
            return false;
        }
        if self.page() == "graph" && self.exporter_key(code) {
            return false;
        }
        if self.page() == "graph" && !self.exporter.is_open() && self.compare_key(code) {
            return false;
        }
        if self.page() == "graph" && self.graph_key(code) {
            return false;
        }
        if self.page() == "graph" && self.live.key(code) {
            return false;
        }
        if self.page() == "doctor" {
            match self.doctor_fix.key(code, &self.cfg) {
                Keyed::Pass => {}
                Keyed::Taken => return false,
                Keyed::Wrote => {
                    self.request();
                    return false;
                }
            }
        }
        if self.page() == "hosts" {
            match self.junk.key(code, &self.cfg) {
                Keyed::Pass => {}
                Keyed::Taken => return false,
                Keyed::Wrote => {
                    self.request();
                    return false;
                }
            }
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => true,
            KeyCode::Left => {
                self.step(-1);
                false
            }
            KeyCode::Right => {
                self.step(1);
                false
            }
            // The Plugins tab's row keys; every other page falls through (a Space there
            // reaches the digit arm, whose `to_digit` answer for ' ' is none).
            KeyCode::Up | KeyCode::Down | KeyCode::Enter | KeyCode::Char(' ')
                if self.page() == "plugins" =>
            {
                self.plugins_key(code);
                false
            }
            KeyCode::Char(c) => {
                // 1..=9, not 0: tabs count from one and nine is plenty for a tab bar.
                if let Some(d) = c.to_digit(10).filter(|d| (1..=9).contains(d)) {
                    self.select(d as usize - 1);
                }
                false
            }
            _ => false,
        }
    }

    /// The Plugins tab's keys (T15.4): `Up`/`Down` move the row cursor, `Space`/`Enter`
    /// toggle the selected plugin.
    fn plugins_key(&mut self, code: KeyCode) {
        let len = self.snapshot.plugins.len();
        match code {
            KeyCode::Up => {
                self.plugin_cursor = self.plugin_cursor.saturating_sub(1);
                self.plugin_status.clear();
            }
            KeyCode::Down => {
                if len > 0 {
                    self.plugin_cursor = (self.plugin_cursor + 1).min(len - 1);
                }
                self.plugin_status.clear();
            }
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_selected(),
            _ => {}
        }
    }

    /// Flip the selected plugin's `[plugins.<id>] enabled` (T15.4) through the one
    /// config writer — [`validate::set`], the API `rtok config set` uses — never a
    /// second one. The write always targets `<home>/config.toml`, as `config set`
    /// does; the row then re-renders off the effective value through the layering (an
    /// env override of the key still wins over the file, so the status line names what
    /// a fresh `rtok plugins` would print, not what the key press hoped for). A
    /// refusal — unknown plugin id, unwritable file — is a status line, not a crash:
    /// an operator surface fails open (D1).
    fn toggle_selected(&mut self) {
        let Some(id) = self.snapshot.plugins.get(self.plugin_cursor).map(|p| p.id) else {
            self.plugin_status = "no plugin to toggle".into();
            return;
        };
        let written = !self.snapshot.plugins[self.plugin_cursor].enabled;
        let key = format!("plugins.{id}.enabled");
        match validate::set(&self.cfg.home, &key, &written.to_string(), false) {
            Ok(_) => {
                let on = Config::load_from(&self.cfg.home)
                    .map(|reloaded| reloaded.plugin_enabled(id, true))
                    .unwrap_or(written);
                self.cfg.set_plugin_enabled(id, on);
                self.plugin_status = format!("{id} {}", if on { "on" } else { "off" });
                self.request();
            }
            Err(e) => self.plugin_status = format!("config set {key}: {e:#}"),
        }
    }

    /// Jump to tab `i`, modulo the list — out of range wraps rather than panics.
    fn select(&mut self, i: usize) {
        if !self.tabs.is_empty() {
            self.selected = i % self.tabs.len();
        }
    }

    fn step(&mut self, dir: i32) {
        if !self.tabs.is_empty() {
            let len = self.tabs.len() as i32;
            self.selected = (self.selected as i32 + dir).rem_euclid(len) as usize;
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A config on its own temp home whose every path stays inside it. T74: a snapshot on the
    /// real `~/.claude/projects` JSONL cost ≈30 s CPU on a busy box (≈150 s for the toggle
    /// test) and once tripped the suite-level 180 s kill.
    pub(in crate::tui) fn config() -> Config {
        // A dir per call: one shared `rtok-tui-<pid>` was deleted by each parallel test
        // while another was still opening its store in it (os error 22).
        crate::testutil::config_file_in(&crate::testutil::tmp_dir("tui"))
    }

    /// The screen of `app` as text, `width` columns and 40 rows.
    pub(in crate::tui) fn render(app: &App, width: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 40)).unwrap();
        terminal
            .draw(|frame| crate::tui::view::draw(frame, app))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        buffer
            .content()
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The Plugins tab with the row cursor on `id` — where the T15.4 toggle tests
    /// start. `[tui] tab` picks the page (T15.8), so the helper says which tab it
    /// means rather than counting pages; a model without the row fails the test.
    pub(in crate::tui) fn cursor_on_plugin(cfg: &Config, id: &str) -> App {
        let mut cfg = cfg.clone();
        cfg.tui.tab = "plugins".into();
        let mut app = App::new(&cfg);
        assert_eq!(app.page(), "plugins");
        let i = app
            .snapshot()
            .plugins
            .iter()
            .position(|p| p.id == id)
            .unwrap_or_else(|| panic!("no {id} row on the Plugins page"));
        for _ in 0..i {
            app.key(KeyCode::Down, KeyModifiers::NONE);
        }
        app
    }

    /// D23: the tab bar is the model's page list, never a second one.
    #[test]
    fn tabs_are_the_model_pages() {
        let app = App::new(&config());
        assert_eq!(
            app.tab_names(),
            model::pages()
                .iter()
                .map(|(page, _)| *page)
                .collect::<Vec<_>>()
        );
        assert_eq!(app.page(), "overview");
    }

    /// T15.8: `[tui] tab` picks the opening tab; empty or unknown is the first page.
    #[test]
    fn tui_tab_picks_the_opening_tab() {
        let names: Vec<&str> = model::pages().iter().map(|(page, _)| *page).collect();
        assert!(names.len() >= 2, "needs two pages to choose between");
        let mut cfg = config();
        cfg.tui.tab = names[1].to_string();
        assert_eq!(App::new(&cfg).page(), names[1]);
        cfg.tui.tab = "no-such-page".to_string();
        assert_eq!(App::new(&cfg).page(), names[0]);
        cfg.tui.tab = String::new();
        assert_eq!(App::new(&cfg).page(), names[0]);
    }

    #[test]
    fn left_right_wrap_and_digits_jump() {
        let mut app = App::new(&config());
        let names = app.tab_names();
        let n = names.len();
        assert!(n >= 2, "the model serves at least the two T15.0 pages");
        app.key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(app.page(), names[1]);
        app.key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(app.page(), names[2 % n], "wraps past the last tab");
        app.key(KeyCode::Left, KeyModifiers::NONE);
        app.key(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(
            app.page(),
            names[0],
            "two steps each way land back on tab one"
        );
        app.key(KeyCode::Char('1'), KeyModifiers::NONE);
        assert_eq!(app.page(), names[0]);
        app.key(KeyCode::Char('2'), KeyModifiers::NONE);
        assert_eq!(app.page(), names[1]);
        app.key(KeyCode::Char('9'), KeyModifiers::NONE);
        assert_eq!(app.page(), names[8 % n], "a digit past the list wraps");
        app.key(KeyCode::Char('0'), KeyModifiers::NONE);
        assert_eq!(app.page(), names[8 % n], "0 is not a tab");
    }

    /// The running TUI never reads the model on the key thread: it opens on an empty
    /// snapshot, tabs switch while the read runs, and `poll` lands the worker's result.
    #[test]
    fn background_app_switches_tabs_while_the_model_loads() {
        let cfg = config();
        let mut app = App::background(&cfg);
        assert!(app.loading(), "the first read is on the worker");
        app.key(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(app.page(), app.tab_names()[1], "keys work mid-read");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while app.loading() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
            app.poll();
        }
        assert!(!app.loading(), "the worker delivered");
        assert_eq!(
            app.snapshot().plugins.len(),
            model::snapshot(&cfg).plugins.len()
        );
        app.tick();
        assert!(app.loading(), "a tick requests another read");
        app.tick();
        assert_eq!(app.requested, 2, "a tick mid-read is skipped");
    }

    #[test]
    fn q_esc_and_ctrl_c_quit() {
        let mut app = App::new(&config());
        assert!(app.key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!app.key(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(!app.key(KeyCode::Char('x'), KeyModifiers::NONE));
    }

    #[test]
    fn refresh_serves_the_new_snapshot_and_stamps_it() {
        let cfg = config();
        let mut app = App::new(&cfg);
        let before = app.updated();
        app.refresh(model::snapshot(&cfg));
        assert!(app.updated() >= before);
        assert_eq!(
            app.snapshot().plugins.len(),
            model::snapshot(&cfg).plugins.len()
        );
    }

    /// T15.4: `Up`/`Down` move the Plugins tab's row cursor and clamp at both ends.
    #[test]
    fn plugins_cursor_moves_and_clamps() {
        let mut app = App::new(&config());
        app.key(KeyCode::Right, KeyModifiers::NONE);
        let len = app.snapshot().plugins.len();
        assert!(len >= 2, "the catalogue has rows to move between");
        assert_eq!(app.plugin_cursor(), 0);
        app.key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(app.plugin_cursor(), 0, "Up at the top stays put");
        app.key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.plugin_cursor(), 1);
        for _ in 0..=len {
            app.key(KeyCode::Down, KeyModifiers::NONE);
        }
        assert_eq!(app.plugin_cursor(), len - 1, "Down clamps at the last row");
    }

    /// T15.4: the row keys are the Plugins page's, not global — on Overview they are
    /// nothing, and a Space is not a quit either.
    #[test]
    fn row_keys_do_nothing_off_the_plugins_page() {
        let mut app = App::new(&config());
        for code in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Char(' '),
        ] {
            assert!(!app.key(code, KeyModifiers::NONE), "{code:?} is not a quit");
            assert_eq!(app.page(), "overview", "{code:?} moved nothing");
        }
    }

    /// T15.4: Space flips `plugins.<id>.enabled` through `rtok config set`'s own writer
    /// (`validate::set` on `<home>/config.toml`), the row re-renders off the re-read
    /// model, and the next tick serves the same answer. Its own temp home — the write
    /// is real, just never the operator's file.
    #[test]
    fn space_toggles_the_selected_plugin_through_config_set() {
        let dir = std::env::temp_dir().join(format!("rtok-tui-toggle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = crate::testutil::config_file_in(&dir);
        let mut app = cursor_on_plugin(&cfg, "cmd");
        let row = |app: &App| {
            app.snapshot()
                .plugins
                .iter()
                .find(|p| p.id == "cmd")
                .expect("cmd row")
                .enabled
        };
        assert!(Config::load_from(&dir).unwrap().plugin_enabled("cmd", true));
        app.key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(
            !Config::load_from(&dir).unwrap().plugin_enabled("cmd", true),
            "the file changed — the write went through config set's writer"
        );
        assert!(!row(&app), "the row re-read the write");
        assert!(
            app.plugin_status().contains("cmd off"),
            "the status names it"
        );
        app.tick();
        assert!(
            !row(&app),
            "the next tick keeps the write, not the launch copy"
        );
        app.key(KeyCode::Enter, KeyModifiers::NONE); // Enter toggles too
        assert!(
            Config::load_from(&dir).unwrap().plugin_enabled("cmd", true),
            "toggled back on"
        );
        assert!(row(&app));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T15.5: the Calls page claims `Up`/`Down`/`Enter`/`z` — the selection clamps to
    /// the rows it holds and the detail toggles — while the shell's keys keep working
    /// on the same page.
    #[test]
    fn calls_page_claims_its_keys_and_clamps_the_selection() {
        let cfg = config();
        let mut app = App::new(&cfg);
        let calls = model::pages()
            .iter()
            .position(|(p, _)| *p == "calls")
            .unwrap();
        app.select(calls);
        assert!(app.snapshot().calls.is_empty(), "nothing seeded");
        assert_eq!(app.calls_selected(), 0);
        app.key(KeyCode::Down, KeyModifiers::NONE);
        app.key(KeyCode::Up, KeyModifiers::NONE);
        app.key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.calls_selected(), 0, "clamped on an empty page");
        assert!(!app.calls_detail(), "Enter does nothing with no rows");

        // Two rows: the selection walks and clamps at both ends, Enter/z toggle.
        app.refresh({
            let mut snap = model::snapshot(&cfg);
            snap.calls = vec![row(7), row(8)];
            snap
        });
        app.key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(app.calls_selected(), 0, "clamped at the newest row");
        app.key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.calls_selected(), 1);
        app.key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.calls_selected(), 1, "clamped at the oldest row");
        app.key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.calls_detail());
        app.key(KeyCode::Char('z'), KeyModifiers::NONE);
        assert!(!app.calls_detail(), "z closes what Enter opened");

        // A refresh that shrinks the page moves the selection back inside it.
        app.refresh({
            let mut snap = model::snapshot(&cfg);
            snap.calls = vec![row(9)];
            snap
        });
        assert_eq!(app.calls_selected(), 0);

        // The shell's keys still work on the Calls page.
        assert!(app.key(KeyCode::Char('q'), KeyModifiers::NONE), "q quits");
    }

    /// T60.8: `?` toggles the help overlay state and `r` re-reads the model — both
    /// global, neither quits.
    #[test]
    fn question_mark_and_r_are_global() {
        let cfg = config();
        let mut app = App::new(&cfg);
        assert!(!app.help_open());
        app.key(KeyCode::Char('?'), KeyModifiers::NONE);
        assert!(app.help_open());
        app.key(KeyCode::Char('?'), KeyModifiers::NONE);
        assert!(!app.help_open(), "? toggles closed again");
        let before = app.updated();
        app.key(KeyCode::Char('r'), KeyModifiers::NONE);
        assert!(app.updated() >= before, "the refresh restamped the tick");
        assert!(
            !app.key(KeyCode::Char('r'), KeyModifiers::NONE),
            "r is not a quit"
        );
    }

    /// T60.8: the KEYS table documents every page whose keys the handler claims —
    /// the one-source check the footer and the overlay render from.
    #[test]
    fn keys_table_covers_the_row_state_pages() {
        for page in ["plugins", "calls", "sessions"] {
            assert!(
                keys_for(page).len() >= 2 + keys_for("").len(),
                "{page} documents its row keys"
            );
        }
        assert_eq!(
            keys_for("overview").len(),
            keys_for("").len(),
            "overview claims no row keys"
        );
    }

    /// T60.10: the Sessions page claims `Up`/`Down`/`l` — the selection clamps to
    /// the rows it shows (the filtered list under `l`), and the shell's keys keep
    /// working on the same page.
    #[test]
    fn sessions_page_claims_its_keys_and_clamps_the_selection() {
        let mut cfg = config();
        cfg.tui.tab = "sessions".into();
        let mut app = App::new(&cfg);
        assert_eq!(app.page(), "sessions");
        assert!(app.snapshot().sessions.is_empty(), "nothing seeded");
        app.key(KeyCode::Down, KeyModifiers::NONE);
        app.key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(app.sessions_selected(), 0, "clamped on an empty page");
        assert!(!app.sessions_live_only(), "the filter starts off");

        // Two rows: the selection walks and clamps at both ends.
        app.refresh({
            let mut snap = model::snapshot(&cfg);
            snap.sessions = vec![session("a", None), session("b", None)];
            snap
        });
        app.key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(app.sessions_selected(), 0, "clamped at the top");
        app.key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.sessions_selected(), 1);
        app.key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.sessions_selected(), 1, "clamped at the last row");

        // `l` narrows to live rows; the cursor re-clamps against the filtered list.
        app.key(KeyCode::Char('l'), KeyModifiers::NONE);
        assert!(app.sessions_live_only());
        app.refresh({
            let mut snap = model::snapshot(&cfg);
            snap.sessions = vec![session("a", None), session("b", Some(9))];
            snap
        });
        assert_eq!(
            app.sessions_selected(),
            0,
            "one visible row under the live-only filter"
        );
        app.key(KeyCode::Char('l'), KeyModifiers::NONE);
        assert!(!app.sessions_live_only(), "l toggles back");

        app.key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            app.sessions_detail(),
            "Enter opens the pane when rows exist"
        );
        app.key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(!app.sessions_detail(), "Enter toggles it closed");

        // The shell's keys still work on the Sessions page.
        assert!(app.key(KeyCode::Char('q'), KeyModifiers::NONE), "q quits");
    }

    /// A bare session total for the selection test — the view's tests seed real ones.
    fn session(id: &str, ended: Option<i64>) -> crate::store::SessionTotals {
        crate::store::SessionTotals {
            id: id.into(),
            host: None,
            project: None,
            provider: None,
            api: None,
            model: None,
            input: 0,
            cache_create: 0,
            cache_read: 0,
            output: 0,
            started_at: 0,
            last_activity: 0,
            ended_at: ended,
        }
    }

    /// A bare ledger row for the selection test — the view's tests seed real ones.
    fn row(id: i32) -> crate::store::CallRow {
        crate::store::CallRow {
            id,
            ts: 0,
            session: "s".into(),
            surface: "hook".into(),
            kind: "hook".into(),
            plugin: None,
            name: None,
            parent_id: None,
            ms: None,
            ok: 1,
            error: None,
            host: None,
            provider: None,
            model: None,
            api: None,
            input: None,
            cache_create: None,
            cache_read: None,
            output: None,
        }
    }
}

// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok doctor --fix` (T331.5, T331.6): the first doctor code that writes a host config. It
//! removes only the entries the check classed `broken-hook`, and the extra copies of a
//! `duplicate-hook` or `duplicate-mcp`, that it may fix, through the JSONC editor `agents install`
//! uses (so every other byte of the file stays; a TOML server table goes through `toml_edit`),
//! after a backup and with an atomic swap. Without `--yes` it only prints the diff it would apply.
//! A file that changed since the check, or whose edit would change anything but the selected
//! entries, is skipped and reported, never written. The kept copy of a duplicate, a plugin's
//! files, rtok's own entries and every hook that is not plainly broken are never removed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::checklist::{self, Choice, Prompt};
use super::hooks::{self, Probes, Problem, entries};
use super::mcp_dupes::{self, Loc};
use super::mcp_fix;
use super::probe::Writer;
use crate::agents::jsonc::{self, Seg};
use crate::config::Config;

/// What happened to one file.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Dry run: the diff is what `--yes` would write.
    Planned,
    Written,
    /// Left as it was, with the reason (changed since the check, the edit check failed).
    Skipped(String),
    /// The backup or the write failed; the file is as it was.
    Failed(String),
}

#[derive(Debug)]
pub struct FileFix {
    pub source: PathBuf,
    pub removed: Vec<Problem>,
    pub diff: String,
    pub backup: Option<PathBuf>,
    pub outcome: Outcome,
}

#[derive(Debug, Default)]
pub struct FixReport {
    pub files: Vec<FileFix>,
    /// Selected entries this command will not remove, with why.
    pub refused: Vec<(Problem, &'static str)>,
    /// Selected findings left after the writes (the re-check).
    pub left: usize,
}

impl FixReport {
    /// 1 when a selected hook was not fixed (skipped or failed), else 0.
    pub fn exit_code(&self) -> i32 {
        i32::from(
            self.files
                .iter()
                .any(|f| matches!(f.outcome, Outcome::Skipped(_) | Outcome::Failed(_))),
        )
    }
}

/// rtok's own hook entries are installed, updated and removed by `rtok agents`, which knows
/// the plugin rules (T275); a doctor removal would be undone by the next update.
fn is_rtok_own(command: &str) -> bool {
    let words = shlex::split(command).unwrap_or_default();
    words.iter().any(|w| {
        crate::agents::is_rtok_bin(w) || Path::new(w).file_stem().is_some_and(|s| s == "rtok-hook")
    })
}

/// What install writes again: a hook that runs rtok, or the MCP entry named `rtok`. A copy of
/// the server under another name (`rtok-mcp`) is a hand-written extra and may go (T331.10).
fn is_own(x: &Problem) -> bool {
    if x.kind == "duplicate-mcp" {
        return x.path.rsplit('.').next() == Some(rtok_mcp::registry::RTOK.name);
    }
    is_rtok_own(&x.command)
}

/// `(event, group, hook)` of a `hooks.<event>[g]` or `hooks.<event>[g].hooks[h]` key path.
fn parse_path(path: &str) -> Option<(&str, usize, Option<usize>)> {
    let rest = path.strip_prefix("hooks.")?;
    let (event, rest) = rest.split_once('[')?;
    let (g, rest) = rest.split_once(']')?;
    let hook = match rest {
        "" => None,
        r => {
            let h = r.strip_prefix(".hooks[")?.strip_suffix(']')?;
            Some(h.parse().ok()?)
        }
    };
    Some((event, g.parse().ok()?, hook))
}

/// `raw` without the entries at `paths`, each followed by the removal of the group and the
/// event key it leaves empty. Removal runs from the last entry to the first so earlier indices
/// stay valid. `None` when any path leads nowhere.
fn remove_entries(raw: &str, file: &Path, paths: &[&str]) -> Option<String> {
    let mut targets: Vec<(&str, usize, Option<usize>)> =
        paths.iter().map(|p| parse_path(p)).collect::<Option<_>>()?;
    targets.sort_by(|a, b| b.cmp(a));
    let mut body = raw.to_string();
    for (event, g, hook) in targets {
        let group = [Seg::Key("hooks"), Seg::Key(event), Seg::Index(g)];
        let inner = [group[0], group[1], group[2], Seg::Key("hooks")];
        let target: Vec<Seg> = match hook {
            Some(h) => inner.iter().copied().chain([Seg::Index(h)]).collect(),
            None => group.to_vec(),
        };
        let (next, found) = jsonc::remove_at(&body, file, &target).ok()?;
        if !found {
            return None;
        }
        body = next;
        if hook.is_some() && jsonc::is_empty_at(&body, &inner) {
            body = jsonc::remove_at(&body, file, &group).ok()?.0;
        }
        let events = [Seg::Key("hooks"), Seg::Key(event)];
        if jsonc::is_empty_at(&body, &events) {
            body = jsonc::remove_at(&body, file, &events).ok()?.0;
        }
    }
    Some(body)
}

/// The `(event, matcher, command)` of every hook of `raw`, in order.
fn hook_list(raw: &str) -> Option<Vec<(String, Option<String>, String)>> {
    let doc = jsonc::parse(raw).ok()?;
    Some(
        entries(&doc)
            .into_iter()
            .map(|e| (e.event, e.matcher, e.command))
            .collect(),
    )
}

/// The edit of `raw` that drops `removed`, or the reason it cannot be trusted: the new text must
/// still parse and hold exactly the old hooks and servers minus the removed ones.
fn edit(
    raw: &str,
    file: &Path,
    removed: &[&Problem],
    locs: &[Loc],
) -> Result<String, &'static str> {
    let (mcp, hooks): (Vec<&Problem>, Vec<&Problem>) =
        removed.iter().partition(|p| p.kind == "duplicate-mcp");
    let body = if hooks.is_empty() {
        raw.to_string()
    } else {
        edit_hooks(raw, file, &hooks)?
    };
    if mcp.is_empty() {
        return Ok(body);
    }
    let at: Option<Vec<&Loc>> = mcp
        .iter()
        .map(|p| {
            locs.iter()
                .find(|l| l.source == p.source && l.path == p.path)
        })
        .collect();
    mcp_fix::remove(&body, file, &at.ok_or("an entry could not be located")?)
}

fn edit_hooks(raw: &str, file: &Path, removed: &[&Problem]) -> Result<String, &'static str> {
    let paths: Vec<&str> = removed.iter().map(|p| p.path.as_str()).collect();
    let body = remove_entries(raw, file, &paths).ok_or("an entry could not be located")?;
    let (before, after) = (hook_list(raw), hook_list(&body));
    let expect: Option<Vec<_>> = before.map(|all| {
        let mut gone: Vec<_> = removed
            .iter()
            .map(|p| (p.event.clone(), p.matcher.clone(), p.command.clone()))
            .collect();
        all.into_iter()
            .filter(|h| match gone.iter().position(|g| g == h) {
                Some(i) => {
                    gone.swap_remove(i);
                    false
                }
                None => true,
            })
            .collect()
    });
    if after.is_none() || after != expect {
        return Err("the edit would change more than the selected hooks");
    }
    Ok(body)
}

/// What `--fix` works on, by [`Problem::kind`].
pub const KINDS: [&str; 3] = ["broken-hook", "duplicate-hook", "duplicate-mcp"];

/// The hook and MCP findings of this machine, for one host or all.
fn findings(cfg: &Config, p: &Probes, agent: Option<&str>) -> Vec<Problem> {
    let (mut all, plugins) = hooks::check_with_plugins(cfg, p);
    all.retain(|x| agent.is_none_or(|a| x.agent == a));
    let mcp = mcp_dupes::check(cfg, p, &plugins);
    all.extend(
        mcp.into_iter()
            .filter(|x| agent.is_none_or(|a| x.agent == a)),
    );
    all
}

/// A finding `--fix` would remove: a broken hook, or a copy of a duplicate that is not the one
/// kept. `kinds` is what the user selected.
fn wanted(x: &Problem, kinds: &[&str]) -> bool {
    kinds.contains(&x.kind) && (x.kind == "broken-hook" || !x.keep)
}

/// Remove every fixable broken hook. `apply` false only plans. `keep` is how many backup
/// generations to keep per file.
pub fn fix_broken(cfg: &Config, p: &Probes, w: &dyn Writer, apply: bool, keep: usize) -> FixReport {
    fix_for(cfg, p, w, apply, keep, None, &KINDS[..1])
}

/// What a run selects: how many backup generations to keep per file, one host (`--agent`;
/// `None` is every host) and the kinds of finding.
pub struct Opts<'a> {
    pub keep: usize,
    pub agent: Option<&'a str>,
    pub kinds: &'a [&'a str],
}

/// A finding that `--fix` removes unless the user's selection says otherwise.
pub fn removable(x: &Problem, kinds: &[&str]) -> bool {
    wanted(x, kinds) && x.fixable && !is_own(x)
}

/// [`fix_broken`] for the `kinds` selected and one host's entries (`--agent`); `None` is every
/// host.
pub fn fix_for(
    cfg: &Config,
    p: &Probes,
    w: &dyn Writer,
    apply: bool,
    keep: usize,
    agent: Option<&str>,
    kinds: &[&str],
) -> FixReport {
    let opts = Opts { keep, agent, kinds };
    let found = findings(cfg, p, agent);
    fix_found(cfg, p, w, apply, &opts, found, &BTreeSet::new())
}

/// The findings `--fix` starts from, for the checklist to show and change.
pub fn candidates(cfg: &Config, p: &Probes, agent: Option<&str>) -> Vec<Problem> {
    findings(cfg, p, agent)
}

/// [`fix_for`] over `found`, without the entries whose `(source, path)` is in `skip` (what the
/// user deselected in the checklist).
pub fn fix_found(
    cfg: &Config,
    p: &Probes,
    w: &dyn Writer,
    apply: bool,
    o: &Opts,
    found: Vec<Problem>,
    skip: &BTreeSet<(String, String)>,
) -> FixReport {
    let (keep, agent, kinds) = (o.keep, o.agent, o.kinds);
    let mut report = FixReport::default();
    let mut by_file: Vec<(String, Vec<Problem>)> = Vec::new();
    // A copy that goes as broken must not be the one kept: the extras then stay, so a hook
    // is never removed from every file at once because it was a duplicate.
    let at = |x: &Problem| (x.source.clone(), x.path.clone());
    let broken: BTreeSet<_> = found
        .iter()
        .filter(|x| x.kind == "broken-hook" && x.fixable && kinds.contains(&x.kind))
        .filter(|x| !skip.contains(&at(x)))
        .map(at)
        .collect();
    let lost: BTreeSet<_> = found
        .iter()
        .filter(|x| x.kind == "duplicate-hook" && x.keep && broken.contains(&at(x)))
        .filter_map(|x| x.group)
        .collect();
    let mut taken = BTreeSet::new();
    let chosen = |x: &&Problem| wanted(x, kinds) && !skip.contains(&at(x));
    for problem in found.iter().filter(chosen).cloned() {
        if problem.kind == "duplicate-hook" && problem.group.is_some_and(|g| lost.contains(&g)) {
            continue;
        }
        if !problem.fixable {
            let why = if problem.source.ends_with(".toml") && problem.kind != "duplicate-mcp" {
                "TOML hook files are not edited yet"
            } else {
                "not a file of yours to edit"
            };
            report.refused.push((problem, why));
        } else if is_own(&problem) {
            report.refused.push((
                problem,
                "rtok's own entry: `rtok agents install` rewrites it",
            ));
        } else if !taken.insert(at(&problem)) {
            // Broken and an extra copy at once: one removal.
        } else if let Some((_, list)) = by_file.iter_mut().find(|(s, _)| *s == problem.source) {
            list.push(problem);
        } else {
            by_file.push((problem.source.clone(), vec![problem]));
        }
    }
    let locs = if kinds.contains(&"duplicate-mcp") {
        mcp_dupes::locations(cfg, p)
    } else {
        Vec::new()
    };
    for (source, problems) in by_file {
        let path = PathBuf::from(&source);
        report
            .files
            .push(fix_file(p, w, &path, problems, apply, keep, &locs));
    }
    if apply {
        report.left = findings(cfg, p, agent)
            .iter()
            .filter(|x| wanted(x, kinds))
            .count();
    }
    report
}

fn fix_file(
    p: &Probes,
    w: &dyn Writer,
    path: &Path,
    removed: Vec<Problem>,
    apply: bool,
    keep: usize,
    locs: &[Loc],
) -> FileFix {
    let mut fix = FileFix {
        source: path.to_path_buf(),
        removed,
        diff: String::new(),
        backup: None,
        outcome: Outcome::Planned,
    };
    let refs: Vec<&Problem> = fix.removed.iter().collect();
    let planned =
        p.fs.read(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))
            .and_then(|raw| {
                let body = edit(&raw, path, &refs, locs).map_err(str::to_string)?;
                Ok((raw, body))
            });
    let (raw, body) = match planned {
        Ok(ok) => ok,
        Err(why) => {
            fix.outcome = Outcome::Skipped(why);
            return fix;
        }
    };
    fix.diff = change_preview::unified_diff(path, &raw, &body);
    if !apply {
        return fix;
    }
    // The window between the check and this write is short but real: an agent or an editor may
    // have saved the file, and its change must not be overwritten with a copy of the old text.
    if p.fs.read(path).ok().as_deref() != Some(raw.as_str()) {
        fix.outcome = Outcome::Skipped("changed since the check".into());
        return fix;
    }
    match w.backup(path, keep) {
        Ok(bak) => fix.backup = bak,
        Err(e) => {
            fix.outcome = Outcome::Failed(format!("backup failed: {e}"));
            return fix;
        }
    }
    fix.outcome = match w.write(path, &body) {
        Ok(()) => Outcome::Written,
        Err(e) => Outcome::Failed(format!("write failed: {e}")),
    };
    fix
}

/// One line per selected entry the engine will not remove.
pub fn refusals(r: &FixReport) -> Vec<String> {
    let line = |(p, why): &(Problem, &str)| {
        format!(
            "not removed: {} `{}` in {}: {why}",
            label(p),
            p.command,
            p.source
        )
    };
    r.refused.iter().map(line).collect()
}

/// The diffs of the files a planned run would change, and why a file is skipped.
pub fn diffs(r: &FixReport) -> String {
    let shown = r.files.iter().map(|f| match &f.outcome {
        Outcome::Skipped(why) => format!("{}: skipped: {why}\n", f.source.display()),
        _ => f.diff.clone(),
    });
    shown.collect()
}

/// What a finding is called in the text: a hook by event and matcher, a server by its path.
pub fn label(p: &Problem) -> String {
    match (p.kind, p.matcher.as_deref()) {
        ("duplicate-mcp", _) => format!("mcp {}", p.path),
        (_, Some(m)) => format!("{}[{m}]", p.event),
        _ => p.event.clone(),
    }
}

/// The text `rtok doctor --fix` prints.
pub fn render(r: &FixReport, apply: bool) -> String {
    let mut out = String::new();
    if r.files.is_empty() && r.refused.is_empty() {
        return "nothing to remove\n".into();
    }
    if !apply {
        out.push_str("dry run: nothing is written. Add --yes to remove the entries below.\n");
    }
    let mut planned = Vec::new();
    for f in &r.files {
        out.push_str(&format!("{}\n", f.source.display()));
        for p in &f.removed {
            let verb = if apply { "remove" } else { "would remove" };
            out.push_str(&format!(
                "  {verb} {} `{}`: {}\n",
                label(p),
                p.command,
                p.detail
            ));
        }
        if let Some(b) = &f.backup {
            out.push_str(&format!("  backup {}\n", b.display()));
        }
        match &f.outcome {
            Outcome::Planned => {
                planned.push((f.source.clone(), f.diff.clone()));
                if !crate::stat_preview::on() {
                    out.push_str(&f.diff);
                }
            }
            Outcome::Written => out.push_str("  written\n"),
            Outcome::Skipped(why) => out.push_str(&format!("  skipped: {why}\n")),
            Outcome::Failed(why) => out.push_str(&format!("  failed: {why}\n")),
        }
    }
    if !planned.is_empty() {
        out.push_str(&crate::stat_preview::show(&planned, ""));
        out.push('\n');
    }
    for line in refusals(r) {
        out.push_str(&format!("{line}\n"));
    }
    if apply {
        let done: usize = r
            .files
            .iter()
            .filter(|f| f.outcome == Outcome::Written)
            .map(|f| f.removed.len())
            .sum();
        let noun = if done == 1 { "entry" } else { "entries" };
        out.push_str(&format!("{done} {noun} removed, {} left\n", r.left));
    }
    out
}

/// `--fix` against this machine: the report text and the exit code. With a `prompt` and no
/// `--yes` the user picks what goes, in the checklist.
pub fn run(
    cfg: &Config,
    apply: bool,
    agent: Option<&str>,
    kinds: &[&str],
    prompt: Option<&mut dyn Prompt>,
) -> (String, i32) {
    on_this_machine(|probes, w| {
        let o = Opts {
            keep: cfg.setup.backup_files as usize,
            agent,
            kinds,
        };
        match prompt.filter(|_| !apply) {
            Some(prompt) => interactive(cfg, probes, w, &o, prompt),
            None => {
                let r = fix_for(cfg, probes, w, apply, o.keep, agent, kinds);
                (render(&r, apply), r.exit_code())
            }
        }
    })
}

/// `f` over this machine's files, environment, `PATH` and the real backup-and-write.
pub fn on_this_machine<R>(f: impl FnOnce(&Probes, &dyn Writer) -> R) -> R {
    let probes = Probes {
        fs: &super::probe::RealFs,
        env: &super::probe::RealEnv,
        which: &super::probe::RealWhich,
    };
    f(&probes, &super::probe::RealWriter)
}

/// The checklist, then the write of what the user confirmed. Cancelling writes nothing.
pub fn interactive(
    cfg: &Config,
    p: &Probes,
    w: &dyn Writer,
    o: &Opts,
    prompt: &mut dyn Prompt,
) -> (String, i32) {
    let mut found = candidates(cfg, p, o.agent);
    let (cwd, home) = (p.env.cwd(), p.env.home());
    let shared = checklist::shared_in(cwd.as_deref(), home.as_deref());
    if !found.iter().any(|x| removable(x, o.kinds)) {
        let r = fix_found(cfg, p, w, false, o, found, &BTreeSet::new());
        return (render(&r, false), 0);
    }
    let preview = |all: &[Problem], skip: &BTreeSet<(String, String)>| {
        let plan = fix_found(cfg, p, w, false, o, all.to_vec(), skip);
        diffs(&plan)
    };
    match checklist::run(&mut found, o.kinds, &shared, &preview, prompt) {
        Choice::Cancel => ("cancelled: nothing was written\n".into(), 0),
        Choice::Apply(skip) => {
            let r = fix_found(cfg, p, w, true, o, found, &skip);
            (render(&r, true), r.exit_code())
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::doctor::probe::{Env, Fs, PathKind, Which};
    use proptest::prelude::*;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::io;

    /// An in-memory machine whose files the `Writer` side can change.
    #[derive(Default)]
    pub(crate) struct Machine {
        pub(crate) files: RefCell<BTreeMap<PathBuf, String>>,
        fail_backup: bool,
        fail_write: bool,
        /// Rewrites the file when it is read for the `race_at`-th time (an editor saving in
        /// between the plan and the write).
        race: RefCell<Option<(PathBuf, String)>>,
        race_at: Cell<usize>,
        reads: RefCell<usize>,
        pub(crate) backups: RefCell<Vec<PathBuf>>,
    }

    impl Fs for Machine {
        fn canonical(&self, path: &Path) -> PathBuf {
            path.to_path_buf()
        }
        fn read(&self, path: &Path) -> io::Result<String> {
            *self.reads.borrow_mut() += 1;
            let hit = self.race.borrow().clone();
            if let Some((p, body)) = hit
                && p == path
                && *self.reads.borrow() == self.race_at.get()
            {
                self.files.borrow_mut().insert(p, body);
            }
            self.files
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn kind(&self, path: &Path) -> PathKind {
            if self.files.borrow().contains_key(path) {
                PathKind::File { executable: true }
            } else {
                PathKind::Missing
            }
        }
    }
    impl Env for Machine {
        fn var(&self, _: &str) -> Option<String> {
            None
        }
        fn home(&self) -> Option<PathBuf> {
            Some("/h".into())
        }
        fn cwd(&self) -> Option<PathBuf> {
            Some("/proj".into())
        }
    }
    impl Which for Machine {
        fn find(&self, _: &str) -> Option<PathBuf> {
            None
        }
    }
    impl Writer for Machine {
        fn backup(&self, path: &Path, _keep: usize) -> io::Result<Option<PathBuf>> {
            if self.fail_backup {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            let bak = PathBuf::from(format!("/h/_backup/{}.bak", path.display()));
            self.backups.borrow_mut().push(bak.clone());
            Ok(Some(bak))
        }
        fn write(&self, path: &Path, body: &str) -> io::Result<()> {
            if self.fail_write {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            self.files.borrow_mut().insert(path.into(), body.into());
            Ok(())
        }
    }

    pub(crate) const SETTINGS: &str = "/h/.claude/settings.json";

    pub(crate) fn cfg() -> Config {
        let mut c = Config::default();
        c.doctor.settings_path = SETTINGS.into();
        c.setup.claude.settings_path = SETTINGS.into();
        c.setup.cursor.hooks_path = "/h/.cursor/hooks.json".into();
        c.setup.gemini.dir = "/h/.gemini".into();
        c.doctor.claude_json = "/h/.claude.json".into();
        c.setup.codex.config_path = "/h/.codex/config.toml".into();
        c
    }

    pub(crate) fn machine(settings: &str) -> Machine {
        let m = Machine::default();
        m.files
            .borrow_mut()
            .insert(SETTINGS.into(), settings.into());
        m
    }

    fn fix(m: &Machine, apply: bool) -> FixReport {
        let probes = Probes {
            fs: m,
            env: m,
            which: m,
        };
        fix_broken(&cfg(), &probes, m, apply, 3)
    }

    /// `fix` over every kind, as `rtok doctor --fix` runs.
    fn fix_all(m: &Machine, apply: bool, only: &[&str]) -> FixReport {
        let probes = Probes {
            fs: m,
            env: m,
            which: m,
        };
        fix_for(&cfg(), &probes, m, apply, 3, None, only)
    }

    pub(crate) fn put(m: &Machine, path: &str, body: &str) {
        m.files.borrow_mut().insert(path.into(), body.into());
    }

    pub(crate) fn text(m: &Machine, path: &str) -> String {
        m.files
            .borrow()
            .get(Path::new(path))
            .cloned()
            .unwrap_or_default()
    }

    /// The same valid hook in the user's and in the project's settings.
    pub(crate) const USER_DUP: &str = r#"{
  // mine
  "theme": "dark",
  "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/h/.claude/settings.json"}]}]}
}
"#;
    pub(crate) const PROJECT_DUP: &str = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/h/.claude/settings.json"}]}]}}"#;

    #[cfg(unix)] // POSIX paths and command words
    #[test]
    fn the_extra_copy_of_a_duplicate_hook_goes_and_the_kept_one_stays() {
        let m = machine(USER_DUP);
        put(&m, "/proj/.claude/settings.json", PROJECT_DUP);
        let r = fix_all(&m, true, &KINDS);
        assert_eq!(r.files.len(), 1, "{r:?}");
        assert_eq!(r.files[0].outcome, Outcome::Written);
        // The project's shared copy is kept; the user's copy loses its group and event too.
        assert_eq!(
            text(&m, SETTINGS),
            "{\n  // mine\n  \"theme\": \"dark\",\n  \"hooks\": {}\n}\n"
        );
        assert_eq!(text(&m, "/proj/.claude/settings.json"), PROJECT_DUP);
        assert_eq!(m.backups.borrow().len(), 1);
        assert_eq!(
            (r.left, render(&r, true).contains("1 entry removed, 0 left")),
            (0, true)
        );
        assert_eq!(
            render(&fix_all(&m, true, &KINDS), true),
            "nothing to remove\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn three_copies_in_one_file_leave_exactly_one() {
        let hook = r#"{"type": "command", "command": "/h/.claude/settings.json"}"#;
        let m = machine(&format!(
            r#"{{"hooks": {{"Stop": [{{"hooks": [{hook}, {hook}]}}, {{"hooks": [{hook}]}}]}}}}"#
        ));
        let r = fix_all(&m, true, &KINDS);
        assert_eq!(r.files[0].removed.len(), 2, "{r:?}");
        assert_eq!(text(&m, SETTINGS).matches("settings.json").count(), 1);
        assert_eq!(r.left, 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_hook_broken_and_duplicated_is_removed_once_and_only_what_was_selected() {
        let broken = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/h/gone/old.sh"}]}]}}"#;
        let m = machine(broken);
        put(&m, "/proj/.claude/settings.json", broken);
        let planned = fix_all(&m, false, &KINDS);
        assert_eq!(
            planned.files.iter().map(|f| f.removed.len()).sum::<usize>(),
            2
        );
        // Only the duplicate class: the extra copy goes, one (broken) copy stays.
        let only = fix_all(&m, true, &KINDS[1..2]);
        assert_eq!(only.files.len(), 1);
        assert!(!text(&m, SETTINGS).contains("old.sh"));
        assert_eq!(text(&m, "/proj/.claude/settings.json"), broken);
        // Then the broken class takes the last one: that is the user's call, not a duplicate rule.
        fix_all(&m, true, &KINDS[..1]);
        assert!(!text(&m, "/proj/.claude/settings.json").contains("old.sh"));
    }

    #[cfg(unix)]
    #[test]
    fn an_mcp_copy_goes_from_json_and_the_rest_of_the_file_is_byte_for_byte() {
        let m = Machine::default();
        put(
            &m,
            "/h/.claude.json",
            "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"a\": {\"command\": \"/bin/tool\", \"args\": [\"--x\"]},\n    \"b\": {\"command\": \"/bin/tool\", \"args\": [\"--x\"]},\n    \"c\": {\"command\": \"/bin/other\"}\n  }\n}\n",
        );
        let r = fix_all(&m, true, &KINDS);
        assert_eq!(r.files.len(), 1, "{r:?}");
        assert_eq!(
            text(&m, "/h/.claude.json"),
            "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"a\": {\"command\": \"/bin/tool\", \"args\": [\"--x\"]},\n    \"c\": {\"command\": \"/bin/other\"}\n  }\n}\n"
        );
        assert!(render(&planned_mcp(), false).contains("would remove mcp mcpServers.b"));
    }

    fn planned_mcp() -> FixReport {
        let m = Machine::default();
        put(
            &m,
            "/h/.claude.json",
            r#"{"mcpServers": {"a": {"command": "/bin/t"}, "b": {"command": "/bin/t"}}}"#,
        );
        fix_all(&m, false, &KINDS)
    }

    #[cfg(unix)]
    #[test]
    fn a_shadowed_server_is_removed_and_the_one_the_host_uses_stays() {
        let m = Machine::default();
        put(
            &m,
            "/h/.claude.json",
            r#"{"mcpServers": {"srv": {"command": "/bin/old"}}}"#,
        );
        let project = r#"{"mcpServers": {"srv": {"command": "/bin/new"}}}"#;
        put(&m, "/proj/.mcp.json", project);
        let r = fix_all(&m, true, &KINDS);
        assert_eq!(r.files.len(), 1, "{r:?}");
        assert_eq!(text(&m, "/h/.claude.json"), r#"{"mcpServers": {}}"#);
        assert_eq!(text(&m, "/proj/.mcp.json"), project);
    }

    #[cfg(unix)]
    #[test]
    fn a_toml_server_table_is_removed_and_every_other_byte_stays() {
        let m = Machine::default();
        let head = "# my codex config\nmodel = \"x\"\n\n[mcp_servers.a]\ncommand = \"/bin/tool\"\nargs = [\"--x\"]\n";
        let gone = "\n# the copy\n[mcp_servers.b]\ncommand = \"/bin/tool\"\nargs = [\"--x\"]\n";
        let tail = "\n[other]\nk = 1 # keep\n";
        put(&m, "/h/.codex/config.toml", &format!("{head}{gone}{tail}"));
        let r = fix_all(&m, true, &KINDS);
        assert_eq!(r.files[0].outcome, Outcome::Written, "{r:?}");
        assert_eq!(text(&m, "/h/.codex/config.toml"), format!("{head}{tail}"));
    }

    /// Answers read from a script; every screen shown is kept.
    struct Script {
        answers: std::vec::IntoIter<String>,
        screens: Vec<String>,
    }

    impl Script {
        fn new(answers: &[&str]) -> Self {
            Script {
                answers: answers
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .into_iter(),
                screens: Vec::new(),
            }
        }
    }

    impl Prompt for Script {
        fn ask(&mut self, screen: &str) -> Option<String> {
            self.screens.push(screen.into());
            self.answers.next()
        }
    }

    fn pick(m: &Machine, answers: &[&str]) -> (String, Script) {
        let probes = Probes {
            fs: m,
            env: m,
            which: m,
        };
        let o = Opts {
            keep: 3,
            agent: None,
            kinds: &KINDS,
        };
        let mut script = Script::new(answers);
        let (text, _) = interactive(&cfg(), &probes, m, &o, &mut script);
        (text, script)
    }

    pub(crate) const BROKEN: &str =
        r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/h/gone/old.sh"}]}]}}"#;

    #[cfg(unix)]
    #[test]
    fn a_shared_project_file_starts_unselected_and_the_rest_is_written_after_confirmation() {
        let m = machine(BROKEN);
        put(&m, "/proj/.claude/settings.json", BROKEN);
        let (text, script) = pick(&m, &["y", "y"]);
        assert!(
            script.screens[0].contains("[x] broken hook Stop"),
            "{}",
            script.screens[0]
        );
        assert!(script.screens[0].contains("(shared project file)"));
        assert!(script.screens[0].contains("[ ] broken hook Stop"));
        assert!(
            script.screens[1].contains("Write 1 change(s)? [y/N] "),
            "{}",
            script.screens[1]
        );
        assert!(
            script.screens[1].contains("--- a/"),
            "the diff is shown first"
        );
        assert!(text.contains("1 entry removed"), "{text}");
        assert!(!text_of(&m, SETTINGS).contains("old.sh"));
        assert_eq!(text_of(&m, "/proj/.claude/settings.json"), BROKEN);
    }

    fn text_of(m: &Machine, path: &str) -> String {
        text(m, path)
    }

    #[cfg(unix)]
    #[test]
    fn toggling_deselects_and_nothing_is_written_until_the_last_yes() {
        let m = machine(BROKEN);
        for answers in [
            &["1", "y"][..],
            &["q"],
            &[],
            &["y", "n", "q"],
            &["x", "9", "k 1", "q"],
        ] {
            let (text, _) = pick(&m, answers);
            assert!(
                text.starts_with("cancelled") || text.is_empty(),
                "{answers:?}: {text}"
            );
            assert_eq!(text_of(&m, SETTINGS), BROKEN, "{answers:?}");
        }
        assert!(m.backups.borrow().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn k_keeps_the_chosen_copy_and_removes_the_other() {
        let m = machine(USER_DUP);
        put(&m, "/proj/.claude/settings.json", PROJECT_DUP);
        // Item 1 is the user's extra copy. Keeping it turns the shared project copy into the
        // extra one, which starts unselected: toggle it on.
        let (text, script) = pick(&m, &["k 1", "1", "y", "y"]);
        assert!(
            script.screens[1].contains("[ ] extra hook Stop"),
            "{}",
            script.screens[1]
        );
        assert!(text.contains("1 entry removed"), "{text}");
        assert!(text_of(&m, SETTINGS).contains("/h/.claude/settings.json"));
        assert!(!text_of(&m, "/proj/.claude/settings.json").contains("Stop"));
    }

    #[test]
    fn a_kept_copy_that_is_not_the_users_cannot_be_swapped_out() {
        let copy = |keep, fixable, detail: &str| Problem {
            kind: "duplicate-hook",
            agent: "claude",
            source: format!("/f{keep}"),
            path: "p".into(),
            event: "Stop".into(),
            matcher: None,
            command: "c".into(),
            detail: detail.into(),
            fixable,
            group: Some(0),
            keep,
        };
        let mut plugin_kept = vec![
            copy(true, false, "runs 2 times; keep this copy (x)"),
            copy(false, true, "runs 2 times; an extra copy"),
        ];
        assert!(checklist::keep_copy(&mut plugin_kept, 1).is_err());
        let mut shadowed = vec![
            copy(true, true, "`a` is defined 2 times; the host uses this one"),
            copy(
                false,
                true,
                "`a` is defined 2 times; unused, overridden by another scope",
            ),
        ];
        assert!(checklist::keep_copy(&mut shadowed, 1).is_err());
        let mut broken = vec![Problem {
            kind: "broken-hook",
            ..copy(false, true, "gone")
        }];
        assert!(checklist::keep_copy(&mut broken, 0).is_err());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        /// Whatever the user types, a duplicate never loses its last copy and an unrelated
        /// hook never moves.
        #[cfg(unix)] // POSIX paths and command words
        #[test]
        fn no_answers_remove_the_last_copy_or_an_unrelated_hook(
            answers in prop::collection::vec(
                prop::sample::select(vec!["1", "2", "3", "k 1", "k 2", "k 3", "d", "y", "n", "q", "k 9", "?"]),
                0..14,
            )
        ) {
            let hook = r#"{"type": "command", "command": "/h/.claude/settings.json"}"#;
            let other = r#"{"type": "command", "command": "/h/.claude/other.sh"}"#;
            let m = machine(&format!(r#"{{"hooks": {{"Stop": [{{"hooks": [{hook}, {other}]}}]}}}}"#));
            put(&m, "/h/.claude/other.sh", "#!/bin/sh\n");
            let dup = format!(r#"{{"hooks": {{"Stop": [{{"hooks": [{hook}]}}]}}}}"#);
            put(&m, "/proj/.claude/settings.json", &dup);
            put(&m, "/proj/.claude/settings.local.json", &dup);
            pick(&m, &answers);
            let all: String = [SETTINGS, "/proj/.claude/settings.json", "/proj/.claude/settings.local.json"]
                .iter()
                .map(|f| text(&m, f))
                .collect();
            prop_assert!(all.matches("\"/h/.claude/settings.json\"").count() >= 1);
            prop_assert!(text(&m, SETTINGS).contains("other.sh"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_rtok_entry_stays_and_only_a_copy_under_another_name_goes() {
        let m = Machine::default();
        let own = r#"{"command": "rtok", "args": ["mcp"]}"#;
        put(
            &m,
            "/h/.claude.json",
            &format!(r#"{{"mcpServers": {{"rtok": {own}, "rtok-mcp": {own}}}}}"#),
        );
        let r = fix_all(&m, true, &KINDS);
        assert_eq!((r.files.len(), r.left), (1, 0), "{r:?}");
        assert!(render(&r, true).ends_with("1 entry removed, 0 left\n"));
        assert_eq!(
            text(&m, "/h/.claude.json"),
            format!(r#"{{"mcpServers": {{"rtok": {own}}}}}"#)
        );
        // Nothing fixable is left, so a second run has nothing to do.
        assert_eq!(
            render(&fix_all(&m, true, &KINDS), true),
            "nothing to remove\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn same_name_and_plugin_copies_of_rtok_are_information_that_fix_never_touches() {
        let m = Machine::default();
        let own = r#"{"command": "rtok", "args": ["mcp"]}"#;
        let files = [
            (
                "/h/.claude.json",
                format!(r#"{{"mcpServers": {{"rtok": {own}}}}}"#),
            ),
            (
                "/proj/.mcp.json",
                format!(r#"{{"mcpServers": {{"rtok": {own}}}}}"#),
            ),
            (
                "/h/.gemini/settings.json",
                format!(r#"{{"mcpServers": {{"rtok": {own}}}}}"#),
            ),
            (
                "/h/.gemini/extensions/rtok/gemini-extension.json",
                format!(r#"{{"mcpServers": {{"rtok": {own}}}}}"#),
            ),
        ];
        for (path, body) in &files {
            put(&m, path, body);
        }
        let probes = Probes {
            fs: &m,
            env: &m,
            which: &m,
        };
        let info = candidates(&cfg(), &probes, None);
        assert_eq!(info.iter().filter(|x| x.kind == "own-mcp").count(), 4);
        assert!(info.iter().all(|x| !removable(x, &KINDS)));
        let r = fix_all(&m, true, &KINDS);
        assert_eq!(
            (render(&r, true).as_str(), r.left),
            ("nothing to remove\n", 0)
        );
        for (path, body) in &files {
            assert_eq!(&text(&m, path), body);
        }
    }

    const TWO_HOOKS: &str = r#"{
  // keep this comment
  "theme": "dark",
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "/h/gone/old.sh" },
          { "type": "command", "command": "/h/.claude/settings.json" }
        ]
      }
    ]
  }
}
"#;

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn a_dry_run_writes_nothing_and_shows_the_diff() {
        let m = machine(TWO_HOOKS);
        let r = fix(&m, false);
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].outcome, Outcome::Planned);
        assert!(
            r.files[0]
                .diff
                .contains("-          { \"type\": \"command\", \"command\": \"/h/gone/old.sh\" },")
        );
        assert_eq!(m.files.borrow()[Path::new(SETTINGS)], TWO_HOOKS);
        assert!(m.backups.borrow().is_empty());
        assert_eq!(r.exit_code(), 0);
        let text = render(&r, false);
        assert!(text.starts_with("dry run: nothing is written."), "{text}");
        assert!(text.contains("would remove PreToolUse[Bash]"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn apply_backs_up_first_then_removes_only_the_broken_entry() {
        let m = machine(TWO_HOOKS);
        let r = fix(&m, true);
        assert_eq!(r.files[0].outcome, Outcome::Written);
        assert_eq!(m.backups.borrow().len(), 1);
        assert_eq!(r.files[0].backup.as_ref(), m.backups.borrow().first());
        let after = m.files.borrow()[Path::new(SETTINGS)].clone();
        assert_eq!(
            after,
            TWO_HOOKS.replace(
                "{ \"type\": \"command\", \"command\": \"/h/gone/old.sh\" },",
                ""
            )
        );
        assert_eq!(r.left, 0);
        assert!(render(&r, true).contains("1 entry removed, 0 left"));
        // Idempotent: the second run has nothing to do.
        assert_eq!(render(&fix(&m, true), true), "nothing to remove\n");
    }

    #[cfg(unix)]
    #[test]
    fn the_last_hook_takes_its_group_and_its_event_with_it() {
        let raw = r#"{
  "hooks": {
    "Stop": [ { "hooks": [ { "type": "command", "command": "/h/gone/a.sh" } ] } ],
    "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "/h/gone/b.sh" } ] } ]
  },
  "theme": "dark"
}
"#;
        let m = machine(raw);
        let r = fix(&m, true);
        assert_eq!(r.files[0].outcome, Outcome::Written);
        let after = m.files.borrow()[Path::new(SETTINGS)].clone();
        let doc = jsonc::parse(&after).unwrap();
        assert_eq!(doc["hooks"], serde_json::json!({}));
        assert_eq!(doc["theme"], "dark");
        assert_eq!(r.left, 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_changed_since_the_check_is_skipped_and_untouched() {
        let m = machine(TWO_HOOKS);
        let edited = TWO_HOOKS.replace("\"dark\"", "\"light\"");
        let planned = fix(&m, false);
        assert_eq!(planned.files[0].outcome, Outcome::Planned);
        // The apply run repeats the dry run's reads and then re-reads once before the write.
        let dry_reads = m.reads.replace(0);
        m.race_at.set(dry_reads + 1);
        *m.race.borrow_mut() = Some((SETTINGS.into(), edited.clone()));
        let r = fix(&m, true);
        assert_eq!(
            r.files[0].outcome,
            Outcome::Skipped("changed since the check".into())
        );
        assert_eq!(r.exit_code(), 1);
        assert!(m.backups.borrow().is_empty());
        assert_eq!(m.files.borrow()[Path::new(SETTINGS)], edited);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_backup_or_write_leaves_the_file_and_exits_1() {
        let mut m = machine(TWO_HOOKS);
        m.fail_backup = true;
        let r = fix(&m, true);
        assert!(
            matches!(&r.files[0].outcome, Outcome::Failed(w) if w.starts_with("backup failed"))
        );
        assert_eq!(r.exit_code(), 1);
        assert_eq!(m.files.borrow()[Path::new(SETTINGS)], TWO_HOOKS);

        let mut m = machine(TWO_HOOKS);
        m.fail_write = true;
        let r = fix(&m, true);
        assert!(matches!(&r.files[0].outcome, Outcome::Failed(w) if w.starts_with("write failed")));
        assert_eq!(r.exit_code(), 1);
        assert_eq!(m.files.borrow()[Path::new(SETTINGS)], TWO_HOOKS);
    }

    #[cfg(unix)]
    #[test]
    fn rtok_own_entries_and_non_broken_hooks_are_never_removed() {
        let raw = r#"{"hooks":{"PreToolUse":[{"hooks":[
{"type":"command","command":"/h/gone/rtok hook pre-tool-use"},
{"type":"command","command":"/h/gone/rtok-hook"},
{"type":"command","command":"/h/.claude/settings.json"},
{"type":"command","command":"echo hi"}
]}]}}"#;
        let m = machine(raw);
        let r = fix(&m, true);
        assert!(r.files.is_empty(), "{r:?}");
        assert_eq!(r.refused.len(), 2);
        assert_eq!(m.files.borrow()[Path::new(SETTINGS)], raw);
        assert_eq!(r.exit_code(), 0);
        assert!(render(&r, true).contains("not removed:"));
    }

    #[test]
    fn is_rtok_own_matches_the_binary_and_the_hook_shim_only() {
        assert!(is_rtok_own("/usr/local/bin/rtok hook pre-tool-use"));
        assert!(is_rtok_own("'/x y/rtok-hook' run"));
        assert!(!is_rtok_own("/h/scripts/rtokish.sh"));
    }

    #[test]
    fn parse_path_reads_nested_and_flat_key_paths() {
        assert_eq!(
            parse_path("hooks.PreToolUse[1].hooks[2]"),
            Some(("PreToolUse", 1, Some(2)))
        );
        assert_eq!(parse_path("hooks.Stop[0]"), Some(("Stop", 0, None)));
        assert_eq!(parse_path("hooks.Stop"), None);
        assert_eq!(parse_path("other.Stop[0]"), None);
        assert_eq!(parse_path("hooks.Stop[x]"), None);
    }

    // ---- property: the bytes outside the removed entry are unchanged ----

    /// Filler placed between JSON tokens: whitespace and both comment forms.
    const FILLERS: [&str; 7] = ["", " ", "\n", "\n    ", "// note\n", "/* é — ü */", "\t"];

    #[derive(Debug, Clone)]
    struct Spec {
        /// Per event: per group: `Some(hooks)` nested with that many hooks, `None` flat.
        events: Vec<Vec<Option<usize>>>,
        fillers: Vec<usize>,
        pick: (usize, usize, usize),
    }

    fn spec() -> impl Strategy<Value = Spec> {
        (
            prop::collection::vec(
                prop::collection::vec(prop::option::of(1usize..4), 1..4),
                1..4,
            ),
            prop::collection::vec(0..FILLERS.len(), 64..65),
            (0usize..8, 0usize..8, 0usize..8),
        )
            .prop_map(|(events, fillers, pick)| Spec {
                events,
                fillers,
                pick,
            })
    }

    const EVENTS: [&str; 3] = ["PreToolUse", "PostToolUse", "Stop"];

    /// The document, the key path to remove, and the command that path holds.
    fn build(s: &Spec) -> (String, String, String) {
        let mut f = s.fillers.iter().cycle().map(|i| FILLERS[*i]);
        let mut next = move || f.next().unwrap();
        let (pe, pg, ph) = (
            s.pick.0 % s.events.len(),
            s.pick.1 % s.events[s.pick.0 % s.events.len()].len(),
            s.pick.2,
        );
        let mut path = String::new();
        let mut command = String::new();
        let mut out = format!(
            "{}{{{}\"theme\"{}:{}\"dark\",{}\"hooks\"{}:{}{{{}",
            next(),
            next(),
            next(),
            next(),
            next(),
            next(),
            next(),
            next()
        );
        for (e, groups) in s.events.iter().enumerate() {
            if e > 0 {
                out += &format!(",{}", next());
            }
            out += &format!("\"{}\"{}:{}[{}", EVENTS[e], next(), next(), next());
            for (g, group) in groups.iter().enumerate() {
                if g > 0 {
                    out += &format!(",{}", next());
                }
                let cmd = |h: usize| format!("/gone/{e}-{g}-{h}.sh");
                match group {
                    None => {
                        out += &format!(
                            "{{\"type\":{}\"command\",{}\"command\":\"{}\"}}",
                            next(),
                            next(),
                            cmd(0)
                        );
                        if (e, g) == (pe, pg) {
                            path = format!("hooks.{}[{g}]", EVENTS[e]);
                            command = cmd(0);
                        }
                    }
                    Some(n) => {
                        out += &format!(
                            "{{{}\"matcher\":{}\"Bash\",{}\"hooks\"{}:{}[{}",
                            next(),
                            next(),
                            next(),
                            next(),
                            next(),
                            next()
                        );
                        for h in 0..*n {
                            if h > 0 {
                                out += &format!(",{}", next());
                            }
                            out += &format!(
                                "{{\"type\":{}\"command\",\"command\":\"{}\"}}",
                                next(),
                                cmd(h)
                            );
                        }
                        out += &format!("{}]{}}}", next(), next());
                        if (e, g) == (pe, pg) {
                            let h = ph % n;
                            path = format!("hooks.{}[{g}].hooks[{h}]", EVENTS[e]);
                            command = cmd(h);
                        }
                    }
                }
            }
            out += &format!("{}]", next());
        }
        out += &format!("{}}}{}}}{}", next(), next(), next());
        (out, path, command)
    }

    /// `after` is `before` with one contiguous region removed.
    fn only_a_region_is_gone(before: &str, after: &str) -> bool {
        let lcp = before
            .bytes()
            .zip(after.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        before.len() >= after.len() && before.as_bytes().ends_with(&after.as_bytes()[lcp..])
    }

    proptest! {
        #[test]
        fn removing_one_hook_changes_only_that_hook(s in spec()) {
            let (raw, path, command) = build(&s);
            prop_assume!(!path.is_empty());
            let file = Path::new("settings.json");
            prop_assert!(jsonc::parse(&raw).is_ok(), "generator made invalid JSONC: {raw}");
            let body = remove_entries(&raw, file, &[path.as_str()]).expect("entry located");
            prop_assert!(only_a_region_is_gone(&raw, &body), "before: {raw}\nafter: {body}");
            let before = hook_list(&raw).unwrap();
            let after = hook_list(&body).expect("still parses");
            let mut expect = before.clone();
            let at = expect.iter().position(|h| h.2 == command).unwrap();
            expect.remove(at);
            prop_assert_eq!(after, expect);
            let quoted = format!("\"{command}\"");
            prop_assert!(!body.contains(&quoted));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_toml_hook_is_reported_and_never_edited() {
        let toml = "[[hooks]]\nevent = \"Stop\"\ncommand = \"/h/gone.sh\"\n";
        let m = machine("{}");
        m.files
            .borrow_mut()
            .insert("/h/.kimi-code/config.toml".into(), toml.into());
        let mut c = cfg();
        c.setup.kimi.config_path = "/h/.kimi-code/config.toml".into();
        let probes = Probes {
            fs: &m,
            env: &m,
            which: &m,
        };
        let r = fix_broken(&c, &probes, &m, true, 3);
        assert!(r.files.is_empty(), "{r:?}");
        assert_eq!(r.refused.len(), 1);
        assert_eq!(r.refused[0].1, "TOML hook files are not edited yet");
        assert_eq!(
            m.files.borrow()[Path::new("/h/.kimi-code/config.toml")],
            toml
        );
        assert!(m.backups.borrow().is_empty());
    }
}

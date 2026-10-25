// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Broken hooks (T331.1): a hook whose command leads nowhere. Every hook of Claude Code's
//! settings files is resolved without running it: the command is split like a POSIX shell,
//! variables are expanded, and the target is the first word, the script of a known
//! interpreter, or a program on `PATH`. A target that is not there is `broken-hook`; a script
//! that exists but is not executable is `suspect-hook`; a command that cannot be judged
//! statically (substitution, operators, unknown variables) is `unverified-hook`. Only the
//! first class will ever be offered for removal (T331.5). Nothing here edits a file, and
//! nothing reaches the machine except through [`super::probe`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use super::probe::{Env, Fs, PathKind, Which};
use crate::config::Config;

/// One finding of a doctor config check. The list is shared by every T331 detector.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Problem {
    /// `broken-hook`, `suspect-hook`, `unverified-hook`, `duplicate-hook`, `duplicate-mcp`,
    /// `conflicting-mcp`, `own-mcp`, `stale-plugin`, `unreadable-config` or `untrusted-folder`.
    pub kind: &'static str,
    pub agent: &'static str,
    /// The config file the entry lives in.
    pub source: String,
    /// The key path of the entry inside `source`.
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    /// The command as written, never expanded.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    pub detail: String,
    /// Whether `doctor --fix` may remove it (T331.5): only `broken-hook`.
    pub fixable: bool,
    /// Copies of one duplicate share a `group`; `keep` marks the copy to keep (T331.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep: bool,
}

/// What a hook command may rely on besides the machine.
#[derive(Clone, Copy)]
struct Scope<'a> {
    project: &'a Path,
    /// `${CLAUDE_PLUGIN_ROOT}`, known only inside a plugin's own `hooks.json`.
    plugin_root: Option<&'a Path>,
    /// Whether a relative path resolves against `project`. Only Claude Code documents that;
    /// for any other host a relative path is not judged, since guessing its base would offer
    /// a working hook for removal.
    relative_ok: bool,
}

/// The machine, as the checks see it.
pub struct Probes<'a> {
    pub fs: &'a dyn Fs,
    pub env: &'a dyn Env,
    pub which: &'a dyn Which,
}

#[derive(Debug, PartialEq)]
pub(super) struct Entry {
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
    pub key: String,
}

#[derive(Debug, PartialEq)]
enum Verdict {
    Ok,
    Broken(String),
    Suspect(String),
    Unverified(String),
}

/// Programs that run a script given as an argument.
const INTERPRETERS: &[&str] = &[
    "bash", "sh", "zsh", "node", "python", "python3", "deno", "bun", "ruby", "pwsh", "tsx",
];

/// Shell builtins a hook may name directly; they need no file.
const BUILTINS: &[&str] = &[
    ":", ".", "[", "cd", "command", "echo", "exit", "export", "false", "printf", "pwd", "read",
    "set", "source", "test", "true", "type", "unset",
];

/// Which copy of a duplicate hook is kept, best first (T331): the one a plugin owns, so plugin
/// updates keep working; a project `settings.json`, which teammates share; the user's own file;
/// then a `settings.local.json`, and among equals the first in load order.
pub(super) const RANK_PLUGIN: u8 = 0;
const RANK_PROJECT: u8 = 1;
const RANK_USER: u8 = 2;
const RANK_LOCAL: u8 = 3;

/// The Claude Code settings files in load order (user, user local, project, project local), each
/// with its keep rank.
fn sources(cfg: &Config, project: Option<&Path>) -> Vec<(PathBuf, u8)> {
    let user = cfg.doctor.settings_path.clone();
    let mut out = vec![
        (user.clone(), RANK_USER),
        (user.with_file_name("settings.local.json"), RANK_LOCAL),
    ];
    if let Some(p) = project {
        let dir = p.join(".claude");
        out.push((dir.join("settings.json"), RANK_PROJECT));
        out.push((dir.join("settings.local.json"), RANK_LOCAL));
    }
    // No file twice: the working directory may be `$HOME`.
    let mut seen = BTreeSet::new();
    out.retain(|(p, _)| seen.insert(p.clone()));
    out
}

/// Every command hook of a settings document: `hooks.<Event>[].{matcher, hooks[].command}`,
/// plus the flat `hooks.<Event>[].command` shape `doctor`'s hook count also accepts.
pub(super) fn entries(doc: &Value) -> Vec<Entry> {
    let mut out = Vec::new();
    let Some(events) = doc.get("hooks").and_then(Value::as_object) else {
        return out;
    };
    for (event, groups) in events {
        for (gi, group) in groups.as_array().into_iter().flatten().enumerate() {
            let matcher = group
                .get("matcher")
                .and_then(Value::as_str)
                .map(String::from);
            let mut push = |hook: &Value, key: String| {
                let is_command = hook
                    .get("type")
                    .and_then(Value::as_str)
                    .is_none_or(|t| t == "command");
                if let (true, Some(command)) =
                    (is_command, hook.get("command").and_then(Value::as_str))
                {
                    out.push(Entry {
                        event: event.clone(),
                        matcher: matcher.clone(),
                        command: command.to_string(),
                        key,
                    });
                }
            };
            match group.get("hooks").and_then(Value::as_array) {
                Some(hooks) => {
                    for (hi, hook) in hooks.iter().enumerate() {
                        push(hook, format!("hooks.{event}[{gi}].hooks[{hi}]"));
                    }
                }
                None => push(group, format!("hooks.{event}[{gi}]")),
            }
        }
    }
    out
}

/// Syntax a static check cannot judge: substitution, operators, subshells, line breaks.
fn shell_syntax(raw: &str) -> Option<&'static str> {
    if raw.contains("$(") || raw.contains('`') {
        Some("command substitution")
    } else if raw.contains(['|', '&', ';', '<', '>', '(', ')', '\n']) {
        Some("shell operators")
    } else if raw.split_whitespace().next() == Some("eval") {
        Some("eval")
    } else {
        None
    }
}

/// `word` with `~`, `$NAME` and `${NAME}` expanded. `Err` names what could not be: an unset
/// variable, or one only a plugin install defines.
fn expand(word: &str, p: &Probes, scope: &Scope) -> Result<String, String> {
    let word = &if p.env.windows() {
        percent_vars(word)
    } else {
        word.to_string()
    };
    let mut out = String::new();
    let rest = if word == "~" || word.starts_with("~/") {
        let home = p.env.home().ok_or("HOME")?;
        out.push_str(&home.to_string_lossy());
        &word[1..]
    } else {
        word
    };
    let mut chars = rest.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        let tail = &rest[i + 1..];
        let (name, len) = match tail.strip_prefix('{') {
            Some(inner) => {
                let end = inner.find('}').ok_or("$")?;
                (&inner[..end], end + 2)
            }
            None => {
                let end = tail
                    .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                    .unwrap_or(tail.len());
                (&tail[..end], end)
            }
        };
        if name.is_empty() || name.starts_with(|ch: char| ch.is_ascii_digit()) {
            return Err("$".into());
        }
        let value = match name {
            "HOME" => p.env.home().map(|h| h.to_string_lossy().into_owned()),
            "CLAUDE_PROJECT_DIR" => Some(scope.project.to_string_lossy().into_owned()),
            "CLAUDE_PLUGIN_ROOT" => scope.plugin_root.map(|r| r.to_string_lossy().into_owned()),
            other => p.env.var(other),
        };
        out.push_str(&value.ok_or_else(|| name.to_string())?);
        for _ in 0..len {
            chars.next();
        }
    }
    Ok(out)
}

/// `raw` split into words by the host shell's rules. POSIX by default; with `win`, `"` groups,
/// `'` and `\` are plain characters except a `\` run right before a `"` (the
/// `CommandLineToArgvW` rule), so an unquoted `C:\tools\x.cmd` stays one intact word.
fn split_words(raw: &str, win: bool) -> Option<Vec<String>> {
    if !win {
        return shlex::split(raw);
    }
    let (mut words, mut cur, mut quoted, mut open) = (Vec::new(), String::new(), false, false);
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        open |= !c.is_whitespace() || quoted;
        match c {
            '\\' => {
                let mut run = 1;
                while chars.next_if_eq(&'\\').is_some() {
                    run += 1;
                }
                let escapes = chars.peek() == Some(&'"');
                cur.extend(std::iter::repeat_n(
                    '\\',
                    if escapes { run / 2 } else { run },
                ));
                if escapes && run % 2 == 1 {
                    cur.push('"');
                    chars.next();
                }
            }
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if std::mem::take(&mut open) {
                    words.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if open {
        words.push(cur);
    }
    (!quoted).then_some(words)
}

/// `raw` without its `"..."` parts: a `(` in `C:\Program Files (x86)` is not shell syntax.
fn outside_quotes(raw: &str) -> String {
    let mut quoted = false;
    raw.chars()
        .filter(|&c| {
            quoted ^= c == '"';
            !quoted && c != '"'
        })
        .collect()
}

/// `%NAME%` as `${NAME}`, so the one expansion below serves `cmd` variables too.
fn percent_vars(word: &str) -> String {
    let mut out = String::new();
    let mut rest = word;
    while let Some(i) = rest.find('%') {
        let (head, tail) = rest.split_at(i);
        out.push_str(head);
        match tail[1..].split_once('%') {
            Some((name, after))
                if !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                out.push_str(&format!("${{{name}}}"));
                rest = after;
            }
            _ => {
                out.push('%');
                rest = &tail[1..];
            }
        }
    }
    out + rest
}

/// `C:\x`, `C:/x` or `\\server\share`: absolute by Windows rules whatever OS runs the check.
fn is_abs_windows(t: &str) -> bool {
    let b = t.as_bytes();
    t.starts_with("\\\\")
        || (b.len() > 2
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'\\' | b'/'))
}

fn base_name(program: &str) -> String {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    name.strip_suffix(".exe").unwrap_or(name).to_lowercase()
}

/// What a command runs: the path or program to look up, and whether the shell would exec it
/// directly (so its own exec bit matters) rather than an interpreter reading it.
fn target(words: &[String], win: bool) -> Result<(String, bool), Verdict> {
    let program = &words[0];
    let base = base_name(program);
    let args = &words[1..];
    let first_arg = |args: &[String]| args.iter().find(|a| !a.starts_with('-')).cloned();
    let flag = |names: &[&str]| {
        args.iter()
            .position(|a| names.contains(&a.to_lowercase().as_str()))
    };
    // `cmd /c <command>` runs the command itself; PowerShell runs the script after `-File`.
    if win && base == "cmd" {
        return match flag(&["/c", "/k"]) {
            Some(i) if i + 1 < args.len() => target(&args[i + 1..], win),
            _ => Err(Verdict::Unverified("no command after /c".into())),
        };
    }
    if win && (base == "powershell" || base == "pwsh") {
        return match flag(&["-file", "-f"]).and_then(|i| args.get(i + 1)) {
            Some(script) => Ok((script.clone(), false)),
            None => Err(Verdict::Unverified("an inline PowerShell command".into())),
        };
    }
    if INTERPRETERS.contains(&base.as_str()) {
        if args
            .iter()
            .any(|a| matches!(a.as_str(), "-c" | "-e" | "--eval" | "-p" | "--print"))
        {
            return Err(Verdict::Unverified("an inline script, not a file".into()));
        }
        let args: Vec<String> = match base.as_str() {
            "deno" | "bun" => args.iter().filter(|a| *a != "run").cloned().collect(),
            _ => args.to_vec(),
        };
        return first_arg(&args)
            .map(|script| (script, false))
            .ok_or_else(|| Verdict::Unverified("no script argument".into()));
    }
    // Launchers that hand the rest of the line to another program.
    let rest_after = |word: &str| {
        args.iter()
            .skip_while(|a| *a != word)
            .skip(1)
            .cloned()
            .collect::<Vec<_>>()
    };
    match (base.as_str(), first_arg(args).as_deref()) {
        ("npx", Some("tsx")) => target(
            &[&["tsx".to_string()][..], &rest_after("tsx")].concat(),
            win,
        ),
        ("uv", Some("run")) => {
            let rest = rest_after("run");
            if rest.is_empty() {
                return Err(Verdict::Unverified("no script argument".into()));
            }
            target(&rest, win).map(|(t, _)| (t, false))
        }
        _ => Ok((program.clone(), true)),
    }
}

/// `command` as the shell would see it: words split, variables and `~` expanded, a `./` path
/// made absolute where the host documents its base. What cannot be expanded or split is kept as
/// written with its spacing collapsed, so two copies compare equal only when they really match.
fn normalize(command: &str, p: &Probes, scope: &Scope) -> String {
    let raw = command.trim();
    let win = p.env.windows();
    let words = match (shell_syntax(&syntax_text(raw, win)), split_words(raw, win)) {
        (None, Some(words)) => words,
        _ => return raw.split_whitespace().collect::<Vec<_>>().join(" "),
    };
    words
        .iter()
        .map(|w| {
            let w = expand(w, p, scope).unwrap_or_else(|_| w.clone());
            match w
                .strip_prefix("./")
                .or_else(|| w.strip_prefix(".\\").filter(|_| win))
            {
                Some(rest) if scope.relative_ok => join_project(scope.project, rest, win),
                _ => w,
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The text `shell_syntax` judges: on Windows without the quoted parts.
fn syntax_text(raw: &str, win: bool) -> String {
    if win {
        outside_quotes(raw)
    } else {
        raw.to_string()
    }
}

/// `rest` under the project directory, with the separator of the rules in force.
fn join_project(project: &Path, rest: &str, win: bool) -> String {
    if win {
        format!("{}\\{}", project.display(), rest.replace('/', "\\"))
    } else {
        project.join(rest).display().to_string()
    }
}

fn path_like(t: &str) -> bool {
    t.contains(['/', '\\']) || t.starts_with(['~', '.'])
}

fn classify(command: &str, p: &Probes, scope: &Scope) -> Verdict {
    let raw = command.trim();
    if raw.is_empty() {
        return Verdict::Unverified("empty command".into());
    }
    // POSIX word splitting reads `\` as an escape, so Windows commands get their own rules.
    let win = p.env.windows();
    if let Some(why) = shell_syntax(&syntax_text(raw, win)) {
        return Verdict::Unverified(format!("{why}: cannot be checked without running it"));
    }
    let Some(words) = split_words(raw, win) else {
        return Verdict::Unverified("unbalanced quotes".into());
    };
    let skip = words
        .iter()
        .take_while(|w| {
            !win && w.split_once('=').is_some_and(|(k, _)| {
                !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
        .count();
    if skip == words.len() {
        return Verdict::Unverified("only variable assignments".into());
    }
    let mut expanded = Vec::new();
    for w in &words[skip..] {
        match expand(w, p, scope) {
            Ok(x) => expanded.push(x),
            Err(name) if name == "CLAUDE_PLUGIN_ROOT" => {
                return Verdict::Unverified(
                    "`CLAUDE_PLUGIN_ROOT` is only known inside a plugin's own hooks.json".into(),
                );
            }
            Err(name) => return Verdict::Unverified(format!("`{name}` is not set here")),
        }
    }
    let (target, direct) = match target(&expanded, win) {
        Ok(t) => t,
        Err(v) => return v,
    };
    if win {
        return classify_windows(&target, p, scope);
    }
    if !path_like(&target) && direct {
        return if BUILTINS.contains(&target.as_str()) || p.which.find(&target).is_some() {
            Verdict::Ok
        } else {
            Verdict::Broken(format!("`{target}` not on PATH"))
        };
    }
    let path = if Path::new(&target).is_absolute() {
        PathBuf::from(&target)
    } else if scope.relative_ok {
        scope
            .project
            .join(target.strip_prefix("./").unwrap_or(&target))
    } else {
        return Verdict::Unverified(
            "relative path: this host's base directory is not known".into(),
        );
    };
    if let Some(volume) = unmounted_volume(&path, p.fs) {
        return Verdict::Broken(format!("path is on {volume}, which is not mounted"));
    }
    verdict(&path, p.fs.kind(&path), direct)
}

/// What `kind` of the file at `path` means for a hook that runs it. `exec_bit` is whether the
/// shell execs it directly, so its exec bit matters (Windows has none).
fn verdict(path: &Path, kind: PathKind, exec_bit: bool) -> Verdict {
    match kind {
        PathKind::Missing => Verdict::Broken(format!("file not found: {}", path.display())),
        PathKind::DanglingSymlink(to) => {
            Verdict::Broken(format!("dangling symlink to {}", to.display()))
        }
        PathKind::Dir => {
            Verdict::Broken(format!("{} is a directory, not a script", path.display()))
        }
        PathKind::File { executable: false } if exec_bit => Verdict::Suspect(format!(
            "{} is not executable: run `chmod +x`",
            path.display()
        )),
        PathKind::File { .. } => Verdict::Ok,
    }
}

/// Programs `cmd` runs without a file, besides the POSIX [`BUILTINS`] the Git Bash hooks share.
const CMD_BUILTINS: &[&str] = &[
    "call", "copy", "del", "dir", "md", "mkdir", "move", "rd", "ren", "rem", "start", "title",
    "ver",
];

/// Where a hook's target is on Windows (T331.9): a path as written, a bare name on `PATH`;
/// each tried as is and with every `PATHEXT` extension, as `cmd` does. The first file wins.
fn classify_windows(target: &str, p: &Probes, scope: &Scope) -> Verdict {
    let lower = target.to_lowercase();
    let bare = !path_like(target);
    if bare && (BUILTINS.contains(&lower.as_str()) || CMD_BUILTINS.contains(&lower.as_str())) {
        return Verdict::Ok;
    }
    let exts: Vec<String> = p
        .env
        .var("PATHEXT")
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|e| !e.is_empty())
        .map(str::to_string)
        .collect();
    let has_ext = exts.iter().any(|e| lower.ends_with(&e.to_lowercase()));
    let bases: Vec<String> = if bare {
        let path = p.env.var("PATH").unwrap_or_default();
        path.split(';')
            .filter(|d| !d.is_empty())
            .map(|d| format!("{}\\{target}", d.trim_end_matches('\\')))
            .collect()
    } else if is_abs_windows(target) {
        vec![target.to_string()]
    } else if scope.relative_ok {
        let rel = target
            .strip_prefix("./")
            .or_else(|| target.strip_prefix(".\\"));
        vec![join_project(scope.project, rel.unwrap_or(target), true)]
    } else {
        return Verdict::Unverified(
            "relative path: this host's base directory is not known".into(),
        );
    };
    for base in &bases {
        let tries = std::iter::once(String::new()).chain(exts.iter().filter(|_| !has_ext).cloned());
        for ext in tries {
            let path = PathBuf::from(format!("{base}{ext}"));
            if let kind @ PathKind::File { .. } = p.fs.kind(&path) {
                return verdict(&path, kind, false);
            }
        }
    }
    match bases.first() {
        Some(base) if !bare => verdict(Path::new(base), p.fs.kind(Path::new(base)), false),
        _ => Verdict::Broken(format!("`{target}` not on PATH")),
    }
}

/// `/Volumes/X` when `path` sits on a macOS volume that is not mounted. `/Volumes` exists only
/// on macOS; elsewhere such a path is an ordinary path and the missing-file rule covers it.
#[cfg(target_os = "macos")]
fn unmounted_volume(path: &Path, fs: &dyn Fs) -> Option<String> {
    let mut parts = path.components();
    let root = parts.nth(1)?; // the component after `/`
    let volume = parts.next()?;
    if root.as_os_str() != "Volumes" || path.parent().is_none() {
        return None;
    }
    let mount = Path::new("/Volumes").join(volume.as_os_str());
    (fs.kind(&mount) == PathKind::Missing).then(|| mount.display().to_string())
}

#[cfg(not(target_os = "macos"))]
fn unmounted_volume(_path: &Path, _fs: &dyn Fs) -> Option<String> {
    None
}

/// [`check`] limited to one host's findings (an id of [`crate::agents::HOSTS`]); `None` is all.
pub fn check_for(cfg: &Config, p: &Probes, agent: Option<&str>) -> Vec<Problem> {
    let mut found = check(cfg, p);
    found.retain(|x| agent.is_none_or(|a| x.agent == a));
    found
}

/// `id` as a host id, or an error naming the valid ones.
pub fn host_id(id: &str) -> Result<&'static str, String> {
    crate::agents::HOSTS
        .iter()
        .find(|h| **h == id)
        .copied()
        .ok_or_else(|| {
            format!(
                "unknown host `{id}`; valid hosts: {}",
                crate::agents::HOSTS.join(", ")
            )
        })
}

/// Check every hook of Claude Code's settings files and enabled plugins, and of the JSON and
/// TOML config files the other hosts' installers write. A file that does not exist is skipped; one
/// that cannot be read or parsed is reported and left alone.
pub fn check(cfg: &Config, p: &Probes) -> Vec<Problem> {
    check_with_plugins(cfg, p).0
}

/// [`check`], and the install directory of each enabled Claude plugin it found (one per plugin
/// id, keyed by id), for the checks that read other files of the plugin.
pub fn check_with_plugins(cfg: &Config, p: &Probes) -> (Vec<Problem>, Vec<(String, PathBuf)>) {
    let project = p.env.cwd().unwrap_or_default();
    let claude = Scope {
        project: &project,
        plugin_root: None,
        relative_ok: true,
    };
    let other = Scope {
        relative_ok: false,
        ..claude
    };
    let mut acc = Acc::default();
    let mut seen = BTreeSet::new();
    let mut enabled = BTreeSet::new();
    let user_sources = sources(
        cfg,
        Some(project.as_path()).filter(|d| !d.as_os_str().is_empty()),
    );
    for (source, rank) in &user_sources {
        // The working directory may reach `$HOME` through a symlink; the file is still one file.
        if !seen.insert(p.fs.canonical(source)) {
            continue;
        }
        if let Some(doc) = scan(
            p,
            "claude",
            source,
            (*rank, Format::Json),
            &claude,
            &mut acc,
        ) {
            enabled.extend(enabled_plugins(&doc));
        }
    }
    for (agent, source, format) in host_sources(cfg) {
        if seen.insert(p.fs.canonical(&source)) {
            scan(p, agent, &source, (RANK_USER, format), &other, &mut acc);
        }
    }
    plugins(cfg, p, &enabled, &claude, &mut acc);
    host_plugin_links(cfg, &mut acc);
    let dupes = super::dupes::find(&acc.seen);
    acc.problems.extend(dupes);
    (acc.problems, acc.plugin_dirs)
}

/// What one pass over the config files collects: the findings, and every hook seen (the input
/// of the duplicate check).
#[derive(Default)]
struct Acc {
    problems: Vec<Problem>,
    seen: Vec<Seen>,
    plugin_dirs: Vec<(String, PathBuf)>,
}

/// One hook entry as found, whether or not it is a problem.
pub(super) struct Seen {
    pub agent: &'static str,
    pub source: String,
    pub path: String,
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
    /// The command with variables expanded and quotes and spacing removed: two entries that
    /// differ only there run the same thing.
    pub normal: String,
    pub rank: u8,
    /// Whether `--fix` may edit the file the entry lives in: not a plugin's, and JSON.
    pub editable: bool,
}

/// The hosts whose hooks live in a `config.toml`, each shape documented by the host (see
/// [`toml_entries`]). Another host's TOML is not guessed at.
const TOML_HOSTS: &[&str] = &["kimi", "codewhale", "codex"];

/// The config files every host but Claude Code keeps hooks in, as its installer names them, once
/// each (Cursor's variants share theirs): JSON, and the TOML of [`TOML_HOSTS`].
fn host_sources(cfg: &Config) -> Vec<(&'static str, PathBuf, Format)> {
    let mut out: Vec<(&'static str, PathBuf, Format)> = Vec::new();
    for agent in crate::agents::HOSTS
        .iter()
        .filter(|id| **id != "claude")
        .filter_map(|id| crate::agents::host(id))
    {
        for v in agent.variants() {
            for path in agent.files(cfg, v.kind) {
                let ext = path.extension().and_then(|e| e.to_str());
                let format = match ext {
                    Some("json" | "jsonc") => Format::Json,
                    Some("toml") if TOML_HOSTS.contains(&agent.id()) => Format::Toml,
                    _ => continue,
                };
                if !out.iter().any(|(_, seen, _)| *seen == path) {
                    out.push((agent.id(), path, format));
                }
            }
        }
    }
    out
}

/// `enabledPlugins` of a settings document: the ids switched on.
fn enabled_plugins(doc: &Value) -> impl Iterator<Item = String> + '_ {
    doc.get("enabledPlugins")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, on)| on.as_bool() == Some(true))
        .map(|(id, _)| id.clone())
}

/// The hooks of every enabled Claude plugin, with `${CLAUDE_PLUGIN_ROOT}` resolved against its
/// install directory. A plugin that is enabled but whose directory is gone is reported as
/// stale. These files belong to the plugin, so nothing found in them is ever fixable.
fn plugins(cfg: &Config, p: &Probes, enabled: &BTreeSet<String>, claude: &Scope, acc: &mut Acc) {
    let index = crate::agents::claude::config_dir(cfg).join("plugins/installed_plugins.json");
    let Some(root) =
        p.fs.read(&index)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
    else {
        return;
    };
    let Some(plugins) = root.get("plugins").and_then(Value::as_object) else {
        return;
    };
    for id in plugins.keys().filter(|id| enabled.contains(*id)) {
        for (i, dir) in super::plugin_install_paths(&root, id).iter().enumerate() {
            let dir = PathBuf::from(dir);
            if p.fs.kind(&dir) != PathKind::Dir {
                acc.problems.push(Problem {
                    path: format!("plugins.{id}[{i}]"),
                    detail: format!(
                        "`{id}` is enabled but its install directory {} is gone",
                        dir.display()
                    ),
                    ..problem("stale-plugin", "claude", &index)
                });
                continue;
            }
            if !acc.plugin_dirs.iter().any(|(known, _)| known == id) {
                acc.plugin_dirs.push((id.clone(), dir.clone()));
            }
            let scope = Scope {
                plugin_root: Some(&dir),
                ..*claude
            };
            scan(
                p,
                "claude",
                &dir.join("hooks/hooks.json"),
                (RANK_PLUGIN, Format::Json),
                &scope,
                acc,
            );
        }
    }
}

/// Each host plugin rtok installed by link: one whose target is gone loads nothing, and one
/// into a versioned ketch store dangles at the next upgrade (T925). Unix only: a Windows
/// install is a copy, and Cursor's differs from the source on purpose. The link itself is
/// read through [`PluginLink`](rtok_agent_sdk::PluginLink), not [`Probes`].
fn host_plugin_links(cfg: &Config, acc: &mut Acc) {
    if !cfg!(unix) {
        return;
    }
    let mut seen = BTreeSet::new();
    for (agent, plugin) in crate::agents::plugin::all() {
        let dest = plugin.path(cfg);
        if !seen.insert(dest.clone()) {
            continue;
        }
        if let Some(detail) = plugin.problem(cfg) {
            acc.problems.push(Problem {
                detail: format!("{detail}; `rtok agents install {agent}` replaces it"),
                ..problem("stale-plugin", agent, &dest)
            });
        }
    }
}

/// How a hook file is written.
#[derive(Clone, Copy, PartialEq)]
enum Format {
    Json,
    Toml,
}

/// The document of `raw` as JSON, whichever the format, so one classification serves both.
fn parse(raw: &str, format: Format) -> Result<Value, String> {
    match format {
        Format::Json => crate::agents::jsonc::parse(raw).map_err(|e| e.to_string()),
        Format::Toml => raw
            .parse::<toml_edit::DocumentMut>()
            .map(|d| crate::agents::mcp::toml_item_to_json(d.as_item()))
            .map_err(|e| e.to_string()),
    }
}

/// The hooks of a TOML config. Kimi writes `[[hooks]]` tables and CodeWhale `[[hooks.hooks]]`,
/// both `{event, matcher?, command}`; Codex's `[[hooks.<Event>]]` with nested
/// `[[hooks.<Event>.hooks]]` has the settings.json shape and goes through [`entries`].
fn toml_entries(doc: &Value) -> Vec<Entry> {
    let flat = |list: &Value, prefix: &str| -> Vec<Entry> {
        let text = |h: &Value, k: &str| h.get(k).and_then(Value::as_str).map(String::from);
        list.as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, h)| {
                Some(Entry {
                    event: text(h, "event").unwrap_or_default(),
                    matcher: text(h, "matcher").filter(|m| !m.is_empty()),
                    command: text(h, "command")?,
                    key: format!("{prefix}[{i}]"),
                })
            })
            .collect()
    };
    match doc.get("hooks") {
        Some(list @ Value::Array(_)) => flat(list, "hooks"),
        Some(Value::Object(o)) if o.get("hooks").is_some_and(Value::is_array) => {
            flat(&o["hooks"], "hooks.hooks")
        }
        _ => entries(doc),
    }
}

/// One config file: its hooks classified into `acc`. A plugin's file (`RANK_PLUGIN`) is not the
/// user's to edit, and `--fix` edits JSON only, so nothing in those is fixable. Returns the
/// parsed document.
fn scan(
    p: &Probes,
    agent: &'static str,
    source: &Path,
    (rank, format): (u8, Format),
    scope: &Scope,
    acc: &mut Acc,
) -> Option<Value> {
    let editable = rank != RANK_PLUGIN && format == Format::Json;
    let raw = match p.fs.read(source) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            acc.problems.push(unreadable(agent, source, &e.to_string()));
            return None;
        }
    };
    let doc = match parse(&raw, format) {
        Ok(doc) => doc,
        Err(e) => {
            acc.problems.push(unreadable(agent, source, &e));
            return None;
        }
    };
    let found = match format {
        Format::Json => entries(&doc),
        Format::Toml => toml_entries(&doc),
    };
    for e in found {
        acc.seen.push(Seen {
            agent,
            source: source.display().to_string(),
            path: e.key.clone(),
            event: e.event.clone(),
            matcher: e.matcher.clone(),
            command: e.command.clone(),
            normal: normalize(&e.command, p, scope),
            rank,
            editable,
        });
        let (kind, detail, fixable) = match classify(&e.command, p, scope) {
            Verdict::Ok => continue,
            Verdict::Broken(d) => ("broken-hook", d, editable),
            Verdict::Suspect(d) => ("suspect-hook", d, false),
            Verdict::Unverified(d) => ("unverified-hook", d, false),
        };
        acc.problems.push(Problem {
            path: e.key,
            event: e.event,
            matcher: e.matcher,
            command: e.command,
            detail,
            fixable,
            ..problem(kind, agent, source)
        });
    }
    Some(doc)
}

/// A finding with only its identity set.
fn problem(kind: &'static str, agent: &'static str, source: &Path) -> Problem {
    Problem {
        kind,
        agent,
        source: source.display().to_string(),
        path: String::new(),
        event: String::new(),
        matcher: None,
        command: String::new(),
        detail: String::new(),
        fixable: false,
        group: None,
        keep: false,
    }
}

fn unreadable(agent: &'static str, source: &Path, why: &str) -> Problem {
    Problem {
        detail: format!("cannot read {}: {why}", source.display()),
        ..problem("unreadable-config", agent, source)
    }
}

/// The `hooks check` lines of the doctor text: grouped by class, or "none found".
pub fn render(problems: &[Problem]) -> String {
    let hook_problems: Vec<&Problem> = problems
        .iter()
        .filter(|p| {
            matches!(
                p.kind,
                "broken-hook"
                    | "suspect-hook"
                    | "unverified-hook"
                    | "unreadable-config"
                    | "stale-plugin"
                    | "untrusted-folder"
            )
        })
        .collect();
    let mut out = String::new();
    if hook_problems.is_empty() {
        out.push_str("hooks check none found\n");
    } else {
        out.push_str("hooks check\n");
    }
    for (kind, title) in [
        ("broken-hook", "broken"),
        ("suspect-hook", "suspect"),
        ("unverified-hook", "cannot verify"),
        ("stale-plugin", "stale plugin"),
        ("unreadable-config", "unreadable"),
        ("untrusted-folder", "untrusted folder"),
    ] {
        for p in hook_problems.iter().filter(|p| p.kind == kind) {
            let matcher = p
                .matcher
                .as_deref()
                .map(|m| format!("[{m}]"))
                .unwrap_or_default();
            if matches!(
                kind,
                "unreadable-config" | "stale-plugin" | "untrusted-folder"
            ) {
                out.push_str(&format!("  {title} {} {}\n", p.agent, p.detail));
                continue;
            }
            let note = if p.fixable {
                " (can be cleaned up)"
            } else {
                ""
            };
            out.push_str(&format!(
                "  {title} {} {}{matcher} `{}`: {}{note}\n    {} {}\n",
                p.agent, p.event, p.command, p.detail, p.source, p.path
            ));
        }
    }
    out.push_str(&super::dupes::render(problems));
    out
}

#[cfg(test)]
// T902: every `#[test]` below is unix-only (POSIX paths), so on Windows the shared helpers are unused.
#[cfg_attr(windows, allow(dead_code, unused_imports))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io;

    /// An in-memory machine: files, kinds, programs on `PATH` and the environment.
    #[derive(Default, Clone)]
    struct Mock {
        files: BTreeMap<PathBuf, String>,
        kinds: BTreeMap<PathBuf, PathKind>,
        path: BTreeMap<String, PathBuf>,
        env: BTreeMap<String, String>,
        unreadable: BTreeSet<PathBuf>,
        /// Windows rules for hook commands (`Env::windows`).
        win: bool,
        /// A case-insensitive file system, as on Windows: paths match ignoring case.
        insensitive: bool,
    }

    /// `path` as `m` keys it: `/` and `\` alike (the keys are POSIX literals while `join` and
    /// `with_file_name` build `\` paths on Windows), and case-folded in the case-insensitive mode.
    fn key(m: &Mock, path: &Path) -> String {
        let s = path.to_string_lossy().replace('\\', "/");
        if m.insensitive { s.to_lowercase() } else { s }
    }

    impl Fs for Mock {
        fn canonical(&self, path: &Path) -> PathBuf {
            path.to_path_buf()
        }
        fn read(&self, path: &Path) -> io::Result<String> {
            if self
                .unreadable
                .iter()
                .any(|p| key(self, p) == key(self, path))
            {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            let want = key(self, path);
            self.files
                .iter()
                .find(|(k, _)| key(self, k) == want)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn kind(&self, path: &Path) -> PathKind {
            let want = key(self, path);
            self.kinds
                .iter()
                .find(|(k, _)| key(self, k) == want)
                .map_or(PathKind::Missing, |(_, v)| v.clone())
        }
    }
    impl Env for Mock {
        fn var(&self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
        fn home(&self) -> Option<PathBuf> {
            Some("/h".into())
        }
        fn cwd(&self) -> Option<PathBuf> {
            Some("/proj".into())
        }
        fn windows(&self) -> bool {
            self.win
        }
    }
    impl Which for Mock {
        fn find(&self, program: &str) -> Option<PathBuf> {
            self.path.get(program).cloned()
        }
    }

    impl Mock {
        fn script(&mut self, path: &str, executable: bool) {
            self.kinds
                .insert(path.into(), PathKind::File { executable });
        }
        fn settings(&mut self, path: &str, commands: &[&str]) {
            let hooks: Vec<Value> = commands
                .iter()
                .map(|c| serde_json::json!({"type": "command", "command": c}))
                .collect();
            let doc =
                serde_json::json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": hooks}]}});
            self.files.insert(path.into(), doc.to_string());
        }
    }

    fn cfg() -> Config {
        let mut c = Config::default();
        c.doctor.settings_path = "/h/.claude/settings.json".into();
        c.setup.claude.settings_path = "/h/.claude/settings.json".into();
        c.setup.cursor.hooks_path = "/h/.cursor/hooks.json".into();
        c.setup.gemini.dir = "/h/.gemini".into();
        c.setup.kimi.config_path = "/h/.kimi-code/config.toml".into();
        c.setup.codewhale.dir = "/h/.codewhale".into();
        c.setup.codex.config_path = "/h/.codex/config.toml".into();
        c
    }

    fn run(m: &Mock, commands: &[&str]) -> Vec<Problem> {
        let mut m2 = m.clone();
        m2.settings("/h/.claude/settings.json", commands);
        check(
            &cfg(),
            &Probes {
                fs: &m2,
                env: &m2,
                which: &m2,
            },
        )
    }

    fn verdicts(m: &Mock, commands: &[&str]) -> Vec<(&'static str, String)> {
        run(m, commands)
            .into_iter()
            .filter(|p| p.kind != "duplicate-hook")
            .map(|p| (p.kind, p.detail))
            .collect()
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn a_missing_script_is_broken_and_fixable() {
        let found = run(&Mock::default(), &["~/scripts/old-guard.sh"]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, "broken-hook");
        assert!(found[0].fixable);
        assert_eq!(found[0].detail, "file not found: /h/scripts/old-guard.sh");
        assert_eq!(found[0].event, "PreToolUse");
        assert_eq!(found[0].matcher.as_deref(), Some("Bash"));
        assert_eq!(found[0].path, "hooks.PreToolUse[0].hooks[0]");
    }

    /// An absolute path on every OS, quoted so POSIX word splitting keeps a Windows `\`.
    fn quoted_abs(name: &str) -> (PathBuf, String) {
        let path = std::env::temp_dir().join("rtok-hooks-test").join(name);
        let cmd = format!("'{}'", path.display());
        (path, cmd)
    }

    #[test]
    fn a_dangling_symlink_and_a_directory_are_broken() {
        let mut m = Mock::default();
        let (link, link_cmd) = quoted_abs("link.sh");
        let (dir, dir_cmd) = quoted_abs("dir");
        let target = std::env::temp_dir().join("rtok-hooks-test").join("gone.js");
        m.kinds
            .insert(link, PathKind::DanglingSymlink(target.clone()));
        m.kinds.insert(dir, PathKind::Dir);
        let v = verdicts(&m, &[&link_cmd, &dir_cmd]);
        assert_eq!(
            v[0],
            (
                "broken-hook",
                format!("dangling symlink to {}", target.display())
            )
        );
        assert_eq!(v[1].0, "broken-hook");
        assert!(v[1].1.contains("is a directory"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_path_on_an_unmounted_volume_is_broken() {
        let v = verdicts(&Mock::default(), &["/Volumes/X/hook.js"]);
        assert_eq!(
            v[0],
            (
                "broken-hook",
                "path is on /Volumes/X, which is not mounted".into()
            )
        );
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn every_interpreter_with_a_missing_script_is_broken_and_with_a_present_one_is_fine() {
        let mut m = Mock::default();
        m.script("/proj/ok.js", false);
        for interp in [
            "bash", "sh", "zsh", "node", "python", "python3", "deno run", "bun", "ruby", "pwsh",
            "npx tsx", "uv run",
        ] {
            let missing = verdicts(&m, &[&format!("{interp} /proj/gone.js")]);
            assert_eq!(missing.len(), 1, "{interp}");
            assert_eq!(missing[0].0, "broken-hook", "{interp}");
            assert!(
                missing[0].1.contains("/proj/gone.js"),
                "{interp}: {missing:?}"
            );
            assert!(
                verdicts(&m, &[&format!("{interp} /proj/ok.js")]).is_empty(),
                "{interp}"
            );
        }
    }

    #[test]
    fn a_program_is_looked_up_on_path_and_a_builtin_needs_no_file() {
        let mut m = Mock::default();
        m.path.insert("jq".into(), "/usr/bin/jq".into());
        let v = verdicts(&m, &["jq .", "foo --x", "echo hi", "true"]);
        assert_eq!(v, vec![("broken-hook", "`foo` not on PATH".to_string())]);
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn variables_expand_and_relative_paths_resolve_against_the_project() {
        let mut m = Mock::default();
        m.script("/proj/.claude/hooks/a.sh", true);
        m.script("/h/b.sh", true);
        m.script("/proj/rel/c.sh", true);
        m.env.insert("MY_DIR".into(), "/proj/rel".into());
        let v = verdicts(
            &m,
            &[
                "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/a.sh",
                "$HOME/b.sh",
                "${HOME}/b.sh",
                "./rel/c.sh",
                "$MY_DIR/c.sh",
            ],
        );
        assert!(v.is_empty(), "{v:?}");
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn a_quoted_path_with_spaces_is_one_word() {
        let mut m = Mock::default();
        m.script("/h/My Scripts/guard.sh", true);
        assert!(verdicts(&m, &["\"/h/My Scripts/guard.sh\""]).is_empty());
        let gone = verdicts(&m, &["'/h/Other Scripts/guard.sh'"]);
        assert_eq!(gone[0].1, "file not found: /h/Other Scripts/guard.sh");
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn leading_variable_assignments_are_skipped() {
        let mut m = Mock::default();
        m.script("/h/a.sh", true);
        assert!(verdicts(&m, &["RTOK_X=1 /h/a.sh"]).is_empty());
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn a_script_that_exists_but_is_not_executable_is_suspect_only_when_run_directly() {
        let mut m = Mock::default();
        m.script("/h/a.sh", false);
        let direct = run(&m, &["/h/a.sh"]);
        assert_eq!(direct[0].kind, "suspect-hook");
        assert!(!direct[0].fixable);
        assert!(direct[0].detail.contains("chmod +x"));
        m.path.insert("bash".into(), "/bin/bash".into());
        assert!(verdicts(&m, &["bash /h/a.sh"]).is_empty());
    }

    #[test]
    fn what_cannot_be_judged_statically_is_unverified_and_never_fixable() {
        let m = Mock::default();
        for cmd in [
            "$(which foo)",
            "`which foo`",
            "eval \"$X\"",
            "cat x | sh",
            "a && b",
            "$UNSET_THING/run.sh",
            "${CLAUDE_PLUGIN_ROOT}/hooks/run.sh",
            "bash -c 'echo hi'",
            "bash",
            "",
            "'unbalanced",
        ] {
            let found = run(&m, &[cmd]);
            assert_eq!(found.len(), 1, "{cmd}");
            assert_eq!(found[0].kind, "unverified-hook", "{cmd}: {found:?}");
            assert!(!found[0].fixable, "{cmd}");
        }
    }

    #[test]
    fn user_local_and_project_files_are_all_read() {
        let mut m = Mock::default();
        m.settings("/h/.claude/settings.json", &["/h/a"]);
        m.settings("/h/.claude/settings.local.json", &["/h/b"]);
        m.settings("/proj/.claude/settings.json", &["/h/c"]);
        m.settings("/proj/.claude/settings.local.json", &["/h/d"]);
        let found = check(
            &cfg(),
            &Probes {
                fs: &m,
                env: &m,
                which: &m,
            },
        );
        // Compared as paths: `join` writes `\` on Windows where the literals have `/`.
        let mut sources: Vec<PathBuf> = found.iter().map(|p| PathBuf::from(&p.source)).collect();
        sources.sort();
        assert_eq!(
            sources,
            vec![
                PathBuf::from("/h/.claude/settings.json"),
                PathBuf::from("/h/.claude/settings.local.json"),
                PathBuf::from("/proj/.claude/settings.json"),
                PathBuf::from("/proj/.claude/settings.local.json")
            ]
        );
    }

    #[test]
    fn jsonc_comments_and_the_flat_hook_shape_are_read() {
        let mut m = Mock::default();
        m.files.insert(
            "/h/.claude/settings.json".into(),
            "{ // user hooks\n \"hooks\": { \"Stop\": [ { \"command\": \"/h/gone.sh\", }, ], },\n}"
                .into(),
        );
        let found = check(
            &cfg(),
            &Probes {
                fs: &m,
                env: &m,
                which: &m,
            },
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "hooks.Stop[0]");
        assert_eq!(found[0].event, "Stop");
    }

    #[test]
    fn an_unparsable_or_unreadable_file_is_reported_and_a_missing_one_is_not() {
        let mut m = Mock::default();
        m.files
            .insert("/h/.claude/settings.json".into(), "{ nope".into());
        m.files
            .insert("/h/.claude/settings.local.json".into(), "{}".into());
        m.unreadable.insert("/proj/.claude/settings.json".into());
        let found = check(
            &cfg(),
            &Probes {
                fs: &m,
                env: &m,
                which: &m,
            },
        );
        let kinds: Vec<&str> = found.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec!["unreadable-config", "unreadable-config"]);
        assert!(
            found
                .iter()
                .all(|p| !p.fixable && p.detail.starts_with("cannot read "))
        );
    }

    #[test]
    fn a_file_without_hooks_has_no_problem_and_non_command_hooks_are_ignored() {
        let mut m = Mock::default();
        m.files.insert(
            "/h/.claude/settings.json".into(),
            r#"{"hooks": {"Stop": [{"hooks": [{"type": "prompt", "prompt": "x"}]}]}}"#.into(),
        );
        m.files.insert(
            "/h/.claude/settings.local.json".into(),
            "{\"model\": \"x\"}".into(),
        );
        assert!(
            check(
                &cfg(),
                &Probes {
                    fs: &m,
                    env: &m,
                    which: &m
                }
            )
            .is_empty()
        );
    }

    fn check_with(m: &Mock) -> Vec<Problem> {
        check(
            &cfg(),
            &Probes {
                fs: m,
                env: m,
                which: m,
            },
        )
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn another_hosts_json_hooks_are_checked_under_its_own_name_and_a_shared_file_once() {
        let mut m = Mock::default();
        m.files.insert(
            "/h/.cursor/hooks.json".into(),
            r#"{"version": 1, "hooks": {"beforeShellExecution": [{"command": "/h/gone.sh"}]}}"#
                .into(),
        );
        m.files.insert(
            "/h/.gemini/settings.json".into(),
            r#"{"hooks": {"BeforeTool": [{"matcher": "run_shell_command", "hooks": [{"type": "command", "command": "/h/gone2.sh"}]}]}}"#
                .into(),
        );
        let found = check_with(&m);
        let by: Vec<(&str, &str, bool)> = found
            .iter()
            .map(|p| (p.agent, p.event.as_str(), p.fixable))
            .collect();
        assert_eq!(
            by,
            vec![
                ("cursor", "beforeShellExecution", true),
                ("gemini", "BeforeTool", true)
            ]
        );
        assert_eq!(found[1].matcher.as_deref(), Some("run_shell_command"));
    }

    #[test]
    fn the_host_filter_keeps_one_hosts_findings_and_names_the_valid_ids() {
        let mut m = Mock::default();
        m.files.insert(
            "/h/.cursor/hooks.json".into(),
            r#"{"version": 1, "hooks": {"beforeShellExecution": [{"command": "/h/gone.sh"}]}}"#
                .into(),
        );
        m.files.insert(
            "/h/.gemini/settings.json".into(),
            r#"{"hooks": {"BeforeTool": [{"hooks": [{"type": "command", "command": "/h/gone2.sh"}]}]}}"#
                .into(),
        );
        m.settings("/h/.claude/settings.json", &["/h/gone3.sh"]);
        let agents = |only: Option<&str>| -> Vec<&str> {
            let probes = Probes {
                fs: &m,
                env: &m,
                which: &m,
            };
            check_for(&cfg(), &probes, only)
                .iter()
                .map(|p| p.agent)
                .collect()
        };
        assert_eq!(agents(None), ["claude", "cursor", "gemini"]);
        assert_eq!(agents(Some("cursor")), ["cursor"]);
        assert_eq!(agents(Some("claude")), ["claude"]);
        assert!(
            agents(Some("zed")).is_empty(),
            "a host without hooks has none"
        );
        assert_eq!(host_id("gemini"), Ok("gemini"));
        let err = host_id("nope").unwrap_err();
        assert!(
            err.starts_with("unknown host `nope`; valid hosts: claude, cursor"),
            "{err}"
        );
    }

    fn windows_mock() -> Mock {
        Mock {
            win: true,
            insensitive: true,
            ..Mock::default()
        }
    }

    #[test]
    fn windows_words_keep_backslashes_and_quotes_group() {
        let words = |s: &str| split_words(s, true).unwrap();
        assert_eq!(words(r"C:\hooks\x.cmd a  b"), [r"C:\hooks\x.cmd", "a", "b"]);
        assert_eq!(
            words(r#""C:\Program Files (x86)\x.cmd" --y"#),
            [r"C:\Program Files (x86)\x.cmd", "--y"]
        );
        assert_eq!(words(r#"a "b\"c" 'd'"#), ["a", r#"b"c"#, "'d'"]);
        assert_eq!(words(r#"a "" b"#), ["a", "", "b"]);
        assert_eq!(split_words(r#"a "b"#, true), None, "unbalanced");
    }

    #[test]
    fn a_windows_path_resolves_by_pathext_ignoring_case() {
        let mut m = windows_mock();
        m.script(r"C:\Hooks\guard.cmd", false);
        m.script(r"C:\Hooks\run.EXE", false);
        m.env.insert("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into());
        // The extension is optional; the file's own case does not matter; no exec bit exists.
        assert!(
            verdicts(
                &m,
                &[r"c:\hooks\GUARD", r"C:\Hooks\run", r"C:\hooks\guard.cmd"]
            )
            .is_empty()
        );
        let gone = verdicts(&m, &[r"C:\Hooks\missing"]);
        assert_eq!(
            gone,
            [(
                "broken-hook",
                r"file not found: C:\Hooks\missing".to_string()
            )]
        );
        // An extension PATHEXT does not list is not tried.
        m.env.insert("PATHEXT".into(), ".EXE".into());
        assert_eq!(verdicts(&m, &[r"C:\Hooks\guard"]).len(), 1);
        assert!(verdicts(&m, &[r"C:\Hooks\guard.cmd"]).is_empty());
    }

    #[test]
    fn a_windows_program_is_searched_on_path_and_builtins_need_no_file() {
        let mut m = windows_mock();
        m.script(r"C:\Tools\rtok.exe", false);
        m.env.insert("PATH".into(), r"C:\bin;c:\tools\".into());
        assert!(verdicts(&m, &["rtok hook PreToolUse", "echo hi", "COPY a b"]).is_empty());
        assert_eq!(
            verdicts(&m, &["nothere run"]),
            [("broken-hook", "`nothere` not on PATH".to_string())]
        );
    }

    #[test]
    fn windows_quotes_variables_and_launchers_are_read_by_their_own_rules() {
        let mut m = windows_mock();
        m.script(r"C:\Program Files (x86)\rtok\hook.cmd", false);
        m.script(r"C:\Users\me\hooks\x.cmd", false);
        m.script(r"C:\h\run.bat", false);
        m.script(r"C:\h\x.ps1", false);
        m.env.insert("USERPROFILE".into(), r"C:\Users\me".into());
        assert!(
            verdicts(
                &m,
                &[
                    r#""C:\Program Files (x86)\rtok\hook.cmd" --x"#,
                    r"%USERPROFILE%\hooks\x.cmd",
                    r"cmd /c C:\h\run.bat arg",
                    r"powershell -NoProfile -File C:\h\x.ps1",
                ]
            )
            .is_empty()
        );
        let found = verdicts(
            &m,
            &[
                r"cmd /c C:\h\gone.bat",
                r"powershell -File C:\h\gone.ps1",
                r"powershell -Command Get-Date",
                r"C:\a & C:\b",
            ],
        );
        let kinds: Vec<&str> = found.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            kinds,
            [
                "broken-hook",
                "broken-hook",
                "unverified-hook",
                "unverified-hook"
            ]
        );
    }

    #[test]
    fn a_windows_relative_path_joins_the_project_for_claude_only() {
        let mut m = windows_mock();
        m.script(r"/proj\hook.cmd", false);
        assert!(verdicts(&m, &[r".\hook.cmd"]).is_empty());
        assert_eq!(verdicts(&m, &[r".\gone.cmd"])[0].0, "broken-hook");
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn a_relative_path_is_not_judged_for_a_host_that_does_not_document_its_base() {
        let mut m = Mock::default();
        m.files.insert(
            "/h/.cursor/hooks.json".into(),
            r#"{"hooks": {"stop": [{"command": "./hooks/a.sh"}, {"command": "/h/gone.sh"}]}}"#
                .into(),
        );
        let found = check_with(&m);
        assert_eq!(found[0].kind, "unverified-hook");
        assert!(found[0].detail.starts_with("relative path"));
        assert!(!found[0].fixable);
        assert_eq!(found[1].kind, "broken-hook");
        // Claude Code runs hooks in the project directory.
        m.settings("/h/.claude/settings.json", &["./hooks/a.sh"]);
        let claude = check_with(&m);
        assert_eq!(claude[0].agent, "claude");
        assert_eq!(claude[0].detail, "file not found: /proj/hooks/a.sh");
    }

    fn plugin_home(m: &mut Mock, enabled: bool, install_dir: bool, hooks: &str) {
        m.files.insert(
            "/h/.claude/plugins/installed_plugins.json".into(),
            r#"{"plugins": {"demo@mkt": [{"installPath": "/h/plug/demo"}], "off@mkt": [{"installPath": "/h/plug/off"}]}}"#
                .into(),
        );
        m.files.insert(
            "/h/.claude/settings.json".into(),
            serde_json::json!({"enabledPlugins": {"demo@mkt": enabled, "off@mkt": false}})
                .to_string(),
        );
        if install_dir {
            m.kinds.insert("/h/plug/demo".into(), PathKind::Dir);
            m.files
                .insert("/h/plug/demo/hooks/hooks.json".into(), hooks.into());
        }
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn a_plugin_hook_resolves_the_plugin_root_and_is_never_fixable() {
        let mut m = Mock::default();
        let hooks = r#"{"hooks": {"SessionStart": [{"hooks": [
            {"type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/scripts/ok.sh"},
            {"type": "command", "command": "bash \"${CLAUDE_PLUGIN_ROOT}/scripts/gone.sh\" start"}]}]}}"#;
        plugin_home(&mut m, true, true, hooks);
        m.script("/h/plug/demo/scripts/ok.sh", true);
        // A disabled plugin is not loaded, so its hooks are not checked.
        m.files.insert(
            "/h/plug/off/hooks/hooks.json".into(),
            r#"{"hooks": {"Stop": [{"command": "/h/never.sh"}]}}"#.into(),
        );
        let found = check_with(&m);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, "broken-hook");
        assert_eq!(
            found[0].detail,
            "file not found: /h/plug/demo/scripts/gone.sh"
        );
        assert_eq!(found[0].source, "/h/plug/demo/hooks/hooks.json");
        assert!(!found[0].fixable);
    }

    #[test]
    fn the_install_directory_of_each_enabled_present_plugin_is_returned() {
        let mut m = Mock::default();
        plugin_home(&mut m, true, true, "{}");
        let probes = Probes {
            fs: &m,
            env: &m,
            which: &m,
        };
        let (_, dirs) = check_with_plugins(&cfg(), &probes);
        assert_eq!(
            dirs,
            [("demo@mkt".to_string(), PathBuf::from("/h/plug/demo"))]
        );
        // Disabled, and enabled but gone: nothing for a later check to read.
        plugin_home(&mut m, false, true, "{}");
        let probes = Probes {
            fs: &m,
            env: &m,
            which: &m,
        };
        assert!(check_with_plugins(&cfg(), &probes).1.is_empty());
        plugin_home(&mut m, true, false, "");
        m.kinds.clear();
        let probes = Probes {
            fs: &m,
            env: &m,
            which: &m,
        };
        assert!(check_with_plugins(&cfg(), &probes).1.is_empty());
    }

    /// T925: Cursor's plugin link into a ketch store version that an upgrade removed loads
    /// nothing; doctor says so, and says nothing once it is gone.
    #[cfg(unix)]
    #[test]
    fn a_dangling_host_plugin_link_is_reported_as_a_stale_plugin() {
        let home = std::env::temp_dir().join(format!("rtok-doctor-plugin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let local = home.join(".cursor/plugins/local");
        std::fs::create_dir_all(&local).unwrap();
        let gone = home.join(".ketch/store/rtok/v0.15.1/plugins/cursor");
        std::os::unix::fs::symlink(&gone, local.join("rtok")).unwrap();
        let mut c = cfg();
        c.setup.cursor.hooks_path = home.join(".cursor/hooks.json");
        let probes = |m: &Mock| {
            check(
                &c,
                &Probes {
                    fs: m,
                    env: m,
                    which: m,
                },
            )
        };
        let m = Mock::default();
        let found = probes(&m);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, "stale-plugin");
        assert_eq!(found[0].agent, "cursor");
        assert!(found[0].detail.contains("v0.15.1"), "{}", found[0].detail);
        assert!(found[0].detail.contains("agents install cursor"));
        assert!(!found[0].fixable);
        std::fs::remove_file(local.join("rtok")).unwrap();
        assert!(
            probes(&m).is_empty(),
            "nothing installed, nothing to report"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_enabled_plugin_whose_directory_is_gone_is_stale_and_a_disabled_one_is_ignored() {
        let mut m = Mock::default();
        plugin_home(&mut m, true, false, "");
        let found = check_with(&m);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, "stale-plugin");
        assert_eq!(found[0].path, "plugins.demo@mkt[0]");
        assert!(found[0].detail.contains("/h/plug/demo is gone"));
        assert!(!found[0].fixable);
        plugin_home(&mut m, false, false, "");
        assert!(check_with(&m).is_empty());
    }

    #[test]
    fn plugin_root_outside_a_plugin_is_still_unverified() {
        let m = Mock::default();
        let found = run(&m, &["${CLAUDE_PLUGIN_ROOT}/x.sh"]);
        assert_eq!(found[0].kind, "unverified-hook");
    }

    #[test]
    fn a_foreign_non_json_host_file_is_left_alone() {
        let mut m = Mock::default();
        m.files
            .insert("/h/.cursor/hooks.json".into(), "{ nope".into());
        let found = check_with(&m);
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].kind, found[0].agent),
            ("unreadable-config", "cursor")
        );
    }

    fn hooks_doc(event: &str, matcher: Option<&str>, commands: &[&str]) -> String {
        let hooks: Vec<Value> = commands
            .iter()
            .map(|c| serde_json::json!({"type": "command", "command": c}))
            .collect();
        let mut group = serde_json::json!({ "hooks": hooks });
        if let Some(m) = matcher {
            group["matcher"] = m.into();
        }
        serde_json::json!({"hooks": {event: [group]}}).to_string()
    }

    /// `(source, keep)` of every duplicate copy, in report order.
    fn copies(m: &Mock) -> Vec<(PathBuf, bool)> {
        check_with(m)
            .into_iter()
            .filter(|p| p.kind == "duplicate-hook")
            .map(|p| (PathBuf::from(p.source), p.keep))
            .collect()
    }

    #[test]
    fn the_same_hook_in_user_and_project_settings_is_a_duplicate_and_the_shared_copy_is_kept() {
        let mut m = Mock::default();
        let doc = hooks_doc("PreToolUse", Some("Bash"), &["jq ."]);
        m.files
            .insert("/h/.claude/settings.json".into(), doc.clone());
        m.files.insert("/proj/.claude/settings.json".into(), doc);
        m.path.insert("jq".into(), "/usr/bin/jq".into());
        let found = check_with(&m);
        let d: Vec<&Problem> = found
            .iter()
            .filter(|p| p.kind == "duplicate-hook")
            .collect();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].group, d[1].group);
        assert!(d[0].detail.starts_with("runs 2 times"));
        assert!(
            d.iter().all(|p| p.fixable),
            "both files are the user's to edit"
        );
        assert_eq!(
            copies(&m),
            vec![
                (PathBuf::from("/h/.claude/settings.json"), false),
                (PathBuf::from("/proj/.claude/settings.json"), true)
            ]
        );
    }

    #[test]
    fn a_user_file_wins_over_a_local_one_and_a_repeat_in_one_file_keeps_the_first() {
        let mut m = Mock::default();
        let doc = hooks_doc("Stop", None, &["jq .", "jq ."]);
        m.files.insert(
            "/h/.claude/settings.local.json".into(),
            hooks_doc("Stop", None, &["jq ."]),
        );
        m.files.insert("/h/.claude/settings.json".into(), doc);
        m.path.insert("jq".into(), "/usr/bin/jq".into());
        let found = check_with(&m);
        let d: Vec<&Problem> = found
            .iter()
            .filter(|p| p.kind == "duplicate-hook")
            .collect();
        assert_eq!(d.len(), 3);
        assert!(d[0].detail.starts_with("runs 3 times"));
        let kept: Vec<(&str, bool)> = d.iter().map(|p| (p.path.as_str(), p.keep)).collect();
        assert_eq!(
            kept,
            vec![
                ("hooks.Stop[0].hooks[0]", true),
                ("hooks.Stop[0].hooks[1]", false),
                ("hooks.Stop[0].hooks[0]", false)
            ]
        );
        assert_eq!(
            PathBuf::from(&d[2].source),
            PathBuf::from("/h/.claude/settings.local.json")
        );
    }

    #[test]
    fn a_plugin_copy_is_kept_over_a_hand_written_one_and_is_never_the_extra() {
        let mut m = Mock::default();
        m.files.insert(
            "/h/.claude/plugins/installed_plugins.json".into(),
            r#"{"plugins": {"demo@mkt": [{"installPath": "/h/plug/demo"}]}}"#.into(),
        );
        let mut settings: Value =
            serde_json::from_str(&hooks_doc("PreToolUse", Some("Bash"), &["jq ."])).unwrap();
        settings["enabledPlugins"] = serde_json::json!({"demo@mkt": true});
        m.files
            .insert("/h/.claude/settings.json".into(), settings.to_string());
        m.kinds.insert("/h/plug/demo".into(), PathKind::Dir);
        m.files.insert(
            "/h/plug/demo/hooks/hooks.json".into(),
            hooks_doc("PreToolUse", Some("Bash"), &["jq ."]),
        );
        m.path.insert("jq".into(), "/usr/bin/jq".into());
        assert_eq!(
            copies(&m),
            vec![
                (PathBuf::from("/h/.claude/settings.json"), false),
                (PathBuf::from("/h/plug/demo/hooks/hooks.json"), true)
            ]
        );
    }

    #[test]
    fn only_a_hand_written_extra_is_fixable_never_a_plugin_copy() {
        let mut m = Mock::default();
        m.files.insert(
            "/h/.claude/plugins/installed_plugins.json".into(),
            r#"{"plugins": {"demo@mkt": [{"installPath": "/h/plug/demo"}]}}"#.into(),
        );
        let mut settings: Value =
            serde_json::from_str(&hooks_doc("Stop", None, &["jq ."])).unwrap();
        settings["enabledPlugins"] = serde_json::json!({"demo@mkt": true});
        m.files
            .insert("/h/.claude/settings.json".into(), settings.to_string());
        m.kinds.insert("/h/plug/demo".into(), PathKind::Dir);
        // The plugin repeats the hook inside its own file too: an extra, but not the user's to edit.
        m.files.insert(
            "/h/plug/demo/hooks/hooks.json".into(),
            hooks_doc("Stop", None, &["jq .", "jq ."]),
        );
        m.path.insert("jq".into(), "/usr/bin/jq".into());
        // Compared as paths: `join` writes `\` on Windows where the literals have `/`.
        let fixable: Vec<(PathBuf, bool, bool)> = check_with(&m)
            .into_iter()
            .filter(|p| p.kind == "duplicate-hook")
            .map(|p| (PathBuf::from(p.source), p.keep, p.fixable))
            .collect();
        assert_eq!(
            fixable,
            vec![
                (PathBuf::from("/h/.claude/settings.json"), false, true),
                (PathBuf::from("/h/plug/demo/hooks/hooks.json"), true, false),
                (PathBuf::from("/h/plug/demo/hooks/hooks.json"), false, false),
            ]
        );
    }

    #[test]
    fn what_differs_in_event_matcher_command_agent_or_load_state_is_not_a_duplicate() {
        let mut m = Mock::default();
        m.path.insert("jq".into(), "/usr/bin/jq".into());
        m.path.insert("fmt".into(), "/usr/bin/fmt".into());
        // Different events, different matchers, different commands.
        m.files.insert(
            "/h/.claude/settings.json".into(),
            serde_json::json!({"hooks": {
                "Stop": [{"hooks": [{"type": "command", "command": "jq ."}]}],
                "SessionEnd": [{"hooks": [{"type": "command", "command": "jq ."}]}],
                "PreToolUse": [
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": "jq ."}]},
                    {"matcher": "Edit", "hooks": [{"type": "command", "command": "jq ."}]},
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": "jq -r ."}]}
                ]
            }})
            .to_string(),
        );
        // The same command under another agent is a different process tree.
        m.files.insert(
            "/h/.cursor/hooks.json".into(),
            r#"{"hooks": {"stop": [{"command": "jq ."}]}}"#.into(),
        );
        m.files.insert(
            "/h/.gemini/settings.json".into(),
            r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "jq ."}]}]}}"#.into(),
        );
        // A disabled plugin is not loaded, so its copy does not run.
        m.files.insert(
            "/h/.claude/plugins/installed_plugins.json".into(),
            r#"{"plugins": {"off@mkt": [{"installPath": "/h/plug/off"}]}}"#.into(),
        );
        m.kinds.insert("/h/plug/off".into(), PathKind::Dir);
        m.files.insert(
            "/h/plug/off/hooks/hooks.json".into(),
            hooks_doc("Stop", None, &["jq ."]),
        );
        assert!(copies(&m).is_empty(), "{:?}", check_with(&m));
    }

    #[test]
    fn copies_that_differ_only_in_spelling_are_duplicates() {
        let mut m = Mock::default();
        m.path.insert("jq".into(), "/usr/bin/jq".into());
        // Matcher: order, spacing, and "no matcher" equal `*`.
        m.files.insert(
            "/h/.claude/settings.json".into(),
            serde_json::json!({"hooks": {"PreToolUse": [
                {"matcher": "Bash|Edit", "hooks": [{"type": "command", "command": "jq  -r  ."}]},
                {"matcher": " Edit | Bash ", "hooks": [{"type": "command", "command": "jq -r '.'", "timeout": 5}]},
                {"hooks": [{"type": "command", "command": "jq a"}]},
                {"matcher": "*", "hooks": [{"type": "command", "command": "jq \"a\""}]}
            ]}})
            .to_string(),
        );
        let found = check_with(&m);
        let d: Vec<&Problem> = found
            .iter()
            .filter(|p| p.kind == "duplicate-hook")
            .collect();
        assert_eq!(d.len(), 4, "{found:?}");
        assert_eq!(d.iter().filter(|p| p.group == Some(0)).count(), 2);
        assert_eq!(d.iter().filter(|p| p.group == Some(1)).count(), 2);
    }

    #[test]
    fn variables_expand_before_copies_are_compared() {
        let mut m = Mock::default();
        m.path.insert("jq".into(), "/usr/bin/jq".into());
        m.script("/h/hooks/a.sh", true);
        m.files.insert(
            "/h/.claude/settings.json".into(),
            hooks_doc(
                "Stop",
                None,
                &["/h/hooks/a.sh", "$HOME/hooks/a.sh", "~/hooks/a.sh"],
            ),
        );
        assert_eq!(copies(&m).len(), 3);
    }

    #[cfg(unix)] // `./x` joins the project directory with `/`
    #[test]
    fn a_relative_path_equals_its_absolute_form_for_claude_only() {
        let mut m = Mock::default();
        m.script("/proj/hooks/a.sh", true);
        m.files.insert(
            "/h/.claude/settings.json".into(),
            hooks_doc("Stop", None, &["./hooks/a.sh", "/proj/hooks/a.sh"]),
        );
        assert_eq!(copies(&m).len(), 2);
    }

    #[test]
    fn the_text_lists_each_group_once_with_its_copies_and_says_none_found() {
        assert_eq!(
            super::super::dupes::render(&[]),
            "duplicate hooks none found\n"
        );
        let mut m = Mock::default();
        let doc = hooks_doc("Stop", None, &["jq ."]);
        m.files
            .insert("/h/.claude/settings.json".into(), doc.clone());
        m.files.insert("/proj/.claude/settings.json".into(), doc);
        m.path.insert("jq".into(), "/usr/bin/jq".into());
        let text = render(&check_with(&m));
        assert!(text.contains("hooks check none found\n"), "{text}");
        assert!(
            text.contains("duplicate hooks\n  claude Stop `jq .`: runs 2 times\n"),
            "{text}"
        );
        assert_eq!(text.matches("runs 2 times").count(), 1);
        assert!(text.contains("    keep "), "{text}");
        assert!(text.contains("    extra "), "{text}");
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn the_text_groups_by_class_and_says_none_found() {
        assert!(render(&[]).starts_with("hooks check none found\n"));
        let found = run(&Mock::default(), &["/h/gone.sh", "$(x)"]);
        let text = render(&found);
        assert!(text.starts_with("hooks check\n"), "{text}");
        assert!(text.contains("broken claude PreToolUse[Bash] `/h/gone.sh`: file not found: /h/gone.sh (can be cleaned up)"), "{text}");
        assert!(
            text.contains("cannot verify claude PreToolUse[Bash] `$(x)`"),
            "{text}"
        );
        assert!(
            text.contains("/h/.claude/settings.json hooks.PreToolUse[0].hooks[0]"),
            "{text}"
        );
    }

    /// The doctor modules reach files, the environment and `PATH` only through `probe`.
    #[test]
    fn only_the_probe_module_touches_the_machine() {
        let banned = [
            concat!("std::", "fs"),
            concat!("std::", "env"),
            concat!("which", "::"),
            concat!("fs", "::read"),
        ];
        for (name, src) in [
            ("hooks.rs", include_str!("hooks.rs")),
            ("dupes.rs", include_str!("dupes.rs")),
        ] {
            let code = src.split("#[cfg(test)]").next().unwrap_or(src);
            for b in banned {
                assert!(
                    !code.contains(b),
                    "{name} calls `{b}` directly; go through probe"
                );
            }
        }
    }

    /// What the TOML hosts report, one `agent kind event [matcher] path fixable` line each.
    fn toml_rows(m: &Mock) -> Vec<String> {
        check_with(m)
            .into_iter()
            .map(|p| {
                let matcher = p.matcher.map(|m| format!(" [{m}]")).unwrap_or_default();
                format!(
                    "{} {} {}{matcher} {} {}",
                    p.agent, p.kind, p.event, p.path, p.fixable
                )
            })
            .collect()
    }

    #[cfg(unix)] // POSIX paths and command words; Windows rules are T331.9
    #[test]
    fn kimi_toml_hooks_are_checked_by_their_array_index_and_never_fixable() {
        let mut m = Mock::default();
        m.script("/h/ok.sh", true);
        m.files.insert(
            "/h/.kimi-code/config.toml".into(),
            r#"
theme = "dark"

[[hooks]]
event = "PreToolUse"
matcher = "Bash"
command = "/h/gone.sh"
timeout = 5

[[hooks]]
event = "Stop"
command = "/h/ok.sh"

[[hooks]]
event = "SessionStart"
matcher = ""
command = "echo $(date)"
"#
            .into(),
        );
        assert_eq!(
            toml_rows(&m),
            [
                "kimi broken-hook PreToolUse [Bash] hooks[0] false",
                "kimi unverified-hook SessionStart hooks[2] false",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn codewhale_toml_hooks_use_the_nested_hooks_hooks_table() {
        let mut m = Mock::default();
        m.files.insert(
            "/h/.codewhale/config.toml".into(),
            r#"
[hooks]
enabled = true

[[hooks.hooks]]
name = "rtok"
event = "message_submit"
command = "/h/gone/rtok-hook UserPromptSubmit"
timeout_secs = 5
"#
            .into(),
        );
        assert_eq!(
            toml_rows(&m),
            ["codewhale broken-hook message_submit hooks.hooks[0] false"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_inline_toml_hooks_keep_the_settings_shape() {
        let mut m = Mock::default();
        m.script("/h/ok.sh", true);
        m.files.insert(
            "/h/.codex/config.toml".into(),
            r#"
model = "gpt"

[[hooks.PreToolUse]]
matcher = "^Bash$"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "/h/gone.sh"
timeout = 30

[[hooks.PreToolUse.hooks]]
type = "command"
command = "/h/ok.sh"

[[hooks.Stop.hooks]]
type = "mcp_tool"
command = "/h/never-looked-at.sh"
"#
            .into(),
        );
        assert_eq!(
            toml_rows(&m),
            ["codex broken-hook PreToolUse [^Bash$] hooks.PreToolUse[0].hooks[0] false"]
        );
    }

    #[test]
    fn an_unparsable_toml_is_reported_and_a_valid_one_without_hooks_is_quiet() {
        let mut m = Mock::default();
        m.files.insert(
            "/h/.kimi-code/config.toml".into(),
            "[[hooks]\nevent = ".into(),
        );
        m.files
            .insert("/h/.codex/config.toml".into(), "model = \"gpt\"\n".into());
        let found = check_with(&m);
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].kind, found[0].agent),
            ("unreadable-config", "kimi")
        );
        assert!(!found[0].fixable);
    }

    #[test]
    fn a_toml_of_a_host_with_no_documented_hook_shape_is_not_read() {
        let mut c = cfg();
        c.setup.grok.config_path = "/h/.grok/config.toml".into();
        let mut m = Mock::default();
        m.files.insert(
            "/h/.grok/config.toml".into(),
            "[[hooks]]\nevent = \"Stop\"\ncommand = \"/h/gone.sh\"\n".into(),
        );
        let found = check(
            &c,
            &Probes {
                fs: &m,
                env: &m,
                which: &m,
            },
        );
        assert!(found.is_empty(), "{found:?}");
    }
}

// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Agent hosts (`rtok agents install|remove|list`).
//!
//! Everything the hosts share — backup, the dry-run/idempotence write gate, `mcpServers`
//! registration, the plugin-link offer — lives in `rtok-agent-sdk` (D28); a host declares its
//! own plugin as one [`plugin::HostPlugin`] rather than respelling that cycle. Each host is one
//! folder here: `<host>/mod.rs` implements [`Agent`] (variants, files, installed modules,
//! apply) and `<host>/README.md` says which rtok modules the host takes, which it could take,
//! and why the rest cannot be taken; a unit test keeps the README and `support()` in step.
//! `rtok agents list` and `rtok doctor` read the same files back through the same contract.

pub mod aider;
pub mod antigravity;
pub mod claude;
pub mod cline;
pub mod codebuddy;
pub mod codewhale;
pub mod codex;
pub mod command;
pub mod commandcode;
pub mod copilot;
pub mod cursor;
pub mod devin;
pub mod gemini;
pub mod grok;
pub mod hook_events;
pub mod jsonc;
pub mod junk;
pub mod junk_cache;
pub mod junk_clear;
pub mod junk_items;
pub mod junk_kinds;
pub mod junk_map;
pub mod junk_review;
pub mod junk_sessions;
pub mod junk_web;
pub mod junk_worktrees;
pub mod kilo;
pub mod kimi;
pub mod link;
pub(crate) mod mcp;
pub mod mimo;
pub mod omp;
pub mod opencode;
mod outdated;
pub mod pi;
pub mod plugin;
pub(crate) mod plugin_install;
pub(crate) mod plugin_version;
pub mod post_create;
pub(crate) mod proxy_env;
pub mod qwen;
pub mod restart;
pub mod roo;
pub mod skill;
pub mod task_list;
pub mod usage;
pub mod vscode;
pub mod windsurf;
pub mod zcode;
pub mod zed;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rtok_agent_sdk::NO_CHANGES;
use rtok_plugin_sdk::Surface;
use serde_json::json;
use toml_edit::DocumentMut;

use crate::config::Config;
use crate::ui::style;

/// Every host rtok installs into, in `agents list` order.
pub use crate::config::HOSTS;

/// Every module an rtok install can carry, in print order.
pub const MODULES: &[&str] = &["hooks", "mcp", "proxy", "plugin"];

/// The host behind an id. `None` is refused by the CLI before any backup is taken.
pub fn host(id: &str) -> Option<&'static dyn Agent> {
    match id {
        "claude" => Some(&claude::Claude),
        "cursor" => Some(&cursor::Cursor),
        "codex" => Some(&codex::Codex),
        "opencode" => Some(&opencode::OpenCode),
        "kilo" => Some(&kilo::Kilo),
        "pi" => Some(&pi::Pi),
        "omp" => Some(&omp::Omp),
        "zcode" => Some(&zcode::Zcode),
        "kimi" => Some(&kimi::Kimi),
        "grok" => Some(&grok::Grok),
        "vscode" => Some(&vscode::Vscode),
        "copilot" => Some(&copilot::Copilot),
        "commandcode" => Some(&commandcode::CommandCode),
        "windsurf" => Some(&windsurf::Windsurf),
        "aider" => Some(&aider::Aider),
        "zed" => Some(&zed::Zed),
        "cline" => Some(&cline::Cline),
        "gemini" => Some(&gemini::Gemini),
        "codewhale" => Some(&codewhale::Codewhale),
        "mimo" => Some(&mimo::Mimo),
        "antigravity" => Some(&antigravity::Antigravity),
        "devin" => Some(&devin::Devin),
        "roo" => Some(&roo::Roo),
        "qwen" => Some(&qwen::Qwen),
        "codebuddy" => Some(&codebuddy::Codebuddy),
        _ => None,
    }
}

/// How the app runs: a terminal binary or a desktop application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Cli,
    Desktop,
}

impl Kind {
    /// The block header word: `CLI: Codex`, `Desktop: Claude Desktop`.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Cli => "CLI",
            Kind::Desktop => "Desktop",
        }
    }

    /// The flag spelling (`--cli`, `--desktop`) and the word `rtok doctor` prints.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Cli => "cli",
            Kind::Desktop => "desktop",
        }
    }
}

/// One app of a host. `bins` are looked up on PATH and asked `--version`; `apps` are where a
/// desktop build installs (`~/…`, `$VAR/…` or absolute), probed with `exists()`. Entries for
/// other platforms simply never exist.
pub struct Variant {
    pub kind: Kind,
    pub name: &'static str,
    pub bins: &'static [&'static str],
    pub apps: &'static [&'static str],
}

/// Whether `setup` can write a module into a host variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// Written by a plain `setup`.
    Yes,
    /// Written only when this flag is given (`--proxy`, `--yes`).
    Flag(&'static str),
    /// Never written by `setup`: it prints how to install the module behind this flag and
    /// the host's own installer owns the state (Kimi's `plugins/managed/`), so `expected()`
    /// never demands it read back.
    Offer(&'static str),
    /// Cannot be written today; the reason is the README's, in one line.
    No(&'static str),
}

/// What `rtok agents install|remove` does to a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Install,
    Remove,
    /// `setup claude --replace`: drop legacy token hooks and retarget the proxy.
    Replace,
    /// `rtok agents update` (T242.2): the install path over a variant that already carries
    /// rtok, with the flag modules it has switched back on. A host whose module cannot be
    /// brought current in place (a CLI-managed plugin) reinstalls it under this mode.
    Update,
}

impl Mode {
    /// The word [`crate::ui::style::icon`] looks up to pick this mode's operation icon.
    pub fn verb(self) -> &'static str {
        match self {
            Mode::Install => "install",
            Mode::Remove => "remove",
            Mode::Replace => "replace",
            Mode::Update => "update",
        }
    }
}

/// The contract every host folder implements. The generic [`run`] loop, [`list`] and
/// `rtok doctor` know nothing else about a host.
pub trait Agent: Sync {
    fn id(&self) -> &'static str;
    fn variants(&self) -> &'static [Variant];
    /// The folder's README, for the parity test.
    fn readme(&self) -> &'static str;
    /// True when every variant reads the same files (Cursor): one apply covers them all.
    fn shared(&self) -> bool {
        false
    }
    fn support(&self, kind: Kind, module: &str) -> Support;
    /// Config files an install writes; copied before any write. Empty for a host that owns a
    /// linked directory instead of a file (pi).
    fn files(&self, cfg: &Config, kind: Kind) -> Vec<PathBuf>;
    /// Paths inside the host's own folders; `agents junk list` lists their parents. Not proof
    /// the app is installed — a folder outlives its app (T426). Defaults to [`Agent::files`]; a
    /// host adds its plugin directory.
    fn markers(&self, cfg: &Config, kind: Kind) -> Vec<PathBuf> {
        self.files(cfg, kind)
    }
    /// Modules found in the host's files: the same markers the installer writes.
    fn installed(&self, cfg: &Config, kind: Kind) -> Vec<&'static str>;
    /// The plugin surfaces the linked `plugin` module serves on this host (Cursor: hook and
    /// MCP as one unit; pi: the bash call path). Empty where there is no plugin module.
    fn plugin_surfaces(&self) -> &'static [Surface] {
        &[]
    }
    /// Run the installer. One report per step; a step that touched nothing reports
    /// [`NO_CHANGES`].
    fn apply(&self, cfg: &Config, kind: Kind, mode: Mode) -> Result<Vec<String>>;
}

/// Whether `kind` is wanted given `--cli/--desktop/--all`. No flag (or `--all`) means every
/// variant.
pub fn wants(kind: Kind, cli: bool, desktop: bool, all: bool) -> bool {
    if all || (!cli && !desktop) {
        return true;
    }
    (kind == Kind::Cli && cli) || (kind == Kind::Desktop && desktop)
}

pub(crate) fn home_dir() -> PathBuf {
    crate::config::env_user_home().unwrap_or_default()
}

/// Set (to any value) by the test harness, `.config/nextest.toml` (T280): no test may find or
/// run an agent installed on the machine. rtok then sees only hosts under the home dir — a
/// test's own fakes: host CLIs resolve from PATH entries under it, absolute app paths are
/// looked up beneath it, and a host CLI is spawned by the path found, never by name.
const HOST_SANDBOX_ENV: &str = "RTOK_HOST_SANDBOX";

/// The home dir when [`HOST_SANDBOX_ENV`] is set.
fn host_sandbox() -> Option<PathBuf> {
    std::env::var_os(HOST_SANDBOX_ENV).map(|_| home_dir())
}

/// True under the test harness: a probe of the machine's real processes is off too.
pub(crate) fn host_sandboxed() -> bool {
    host_sandbox().is_some()
}

/// [`expand_spec`], re-rooted under the home dir when [`HOST_SANDBOX_ENV`] is set.
fn expand_app(spec: &str) -> PathBuf {
    let path = expand_spec(spec);
    match host_sandbox() {
        Some(home) => under(&home, path),
        None => path,
    }
}

/// `~/x` and `$VAR/x` as a path on this machine; anything else unchanged.
pub(crate) fn expand_spec(spec: &str) -> PathBuf {
    if let Some(rest) = spec.strip_prefix("~/") {
        return join_rel(&home_dir(), rest);
    }
    if let Some(rest) = spec.strip_prefix('$') {
        let (var, tail) = rest.split_once('/').unwrap_or((rest, ""));
        let Some(root) = std::env::var_os(var) else {
            return PathBuf::from(spec);
        };
        return join_rel(Path::new(&root), tail);
    }
    PathBuf::from(spec)
}

/// `path` when it is inside `home`, else the same path re-rooted beneath `home`
/// (`/Applications/X.app` → `<home>/Applications/X.app`; a drive prefix is dropped).
fn under(home: &Path, path: PathBuf) -> PathBuf {
    if crate::fs::path_starts_with(&path, home) {
        return path;
    }
    let rel: PathBuf = path
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .collect();
    home.join(rel)
}

/// Extensions `cmd` tries for a bare name. `PATHEXT` when set, otherwise cmd's own default.
fn pathext_list() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let raw = std::env::var("PATHEXT").ok();
    pathext_from(raw.as_deref())
}

fn pathext_from(raw: Option<&str>) -> Vec<String> {
    let raw = raw
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(".COM;.EXE;.BAT;.CMD");
    raw.split(';')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(|e| {
            if e.starts_with('.') {
                e.to_string()
            } else {
                format!(".{e}")
            }
        })
        .collect()
}

/// `bin`, then `bin` plus each extension that is not already on the name.
fn file_names(bin: &str, exts: &[String]) -> Vec<String> {
    let mut names = vec![bin.to_string()];
    let lower = bin.to_ascii_lowercase();
    let has = exts
        .iter()
        .any(|e| lower.ends_with(&e.to_ascii_lowercase()));
    if !has {
        for ext in exts {
            names.push(format!("{bin}.{}", ext.trim_start_matches('.')));
        }
    }
    names
}

/// The first file named `bin` (or `bin` + an extension from `exts`) in a `path` directory.
/// When `limit` is set, only directories under it count (ASCII-case-insensitive on Windows).
pub(crate) fn find_bin(
    bin: &str,
    path: Option<&std::ffi::OsStr>,
    limit: Option<&Path>,
    exts: &[String],
) -> Option<PathBuf> {
    let path = path?;
    let names = file_names(bin, exts);
    std::env::split_paths(path)
        .filter(|dir| limit.is_none_or(|home| crate::fs::path_starts_with(dir, home)))
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
}

/// The first `bin` on PATH. On Windows every `PATHEXT` extension is tried (`.exe`, `.cmd`,
/// `.bat`, …). Under [`HOST_SANDBOX_ENV`], only PATH entries under the home dir count.
pub(crate) fn find_on_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let sandbox = host_sandbox();
    find_bin(
        bin,
        Some(path.as_os_str()),
        sandbox.as_deref(),
        &pathext_list(),
    )
}

/// `path` when it exists (a file or an `.app` bundle). On Windows an extensionless path
/// also matches `name.exe` / `name.cmd` beside it (`~/.grok/bin/grok` → `grok.exe`).
pub(crate) fn probe_installed(path: PathBuf) -> Option<PathBuf> {
    if path.exists() {
        return Some(path);
    }
    let exts = pathext_list();
    let name = path.file_name()?.to_str()?;
    for candidate in file_names(name, &exts).into_iter().skip(1) {
        let next = path.with_file_name(candidate);
        if next.is_file() {
            return Some(next);
        }
    }
    None
}

/// What to spawn for host CLI `bin`: the name itself (or an explicit path), or under
/// [`HOST_SANDBOX_ENV`] the path [`find_on_path`] found — `None` when there is none.
fn host_program(bin: &str) -> Option<std::ffi::OsString> {
    if host_sandbox().is_none() || Path::new(bin).is_absolute() {
        return Some(bin.into());
    }
    find_on_path(bin).map(PathBuf::into_os_string)
}

/// Spawn a host CLI (`claude`, `codex`, …) by name. Windows CLIs installed through npm ship
/// as a `.cmd`/`.bat`/`.ps1` shim, not a `.exe` — `Command::new` only ever auto-appends `.exe`
/// (Win32's `CreateProcess`, never `PATHEXT`), so a bare spawn silently fails to find a real,
/// on-PATH shim. Routing through `cmd /C` there reuses the shell's own PATH + `PATHEXT`
/// search, which does try `.cmd`/`.bat` (T139).
pub(crate) fn spawn_cli(bin: &std::ffi::OsStr) -> std::process::Command {
    if cfg!(windows) {
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg(bin);
        cmd
    } else {
        std::process::Command::new(bin)
    }
}

/// Run one `<bin> <args>` call with stdin closed, optionally overriding one environment
/// variable (a config-dir redirect for a non-default settings path). Returns the first
/// stderr line — or the spawn error itself when the binary is not found — as the failure
/// text a host's plugin offer folds into "`<bin>` failed: …".
pub(crate) fn run_cli(
    bin: &str,
    args: &[&str],
    env: Option<(&str, &Path)>,
) -> std::result::Result<(), String> {
    let Some(program) = host_program(bin) else {
        return Err(format!("{bin}: not found"));
    };
    let mut cmd = spawn_cli(&program);
    cmd.args(args).stdin(std::process::Stdio::null());
    if let Some((key, val)) = env {
        cmd.env(key, val);
    }
    match cmd.output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr)
            .lines()
            .next()
            .unwrap_or("non-zero exit")
            .to_string()),
        Err(e) => Err(format!("{bin}: {e}")),
    }
}

/// Where the app is: a desktop bundle first, else the binary on PATH.
pub fn app_path(v: &Variant) -> Option<PathBuf> {
    v.apps
        .iter()
        .find_map(|a| probe_installed(expand_app(a)))
        .or_else(|| v.bins.iter().find_map(|b| find_on_path(b)))
}

/// How long one `<bin> --version` probe may run. `cursor --version` under an empty `$HOME`
/// with stdout on a pipe never prints and leaves a helper holding the pipe for ~5 minutes
/// (T280), so `Command::output` hung `agents list` and every test rendering a block.
const VERSION_PROBE_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);

/// Version of the installed app, or `"-"` when unknown.
/// Probes `<bin> --version` and keeps the first line that looks like a version (T168: some
/// wrappers, e.g. npm-installed CLIs, print noise like `Package extraction took 1234ms`
/// ahead of the real version line); falls back to the first non-empty line when no line
/// looks like a version. 32 chars max. Each probe gets [`VERSION_PROBE_LIMIT`] (T280).
pub fn app_version(v: &Variant) -> String {
    for bin in v.bins {
        let Some(program) = host_program(bin) else {
            continue;
        };
        let mut cmd = version_command(&program);
        cmd.arg("--version");
        let out = crate::proc::capture(cmd, Some(VERSION_PROBE_LIMIT));
        let Ok(out) = out else { continue };
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        if s.trim().is_empty() {
            s = String::from_utf8_lossy(&out.stderr).into_owned();
        }
        let mut fallback: Option<&str> = None;
        for raw in s.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if fallback.is_none() {
                fallback = Some(line);
            }
            if looks_like_a_version(line) {
                return line.chars().take(32).collect();
            }
        }
        if let Some(line) = fallback {
            return line.chars().take(32).collect();
        }
    }
    "-".into()
}

/// A digit, then later a dot, then later another digit — enough to tell a version
/// (`0.1.0 (fake copilot)`, `git version 2.43.0`) from wrapper noise (`Package extraction
/// took 1234ms`) with a small char scan, no regex dependency.
fn looks_like_a_version(line: &str) -> bool {
    let mut saw_digit = false;
    let mut saw_dot_after_digit = false;
    for c in line.chars() {
        if c.is_ascii_digit() {
            if saw_dot_after_digit {
                return true;
            }
            saw_digit = true;
        } else if c == '.' && saw_digit {
            saw_dot_after_digit = true;
        }
    }
    false
}

pub(crate) fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Read a TOML document at `path`, or start from an empty one when the file is simply
/// absent. An unreadable file (non-UTF-8 byte, wrong permissions) is an error, not an empty
/// document: swallowing it would let an installer overwrite a config it never actually read,
/// leaving only the `_backup/` copy as a way back.
pub(crate) fn load_toml(path: &Path) -> Result<DocumentMut> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| path.display().to_string()),
    };
    raw.parse::<DocumentMut>()
        .with_context(|| path.display().to_string())
}

/// True when the app itself is found: its bundle or binary exists. A config folder alone
/// does not count — one left behind by an uninstalled app made setup write into it (T426).
/// Setup refuses a missing app instead of creating its files; removal runs regardless so a
/// half-installed host is cleaned up.
pub fn present(v: &Variant) -> bool {
    app_path(v).is_some()
}

/// The named hosts none of whose wanted variants is [`present`]: `agents install|update`
/// refuses the whole run on them before any backup or write (T426).
fn absent_hosts(agents: &[&'static dyn Agent], want: impl Fn(Kind) -> bool) -> Vec<&'static str> {
    agents
        .iter()
        .filter(|a| !a.variants().iter().any(|v| want(v.kind) && present(v)))
        .map(|a| a.id())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModuleState {
    Installed,
    NotInstalled,
    NotSupported,
}

/// One [`MODULES`] row of a host variant: its state and the note printed after it — the flag
/// that would install it, or the reason it cannot be installed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct ModuleRow {
    pub name: &'static str,
    pub state: ModuleState,
    pub note: String,
}

/// [`Agent::installed`] with `mcp` read per surface through [`mcp::rows`] (T278) wherever the
/// host has an MCP entry of its own, so `list`, `info`, `update` and the installer's own
/// read-back all mean the same thing by "installed".
pub fn installed_modules(agent: &dyn Agent, kind: Kind, cfg: &Config) -> Vec<&'static str> {
    let mut found = agent.installed(cfg, kind);
    let rows = mcp::rows(agent, cfg, kind);
    if !rows.is_empty() {
        found.retain(|m| *m != "mcp");
        if rows.iter().any(mcp::McpRow::found) {
            found.push("mcp");
        }
    }
    found
}

/// A module found in the host's files counts as installed even where setup cannot write it
/// (a hand-added MCP entry).
pub fn module_rows(agent: &dyn Agent, kind: Kind, cfg: &Config) -> Vec<ModuleRow> {
    let found = installed_modules(agent, kind, cfg);
    MODULES
        .iter()
        .map(|&name| {
            let support = agent.support(kind, name);
            let (state, note) = if found.contains(&name) {
                (ModuleState::Installed, String::new())
            } else {
                match support {
                    Support::Yes => (ModuleState::NotInstalled, String::new()),
                    Support::Flag(flag) | Support::Offer(flag) => {
                        (ModuleState::NotInstalled, format!(" ({flag})"))
                    }
                    Support::No(why) => (ModuleState::NotSupported, format!(": {why}")),
                }
            };
            ModuleRow { name, state, note }
        })
        .collect()
}

impl ModuleState {
    fn mark(self) -> &'static str {
        match self {
            ModuleState::Installed => "✓ ",
            ModuleState::NotInstalled => "✗ ",
            ModuleState::NotSupported => "− ",
        }
    }

    fn word(self) -> &'static str {
        match self {
            ModuleState::Installed => "installed",
            ModuleState::NotInstalled => "not installed",
            ModuleState::NotSupported => "not supported",
        }
    }

    /// `{indent}{mark}{text}` — green, red `✗` for not installed, grey `−` for not supported,
    /// coloured only where stdout takes colour (see `render`). `console = false` drops marks
    /// and colour: the doctor text also lands in the PDF report, whose built-in font has no
    /// `✓`.
    fn line(self, indent: &str, text: &str, console: bool) -> String {
        use owo_colors::{OwoColorize, Stream};
        if !console {
            return format!("{indent}{text}\n");
        }
        let line = format!("{indent}{}{text}", self.mark());
        let line = match self {
            ModuleState::Installed => line
                .if_supports_color(Stream::Stdout, |t| t.green())
                .to_string(),
            ModuleState::NotInstalled => line
                .if_supports_color(Stream::Stdout, |t| t.red())
                .to_string(),
            ModuleState::NotSupported => line
                .if_supports_color(Stream::Stdout, |t| t.bright_black())
                .to_string(),
        };
        format!("{line}\n")
    }
}

/// `✓ hooks   installed`, one line per module row.
pub fn module_lines(rows: &[ModuleRow], indent: &str, console: bool) -> String {
    rows.iter()
        .map(|row| {
            let text = format!("{:<7} {}{}", row.name, row.state.word(), row.note);
            row.state.line(indent, &text, console)
        })
        .collect()
}

/// The modules an install should leave behind: every `Yes` module (`mcp` only with
/// `[setup] mcp`), and a flag module only when its flag was given. An `Offer` module is
/// guidance, never expected.
pub fn expected(agent: &dyn Agent, kind: Kind, cfg: &Config) -> Vec<&'static str> {
    MODULES
        .iter()
        .copied()
        .filter(|m| match agent.support(kind, m) {
            Support::Yes => *m != "mcp" || cfg.setup.mcp,
            Support::Flag("--proxy") => cfg.setup.proxy,
            Support::Flag("--yes") => cfg.setup.yes,
            Support::Flag(_) | Support::Offer(_) | Support::No(_) => false,
        })
        .collect()
}

/// Expected modules that do not read back from the host's files: the installer checking its
/// own write (a host that rewrote the file, a marker the reader does not recognise).
pub fn missing(agent: &dyn Agent, kind: Kind, cfg: &Config) -> Vec<&'static str> {
    let have = installed_modules(agent, kind, cfg);
    expected(agent, kind, cfg)
        .into_iter()
        .filter(|m| !have.contains(m))
        .collect()
}

/// True when the `rtok` the configs spawn resolves: bare on PATH, or written absolute.
fn rtok_spawns() -> bool {
    bare_rtok_on_path(std::env::var_os("PATH").as_deref()) || rtok_command() != "rtok"
}

/// One of rtok's own plugins as a host variant reaches it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PluginRow {
    pub id: &'static str,
    /// `[plugins.<id>] enabled`; printed as `(off)` when false.
    pub on: bool,
    pub state: ModuleState,
}

/// The modules that carry a plugin surface into a host: hooks carry `hook` and the bash
/// call path (`cli`), MCP carries `mcp`, the proxy carries `proxy`, and the linked plugin
/// carries whatever [`Agent::plugin_surfaces`] says it serves.
fn modules_for(agent: &dyn Agent, surface: Surface) -> Vec<&'static str> {
    let mut out = match surface {
        Surface::Hook | Surface::Cli => vec!["hooks"],
        Surface::Mcp => vec!["mcp"],
        Surface::Proxy => vec!["proxy"],
    };
    if agent.plugin_surfaces().contains(&surface) {
        out.push("plugin");
    }
    out
}

/// True when at least one of the plugin's surfaces has a module setup can write here.
pub fn reaches(agent: &dyn Agent, kind: Kind, surfaces: &[Surface]) -> bool {
    surfaces.iter().any(|&s| {
        modules_for(agent, s)
            .iter()
            .any(|m| !matches!(agent.support(kind, m), Support::No(_)))
    })
}

/// Every catalogue plugin grouped like the modules: installed when one of its surfaces rides
/// an installed module, not installed when one could, not supported otherwise.
pub fn plugin_rows(agent: &dyn Agent, kind: Kind, cfg: &Config) -> Vec<PluginRow> {
    let modules = module_rows(agent, kind, cfg);
    let state_of = |name: &str| modules.iter().find(|r| r.name == name).map(|r| r.state);
    crate::plugins::Registry::new(cfg)
        .manifests()
        .into_iter()
        .map(|(m, on)| {
            let states: Vec<ModuleState> = m
                .surfaces
                .iter()
                .flat_map(|&s| modules_for(agent, s))
                .filter_map(state_of)
                .collect();
            let state = if states.contains(&ModuleState::Installed) {
                ModuleState::Installed
            } else if states.contains(&ModuleState::NotInstalled) {
                ModuleState::NotInstalled
            } else {
                ModuleState::NotSupported
            };
            PluginRow {
                id: m.id,
                on,
                state,
            }
        })
        .collect()
}

/// ```text
///   plugins
///     ✓ installed      cmd, read, inject
///     ✗ not installed  proxy, compress (off)
///     − not supported  -
/// ```
/// An empty not-installed group is omitted: a lone `-` is not a missing install.
pub fn plugin_lines(rows: &[PluginRow], indent: &str, console: bool) -> String {
    let mut out = format!("{indent}plugins\n");
    let inner = format!("{indent}  ");
    for state in [
        ModuleState::Installed,
        ModuleState::NotInstalled,
        ModuleState::NotSupported,
    ] {
        let ids: Vec<String> = rows
            .iter()
            .filter(|r| r.state == state)
            .map(|r| {
                if r.on {
                    r.id.to_string()
                } else {
                    format!("{} (off)", r.id)
                }
            })
            .collect();
        // A dash under "not installed" reads as a failure when nothing is waiting.
        if ids.is_empty() && state == ModuleState::NotInstalled {
            continue;
        }
        let ids = if ids.is_empty() {
            "-".to_string()
        } else {
            ids.join(", ")
        };
        out.push_str(&state.line(&inner, &format!("{:<14} {ids}", state.word()), console));
    }
    out
}

/// What a block reports about its variant.
pub enum Outcome<'a> {
    /// The app is not on this machine: header, app and config lines only.
    NotFound,
    /// `agents list`: state, no installer ran.
    Listed,
    /// The installer ran with these step reports.
    Applied { mode: Mode, reports: &'a [String] },
    /// A shared-config host already applied under this sibling variant.
    Shared(&'static str),
    /// `agents update` found no rtok module in this variant and wrote nothing (T242.2).
    NotInstalled,
}

/// The block one host variant prints:
///
/// ```text
/// CLI: Codex — already installed
///   app     /opt/homebrew/bin/codex (codex-cli 0.40.0)
///   config  /Users/me/.codex/config.toml
///   ✓ mcp     installed
///   ✗ proxy   not installed (--proxy)
///   − hooks   not supported: Codex has no shell hooks
/// ```
pub fn block(agent: &dyn Agent, v: &Variant, cfg: &Config, outcome: Outcome) -> String {
    let note = match &outcome {
        Outcome::NotFound => " — not found",
        Outcome::Listed => "",
        Outcome::Applied { .. } if cfg.setup.dry_run => " — dry run, nothing written",
        Outcome::Applied { mode, reports } if reports.iter().all(|r| r == NO_CHANGES) => match mode
        {
            Mode::Remove => " — no changes",
            Mode::Update => " — already current",
            _ => " — already installed",
        },
        Outcome::Applied { .. } => "",
        Outcome::Shared(_) => " — same files as above",
        Outcome::NotInstalled => " — not installed",
    };
    let header = format!("{}: {}{note}", v.kind.label(), v.name);
    // An install/update/remove result carries its operation's icon on a terminal; every other
    // block, and any pipe, keeps the bare line the snapshots pin.
    let mut out = match &outcome {
        Outcome::Applied { mode, reports } => {
            let kind = if cfg.setup.dry_run || reports.iter().all(|r| r == NO_CHANGES) {
                style::Kind::Info
            } else {
                style::Kind::Success
            };
            style::line_op(mode.verb(), kind, owo_colors::Stream::Stdout, &header)
        }
        _ => header,
    };
    out.push('\n');
    if matches!(outcome, Outcome::NotInstalled) {
        out.push_str(&format!(
            "  skip    nothing of rtok here; run `rtok agents install {}`\n",
            agent.id()
        ));
        return out;
    }
    match app_path(v) {
        Some(p) => out.push_str(&format!("  app     {} ({})\n", p.display(), app_version(v))),
        None => out.push_str("  app     -\n"),
    }
    let files = agent.files(cfg, v.kind);
    if !files.is_empty() {
        let files: Vec<String> = files.iter().map(|p| p.display().to_string()).collect();
        out.push_str(&format!("  config  {}\n", files.join(", ")));
    }
    if matches!(outcome, Outcome::NotFound) {
        return out;
    }
    if let Outcome::Applied { reports, .. } = &outcome {
        // Steps that changed something print their `+`/`-` lines in diff colours (T12.6).
        let changed: Vec<&str> = reports
            .iter()
            .filter(|r| *r != NO_CHANGES)
            .map(String::as_str)
            .collect();
        if !changed.is_empty() {
            let text = changed.join("\n");
            let shown = if cfg.setup.dry_run {
                // The step lines carry no path, so the stat row is the host's one config file,
                // or the host itself when it writes several.
                let at = match files.as_slice() {
                    [one] => one.clone(),
                    _ => std::path::PathBuf::from(format!("{}: {}", v.kind.label(), v.name)),
                };
                let shown = if crate::stat_preview::on() {
                    String::new()
                } else {
                    change_preview::paint(&text)
                };
                crate::stat_preview::show(&[(at, text)], &shown)
            } else {
                change_preview::paint(&text)
            };
            if !shown.is_empty() {
                out.push_str(&shown);
                out.push('\n');
            }
        }
    }
    // T278: the `mcp` row becomes one line per surface wherever the host has an entry.
    let surfaces = mcp::rows(agent, cfg, v.kind);
    for mut row in module_rows(agent, v.kind, cfg) {
        if row.name == "plugin"
            && row.state == ModuleState::Installed
            && let Some(status) = plugin_status(agent, v.kind, cfg)
        {
            row.note = format!(" {}", status.note);
        }
        if row.name == "mcp" && !surfaces.is_empty() {
            out.push_str(&mcp::lines(&surfaces, "  "));
        } else {
            out.push_str(&module_lines(std::slice::from_ref(&row), "  ", true));
        }
    }
    out.push_str(&plugin_lines(&plugin_rows(agent, v.kind, cfg), "  ", true));
    out
}

/// What `rtok agents install|remove` was asked to do.
pub struct Request {
    pub hosts: Vec<String>,
    pub mode: Mode,
    pub cli: bool,
    pub desktop: bool,
    pub all: bool,
}

/// Every host named, or the first unknown name — checked before any backup is taken.
pub fn resolve(hosts: &[String]) -> Result<Vec<&'static dyn Agent>> {
    if hosts.is_empty() {
        bail!("no host given");
    }
    hosts
        .iter()
        .map(|h| host(h).ok_or_else(|| anyhow::anyhow!("unknown host: {h}")))
        .collect()
}

/// Back up, then install into (or remove from) every wanted variant of every host, one
/// [`block`] each. The copy is taken up front, before any installer runs, so one `.bak-<ts>`
/// per file holds the host exactly as it was — not as it was midway through a multi-file
/// edit; the installers therefore must not take one of their own (`cfg.setup.backup` is
/// cleared here). A run that then wrote nothing removes the copies it took: `already
/// installed` leaves the directory as it found it.
pub fn run(cfg: &mut Config, req: &Request) -> Result<String> {
    let agents = resolve(&req.hosts)?;
    let want = |kind: Kind| req.mode == Mode::Remove || wants(kind, req.cli, req.desktop, req.all);
    if req.mode != Mode::Remove {
        let absent = absent_hosts(&agents, want);
        if !absent.is_empty() {
            bail!("{} is not installed", absent.join(", "));
        }
    }
    let mut out = String::new();
    if req.mode != Mode::Remove && !rtok_spawns() {
        out.push_str(
            "warning: rtok is not on PATH; the hooks and MCP entries spawn `rtok` by name and will fail until it is\n",
        );
    }
    if req.mode != Mode::Remove && !cfg.setup.dry_run {
        ensure_hook_client_link_here();
    }
    let mut taken: Vec<PathBuf> = Vec::new();
    if !cfg.setup.dry_run && cfg.setup.backup {
        let mut seen: Vec<PathBuf> = Vec::new();
        for a in &agents {
            for v in a.variants().iter().filter(|v| want(v.kind)) {
                for path in a.files(cfg, v.kind) {
                    if seen.contains(&path) {
                        continue;
                    }
                    if let Some(bak) = rtok_agent_sdk::backup(&path, 0)? {
                        taken.push(bak);
                    }
                    seen.push(path);
                }
            }
        }
        cfg.setup.backup = false;
    }
    let (blocks, changed) = apply_all(cfg, req, &agents, want)?;
    if changed {
        // Pruned only now: a no-change run deletes its copies, which would cost a generation.
        for bak in &taken {
            rtok_agent_sdk::prune_backups(bak, cfg.setup.backup_files as usize);
            out.push_str(&format!("backup {}\n", bak.display()));
        }
    } else {
        for bak in &taken {
            let _ = std::fs::remove_file(bak);
        }
    }
    out.push_str(&blocks);
    Ok(out)
}

/// Dry-run one host's plan without writing anything (T141): true when it would touch a file.
/// Reuses [`apply_all`] exactly as [`run`] does, only forced into `dry_run` on a throwaway
/// config clone — the restart orchestration uses this to decide whether a running desktop
/// app is worth quitting before the real write.
pub(crate) fn would_change(cfg: &Config, agent: &'static dyn Agent, req: &Request) -> Result<bool> {
    let mut dry = cfg.clone();
    dry.setup.dry_run = true;
    dry.setup.backup = false;
    let want = |kind: Kind| req.mode == Mode::Remove || wants(kind, req.cli, req.desktop, req.all);
    let (_, changed) = apply_all(&dry, req, &[agent], want)?;
    Ok(changed)
}

/// The blocks of every wanted variant, and whether any step changed a file.
pub(crate) fn apply_all(
    cfg: &Config,
    req: &Request,
    agents: &[&'static dyn Agent],
    want: impl Fn(Kind) -> bool,
) -> Result<(String, bool)> {
    let mut out = String::new();
    let mut changed = false;
    for &a in agents {
        let mut done: Option<&'static str> = None;
        let mut any = false;
        for v in a.variants().iter().filter(|v| want(v.kind)) {
            any = true;
            if req.mode != Mode::Remove && !present(v) {
                out.push_str(&block(a, v, cfg, Outcome::NotFound));
                continue;
            }
            if let (Some(first), true) = (done, a.shared()) {
                out.push_str(&block(a, v, cfg, Outcome::Shared(first)));
                continue;
            }
            let carried;
            let cfg = if req.mode == Mode::Update {
                let have = installed_modules(a, v.kind, cfg);
                if have.is_empty() {
                    out.push_str(&block(a, v, cfg, Outcome::NotInstalled));
                    continue;
                }
                carried = carry_flags(cfg, &have);
                &carried
            } else {
                cfg
            };
            let reports = a.apply(cfg, v.kind, req.mode)?;
            done = Some(v.name);
            changed |= reports.iter().any(|r| r != NO_CHANGES);
            out.push_str(&block(
                a,
                v,
                cfg,
                Outcome::Applied {
                    mode: req.mode,
                    reports: &reports,
                },
            ));
            if matches!(req.mode, Mode::Install | Mode::Update) && !cfg.setup.dry_run {
                for m in missing(a, v.kind, cfg) {
                    out.push_str(&format!("  warning: {m} did not read back as installed\n"));
                }
            }
        }
        if !any {
            out.push_str(&format!("skip {}: not selected\n", a.id()));
        }
    }
    Ok((out, changed))
}

/// The config `agents update` installs a variant with (T242.2): the flag modules it already
/// carries switched on — a proxy read back keeps `--proxy`, an installed plugin keeps `--yes`
/// — so a changed port or binary lands and nothing the user never chose appears.
fn carry_flags(cfg: &Config, have: &[&str]) -> Config {
    let mut c = cfg.clone();
    c.setup.proxy |= have.contains(&"proxy");
    c.setup.yes |= have.contains(&"plugin");
    c
}

pub use outdated::{
    EXIT_OUTDATED, Outdated, OutdatedReport, OutdatedSelection, PluginStatus, outdated,
    plugin_status, print_human, report,
};

/// `rtok agents update` with no host named: every host with an rtok module in at least one
/// variant on this machine, in [`HOSTS`] order.
pub fn installed_hosts(cfg: &Config) -> Vec<String> {
    HOSTS
        .iter()
        .filter(|&&id| {
            host(id).is_some_and(|a| {
                a.variants()
                    .iter()
                    .any(|v| present(v) && !installed_modules(a, v.kind, cfg).is_empty())
            })
        })
        .map(ToString::to_string)
        .collect()
}

/// Walk each requested host's variants. Hosts run in parallel; output stays in `ids` order.
pub fn visit_hosts<T: Send>(ids: &[&str], f: impl Fn(&dyn Agent, &Variant) -> T + Sync) -> Vec<T> {
    std::thread::scope(|scope| {
        let mut joins = Vec::with_capacity(ids.len());
        for &id in ids {
            let Some(a) = host(id) else { continue };
            let f = &f;
            joins.push(
                scope.spawn(move || a.variants().iter().map(|v| f(a, v)).collect::<Vec<_>>()),
            );
        }
        joins
            .into_iter()
            .flat_map(|j| j.join().expect("host listing thread"))
            .collect()
    })
}

/// `rtok agents list`: every known host × variant as a [`block`], nothing written.
pub fn list(cfg: &Config) -> String {
    list_ids(cfg, HOSTS)
}

/// Same blocks as [`list`], only for `ids` (already-resolved host ids).
pub fn list_ids(cfg: &Config, ids: &[&str]) -> String {
    visit_hosts(ids, |a, v| {
        let outcome = if present(v) {
            Outcome::Listed
        } else {
            Outcome::NotFound
        };
        block(a, v, cfg, outcome)
    })
    .concat()
}

/// `rtok agents info <host>`: [`list`] filtered to the named host(s).
pub fn info(cfg: &Config, hosts: &[String]) -> Result<String> {
    let agents = resolve(hosts)?;
    let ids: Vec<&str> = agents.iter().map(|a| a.id()).collect();
    Ok(list_ids(cfg, &ids))
}

/// The `[setup]` flags an installer acts on, as the SDK spells them.
pub(crate) fn apply(cfg: &crate::config::Config) -> rtok_agent_sdk::Apply {
    rtok_agent_sdk::Apply {
        dry_run: cfg.setup.dry_run,
        backup: cfg.setup.backup,
        backup_files: cfg.setup.backup_files as usize,
        yes: cfg.setup.yes,
    }
}

/// Drop `<key>.<name>` from a host's JSON config only as far as rtok wrote it: `ours` is the
/// entry the host's installer writes (T246, [`rtok_agent_sdk::unregister_owned`]).
pub(crate) fn unregister_ours(
    cfg: &crate::config::Config,
    path: &std::path::Path,
    key: &str,
    name: &str,
    ours: &serde_json::Value,
) -> Result<String> {
    rtok_agent_sdk::unregister_owned(&apply(cfg), path, key, name, ours, is_rtok_bin)
}

/// Whether a host's `strip_ours` takes an rtok hook it found (T246.3, T246.6): one still as
/// the installer writes it (`unchanged`) goes; one the user changed goes only as
/// [`rtok_agent_sdk::keep_edited`] decides, its `leave`/`?` line (naming `at`) joining `kept`.
pub(crate) fn takes_hook(
    apply: &rtok_agent_sdk::Apply,
    unchanged: bool,
    at: impl FnOnce() -> String,
    kept: &mut Vec<String>,
) -> bool {
    if unchanged {
        return true;
    }
    match rtok_agent_sdk::keep_edited(apply, &at()) {
        Some(line) => {
            kept.push(line);
            false
        }
        None => true,
    }
}

/// `{n} removed`, or [`NO_CHANGES`] for none — the tail of a `strip_ours` report.
pub(crate) fn removed_report(removed: usize) -> String {
    if removed == 0 {
        NO_CHANGES.into()
    } else {
        format!("{removed} removed")
    }
}

/// A `strip_ours` report: the `kept` lines, then `report` unless it is [`NO_CHANGES`].
pub(crate) fn with_kept(mut kept: Vec<String>, report: String) -> String {
    if report != NO_CHANGES {
        kept.push(report);
    }
    if kept.is_empty() {
        NO_CHANGES.into()
    } else {
        kept.join("\n")
    }
}

/// [`unregister_ours`] for the `mcpServers` entry [`rtok_agent_sdk::register_mcp`] writes.
pub(crate) fn unregister_mcp_ours(
    cfg: &crate::config::Config,
    path: &std::path::Path,
    name: &str,
) -> Result<String> {
    let ours = rtok_agent_sdk::mcp_entry("rtok", &["mcp"]);
    unregister_ours(cfg, path, "mcpServers", name, &ours)
}

/// [`Agent::apply`] for a host whose install is exactly "write the hook, then register or
/// unregister the MCP server" — the shape every hooks+MCP host beyond the first repeats
/// verbatim (T185).
pub(crate) fn apply_hook_and_mcp(
    cfg: &Config,
    remove: bool,
    hook: impl FnOnce(&Config, bool) -> Result<String>,
    register_mcp: impl FnOnce(&Config) -> Result<String>,
    unregister_mcp: impl FnOnce(&Config) -> Result<String>,
) -> Result<Vec<String>> {
    let mut lines = vec![hook(cfg, remove)?];
    let mcp_line = if remove {
        Some(unregister_mcp(cfg)?)
    } else if cfg.setup.mcp {
        Some(register_mcp(cfg)?)
    } else {
        None
    };
    lines.extend(mcp_line);
    Ok(lines)
}

/// Register `rtok mcp` under `<key>.rtok` as `{type: "local", command: [..], enabled: true}` —
/// the local-server shape OpenCode and its MiMo Code fork both read (T186 confirmed the fork
/// kept it verbatim). One body for both hosts keeps `just dup` from flagging the near-clone.
pub(crate) fn register_local_mcp(
    cfg: &Config,
    path: &std::path::Path,
    key: &str,
    host: &'static str,
) -> Result<String> {
    let cmd = rtok_command();
    let entry = local_mcp_entry(&cmd, host);
    rtok_agent_sdk::register_server(
        &apply(cfg),
        path,
        key,
        "rtok",
        entry,
        &mcp_summary(&cmd, host),
    )
}

pub(crate) fn local_mcp_entry(cmd: &str, host: &'static str) -> serde_json::Value {
    let [sub, flag, id] = mcp_args(host);
    json!({"type": "local", "command": [cmd, sub, flag, id], "enabled": true})
}

/// What follows the binary in a host's `rtok mcp` entry: `mcp --host <id>` (T283.2), so the MCP
/// process knows which host started it without `[hook] host`. One place, so no host spells it
/// by hand; removal ignores the pair (see `rtok_agent_sdk::judge_owned`), so an entry written
/// without it is still rtok's own.
pub(crate) const fn mcp_args(host: &'static str) -> [&'static str; 3] {
    ["mcp", "--host", host]
}

/// The report text after `<key>.rtok: ` for a host's entry.
pub(crate) fn mcp_summary(cmd: &str, host: &'static str) -> String {
    format!("{cmd} {}", mcp_args(host).join(" "))
}

/// [`register_local_mcp`]'s remove: only the entry as rtok wrote it (T246.2).
pub(crate) fn unregister_local_mcp(
    cfg: &Config,
    path: &std::path::Path,
    key: &str,
    host: &'static str,
) -> Result<String> {
    unregister_ours(cfg, path, key, "rtok", &local_mcp_entry("rtok", host))
}

/// `mcpServers.rtok = {command, args}` with no `type` — the stdio shape Cline, its Roo Code
/// fork, and Windsurf all document. One body so a new host does not copy the JSON.
pub(crate) fn register_stdio_mcp(
    cfg: &Config,
    path: &std::path::Path,
    host: &'static str,
) -> Result<String> {
    let cmd = rtok_command();
    rtok_agent_sdk::register_server(
        &apply(cfg),
        path,
        "mcpServers",
        "rtok",
        stdio_mcp_entry(&cmd, host),
        &mcp_summary(&cmd, host),
    )
}

fn stdio_mcp_entry(cmd: &str, host: &'static str) -> serde_json::Value {
    json!({"command": cmd, "args": mcp_args(host)})
}

/// [`register_stdio_mcp`]'s remove: only the entry as rtok wrote it (T246.2).
pub(crate) fn unregister_stdio_mcp(
    cfg: &Config,
    path: &std::path::Path,
    host: &'static str,
) -> Result<String> {
    unregister_ours(
        cfg,
        path,
        "mcpServers",
        "rtok",
        &stdio_mcp_entry("rtok", host),
    )
}

/// `Agent::installed` for a host whose only module is `mcp`: present iff `path` mentions
/// `"rtok"`. Shared by every MCP-only host (T186) instead of each repeating the same
/// contains-check.
pub(crate) fn installed_mcp_only(path: &std::path::Path) -> Vec<&'static str> {
    if read(path).contains("\"rtok\"") {
        vec!["mcp"]
    } else {
        Vec::new()
    }
}

/// `Agent::apply` for a host whose only module is `mcp`: register/unregister and nothing
/// else to run. Shared by every MCP-only host (T186) instead of each repeating the same
/// remove/register-if-enabled/else-no_changes branch.
pub(crate) fn apply_mcp_only(
    cfg: &Config,
    mode: Mode,
    register: impl FnOnce(&Config) -> Result<String>,
    unregister: impl FnOnce(&Config) -> Result<String>,
) -> Result<Vec<String>> {
    if mode == Mode::Remove {
        Ok(vec![unregister(cfg)?])
    } else if cfg.setup.mcp {
        Ok(vec![register(cfg)?])
    } else {
        Ok(vec![rtok_agent_sdk::NO_CHANGES.into()])
    }
}

/// `Agent::support` for a host whose only module is `mcp`: the mcp/hooks/proxy/plugin
/// four-way match every MCP-only host repeats, with just the No-reasons varying. Shared
/// (T186) instead of each restating the same branch shape.
pub(crate) fn support_mcp_only(
    module: &str,
    hooks_reason: &'static str,
    proxy_reason: &'static str,
    plugin_reason: &'static str,
) -> Support {
    match module {
        "mcp" => Support::Yes,
        "hooks" => Support::No(hooks_reason),
        "proxy" => Support::No(proxy_reason),
        _ => Support::No(plugin_reason),
    }
}

/// True when some file at `dir/<manifest>` parses as JSON naming `name` in a top-level
/// `"name"` field — the plugin-store marker every local-path plugin host (Copilot, Gemini,
/// …) reads back with, never rtok's own to write (D21: `installed()` trusts the host's own
/// record, not a file this module created).
pub(crate) fn manifest_names(dir: &Path, manifest: &str, name: &str) -> bool {
    std::fs::read_to_string(dir.join(manifest))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v.get("name").and_then(serde_json::Value::as_str) == Some(name))
}

/// A host's local-path plugin verbs for [`offer_plugin`] — Copilot's `plugin install
/// <path>`/`plugin uninstall <name>`, Gemini's `extensions link <path>`/`extensions
/// uninstall <name>`, and any future host shaped the same way.
pub(crate) struct PluginOffer<'a> {
    pub bin: &'a str,
    pub name: &'a str,
    pub src_rel: &'a str,
    pub install_verb: &'a [&'a str],
    pub uninstall_verb: &'a [&'a str],
}

/// Offer, install, or uninstall the plugin tree at `plugin_src(offer.src_rel)` through a host
/// CLI's local-path install/uninstall verbs (D21, `Support::Flag("--yes")`). `installed` is
/// the caller's own marker read (never written here — that store is the host's). Dry-run
/// names the resolved line and writes nothing; `call` spawns the host binary, and a
/// spawn/non-zero failure folds into "offer … (`<bin>` failed: …)" instead of failing the
/// caller's whole `apply`.
pub(crate) fn offer_plugin(
    cfg: &Config,
    remove: bool,
    installed: bool,
    offer: PluginOffer,
    call: impl FnOnce(&[&str]) -> std::result::Result<(), String>,
) -> Result<String> {
    let PluginOffer {
        bin,
        name,
        src_rel,
        install_verb,
        uninstall_verb,
    } = offer;
    let a = apply(cfg);
    if remove {
        if !installed {
            return Ok(NO_CHANGES.into());
        }
    } else if installed || !a.yes {
        return Ok(NO_CHANGES.into());
    }
    let src = plugin_src(src_rel);
    let shown = if remove {
        format!("{bin} {} {name}", uninstall_verb.join(" "))
    } else {
        format!("{bin} {} {}", install_verb.join(" "), src.display())
    };
    if a.dry_run {
        return Ok(if remove {
            format!("- plugin {name} ({shown})")
        } else {
            format!(
                "offer {src_rel} → {shown} {}",
                rtok_agent_sdk::KETCH_INSTALL
            )
        });
    }
    let src_str = src.to_str().unwrap_or_default();
    let args: Vec<&str> = if remove {
        uninstall_verb.iter().copied().chain([name]).collect()
    } else {
        install_verb.iter().copied().chain([src_str]).collect()
    };
    match call(&args) {
        Ok(()) => Ok(if remove {
            format!("- plugin {name}")
        } else {
            format!("+ plugin {src_rel} → {name}")
        }),
        Err(e) => Ok(format!("offer {src_rel} → {shown} ({bin} failed: {e})")),
    }
}

/// A plugin whose store only the host writes (T86, `Support::Offer("--yes")`: Kimi's
/// `/plugins install`, Antigravity CLI's `agy plugin install`). rtok prints `install` behind
/// `--yes` — dry-run and apply alike — and never runs it; the flag never turns the line into
/// state (`installed` is the caller's own marker read). Remove leaves a staged copy alone and
/// says so with `keep`.
pub(crate) fn print_offer(
    cfg: &Config,
    remove: bool,
    installed: bool,
    src_rel: &str,
    install: &str,
    keep: &str,
) -> String {
    let a = apply(cfg);
    if remove && !a.dry_run {
        return if installed { keep } else { NO_CHANGES }.into();
    }
    if !a.yes {
        return NO_CHANGES.into();
    }
    format!(
        "offer {src_rel} → {install} {}",
        rtok_agent_sdk::KETCH_INSTALL
    )
}

/// The per-host fn items [`d21_plugin_apply`] threads through: Copilot's and Gemini's own
/// `plugin`, `plugin_installed`, `run`, `register_mcp`, `unregister_mcp` already match these
/// signatures exactly, so a host passes them in by name.
pub(crate) struct D21Plugin {
    pub offer: fn(&Config, bool) -> Result<String>,
    pub plugin_installed: fn(&Config) -> bool,
    pub run: fn(&Config, bool) -> Result<String>,
    pub register_mcp: fn(&Config) -> Result<String>,
    pub unregister_mcp: fn(&Config) -> Result<String>,
}

/// `apply()` skeleton for a host whose local-path plugin (via [`offer_plugin`]) is a D21
/// singleton over hooks only: while it is installed, or on removal, `run`'s own hook document
/// is taken back instead of written — on the very call that installs the plugin too — else the
/// plain hook document goes in. MCP is no longer part of that unit (T275 decision D33): the
/// host's own `mcpServers.rtok` is written on every install/update behind `[setup] mcp`
/// regardless of plugin state, and only a remove takes it back — a plugin no longer suppresses
/// or strips it. `extra` appends a per-host tail line in every case ([`no_extra`] for none,
/// Copilot's skill sync for one); shared by Copilot and Gemini (D21 for hooks, D33 for MCP).
pub(crate) fn d21_plugin_apply(
    cfg: &Config,
    mode: Mode,
    ops: D21Plugin,
    extra: fn(&Config, bool) -> Result<Option<String>>,
) -> Result<Vec<String>> {
    let D21Plugin {
        offer,
        plugin_installed,
        run,
        register_mcp,
        unregister_mcp,
    } = ops;
    let remove = mode == Mode::Remove;
    let head = offer(cfg, remove)?;
    let hook_line = run(cfg, remove || plugin_installed(cfg))?;
    let mcp_line = if remove {
        Some(unregister_mcp(cfg)?)
    } else if cfg.setup.mcp {
        Some(register_mcp(cfg)?)
    } else {
        None
    };
    let mut lines = vec![head, hook_line];
    lines.extend(mcp_line);
    if let Some(e) = extra(cfg, remove)? {
        lines.push(e);
    }
    Ok(lines)
}

/// The `extra` no-op for [`d21_plugin_apply`] on a host with no per-host tail line (Gemini).
pub(crate) fn no_extra(_cfg: &Config, _remove: bool) -> Result<Option<String>> {
    Ok(None)
}

/// The `ANTHROPIC_BASE_URL` `agent setup claude --proxy` writes, and the one it reads back.
pub(crate) fn anthropic_proxy_url(cfg: &crate::config::Config) -> String {
    format!("http://{}:{}", cfg.proxy.bind, cfg.proxy.port)
}

pub(crate) fn openai_proxy_url(cfg: &crate::config::Config) -> String {
    format!("http://{}:{}/v1", cfg.proxy.bind, cfg.proxy.port)
}

/// Join `rel` onto `base` by path components so Windows never gets a single
/// component with embedded slashes (`plugins/cursor` → `plugins\cursor`).
fn join_rel(base: &std::path::Path, rel: &str) -> std::path::PathBuf {
    let mut out = base.to_path_buf();
    for part in rel.split(['/', '\\']).filter(|s| !s.is_empty()) {
        out.push(part);
    }
    out
}

/// Command string written into host configs for hooks and MCP.
///
/// Prefer bare `rtok` when it resolves on PATH. On Windows, a fresh install
/// often updates the user PATH while the host still has the old one — bare
/// `rtok` then fails to spawn. Fall back to the absolute `current_exe`
/// (typically `…\rtok.exe`) so Claude/Cursor/Codex can still start it.
pub(crate) fn rtok_command() -> String {
    resolve_rtok_command(
        std::env::current_exe().ok().as_deref(),
        std::env::var_os("PATH").as_deref(),
    )
}

/// Pure resolution used by [`rtok_command`] and unit tests.
pub(crate) fn resolve_rtok_command(
    exe: Option<&std::path::Path>,
    path_os: Option<&std::ffi::OsStr>,
) -> String {
    if bare_rtok_on_path(path_os) {
        return "rtok".to_string();
    }
    // Non-Windows hosts keep the bare name even when PATH lookup fails: shell
    // hooks expect `rtok` and absolute paths are a Windows spawn edge.
    if !cfg!(windows) {
        return "rtok".to_string();
    }
    if let Some(exe) = exe {
        return dunce::simplified(exe).to_string_lossy().into_owned();
    }
    "rtok".to_string()
}

fn bare_rtok_on_path(path: Option<&std::ffi::OsStr>) -> bool {
    bin_on_path("rtok", path)
}

/// True when `bin` (and, on Windows, `bin` plus `PATHEXT`) is a file in a `path` directory.
/// No sandbox: the caller passes the `PATH` value, so a test controls it.
pub(crate) fn bin_on_path(bin: &str, path: Option<&std::ffi::OsStr>) -> bool {
    find_bin(bin, path, None, &pathext_list()).is_some()
}

/// How to run `program --version`. A bare name or a `.cmd`/`.bat` on Windows goes through
/// `cmd /C`, because `CreateProcess` only auto-appends `.exe` and cannot execute a batch shim.
fn version_command(program: &std::ffi::OsStr) -> std::process::Command {
    if shell_shim(program) {
        spawn_cli(program)
    } else {
        std::process::Command::new(program)
    }
}

fn shell_shim(program: &std::ffi::OsStr) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let path = Path::new(program);
    let bare = !path.is_absolute() && path.components().nth(1).is_none();
    let batch = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    bare || batch
}

/// What [`ensure_hook_client_link`] did (T357); the caller only needs it for tests and doctor.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) enum LinkOutcome {
    /// `rtok-hook` already resolves on `PATH` (not through a stale link of ours).
    AlreadyOnPath,
    /// No `rtok-hook` beside the running executable, no `rtok` on `PATH`, or not unix.
    Nothing,
    /// The link was created.
    Linked,
    /// A stale or dangling link was re-pointed at this version's client.
    Relinked,
    /// The link already points at this version's client.
    Current,
    /// Something else owns the name; left alone.
    Foreign,
    /// An IO error (read-only directory, race): fail soft, nothing changed.
    Failed,
}

/// T357: package managers (ketch) may link only `rtok` onto `PATH`, leaving the fast hook
/// client `rtok-hook` (T178) unused. Link it next to the `rtok` the hooks run, in the first
/// `path` directory that has one. Fail soft: never an error; a few stats, so cheap enough for
/// the start of `rtok mcp`, `rtok hook --serve` and the host-hook install, never per hook.
pub(crate) fn ensure_hook_client_link(path: Option<&std::ffi::OsStr>, exe: &Path) -> LinkOutcome {
    #[cfg(unix)]
    {
        link_hook_client(path, exe)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, exe);
        LinkOutcome::Nothing
    }
}

#[cfg(unix)]
fn link_hook_client(path: Option<&std::ffi::OsStr>, exe: &Path) -> LinkOutcome {
    use std::os::unix::fs::symlink;
    const NAME: &str = "rtok-hook";
    let Some(path) = path else {
        return LinkOutcome::Nothing;
    };
    let Some(dir) = std::env::split_paths(path).find(|d| d.join("rtok").is_file()) else {
        return LinkOutcome::Nothing;
    };
    // Only the `rtok` that IS this executable: a dev build or a test binary must never link
    // into the `PATH` of a different install.
    let Ok(exe) = std::fs::canonicalize(exe) else {
        return LinkOutcome::Nothing;
    };
    if std::fs::canonicalize(dir.join("rtok")).ok().as_deref() != Some(exe.as_path()) {
        return LinkOutcome::Nothing;
    }
    let link = dir.join(NAME);
    let meta = std::fs::symlink_metadata(&link);
    if meta.is_err() && bin_on_path(NAME, Some(path)) {
        return LinkOutcome::AlreadyOnPath;
    }
    let sibling = exe.parent().map(|p| p.join(NAME));
    let Some(sibling) = sibling.filter(|s| s.is_file()) else {
        return LinkOutcome::Nothing;
    };
    let relink = match meta {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return LinkOutcome::Failed,
        Ok(m) if m.file_type().is_symlink() => {
            let Ok(target) = std::fs::read_link(&link) else {
                return LinkOutcome::Failed;
            };
            let target = dir.join(target);
            if std::fs::canonicalize(&target).is_ok_and(|t| t == sibling) {
                return LinkOutcome::Current;
            }
            let dangling = !target.exists();
            if !dangling && target.file_name().is_none_or(|n| n != NAME) {
                return LinkOutcome::Foreign;
            }
            true
        }
        Ok(_) => return LinkOutcome::Foreign,
    };
    let tmp = dir.join(format!(".{NAME}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    if symlink(&sibling, &tmp).is_err() {
        return LinkOutcome::Failed;
    }
    if std::fs::rename(&tmp, &link).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return LinkOutcome::Failed;
    }
    if relink {
        LinkOutcome::Relinked
    } else {
        LinkOutcome::Linked
    }
}

/// [`ensure_hook_client_link`] for the running process: real `PATH` and executable.
pub(crate) fn ensure_hook_client_link_here() {
    if let Ok(exe) = std::env::current_exe() {
        ensure_hook_client_link(std::env::var_os("PATH").as_deref(), &exe);
    }
}

/// Basename of a command path — split on `/` and `\`, drop a trailing `.exe`
/// (case-insensitive). One definition in [`crate::names`]; `measure` and the
/// `cmd` formatters share it (T55.10).
pub(crate) use crate::names::cmd_stem;

/// True when `bin` names the rtok binary (bare, `.exe`, an absolute path, or the running
/// executable itself whatever it is called — `cargo test` names it `rtok-<hash>`).
pub(crate) fn is_rtok_bin(bin: &str) -> bool {
    if std::env::current_exe().is_ok_and(|e| dunce::simplified(&e).to_string_lossy() == bin) {
        return true;
    }
    cmd_stem(bin).eq_ignore_ascii_case("rtok")
}

/// Quote `bin` for a host shell hook command when it contains whitespace.
///
/// MCP JSON/TOML take an unquoted path string; Claude/Cursor hook `command`
/// values are shell lines, so an absolute `…\Ivan Tuhai\…\rtok.exe hook …`
/// would split on the space and fail to start.
pub(crate) fn shell_quote_bin(bin: &str) -> String {
    if !bin.chars().any(|c| c.is_whitespace() || c == '"') {
        return bin.to_string();
    }
    // cmd.exe /C hook lines need `""` for an embedded quote; bash-style `\"` is wrong.
    format!("\"{}\"", bin.replace('"', "\"\""))
}

/// Binary token for hook command lines (quoted when needed).
pub(crate) fn rtok_hook_bin() -> String {
    shell_quote_bin(&rtok_command())
}

/// T174/T250.2 POSIX hook line for a bare `rtok`: a shell without `~/.ketch/bin` on PATH hit
/// exit 127 every event; tries PATH, ketch's layout, then fails open, printing `note` (the
/// host's session-start JSON, no `'`) if given. Claude and Copilot share these bytes.
pub(crate) fn hook_resolver(args: &str, note: Option<&str>) -> String {
    let note = note.map_or(String::new(), |json| {
        debug_assert!(!json.contains('\''), "{json}");
        format!(" && printf '%s' '{json}'")
    });
    format!(
        "command -v rtok >/dev/null 2>&1 && exec rtok {args}; \
         [ -x \"$HOME/.ketch/bin/rtok\" ] && exec \"$HOME/.ketch/bin/rtok\" {args}; \
         true{note}; exit 0"
    )
}

/// Strip one layer of surrounding quotes from a hook binary token.
pub(crate) fn unquote_bin(bin: &str) -> &str {
    let b = bin.trim();
    if b.len() >= 2 {
        let bytes = b.as_bytes();
        if (bytes[0] == b'"' && *bytes.last().unwrap() == b'"')
            || (bytes[0] == b'\'' && *bytes.last().unwrap() == b'\'')
        {
            return &b[1..b.len() - 1];
        }
    }
    b
}

/// Where a Python wheel installs `plugins/` and `skills/`, under the install prefix: a wheel
/// cannot put directories beside a script (uv rejects them), so they go to `<prefix>/share/rtok`.
const PREFIX_SHARE_DIR: &str = "share/rtok";
/// An install prefix's executable directory: `bin` on Unix, `Scripts` in a Windows venv.
const PREFIX_BIN_DIRS: [&str; 2] = ["bin", "Scripts"];

/// The tree this repo ships a host plugin from (D21 (6)).
///
/// Resolution order:
/// 1. `rel` next to the running binary (release archives ship `plugins/` beside `rtok`),
///    then next to the binary a symlink points at: `npm i -g`, Homebrew and `uv tool` put a
///    link on `PATH`, and macOS reports the link, not its target, as `current_exe`;
/// 2. prefix layout (PyPI wheels): when the exe, or the file it links to, lives in
///    `<prefix>/bin/` (`Scripts\` on Windows), `<prefix>/share/rtok/rel`;
/// 3. ketch layout: when the exe lives in `<root>/bin/`, prefer
///    `<root>/store/rtok/v{version}/` matching `CARGO_PKG_VERSION`, else the
///    newest `store/rtok/*/` that contains `rel`;
/// 4. `CARGO_MANIFEST_DIR/rel` for `cargo test` / dev;
/// 5. beside-exe path for a clear error when nothing exists.
pub(crate) fn plugin_src(rel: &str) -> std::path::PathBuf {
    resolve_plugin_src(
        rel,
        std::env::current_exe().ok().as_deref(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        env!("CARGO_PKG_VERSION"),
    )
}

/// A skill tree this repo ships (`skills/<name>/`, see [`skill::SKILLS`]).
///
/// Prefer the top-level hub. Older ketch archives only shipped
/// `plugins/` (no `skills/`), so fall back to the pi-bundled copy.
pub(crate) fn skill_src(name: &str) -> std::path::PathBuf {
    let hub = plugin_src(&format!("skills/{name}"));
    if hub.exists() {
        return hub;
    }
    plugin_src(&format!("plugins/pi/skills/{name}"))
}

/// Pure resolution used by [`plugin_src`] and unit tests (fake exe / ketch layout).
pub(crate) fn resolve_plugin_src(
    rel: &str,
    exe: Option<&std::path::Path>,
    manifest_dir: &std::path::Path,
    pkg_version: &str,
) -> std::path::PathBuf {
    let cargo = join_rel(manifest_dir, rel);
    let beside = exe.and_then(Path::parent).map(|dir| join_rel(dir, rel));

    // `exists()`, not `is_dir()`: a host plugin may be one file (`plugins/opencode/rtok.ts`).
    if let Some(ref p) = beside
        && p.exists()
    {
        return p.clone();
    }

    let real = exe.and_then(|e| dunce::canonicalize(e).ok());
    if let Some(p) = real
        .as_deref()
        .and_then(Path::parent)
        .map(|dir| join_rel(dir, rel))
        && p.exists()
    {
        return p;
    }

    for bin in exe.into_iter().chain(real.as_deref()) {
        if let Some(prefix) = install_prefix(bin) {
            let p = join_rel(&join_rel(prefix, PREFIX_SHARE_DIR), rel);
            if p.exists() {
                return p;
            }
        }
    }

    if let Some(exe) = exe
        && let Some(bin_dir) = exe.parent()
        && bin_dir
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("bin"))
        && let Some(root) = bin_dir.parent()
        && let Some(found) = ketch_store_plugin(root, rel, pkg_version)
    {
        return found;
    }

    if cargo.exists() {
        return cargo;
    }

    beside.unwrap_or(cargo)
}

/// `<prefix>` when `exe` is `<prefix>/bin/<exe>` (or `<prefix>/Scripts/<exe>`).
fn install_prefix(exe: &Path) -> Option<&Path> {
    let bin_dir = exe.parent()?;
    let name = bin_dir.file_name()?;
    PREFIX_BIN_DIRS
        .iter()
        .any(|d| name.eq_ignore_ascii_case(d))
        .then(|| bin_dir.parent())
        .flatten()
}

/// ketch installs the full package under `<root>/store/rtok/vX.Y.Z/` and only
/// copies the binary into `<root>/bin/`. Prefer the version that matches this
/// build; otherwise take the newest store folder that still has `rel`.
fn ketch_store_plugin(
    root: &std::path::Path,
    rel: &str,
    pkg_version: &str,
) -> Option<std::path::PathBuf> {
    let store = root.join("store").join("rtok");
    let versioned = join_rel(&store.join(format!("v{pkg_version}")), rel);
    if versioned.exists() {
        return Some(versioned);
    }
    let mut entries: Vec<_> = std::fs::read_dir(&store)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    // Newest last: by semver (`v0.1.10` > `v0.1.9`); a name that does not parse sorts before
    // every version and falls back to name order among its kind.
    let key = |p: &std::path::PathBuf| {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        let ver = semver::Version::parse(name.strip_prefix('v').unwrap_or(&name)).ok();
        (ver, name.into_owned())
    };
    entries.sort_by_cached_key(key);
    for dir in entries.into_iter().rev() {
        let candidate = join_rel(&dir, rel);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Build a `Config` pointed at a scratch `file` under a fresh temp dir, via `set_path`. Shared
/// test scaffold (T186) for hosts whose test `cfg()` only needs a tempdir, a `Config::default`,
/// and `dry_run`/`backup` set — instead of each repeating the same four lines.
#[cfg(test)]
pub(crate) fn test_scratch_cfg(
    host: &str,
    name: &str,
    file: &str,
    dry: bool,
    set_path: impl FnOnce(&mut Config, PathBuf),
) -> (Config, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rtok-{host}-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(file);
    let mut c = Config::default();
    set_path(&mut c, path.clone());
    c.setup.dry_run = dry;
    c.setup.backup = false;
    (c, path)
}

/// Shared assertion for hosts whose MCP entry is OpenCode's local-argv shape
/// (`{type: "local", command: [rtok, mcp], enabled: true}`, written by [`register_local_mcp`]):
/// register is idempotent, remove keeps foreign entries, both surface through `installed`.
/// `opencode` and its MiMo Code fork call this one body instead of duplicating the round-trip
/// (T186).
#[cfg(test)]
pub(crate) fn assert_local_mcp_roundtrip(
    path: &Path,
    register: impl Fn() -> Result<String>,
    unregister: impl Fn() -> Result<String>,
    installed: impl Fn() -> Vec<&'static str>,
) {
    use serde_json::Value;
    std::fs::write(path, r#"{"mcp":{"other":{"type":"remote","url":"x"}}}"#).unwrap();
    let first = register().unwrap();
    assert!(first.starts_with("mcp.rtok: "), "{first}");
    assert_eq!(register().unwrap(), NO_CHANGES);
    let root: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(root["mcp"]["rtok"]["type"], "local");
    assert_eq!(root["mcp"]["rtok"]["command"][1], "mcp");
    assert_eq!(root["mcp"]["rtok"]["enabled"], true);
    assert_eq!(installed(), ["mcp"]);
    assert_eq!(unregister().unwrap(), "- mcp.rtok");
    assert_eq!(unregister().unwrap(), NO_CHANGES);
    let root: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(root["mcp"]["rtok"].is_null(), "{root}");
    assert_eq!(root["mcp"]["other"]["url"], "x");
}

/// Shared assertion for hosts whose MCP entry is `{command, args}` under `mcpServers`
/// with no `type` ([`register_stdio_mcp`]). Register is idempotent, remove keeps foreign
/// servers, both surface through `installed`.
#[cfg(test)]
pub(crate) fn assert_stdio_mcp_roundtrip(
    path: &Path,
    host: &str,
    register: impl Fn() -> Result<String>,
    unregister: impl Fn() -> Result<String>,
    installed: impl Fn() -> Vec<&'static str>,
) {
    use serde_json::Value;
    std::fs::write(path, r#"{"mcpServers":{"foreign":{"command":"x"}}}"#).unwrap();
    let first = register().unwrap();
    assert!(first.starts_with("mcpServers.rtok: "), "{first}");
    assert!(first.contains(host), "{first}");
    assert_eq!(register().unwrap(), NO_CHANGES);
    let root: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(
        root["mcpServers"]["rtok"].get("type").is_none(),
        "stdio shape has no type: {root}"
    );
    assert_eq!(root["mcpServers"]["rtok"]["args"][0], "mcp");
    assert_eq!(root["mcpServers"]["rtok"]["args"][2], host);
    assert_eq!(installed(), ["mcp"]);
    assert_eq!(unregister().unwrap(), "- mcpServers.rtok");
    assert_eq!(unregister().unwrap(), NO_CHANGES);
    let root: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(root["mcpServers"]["rtok"].is_null(), "{root}");
    assert_eq!(root["mcpServers"]["foreign"]["command"], "x");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::{Path, PathBuf};

    #[test]
    fn each_mode_picks_its_operation_icon() {
        // `replace` has no row of its own and takes its kind's icon, like any other verb.
        let icon = |m: Mode| style::icon(m.verb(), style::Kind::Success);
        assert_eq!(icon(Mode::Install), "📦");
        assert_eq!(icon(Mode::Update), "⏫");
        assert_eq!(icon(Mode::Remove), "🧹");
        assert_eq!(icon(Mode::Replace), style::Kind::Success.emoji());
    }

    #[test]
    fn resolve_rtok_command_keeps_bare_name_when_on_path() {
        let dir = std::env::temp_dir().join(format!("rtok-setup-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join(if cfg!(windows) { "rtok.exe" } else { "rtok" });
        std::fs::write(&bin, b"x").unwrap();
        let path = std::env::join_paths([dir.as_os_str()]).unwrap();
        assert_eq!(
            resolve_rtok_command(
                Some(Path::new(r"C:\nowhere\rtok.exe")),
                Some(path.as_os_str())
            ),
            "rtok"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_rtok_command_uses_absolute_exe_when_path_misses_on_windows() {
        let exe = PathBuf::from(if cfg!(windows) {
            r"C:\Users\u\.ketch\bin\rtok.exe"
        } else {
            "/opt/rtok"
        });
        let empty = std::ffi::OsString::new();
        let got = resolve_rtok_command(Some(&exe), Some(empty.as_os_str()));
        if cfg!(windows) {
            assert!(got.ends_with("rtok.exe"), "{got}");
            assert!(got.contains("ketch") || got.contains("Users"), "{got}");
        } else {
            assert_eq!(got, "rtok");
        }
    }

    #[test]
    fn is_rtok_bin_accepts_absolute_windows_exe() {
        assert!(is_rtok_bin("rtok"));
        assert!(is_rtok_bin("rtok.exe"));
        assert!(is_rtok_bin("RTOK.Exe"));
        assert!(is_rtok_bin(r"C:\Users\u\.ketch\bin\rtok.eXe"));
        assert!(is_rtok_bin(r"C:\Users\u\.ketch\bin\rtok.exe"));
        assert!(!is_rtok_bin("rtok-extra"));
        assert!(!is_rtok_bin(r"C:\bin\other.exe"));
    }

    /// Moved from `formatters` with the definition (T55.10).
    #[test]
    fn cmd_stem_strips_windows_path_and_exe() {
        assert_eq!(cmd_stem(r"C:\Program Files\Git\cmd\git.exe"), "git");
        assert_eq!(cmd_stem(r"C:\Windows\System32\cmd.EXE"), "cmd");
        assert_eq!(cmd_stem("/usr/bin/git"), "git");
        assert_eq!(cmd_stem("sudo"), "sudo");
        assert_eq!(cmd_stem("rtok.exe"), "rtok");
    }

    /// Found by `fuzz/` (cmd-filter): the `.exe` check sliced the last 4 bytes of the
    /// basename, which panicked when they began inside a multi-byte char.
    #[test]
    fn cmd_stem_non_ascii_name_does_not_panic() {
        assert_eq!(cmd_stem("héllo"), "héllo");
        assert_eq!(cmd_stem("/opt/bin/ƞ\0\0)"), "ƞ\0\0)");
        assert_eq!(cmd_stem("dé.exe"), "dé");
    }

    #[test]
    fn shell_quote_bin_quotes_paths_with_spaces() {
        assert_eq!(shell_quote_bin("rtok"), "rtok");
        let spaced = r"C:\Users\Ivan Tuhai\.ketch\bin\rtok.exe";
        let quoted = shell_quote_bin(spaced);
        assert!(quoted.starts_with('"') && quoted.ends_with('"'), "{quoted}");
        assert!(quoted.contains("Ivan Tuhai"), "{quoted}");
        assert_eq!(unquote_bin(&quoted), spaced);
        assert!(is_rtok_bin(unquote_bin(&quoted)));
    }

    #[test]
    fn shell_quote_bin_escapes_embedded_quotes_for_cmd() {
        // cmd.exe needs `""` inside a double-quoted token; bash-style `\"` is wrong.
        let bin = "C:\\Path With Spaces\\rtok\"x\".exe";
        let quoted = shell_quote_bin(bin);
        assert!(quoted.starts_with('"') && quoted.ends_with('"'), "{quoted}");
        assert!(
            quoted.contains("\"\""),
            "expected cmd-style doubled quotes: {quoted}"
        );
        assert!(
            !quoted.contains("\\\""),
            "bash-style escape must not appear: {quoted}"
        );
    }

    #[test]
    fn skill_src_falls_back_to_pi_skill_in_ketch_store() {
        let root = std::env::temp_dir().join(format!("rtok-skill-fallback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("bin");
        let store = root.join("store/rtok/v0.2.0");
        let pi_skill = store.join("plugins/pi/skills/rtok");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&pi_skill).unwrap();
        std::fs::write(pi_skill.join("SKILL.md"), b"pi\n").unwrap();
        let exe = bin.join("rtok");
        std::fs::write(&exe, b"x").unwrap();
        // No top-level skills/ in the store — only the pi copy.
        let got = resolve_plugin_src(
            "skills/rtok",
            Some(&exe),
            &root.join("missing-cargo"),
            "0.2.0",
        );
        assert!(!got.exists(), "hub must be absent: {}", got.display());
        let fallback = resolve_plugin_src(
            "plugins/pi/skills/rtok",
            Some(&exe),
            &root.join("missing-cargo"),
            "0.2.0",
        );
        assert_eq!(fallback, pi_skill);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_carries_the_setup_flags() {
        let mut cfg = Config::default();
        cfg.setup.dry_run = true;
        cfg.setup.yes = true;
        cfg.setup.backup = false;
        let a = apply(&cfg);
        assert!(a.dry_run && a.yes && !a.backup);
    }

    fn write_plugin(dir: &std::path::Path) {
        use std::fs;
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("plugin.json"), "{}").unwrap();
    }

    #[test]
    fn plugin_src_prefers_directory_beside_exe() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("rtok-plugin-src-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let bin_dir = root.join("bin");
        let plugins = bin_dir.join("plugins").join("cursor");
        write_plugin(&plugins);
        let fake_exe = bin_dir.join("rtok");
        fs::write(&fake_exe, b"").unwrap();
        let missing_manifest = root.join("no-such-manifest");
        let got = resolve_plugin_src(
            "plugins/cursor",
            Some(&fake_exe),
            &missing_manifest,
            "0.1.5",
        );
        assert_eq!(got, plugins);
        // When beside-exe is missing, fall back to an existing cargo tree.
        let cargo_root = root.join("cargo");
        let cargo_plugins = cargo_root.join("plugins").join("cursor");
        write_plugin(&cargo_plugins);
        let lonely_exe = root.join("lonely").join("rtok");
        fs::create_dir_all(lonely_exe.parent().unwrap()).unwrap();
        fs::write(&lonely_exe, b"").unwrap();
        let got = resolve_plugin_src("plugins/cursor", Some(&lonely_exe), &cargo_root, "0.1.5");
        assert_eq!(got, cargo_plugins);
        // Neither exists: still return the beside-exe path for a clear error.
        let empty = root.join("empty");
        let empty_exe = empty.join("rtok");
        fs::create_dir_all(&empty).unwrap();
        fs::write(&empty_exe, b"").unwrap();
        let got = resolve_plugin_src("plugins/cursor", Some(&empty_exe), &empty, "0.1.5");
        assert_eq!(got, empty.join("plugins").join("cursor"));
        let _ = fs::remove_dir_all(&root);
    }

    /// `npm i -g rtok` links `<prefix>/bin/rtok` to the package's binary, which has
    /// `plugins/` beside it; the link's own directory has none.
    #[cfg(unix)]
    #[test]
    fn plugin_src_follows_a_symlinked_exe() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("rtok-plugin-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let pkg_bin = root.join("lib").join("rtok").join("bin");
        let plugins = pkg_bin.join("plugins").join("cursor");
        write_plugin(&plugins);
        let real_exe = pkg_bin.join("rtok");
        fs::write(&real_exe, b"").unwrap();
        let link_dir = root.join("prefix-bin");
        fs::create_dir_all(&link_dir).unwrap();
        let link = link_dir.join("rtok");
        std::os::unix::fs::symlink(&real_exe, &link).unwrap();
        let got = resolve_plugin_src(
            "plugins/cursor",
            Some(&link),
            &root.join("no-such-manifest"),
            "0.1.5",
        );
        assert_eq!(got, dunce::canonicalize(&plugins).unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    /// A wheel installs `<venv>/bin/rtok` and `<venv>/share/rtok/plugins/`; `uv tool` then
    /// links the binary from another bin directory.
    #[cfg(unix)]
    #[test]
    fn plugin_src_finds_prefix_share_dir() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("rtok-plugin-share-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let venv = root.join("venv");
        let plugins = venv
            .join("share")
            .join("rtok")
            .join("plugins")
            .join("cursor");
        write_plugin(&plugins);
        let exe = venv.join("bin").join("rtok");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"").unwrap();
        let missing = root.join("no-such-manifest");
        let got = resolve_plugin_src("plugins/cursor", Some(&exe), &missing, "0.1.5");
        assert_eq!(got, plugins);
        let link = root.join("tool-bin").join("rtok");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&exe, &link).unwrap();
        let got = resolve_plugin_src("plugins/cursor", Some(&link), &missing, "0.1.5");
        assert_eq!(
            dunce::canonicalize(got).unwrap(),
            dunce::canonicalize(&plugins).unwrap()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn plugin_src_finds_ketch_store_when_bin_has_no_plugins() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("rtok-ketch-src-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let fake_exe = bin_dir.join("rtok");
        fs::write(&fake_exe, b"").unwrap();
        let store_plugins = root
            .join("store")
            .join("rtok")
            .join("v0.1.5")
            .join("plugins")
            .join("cursor");
        write_plugin(&store_plugins);
        let missing_manifest = root.join("no-such-manifest");
        let got = resolve_plugin_src(
            "plugins/cursor",
            Some(&fake_exe),
            &missing_manifest,
            "0.1.5",
        );
        assert_eq!(got, store_plugins);
        // Still prefer plugins beside the bin exe when both exist.
        let beside = bin_dir.join("plugins").join("cursor");
        write_plugin(&beside);
        let got = resolve_plugin_src(
            "plugins/cursor",
            Some(&fake_exe),
            &missing_manifest,
            "0.1.5",
        );
        assert_eq!(got, beside);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn plugin_src_finds_ketch_store_when_bin_dir_is_capitalised() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("rtok-ketch-Bin-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // Windows (and some shells) can surface the ketch bin dir as `Bin`.
        let bin_dir = root.join("Bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let fake_exe = bin_dir.join("rtok");
        fs::write(&fake_exe, b"").unwrap();
        let store_plugins = root
            .join("store")
            .join("rtok")
            .join("v0.1.6")
            .join("plugins")
            .join("cursor");
        write_plugin(&store_plugins);
        let missing_manifest = root.join("no-such-manifest");
        let got = resolve_plugin_src(
            "plugins/cursor",
            Some(&fake_exe),
            &missing_manifest,
            "0.1.6",
        );
        assert_eq!(got, store_plugins);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn plugin_src_prefers_matching_ketch_store_version() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("rtok-ketch-ver-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let fake_exe = bin_dir.join("rtok");
        fs::write(&fake_exe, b"").unwrap();
        let store = root.join("store").join("rtok");
        let older = store.join("v0.1.4").join("plugins").join("cursor");
        let newer = store.join("v0.1.5").join("plugins").join("cursor");
        write_plugin(&older);
        write_plugin(&newer);
        let missing_manifest = root.join("no-such-manifest");
        // Matching CARGO_PKG_VERSION wins even when a newer folder exists.
        let got = resolve_plugin_src(
            "plugins/cursor",
            Some(&fake_exe),
            &missing_manifest,
            "0.1.4",
        );
        assert_eq!(got, older);
        // When the matching version has no plugins, take the newest that does.
        fs::remove_dir_all(store.join("v0.1.4")).unwrap();
        let got = resolve_plugin_src(
            "plugins/cursor",
            Some(&fake_exe),
            &missing_manifest,
            "0.1.4",
        );
        assert_eq!(got, newer);
        let _ = fs::remove_dir_all(&root);
    }

    /// T328: the newest store folder is chosen by semver, not by name (`v0.1.10` > `v0.1.9`).
    #[test]
    fn ketch_store_takes_the_newest_version_not_the_lexicographic_last() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("rtok-ketch-semver-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let store = root.join("store").join("rtok");
        for v in ["v0.1.9", "v0.1.10"] {
            write_plugin(&store.join(v).join("plugins").join("cursor"));
        }
        let got = ketch_store_plugin(&root, "plugins/cursor", "9.9.9");
        assert_eq!(
            got,
            Some(store.join("v0.1.10").join("plugins").join("cursor"))
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn join_rel_splits_slash_and_backslash() {
        let base = std::path::Path::new("/tmp/root");
        assert_eq!(
            join_rel(base, "plugins/cursor"),
            base.join("plugins").join("cursor")
        );
        assert_eq!(
            join_rel(base, "plugins\\cursor"),
            base.join("plugins").join("cursor")
        );
    }

    #[test]
    fn variant_filter_defaults_to_all() {
        assert!(wants(Kind::Cli, false, false, false));
        assert!(wants(Kind::Desktop, false, false, false));
        assert!(wants(Kind::Cli, false, false, true));
        assert!(wants(Kind::Desktop, false, false, true));
        assert!(wants(Kind::Cli, true, false, false));
        assert!(!wants(Kind::Desktop, true, false, false));
        assert!(!wants(Kind::Cli, false, true, false));
        assert!(wants(Kind::Desktop, false, true, false));
        let kinds = |id: &str| -> Vec<Kind> {
            host(id)
                .unwrap()
                .variants()
                .iter()
                .map(|v| v.kind)
                .collect()
        };
        assert_eq!(kinds("cursor"), [Kind::Cli, Kind::Desktop]);
        assert_eq!(kinds("opencode"), [Kind::Cli, Kind::Desktop]);
        assert_eq!(kinds("claude"), [Kind::Cli, Kind::Desktop]);
        assert_eq!(kinds("codex"), [Kind::Cli]);
        assert!(host("notahost").is_none());
        assert!(resolve(&["claude".into(), "nope".into()]).is_err());
    }

    #[test]
    fn list_prints_one_block_per_app_with_kind_name_app_and_config() {
        let out = list(&Config::default());
        for head in [
            "CLI: Claude Code",
            "Desktop: Claude Desktop",
            "CLI: Cursor CLI",
            "Desktop: Cursor",
            "CLI: Codex",
            "CLI: OpenCode",
            "Desktop: OpenCode Desktop",
            "CLI: pi",
            "Desktop: Windsurf",
            "CLI: Zed CLI",
            "Desktop: Zed",
        ] {
            assert!(
                out.contains(&format!("{head}\n")) || out.contains(&format!("{head} — ")),
                "{head} missing:\n{out}"
            );
        }
        assert!(out.contains("  app     "), "{out}");
        assert!(out.contains("  config  "), "{out}");
        // `plugins` is only printed for installed variants (`Outcome::Listed`).
        let any_installed = out.lines().any(|l| {
            (l.starts_with("CLI:") || l.starts_with("Desktop:")) && !l.contains("not found")
        });
        if any_installed {
            assert!(out.contains("  plugins\n"), "{out}");
        }
    }

    /// Codex with an MCP block and no provider: MCP plugins ride it, proxy-only plugins wait
    /// for `--proxy`, hook-only plugins have nowhere to go. A disabled plugin says so.
    #[test]
    fn plugins_group_by_the_modules_that_carry_their_surfaces() {
        let dir = std::env::temp_dir().join(format!("rtok-plugrows-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default();
        cfg.setup.codex.config_path = dir.join("config.toml");
        std::fs::write(
            &cfg.setup.codex.config_path,
            "[mcp_servers.rtok]\ncommand = \"rtok\"\n",
        )
        .unwrap();
        let rows = plugin_rows(&codex::Codex, Kind::Cli, &cfg);
        let _ = std::fs::remove_dir_all(&dir);
        let state = |id: &str| rows.iter().find(|r| r.id == id).unwrap().state;
        assert_eq!(state("read"), ModuleState::Installed);
        assert_eq!(state("graph"), ModuleState::Installed);
        assert_eq!(state("proxy"), ModuleState::NotInstalled);
        assert_eq!(state("measure"), ModuleState::NotInstalled);
        assert_eq!(state("cmd"), ModuleState::NotInstalled);
        assert_eq!(state("guard"), ModuleState::NotInstalled);
        assert_eq!(state("inject"), ModuleState::NotInstalled);
        let text = plugin_lines(&rows, "", true);
        assert!(text.contains("✓ installed      "), "{text}");
        assert!(!text.contains("compress (off)"), "{text}");
        assert!(
            text.contains("✗ not installed  proxy, compress, cmd, inject, guard")
                || (text.contains("cmd") && text.contains("inject") && text.contains("guard")),
            "{text}"
        );
        // pi reaches bash through the extension (cli) and MCP-surface plugins
        // through `pi.registerTool` (T70.3). Proxy still has no path in.
        assert!(reaches(&pi::Pi, Kind::Cli, &[Surface::Cli]));
        assert!(reaches(&pi::Pi, Kind::Cli, &[Surface::Mcp]));
        assert!(!reaches(&pi::Pi, Kind::Cli, &[Surface::Proxy]));
        assert!(reaches(&cursor::Cursor, Kind::Desktop, &[Surface::Mcp]));
    }

    /// `install` / `update` / `list` share `plugin_lines`. A not-installed group with no
    /// plugins used to print `✗ not installed  -`; that dash is not a missing install.
    #[test]
    fn empty_not_installed_plugin_group_is_omitted() {
        let rows = [PluginRow {
            id: "read",
            on: true,
            state: ModuleState::Installed,
        }];
        let text = plugin_lines(&rows, "", false);
        assert!(
            !text.contains("not installed"),
            "empty group must not print not installed:\n{text}"
        );
        assert!(text.contains("installed      read"), "{text}");
        assert!(text.contains("not supported  -"), "{text}");

        let waiting = [PluginRow {
            id: "proxy",
            on: false,
            state: ModuleState::NotInstalled,
        }];
        let text = plugin_lines(&waiting, "", false);
        assert!(text.contains("not installed  proxy (off)"), "{text}");
    }

    /// What an install must leave behind follows `support()` and the flags given; against
    /// files that do not exist, every expected module is missing.
    #[test]
    fn expected_modules_follow_support_and_flags_and_missing_reads_them_back() {
        let mut cfg = Config::default();
        cfg.setup.codex.config_path = std::env::temp_dir()
            .join("rtok-no-such-dir")
            .join("config.toml");
        cfg.setup.claude.settings_path = std::env::temp_dir().join("rtok-no-such-dir/s.json");
        cfg.doctor.claude_json = std::env::temp_dir().join("rtok-no-such-dir/c.json");
        assert_eq!(
            expected(&codex::Codex, Kind::Cli, &cfg),
            ["hooks", "mcp", "plugin"]
        );
        // pi's plugin is expected by default, without `--yes` (T164).
        assert_eq!(expected(&pi::Pi, Kind::Cli, &cfg), ["plugin"]);
        assert_eq!(expected(&claude::Claude, Kind::Desktop, &cfg), ["mcp"]);
        // Claude Code's plugin is expected by default, without `--yes` (T139).
        assert_eq!(
            expected(&claude::Claude, Kind::Cli, &cfg),
            ["hooks", "mcp", "plugin"]
        );
        cfg.setup.proxy = true;
        cfg.setup.yes = true;
        cfg.setup.mcp = false;
        assert_eq!(
            expected(&codex::Codex, Kind::Cli, &cfg),
            ["hooks", "proxy", "plugin"]
        );
        assert_eq!(
            expected(&claude::Claude, Kind::Cli, &cfg),
            ["hooks", "proxy", "plugin"]
        );
        assert_eq!(expected(&pi::Pi, Kind::Cli, &cfg), ["plugin"]);
        assert_eq!(
            missing(&claude::Claude, Kind::Cli, &cfg),
            ["hooks", "proxy", "plugin"]
        );
        assert_eq!(
            missing(&claude::Claude, Kind::Desktop, &cfg),
            Vec::<&str>::new()
        );
    }

    /// `~/x` follows the home dir, `$VAR/x` the variable, and an unset variable is left as
    /// written so the caller's `exists()` says no instead of probing a wrong root.
    #[test]
    fn expand_spec_resolves_home_and_env_vars() {
        assert_eq!(expand_spec("~/Apps/x"), join_rel(&home_dir(), "Apps/x"));
        let path = std::env::var_os("PATH").expect("PATH");
        assert_eq!(
            expand_spec("$PATH/Claude/claude.exe"),
            join_rel(Path::new(&path), "Claude/claude.exe")
        );
        assert_eq!(
            expand_spec("$RTOK_NO_SUCH_VAR/app"),
            PathBuf::from("$RTOK_NO_SUCH_VAR/app")
        );
        assert_eq!(
            expand_spec("/Applications/Claude.app"),
            PathBuf::from("/Applications/Claude.app")
        );
    }

    /// T280: under the host sandbox an app outside the home dir is looked up beneath it, so
    /// the machine's `/Applications/Cursor.app` is never seen; a path already inside stays.
    #[test]
    fn under_reroots_a_path_outside_home() {
        let home = Path::new("/h/test-home");
        assert_eq!(
            under(home, PathBuf::from("/Applications/Cursor.app")),
            PathBuf::from("/h/test-home/Applications/Cursor.app")
        );
        assert_eq!(
            under(home, PathBuf::from("/h/test-home/.grok/bin/grok")),
            PathBuf::from("/h/test-home/.grok/bin/grok")
        );
        assert_eq!(
            under(home, PathBuf::from("$LOCALAPPDATA/app")),
            PathBuf::from("/h/test-home/$LOCALAPPDATA/app")
        );
    }

    /// T429: a `.cmd` shim is the install, and a directory outside the home limit is skipped.
    #[test]
    fn find_bin_tries_pathext_and_skips_dirs_outside_the_limit() {
        let root = std::env::temp_dir().join(format!("rtok-find-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let inside = root.join("bin");
        let outside = root.join("other");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(inside.join("claude.cmd"), b"@echo off\r\n").unwrap();
        std::fs::write(outside.join("claude.cmd"), b"@echo off\r\n").unwrap();
        let exts = pathext_from(Some(".cmd"));
        let path = std::env::join_paths([outside.as_os_str(), inside.as_os_str()]).unwrap();
        let got = find_bin("claude", Some(path.as_os_str()), Some(&inside), &exts).unwrap();
        assert_eq!(got, inside.join("claude.cmd"));
        assert!(
            find_bin(
                "claude",
                Some(path.as_os_str()),
                Some(&inside.join("missing")),
                &exts
            )
            .is_none()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// T429: `C:\…` and `c:\…` are the same PATH entry under the sandbox home.
    #[cfg(windows)]
    #[test]
    fn find_bin_limit_is_ascii_case_insensitive() {
        let dir = std::env::temp_dir().join(format!("rtok-find-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("grok.EXE"), b"").unwrap();
        let flipped: PathBuf = dir
            .to_string_lossy()
            .chars()
            .map(|c| match c {
                'A'..='Z' => c.to_ascii_lowercase(),
                'a'..='z' => c.to_ascii_uppercase(),
                _ => c,
            })
            .collect::<String>()
            .into();
        let path = std::env::join_paths([dir.as_os_str()]).unwrap();
        let got = find_bin(
            "grok",
            Some(path.as_os_str()),
            Some(&flipped),
            &pathext_from(Some(".EXE")),
        );
        assert!(got.is_some(), "limit {flipped:?} missed {}", dir.display());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T429: `~/.grok/bin/grok` is the install when the file on disk is `grok.exe`.
    #[test]
    fn probe_installed_matches_an_exe_beside_an_extensionless_path() {
        let dir = std::env::temp_dir().join(format!("rtok-probe-exe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("grok.exe");
        std::fs::write(&exe, b"").unwrap();
        if cfg!(windows) {
            let got = probe_installed(dir.join("grok")).unwrap();
            assert!(crate::fs::same_path(&got, &exe), "{got:?}");
        } else {
            assert!(probe_installed(dir.join("grok")).is_none());
        }
        assert_eq!(probe_installed(exe.clone()).unwrap(), exe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T429: a differently cased child of home stays put instead of being re-rooted.
    #[cfg(windows)]
    #[test]
    fn under_keeps_a_different_case_child_of_home() {
        let home = Path::new(r"C:\Users\Me");
        let path = PathBuf::from(r"c:\users\me\AppData\Local\Programs\cursor\Cursor.exe");
        assert_eq!(under(home, path.clone()), path);
    }

    /// T280: `.config/nextest.toml` turns the host sandbox on for every test, so no test sees
    /// the agents installed on the machine it runs on.
    #[test]
    fn tests_run_inside_the_host_sandbox() {
        assert_eq!(host_sandbox(), Some(home_dir()));
        for v in cursor::Cursor.variants() {
            assert!(app_path(v).is_none(), "{} seen outside the sandbox", v.name);
        }
    }

    /// A proxy on a non-default port read as not installed: the check looked for `8790`.
    #[test]
    fn claude_modules_read_back_hooks_and_a_proxy_on_any_port() {
        let dir = std::env::temp_dir().join(format!("rtok-mods-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default();
        cfg.proxy.port = 9123;
        cfg.setup.claude.settings_path = dir.join("settings.json");
        cfg.doctor.claude_json = dir.join("claude.json");
        let settings = serde_json::json!({
            "hooks": {"SessionStart": [{"hooks": [{"command": "rtok hook SessionStart"}]}]},
            "env": {"ANTHROPIC_BASE_URL": anthropic_proxy_url(&cfg)},
        });
        std::fs::write(&cfg.setup.claude.settings_path, settings.to_string()).unwrap();
        let rows = module_rows(&claude::Claude, Kind::Cli, &cfg);
        let _ = std::fs::remove_dir_all(&dir);
        let states: Vec<(&str, ModuleState)> = rows.iter().map(|r| (r.name, r.state)).collect();
        assert_eq!(
            states,
            [
                ("hooks", ModuleState::Installed),
                ("mcp", ModuleState::NotInstalled),
                ("proxy", ModuleState::Installed),
                ("plugin", ModuleState::NotInstalled),
            ]
        );
        let console = module_lines(&rows, "  ", true);
        assert!(console.contains("✓ hooks   installed"), "{console}");
        assert!(console.contains("✗ mcp     not installed"), "{console}");
        assert!(console.contains("✗ plugin  not installed"), "{console}");
        let plain = module_lines(&rows, "  ", false);
        assert!(
            plain.contains("  proxy   installed") && !plain.contains('✓'),
            "{plain}"
        );
    }

    #[test]
    fn a_flag_module_names_its_flag_and_an_unknown_bin_has_no_path_or_version() {
        let mut cfg = Config::default();
        cfg.setup.codex.config_path = std::env::temp_dir()
            .join("rtok-no-such-dir")
            .join("config.toml");
        let rows = module_rows(&codex::Codex, Kind::Cli, &cfg);
        let row = |name: &str| rows.iter().find(|r| r.name == name).unwrap().clone();
        assert_eq!(row("proxy").state, ModuleState::NotInstalled);
        assert_eq!(row("proxy").note, " (--proxy)");
        assert_eq!(row("hooks").state, ModuleState::NotInstalled);
        assert!(
            module_lines(&rows, "", true).contains("✗ proxy   not installed (--proxy)"),
            "{rows:?}"
        );
        let ghost = Variant {
            kind: Kind::Cli,
            name: "ghost",
            bins: &["rtok-no-such-binary"],
            apps: &["~/rtok-no-such-app", "$RTOK_NO_SUCH_VAR/app"],
        };
        assert_eq!(app_version(&ghost), "-");
        assert!(app_path(&ghost).is_none());
    }

    #[test]
    fn looks_like_a_version_tells_a_number_from_wrapper_noise() {
        assert!(looks_like_a_version("0.1.0 (fake copilot)"));
        assert!(looks_like_a_version("git version 2.43.0"));
        assert!(!looks_like_a_version("Package extraction took 1234ms"));
        assert!(!looks_like_a_version("some cli"));
        assert!(!looks_like_a_version(""));
    }

    /// T168: an npm-installed CLI's `--version` can print setup noise (e.g. `Package
    /// extraction took 1234ms`) on the line before the real version. The old code took
    /// the first non-empty line unconditionally and would have returned that noise.
    #[test]
    fn app_version_skips_wrapper_noise_ahead_of_the_real_version() {
        let dir =
            std::env::temp_dir().join(format!("rtok-app-version-noise-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join(if cfg!(windows) {
            "rtok-test-noisy.cmd"
        } else {
            "rtok-test-noisy"
        });
        #[cfg(unix)]
        {
            std::fs::write(
                &bin,
                "#!/bin/sh\necho 'Package extraction took 1234ms'\necho '0.1.0 (fake copilot)'\n",
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&bin).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&bin, perms).unwrap();
        }
        #[cfg(windows)]
        {
            std::fs::write(
                &bin,
                "@echo off\r\necho Package extraction took 1234ms\r\necho 0.1.0 (fake copilot)\r\n",
            )
            .unwrap();
        }
        // Command::new resolves a path containing a separator directly (no PATH search),
        // so an absolute path stands in for a `bins` entry here; `Variant::bins` needs a
        // `'static` str, hence the leak (test-only, scoped to this one process).
        let path: &'static str = Box::leak(bin.to_string_lossy().into_owned().into_boxed_str());
        let bins: &'static [&'static str] = Box::leak(vec![path].into_boxed_slice());
        let noisy = Variant {
            kind: Kind::Cli,
            name: "noisy",
            bins,
            apps: &[],
        };
        assert_eq!(app_version(&noisy), "0.1.0 (fake copilot)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T280: a `--version` that never answers, with a helper left holding its stdout (what
    /// `cursor` does under a fresh `$HOME`), costs one probe limit and reads as unknown.
    #[cfg(unix)]
    #[test]
    fn app_version_gives_up_on_a_cli_that_never_answers() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("rtok-app-version-hang-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("rtok-test-hang");
        std::fs::write(&bin, "#!/bin/sh\nsleep 60 &\nsleep 60\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path: &'static str = Box::leak(bin.to_string_lossy().into_owned().into_boxed_str());
        let bins: &'static [&'static str] = Box::leak(vec![path].into_boxed_slice());
        let hang = Variant {
            kind: Kind::Cli,
            name: "hang",
            bins,
            apps: &[],
        };
        let start = std::time::Instant::now();
        assert_eq!(app_version(&hang), "-");
        assert!(
            start.elapsed() < VERSION_PROBE_LIMIT + std::time::Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `| module | support | why |` rows of a host README, keyed by the first cell.
    fn readme_rows(readme: &str) -> std::collections::HashMap<String, (String, String)> {
        readme
            .lines()
            .filter_map(|l| {
                let cells: Vec<&str> = l
                    .trim()
                    .trim_matches('|')
                    .split('|')
                    .map(str::trim)
                    .collect();
                let is_row = cells.len() == 3 && MODULES.iter().any(|m| cells[0].starts_with(m));
                is_row.then(|| {
                    (
                        cells[0].to_string(),
                        (cells[1].to_string(), cells[2].to_string()),
                    )
                })
            })
            .collect()
    }

    /// `Reachable: a, b` / `Not reachable: c` lines of a host README, keyed by their label.
    fn readme_reach(readme: &str) -> std::collections::HashMap<String, Vec<String>> {
        readme
            .lines()
            .filter_map(|l| l.split_once(": "))
            .filter(|(label, _)| label.ends_with("eachable") || label.contains("eachable ("))
            .map(|(label, ids)| {
                let ids = ids
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty() && s != "-")
                    .collect();
                (label.to_string(), ids)
            })
            .collect()
    }

    /// Each host README carries one row per module (`module` or `module (desktop)`) whose
    /// support cell is `yes`, the flag in backticks, or `no` — and a `no` row's reason is the
    /// very string `support()` prints. Its `Reachable:` / `Not reachable:` lines (per kind
    /// where they differ) name exactly the catalogue plugins the modules carry.
    #[test]
    fn readme_tables_match_support() {
        let manifests = crate::plugins::Registry::new(&Config::default()).manifests();
        for id in HOSTS {
            let agent = host(id).unwrap();
            let rows = readme_rows(agent.readme());
            let reach = readme_reach(agent.readme());
            for v in agent.variants() {
                let suffix = format!(" ({})", v.kind.as_str());
                // Only the plugins this build compiles in: a README names them all.
                let listed = |label: &str| -> Vec<String> {
                    reach
                        .get(&format!("{label}{suffix}"))
                        .or_else(|| reach.get(label))
                        .unwrap_or_else(|| panic!("{id} README has no `{label}:` line"))
                        .iter()
                        .filter(|p| manifests.iter().any(|(m, _)| m.id == p.as_str()))
                        .cloned()
                        .collect()
                };
                let (mut yes, mut no) = (Vec::new(), Vec::new());
                for (m, _) in &manifests {
                    if reaches(agent, v.kind, m.surfaces) {
                        yes.push(m.id.to_string());
                    } else {
                        no.push(m.id.to_string());
                    }
                }
                assert_eq!(listed("Reachable"), yes, "{id} ({})", v.name);
                assert_eq!(listed("Not reachable"), no, "{id} ({})", v.name);
                for module in MODULES {
                    let key = format!("{module} ({})", v.kind.as_str());
                    let (cell, why) = rows
                        .get(&key)
                        .or_else(|| rows.get(*module))
                        .unwrap_or_else(|| panic!("{id} README has no row for {module}"));
                    let want = match agent.support(v.kind, module) {
                        Support::Yes => "yes".to_string(),
                        Support::Flag(f) | Support::Offer(f) => format!("`{f}`"),
                        Support::No(reason) => {
                            assert_eq!(why, reason, "{id}: {key} reason");
                            "no".to_string()
                        }
                    };
                    assert_eq!(cell, &want, "{id}: {key}");
                }
            }
        }
    }

    /// T357 fixture: `pkg/v1` and `pkg/v2` hold `rtok` + `rtok-hook`; `bin/rtok` links `pkg/v1`.
    #[cfg(unix)]
    struct LinkFx {
        root: PathBuf,
    }

    #[cfg(unix)]
    impl LinkFx {
        fn new(name: &str) -> Self {
            use std::os::unix::fs::symlink;
            let root =
                std::env::temp_dir().join(format!("rtok-t357-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            for v in ["v1", "v2"] {
                std::fs::create_dir_all(root.join("pkg").join(v)).unwrap();
                for f in ["rtok", "rtok-hook"] {
                    std::fs::write(root.join("pkg").join(v).join(f), "").unwrap();
                }
            }
            std::fs::create_dir_all(root.join("bin")).unwrap();
            symlink(root.join("pkg/v1/rtok"), root.join("bin/rtok")).unwrap();
            Self { root }
        }
        fn bin(&self) -> PathBuf {
            self.root.join("bin")
        }
        fn path(&self) -> std::ffi::OsString {
            std::env::join_paths([self.bin()]).unwrap()
        }
        fn exe(&self, v: &str) -> PathBuf {
            self.root.join("pkg").join(v).join("rtok")
        }
        fn link(&self) -> PathBuf {
            self.bin().join("rtok-hook")
        }
        fn run(&self, v: &str) -> LinkOutcome {
            ensure_hook_client_link(Some(&self.path()), &self.exe(v))
        }
    }

    #[cfg(unix)]
    impl Drop for LinkFx {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(unix)]
    #[test]
    fn hook_client_link_created_then_current() {
        let fx = LinkFx::new("create");
        assert_eq!(fx.run("v1"), LinkOutcome::Linked);
        let target = std::fs::canonicalize(fx.link()).unwrap();
        assert_eq!(
            target,
            std::fs::canonicalize(fx.root.join("pkg/v1/rtok-hook")).unwrap()
        );
        assert!(bin_on_path("rtok-hook", Some(&fx.path())));
        assert_eq!(fx.run("v1"), LinkOutcome::Current);
    }

    #[cfg(unix)]
    #[test]
    fn hook_client_link_noop_when_already_on_path() {
        let fx = LinkFx::new("onpath");
        let other = fx.root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("rtok-hook"), "").unwrap();
        let path = std::env::join_paths([fx.bin(), other]).unwrap();
        let got = ensure_hook_client_link(Some(&path), &fx.exe("v1"));
        assert_eq!(got, LinkOutcome::AlreadyOnPath);
        assert!(std::fs::symlink_metadata(fx.link()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn hook_client_link_repoints_other_version() {
        use std::os::unix::fs::symlink;
        let fx = LinkFx::new("repoint");
        symlink(fx.root.join("pkg/v1/rtok-hook"), fx.link()).unwrap();
        // The package manager moved `rtok` to v2; the link still names v1.
        std::fs::remove_file(fx.bin().join("rtok")).unwrap();
        symlink(fx.exe("v2"), fx.bin().join("rtok")).unwrap();
        assert_eq!(fx.run("v2"), LinkOutcome::Relinked);
        let want = std::fs::canonicalize(fx.root.join("pkg/v2/rtok-hook")).unwrap();
        assert_eq!(std::fs::canonicalize(fx.link()).unwrap(), want);
    }

    #[cfg(unix)]
    #[test]
    fn hook_client_link_repoints_dangling() {
        use std::os::unix::fs::symlink;
        let fx = LinkFx::new("dangling");
        symlink(fx.root.join("gone/anything"), fx.link()).unwrap();
        assert_eq!(fx.run("v1"), LinkOutcome::Relinked);
        assert!(fx.link().is_file());
    }

    #[cfg(unix)]
    #[test]
    fn hook_client_link_leaves_foreign_names_alone() {
        use std::os::unix::fs::symlink;
        let fx = LinkFx::new("foreign");
        std::fs::write(fx.link(), "mine").unwrap();
        assert_eq!(fx.run("v1"), LinkOutcome::Foreign);
        assert_eq!(std::fs::read_to_string(fx.link()).unwrap(), "mine");
        std::fs::remove_file(fx.link()).unwrap();
        let elsewhere = fx.root.join("pkg/v1/rtok");
        symlink(&elsewhere, fx.link()).unwrap();
        assert_eq!(fx.run("v1"), LinkOutcome::Foreign);
        assert_eq!(std::fs::read_link(fx.link()).unwrap(), elsewhere);
    }

    #[cfg(unix)]
    #[test]
    fn hook_client_link_needs_sibling_and_own_rtok() {
        let fx = LinkFx::new("nosibling");
        // `bin/rtok` is v1, not this executable: never link another install's PATH.
        assert_eq!(fx.run("v2"), LinkOutcome::Nothing);
        assert_eq!(
            ensure_hook_client_link(None, &fx.exe("v1")),
            LinkOutcome::Nothing
        );
        std::fs::remove_file(fx.root.join("pkg/v1/rtok-hook")).unwrap();
        assert_eq!(fx.run("v1"), LinkOutcome::Nothing);
        assert!(std::fs::symlink_metadata(fx.link()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn hook_client_link_read_only_dir_fails_soft() {
        use std::os::unix::fs::PermissionsExt;
        let fx = LinkFx::new("readonly");
        std::fs::set_permissions(fx.bin(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let got = fx.run("v1");
        std::fs::set_permissions(fx.bin(), std::fs::Permissions::from_mode(0o755)).unwrap();
        // root ignores directory modes; either way nothing panics.
        assert!(
            matches!(got, LinkOutcome::Failed | LinkOutcome::Linked),
            "{got:?}"
        );
    }
}

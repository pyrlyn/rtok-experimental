// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The one way rtok finds a host app (T966). Every "is this agent here, and where" question —
//! `agents install|update|info|list`, the installers' own CLI calls, desktop restarts,
//! `mcp ping`, `doctor` — goes through [`locate`], so a path the user gave counts everywhere
//! and no caller can drift into its own idea of "installed" (T426).
//!
//! Order: the user's path from `[setup.paths]`, then the variant's app bundles, then its
//! binaries on PATH. When `agents install|update|info <host>` finds nothing on a terminal,
//! [`ask_missing`] asks for the path; anywhere else the error names the config key.

use std::ffi::OsString;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::{Agent, Kind, Variant, expand_app, expand_spec, find_on_path, host_program};
use crate::config::Config;

/// The config key a user path for `host` lives under.
pub fn key(host: &str) -> String {
    format!("setup.paths.{host}")
}

/// Which of `agent`'s variants a path names: a desktop app's own file name (`Claude.app`,
/// `claude.exe`) first, then a binary's name, then a bundle or folder for the variant with
/// apps, else the first variant with binaries. Needs no file on disk.
pub fn variant_for(agent: &dyn Agent, path: &Path) -> Option<&'static Variant> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let stem = Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let vs = agent.variants();
    let file_name = |spec: &str| {
        Path::new(spec)
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&name))
    };
    vs.iter()
        .find(|v| v.apps.iter().any(|a| file_name(a)))
        .or_else(|| {
            vs.iter()
                .find(|v| v.bins.iter().any(|b| b.eq_ignore_ascii_case(&stem)))
        })
        .or_else(|| {
            (path.is_dir() || name.ends_with(".app"))
                .then(|| vs.iter().find(|v| !v.apps.is_empty()))
                .flatten()
        })
        .or_else(|| vs.iter().find(|v| !v.bins.is_empty()))
        .or_else(|| vs.first())
}

/// The user's path for `host`, when it names variant `v`.
fn user_path(cfg: &Config, host: &str, v: &Variant) -> Option<PathBuf> {
    let raw = cfg.setup.paths.get(host)?;
    let path = expand_app(&raw.to_string_lossy());
    let agent = super::host(host)?;
    (variant_for(agent, &path)?.name == v.name).then_some(path)
}

/// Where variant `v` of `host` is on this machine: the user's path, then an app bundle, then
/// a binary on PATH. `None` means the agent is not installed as far as rtok can tell.
pub fn locate(cfg: &Config, host: &str, v: &Variant) -> Option<PathBuf> {
    user_path(cfg, host, v)
        .into_iter()
        .chain(v.apps.iter().map(|a| expand_app(a)))
        .find_map(super::probe_installed)
        .or_else(|| v.bins.iter().find_map(|b| find_on_path(b)))
}

/// True when [`locate`] finds the app itself. A config folder alone does not count — one left
/// behind by an uninstalled app made setup write into it (T426). Setup refuses a missing app
/// instead of creating its files; removal runs regardless so a half-installed host is cleaned
/// up.
pub fn present(cfg: &Config, host: &str, v: &Variant) -> bool {
    locate(cfg, host, v).is_some()
}

/// The paths a desktop variant's app may be at, the user's first: what a restart looks for
/// among running apps and reopens.
pub(crate) fn app_paths(cfg: &Config, host: &str, v: &Variant) -> Vec<PathBuf> {
    user_path(cfg, host, v)
        .into_iter()
        .chain(v.apps.iter().map(|a| expand_app(a)))
        .map(|p| super::probe_installed(p.clone()).unwrap_or(p))
        .collect()
}

/// What to spawn for variant `v`'s CLI, in the order to try: the user's binary, then each
/// binary name (or, under the test sandbox, the path found for it).
pub(crate) fn programs(cfg: &Config, host: &str, v: &Variant) -> Vec<OsString> {
    user_path(cfg, host, v)
        .filter(|p| p.is_file())
        .map(PathBuf::into_os_string)
        .into_iter()
        .chain(v.bins.iter().filter_map(|b| host_program(b)))
        .collect()
}

/// `host`'s CLI variant: the one its `<bin> plugin …` calls run.
fn cli_variant(host: &str) -> Option<&'static Variant> {
    super::host(host)?
        .variants()
        .iter()
        .find(|v| v.kind == Kind::Cli && !v.bins.is_empty())
}

/// Where `host`'s CLI is, through [`locate`].
pub(crate) fn cli(cfg: &Config, host: &str) -> Option<PathBuf> {
    locate(cfg, host, cli_variant(host)?)
}

/// What to spawn for `host`'s CLI, through [`programs`].
pub(crate) fn cli_program(cfg: &Config, host: &str) -> Option<OsString> {
    programs(cfg, host, cli_variant(host)?).into_iter().next()
}

/// The named hosts none of whose wanted variants is [`present`]: `agents install|update`
/// refuses the whole run on them before any backup or write (T426).
pub fn absent(
    cfg: &Config,
    agents: &[&'static dyn Agent],
    want: impl Fn(Kind) -> bool,
) -> Vec<&'static str> {
    agents
        .iter()
        .filter(|a| {
            !a.variants()
                .iter()
                .any(|v| want(v.kind) && present(cfg, a.id(), v))
        })
        .map(|a| a.id())
        .collect()
}

/// The error for hosts rtok cannot find, naming the key that tells it where they are.
pub fn not_found(hosts: &[&str]) -> anyhow::Error {
    let how: Vec<String> = hosts
        .iter()
        .map(|h| format!("`rtok config set {} <path>`", key(h)))
        .collect();
    anyhow::anyhow!(
        "{} is not installed (not in its app folders or on PATH); if it is, set its path: {}",
        hosts.join(", "),
        how.join(", ")
    )
}

/// A path the user typed for an agent, checked and placed.
#[derive(Debug)]
pub struct Resolved {
    pub host: &'static str,
    pub variant: &'static Variant,
    pub path: PathBuf,
    pub config: Vec<PathBuf>,
}

/// A terminal hands a dragged-in path over quoted (`'/a b/claude'`) or escaped (`/a\ b/claude`).
fn unquote(raw: &str) -> String {
    let s = raw.trim();
    for q in ['\'', '"'] {
        if let Some(inner) = s.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner.to_string();
        }
    }
    if cfg!(windows) {
        s.to_string()
    } else {
        s.replace("\\ ", " ")
    }
}

/// Check a typed path for `agent`: it exists, it is a program (or an app bundle), and it is
/// placed on one of the agent's variants.
pub fn resolve_input(cfg: &Config, agent: &'static dyn Agent, raw: &str) -> Result<Resolved> {
    let typed = expand_spec(&unquote(raw));
    let typed = std::path::absolute(&typed).unwrap_or(typed);
    let Some(path) = super::probe_installed(typed.clone()) else {
        bail!("{}: no such file", typed.display());
    };
    let Some(variant) = variant_for(agent, &path) else {
        bail!("{}: rtok knows no app for {}", path.display(), agent.id());
    };
    let bundle = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("app"));
    if path.is_dir() && !(bundle && !variant.apps.is_empty()) {
        bail!("{}: a folder; give the program itself", path.display());
    }
    if path.is_file() && !executable(&path) {
        bail!("{}: not a program (not executable)", path.display());
    }
    Ok(Resolved {
        host: agent.id(),
        variant,
        path,
        config: agent.files(cfg, variant.kind),
    })
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(_path: &Path) -> bool {
    true
}

/// One line from `input`; `None` at EOF or on a read error.
fn read_line(input: &mut dyn BufRead) -> Option<String> {
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

/// `? {question} [y/N]`, with the SDK's answer rule: only y/yes is a yes.
fn confirm(input: &mut dyn BufRead, out: &mut dyn Write, question: &str) -> Result<bool> {
    write!(out, "? {question} [y/N] ")?;
    out.flush()?;
    Ok(read_line(input).is_some_and(|l| rtok_agent_sdk::answer(&l, false)))
}

/// Ask for `agent`'s path until the user confirms one (`Some`) or gives none (`None`: an empty
/// line or EOF). A path that does not check out is said why and asked again; a "no" to the
/// summary asks again too.
pub fn ask_one(
    cfg: &Config,
    agent: &'static dyn Agent,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<Option<Resolved>> {
    let names: Vec<&str> = agent.variants().iter().map(|v| v.name).collect();
    writeln!(
        out,
        "{} was not found: not in its app folders or on PATH.",
        agent.id()
    )?;
    loop {
        write!(
            out,
            "? Path to {} (the program or app; empty to stop): ",
            names.join(" or ")
        )?;
        out.flush()?;
        let Some(line) = read_line(input).filter(|l| !l.trim().is_empty()) else {
            return Ok(None);
        };
        let found = match resolve_input(cfg, agent, &line) {
            Ok(r) => r,
            Err(e) => {
                writeln!(out, "  {e}")?;
                continue;
            }
        };
        writeln!(
            out,
            "  agent   {} ({})",
            found.variant.name,
            found.variant.kind.label()
        )?;
        writeln!(out, "  path    {}", found.path.display())?;
        let config: Vec<String> = found
            .config
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        writeln!(out, "  config  {}", config.join(", "))?;
        if confirm(input, out, "Is this correct?")? {
            return Ok(Some(found));
        }
    }
}

/// [`ask_one`] for every host in `missing`, each confirmed path put into `cfg` so the run that
/// follows finds it, then offered for the config file through `save` (the key and the path).
/// An unanswered host stops the whole run.
pub fn ask_hosts(
    cfg: &mut Config,
    missing: &[&'static str],
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    save: &mut dyn FnMut(&str, &Path) -> Result<PathBuf>,
) -> Result<()> {
    for &id in missing {
        let Some(agent) = super::host(id) else {
            continue;
        };
        let Some(found) = ask_one(cfg, agent, input, out)? else {
            bail!("stopped: no path for {id}");
        };
        cfg.setup.paths.insert(id.to_string(), found.path.clone());
        let key = key(id);
        if !cfg.setup.dry_run && confirm(input, out, &format!("Save it to the config as `{key}`?"))?
        {
            let file = save(&key, &found.path)?;
            writeln!(out, "  saved   {key} in {}", file.display())?;
        }
    }
    Ok(())
}

/// True when a person can answer: both stdin and stdout are a terminal. Pipes, hooks and the
/// MCP server never are, so they never wait on a question.
pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// `agents install|update|info`: the hosts among `agents` with no wanted variant found get
/// their path asked on a terminal; elsewhere nothing happens and the command reports them
/// itself (an error naming [`key`]).
pub fn ask_missing(
    cfg: &mut Config,
    config_file: Option<&Path>,
    agents: &[&'static dyn Agent],
    want: impl Fn(Kind) -> bool,
) -> Result<()> {
    let missing = absent(cfg, agents, want);
    if missing.is_empty() || !interactive() {
        return Ok(());
    }
    let home = Config::home_dir();
    let mut save = |key: &str, path: &Path| {
        let value = toml_edit::Value::from(path.to_string_lossy().into_owned()).to_string();
        let (file, _) = crate::config::validate::set_with(&home, config_file, key, &value, false)?;
        Ok(file)
    };
    let stdin = std::io::stdin();
    ask_hosts(
        cfg,
        &missing,
        &mut stdin.lock(),
        &mut std::io::stdout(),
        &mut save,
    )
}

#[cfg(test)]
#[path = "locate_tests.rs"]
mod tests;

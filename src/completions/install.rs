// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok completions [<shell>] --install | --uninstall | --list` (T318, T408): the script goes to the shell's
//! standard per-user location. PowerShell also gets one dot-source line in `$PROFILE`; only
//! that line is added or removed, the rest of the profile stays byte-for-byte.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use change_preview::Preview;
use clap::{Command, ValueEnum};

use super::Shell;

/// Where the per-user files live. [`Places::from_env`] reads the process environment; tests
/// build one over a temp dir so no real home is touched.
#[derive(Debug, Clone, Default)]
pub struct Places {
    pub home: PathBuf,
    pub xdg_data: Option<PathBuf>,
    pub xdg_config: Option<PathBuf>,
    pub zdotdir: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    /// `$SHELL`, for picking a shell when none is named.
    pub shell: Option<String>,
    pub windows: bool,
}

impl Places {
    pub fn from_env() -> Result<Self> {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Ok(Self {
            home: crate::config::env_user_home().context("no HOME or USERPROFILE")?,
            xdg_data: var("XDG_DATA_HOME"),
            xdg_config: var("XDG_CONFIG_HOME"),
            zdotdir: var("ZDOTDIR"),
            local_app_data: var("LOCALAPPDATA"),
            shell: std::env::var("SHELL").ok(),
            windows: cfg!(windows),
        })
    }

    fn config(&self) -> PathBuf {
        self.xdg_config
            .clone()
            .unwrap_or_else(|| self.home.join(".config"))
    }

    /// The script path and, for shells that need it, the note on how the shell finds it.
    fn script(&self, shell: Shell) -> Result<(PathBuf, Option<String>)> {
        Ok(match shell {
            Shell::Bash => {
                let data = self
                    .xdg_data
                    .clone()
                    .unwrap_or_else(|| self.home.join(".local/share"));
                (data.join("bash-completion/completions/rtok"), None)
            }
            Shell::Zsh => {
                let dir = self
                    .zdotdir
                    .clone()
                    .unwrap_or_else(|| self.home.clone())
                    .join(".zfunc");
                let note = format!(
                    "zsh loads it when .zshrc has: fpath+=({}); autoload -Uz compinit && compinit",
                    dir.display()
                );
                (dir.join("_rtok"), Some(note))
            }
            Shell::Fish => (self.config().join("fish/completions/rtok.fish"), None),
            Shell::Elvish => (
                self.config().join("elvish/lib/rtok.elv"),
                Some("elvish loads it when rc.elv has: use rtok".into()),
            ),
            Shell::Powershell => (self.ps_profile().with_file_name("rtok.ps1"), None),
            Shell::Clink => {
                let Some(dir) = &self.local_app_data else {
                    bail!("clink needs %LOCALAPPDATA% (Clink runs on Windows)");
                };
                (dir.join("clink/rtok.lua"), None)
            }
        })
    }

    /// pwsh's `$PROFILE` (CurrentUserCurrentHost). A Documents folder moved elsewhere (OneDrive)
    /// is not followed; pass the script to `$PROFILE` by hand there.
    fn ps_profile(&self) -> PathBuf {
        let dir = if self.windows {
            self.home.join("Documents/PowerShell")
        } else {
            self.config().join("powershell")
        };
        dir.join("Microsoft.PowerShell_profile.ps1")
    }

    /// The named shell, else the one `$SHELL` names.
    pub fn pick(&self, shell: Option<Shell>) -> Result<Shell> {
        if let Some(s) = shell {
            return Ok(s);
        }
        let name = self
            .shell
            .as_deref()
            .and_then(|s| Path::new(s).file_stem())
            .and_then(|s| s.to_str());
        match name.and_then(|n| Shell::from_str(n, true).ok()) {
            Some(s) => Ok(s),
            None => {
                let all: Vec<String> = Shell::value_variants().iter().map(|s| s.name()).collect();
                bail!(
                    "cannot tell the shell from $SHELL; name one of: {}",
                    all.join(", ")
                )
            }
        }
    }
}

/// One shell's completions as the per-user files hold them now.
#[derive(Debug, Clone)]
pub struct Status {
    pub shell: Shell,
    /// `None` where the shell has no per-user location here (Clink off Windows).
    pub path: Option<PathBuf>,
    pub installed: bool,
}

/// Every shell's state. Installed means the script exists, the one file [`uninstall`] removes.
pub fn status(places: &Places) -> Vec<Status> {
    Shell::value_variants()
        .iter()
        .map(|&shell| {
            let path = places.script(shell).ok().map(|(p, _)| p);
            let installed = path.as_deref().is_some_and(Path::exists);
            Status {
                shell,
                path,
                installed,
            }
        })
        .collect()
}

fn source_line(script: &Path) -> String {
    format!(". \"{}\"  # rtok completions", script.display())
}

/// The profile with the source line appended; `None` when it is already there.
fn profile_with(old: &str, line: &str) -> Option<String> {
    if old.lines().any(|l| l == line) {
        return None;
    }
    let sep = if old.is_empty() || old.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    Some(format!("{old}{sep}{line}\n"))
}

/// The profile without the source line; `None` when it is not there.
fn profile_without(old: &str, line: &str) -> Option<String> {
    old.lines().any(|l| l == line).then(|| {
        old.split_inclusive('\n')
            .filter(|l| l.trim_end_matches(['\r', '\n']) != line)
            .collect()
    })
}

fn script_body(shell: Shell, cmd: Command) -> Result<String> {
    let mut body = Vec::new();
    super::generate(shell, cmd, &mut body);
    String::from_utf8(body).context("completion script is not UTF-8")
}

/// What [`install`] would change, as the shared diff. Writes nothing.
pub fn preview_install(shell: Shell, cmd: Command, places: &Places) -> Result<Preview> {
    let (script, _) = places.script(shell)?;
    let mut preview = Preview::default();
    preview.edit(&script, &read(&script)?, &script_body(shell, cmd)?);
    if shell == Shell::Powershell {
        let profile = places.ps_profile();
        let old = read(&profile)?;
        if let Some(new) = profile_with(&old, &source_line(&script)) {
            preview.edit(&profile, &old, &new);
        }
    }
    Ok(preview)
}

/// What [`uninstall`] would remove or rewrite. Writes nothing.
pub fn preview_uninstall(shell: Shell, places: &Places) -> Result<Preview> {
    let (script, _) = places.script(shell)?;
    let mut preview = Preview::default();
    if let Ok(meta) = std::fs::metadata(&script) {
        preview.remove(&script, meta.len(), 1);
    }
    if shell == Shell::Powershell {
        let profile = places.ps_profile();
        let old = read(&profile)?;
        if let Some(new) = profile_without(&old, &source_line(&script)) {
            preview.edit(&profile, &old, &new);
        }
    }
    Ok(preview)
}

/// Write the script (and the PowerShell profile line); one report line per file.
pub fn install(shell: Shell, cmd: Command, places: &Places) -> Result<Vec<String>> {
    let (script, note) = places.script(shell)?;
    let body = script_body(shell, cmd)?;
    let mut lines = vec![write_if_changed(&script, &body)?];
    if shell == Shell::Powershell {
        let profile = places.ps_profile();
        let old = read(&profile)?;
        match profile_with(&old, &source_line(&script)) {
            None => lines.push(format!("unchanged {}", profile.display())),
            Some(new) => lines.push(write_if_changed(&profile, &new)?),
        }
    }
    lines.extend(note);
    Ok(lines)
}

/// Remove exactly what [`install`] wrote.
pub fn uninstall(shell: Shell, places: &Places) -> Result<Vec<String>> {
    let (script, _) = places.script(shell)?;
    let mut lines = Vec::new();
    if script.exists() {
        std::fs::remove_file(&script).with_context(|| format!("remove {}", script.display()))?;
        lines.push(format!("removed {}", script.display()));
    } else {
        lines.push(format!("absent {}", script.display()));
    }
    if shell == Shell::Powershell {
        let profile = places.ps_profile();
        let old = read(&profile)?;
        if let Some(kept) = profile_without(&old, &source_line(&script)) {
            rtok_agent_sdk::write_atomic(&profile, &kept)?;
            lines.push(format!("updated {}", profile.display()));
        }
    }
    Ok(lines)
}

fn read(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

fn write_if_changed(path: &Path, body: &str) -> Result<String> {
    if read(path)? == body {
        return Ok(format!("unchanged {}", path.display()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    rtok_agent_sdk::write_atomic(path, body)?;
    Ok(format!("wrote {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places() -> Places {
        Places {
            home: crate::testutil::tmp_dir("completions-install"),
            ..Places::default()
        }
    }

    fn cmd() -> Command {
        // Path tests only need the command name the scripts are written under.
        Command::new("rtok")
    }

    /// The path in a report line. Compared as a `Path`, so `/` and `\` match on Windows.
    fn path(line: &str, verb: &str) -> PathBuf {
        PathBuf::from(line.strip_prefix(verb).unwrap_or_else(|| panic!("{line}")))
    }

    #[test]
    fn each_shell_lands_in_its_standard_place_and_reinstall_is_unchanged() {
        let mut p = places();
        p.local_app_data = Some(p.home.join("AppData/Local"));
        for (shell, rel) in [
            (Shell::Bash, ".local/share/bash-completion/completions/rtok"),
            (Shell::Zsh, ".zfunc/_rtok"),
            (Shell::Fish, ".config/fish/completions/rtok.fish"),
            (Shell::Elvish, ".config/elvish/lib/rtok.elv"),
            (Shell::Powershell, ".config/powershell/rtok.ps1"),
            (Shell::Clink, "AppData/Local/clink/rtok.lua"),
        ] {
            let first = install(shell, cmd(), &p).unwrap();
            assert_eq!(path(&first[0], "wrote "), p.home.join(rel), "{shell:?}");
            let again = install(shell, cmd(), &p).unwrap();
            assert!(again[0].starts_with("unchanged "), "{shell:?}: {again:?}");
            assert_eq!(
                path(&uninstall(shell, &p).unwrap()[0], "removed "),
                p.home.join(rel)
            );
            assert!(!p.home.join(rel).exists());
        }
    }

    #[test]
    fn xdg_dirs_win_over_home_defaults() {
        let mut p = places();
        p.xdg_data = Some(p.home.join("data"));
        let lines = install(Shell::Bash, cmd(), &p).unwrap();
        assert!(
            path(&lines[0], "wrote ").ends_with("data/bash-completion/completions/rtok"),
            "{lines:?}"
        );
    }

    #[test]
    fn powershell_profile_keeps_other_content_byte_for_byte() {
        let p = places();
        let profile = p.ps_profile();
        std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
        let mine = "Set-Alias ll ls\r\n# keep me";
        std::fs::write(&profile, mine).unwrap();
        install(Shell::Powershell, cmd(), &p).unwrap();
        let with = std::fs::read_to_string(&profile).unwrap();
        assert!(with.starts_with(&format!("{mine}\n. \"")), "{with}");
        let again = install(Shell::Powershell, cmd(), &p).unwrap();
        assert_eq!(again[1], format!("unchanged {}", profile.display()));
        uninstall(Shell::Powershell, &p).unwrap();
        assert_eq!(
            std::fs::read_to_string(&profile).unwrap(),
            format!("{mine}\n")
        );
    }

    #[test]
    fn shell_comes_from_dollar_shell_or_the_error_lists_them() {
        let mut p = places();
        p.shell = Some("/usr/local/bin/fish".into());
        assert_eq!(p.pick(None).unwrap(), Shell::Fish);
        assert_eq!(p.pick(Some(Shell::Zsh)).unwrap(), Shell::Zsh);
        p.shell = None;
        let err = p.pick(None).unwrap_err().to_string();
        assert!(
            err.contains("bash, zsh, fish, powershell, elvish, clink"),
            "{err}"
        );
    }

    #[test]
    fn clink_without_local_app_data_is_refused() {
        let err = install(Shell::Clink, cmd(), &places())
            .unwrap_err()
            .to_string();
        assert!(err.contains("LOCALAPPDATA"), "{err}");
    }
}

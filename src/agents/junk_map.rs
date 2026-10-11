// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The host junk map of `research.md` §22 (T182, D36), encoded once (T330.2). Per host id it
//! names the folders the host writes: the temp, log and cache paths §22 documents (marked
//! `documented`), its config/data home, and the platform folders of a desktop app that §22
//! has no row for. Only a `documented` path may ever be cleared (T330.3); the rest is listed
//! read-only. A template starts with a token (`{home}`, `{claude}`, `{xdg_cache}`, ...) so an
//! environment override moves every path under it, and a folder that does not exist is simply
//! absent from the report.

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The host's config or data home. Never junk as a whole (§22).
    Data,
    Temp,
    Logs,
    Cache,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Data => "data",
            Role::Temp => "temp",
            Role::Logs => "logs",
            Role::Cache => "cache",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub role: Role,
    pub path: String,
    /// A row of §22 (official docs or the host's own source), not a guess.
    pub documented: bool,
    /// The folder holds one subfolder per host build (VS Code's `CachedData/<commit>`): only
    /// the ones of builds that are no longer current are cache, never the folder itself.
    pub per_commit: bool,
}

fn doc(role: Role, path: &str) -> Spec {
    Spec {
        role,
        path: path.into(),
        documented: true,
        per_commit: false,
    }
}

fn per_commit(path: &str) -> Spec {
    Spec {
        per_commit: true,
        ..doc(Role::Cache, path)
    }
}

fn guess(role: Role, path: String) -> Spec {
    Spec {
        role,
        path,
        documented: false,
        per_commit: false,
    }
}

/// Where the platform keeps an Electron app's data and caches. §22 has no row for these apps,
/// so every folder is listed read-only; the app names are the products' own.
fn electron(app: &str) -> Vec<Spec> {
    let data = [
        format!("{{home}}/Library/Application Support/{app}"),
        format!("{{xdg_config}}/{app}"),
    ];
    let mut specs: Vec<Spec> = data.iter().map(|d| guess(Role::Data, d.clone())).collect();
    specs.push(guess(Role::Cache, format!("{{home}}/Library/Caches/{app}")));
    specs.push(guess(Role::Cache, format!("{{xdg_cache}}/{app}")));
    // Electron's `sessionData` mixes these with cookies and localStorage, so only these named
    // subfolders can be cache (research.md §22.2); `Service Worker/CacheStorage` is app data.
    for d in &data {
        for sub in ELECTRON_CACHES {
            specs.push(guess(Role::Cache, format!("{d}/{sub}")));
        }
    }
    specs
}

const ELECTRON_CACHES: [&str; 5] = ["Cache", "Code Cache", "GPUCache", "CachedData", "DawnCache"];

/// Hosts with no `specs` row on purpose: §22 reads "not documented" for every path they write
/// (the config folder they already name through their markers is still listed). Keeping the
/// list explicit makes a new host in `HOSTS` fail a test until it has a row or joins this list.
pub const NO_ROWS: [&str; 3] = ["kilo", "aider", "roo"];

/// The §22 rows and the desktop-app folders of `host`. A host §22 reads "not documented" for
/// (Cursor, Kilo, Aider, ...) has only what its own config files already name.
pub fn specs(host: &str) -> Vec<Spec> {
    use Role::{Cache, Data, Logs, Temp};
    match host {
        "claude" => vec![
            doc(Data, "{claude}"),
            doc(Temp, "{claude}/shell-snapshots"),
            doc(Logs, "{claude}/debug"),
            doc(Cache, "{claude}/paste-cache"),
        ],
        "codex" => vec![doc(Data, "{codex}"), doc(Logs, "{codex}/log")],
        "opencode" => vec![
            doc(Data, "{xdg_data}/opencode"),
            doc(Logs, "{xdg_data}/opencode/log"),
            doc(Cache, "{xdg_cache}/opencode"),
        ],
        "pi" => vec![
            doc(Data, "{home}/.pi/agent"),
            doc(Logs, "{home}/.pi/agent/pi-debug.log"),
        ],
        "omp" => vec![
            doc(Data, "{home}/.omp"),
            doc(Cache, "{home}/.omp/cache/github-cache.db"),
        ],
        "zcode" => vec![
            doc(Data, "{home}/.zcode"),
            doc(Temp, "{home}/.zcode/cli/exec"),
        ],
        "kimi" => vec![
            doc(Data, "{home}/.kimi-code"),
            doc(Logs, "{home}/.kimi-code/logs"),
            doc(Cache, "{home}/.kimi-code/bin"),
            doc(Cache, "{home}/.kimi-code/updates/latest.json"),
        ],
        "grok" => vec![doc(Data, "{home}/.grok"), doc(Logs, "{home}/.grok/logs")],
        "copilot" => vec![
            doc(Data, "{home}/.copilot"),
            doc(Logs, "{home}/.copilot/logs"),
            doc(Cache, "{copilot_cache}"),
        ],
        "zed" => vec![
            doc(Data, "{home}/Library/Application Support/Zed"),
            doc(Logs, "{home}/Library/Logs/Zed/Zed.log"),
            doc(Logs, "{xdg_data}/zed/logs/Zed.log"),
            doc(Cache, "{home}/Library/Caches/Zed"),
            doc(Cache, "{xdg_cache}/zed"),
        ],
        "gemini" => vec![doc(Data, "{home}/.gemini")],
        "codewhale" => vec![
            doc(Data, "{home}/.codewhale"),
            doc(Cache, "{home}/.codewhale/update-check.json"),
        ],
        "commandcode" => vec![doc(Data, "{home}/.commandcode")],
        "cline" => vec![
            doc(Data, "{home}/.cline"),
            doc(Logs, "{home}/.cline/data/logs"),
        ],
        "mimo" => vec![
            doc(Data, "{home}/.config/mimocode"),
            doc(Data, "{home}/.local/share/mimocode"),
            doc(Logs, "{home}/.local/share/mimocode/log"),
            doc(Cache, "{home}/.cache/mimocode"),
        ],
        "devin" => vec![
            doc(Data, "{home}/.config/devin"),
            doc(Logs, "{home}/.local/share/devin/cli/logs"),
        ],
        "qwen" => vec![doc(Data, "{home}/.qwen")],
        "codebuddy" => vec![
            doc(Data, "{home}/.codebuddy"),
            doc(Data, "{home}/.workbuddy"),
        ],
        "vscode" => {
            let mut v = vec![doc(Data, "{xdg_config}/Code")];
            // Before `electron`, which lists the same folders read-only: the first spec wins.
            v.push(per_commit(
                "{home}/Library/Application Support/Code/CachedData",
            ));
            v.push(per_commit("{xdg_config}/Code/CachedData"));
            v.extend(electron("Code"));
            v
        }
        "cursor" => electron("Cursor"),
        "windsurf" => electron("Windsurf"),
        "antigravity" => electron("Antigravity"),
        _ => Vec::new(),
    }
}

/// The roots the templates start from, read from the environment once so a test can build its
/// own (no test touches the process environment).
#[derive(Debug, Clone)]
pub struct Roots {
    home: PathBuf,
    claude: PathBuf,
    codex: PathBuf,
    xdg_cache: PathBuf,
    xdg_config: PathBuf,
    xdg_data: PathBuf,
    copilot_cache: PathBuf,
    rtok_cache: PathBuf,
}

impl Roots {
    /// `get` answers an environment variable. An empty or relative value is ignored, as the
    /// XDG spec says for its own variables.
    pub fn new(home: PathBuf, get: impl Fn(&str) -> Option<OsString>) -> Self {
        let dir = |var: &str, default: PathBuf| {
            get(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or(default)
        };
        let xdg_cache = dir("XDG_CACHE_HOME", home.join(".cache"));
        let copilot_default = if cfg!(target_os = "macos") {
            home.join("Library/Caches/copilot")
        } else {
            xdg_cache.join("copilot")
        };
        let rtok_cache = if cfg!(target_os = "macos") {
            home.join("Library/Caches/rtok")
        } else if cfg!(windows) {
            dir("LOCALAPPDATA", home.join("AppData/Local")).join("rtok/cache")
        } else {
            xdg_cache.join("rtok")
        };
        Self {
            rtok_cache,
            claude: dir("CLAUDE_CONFIG_DIR", home.join(".claude")),
            codex: dir("CODEX_HOME", home.join(".codex")),
            xdg_config: dir("XDG_CONFIG_HOME", home.join(".config")),
            xdg_data: dir("XDG_DATA_HOME", home.join(".local/share")),
            copilot_cache: dir("COPILOT_CACHE_HOME", copilot_default),
            xdg_cache,
            home,
        }
    }

    pub fn from_env() -> Self {
        Self::new(super::home_dir(), |k| std::env::var_os(k))
    }

    pub fn home(&self) -> &std::path::Path {
        &self.home
    }

    /// `{token}` or `{token}/rest` as a path. A template without a known token is returned as
    /// written.
    pub fn resolve(&self, template: &str) -> PathBuf {
        let Some((token, rest)) = template.strip_prefix('{').and_then(|r| r.split_once('}')) else {
            return PathBuf::from(template);
        };
        let base = match token {
            "home" => &self.home,
            "claude" => &self.claude,
            "codex" => &self.codex,
            "xdg_cache" => &self.xdg_cache,
            "xdg_config" => &self.xdg_config,
            "xdg_data" => &self.xdg_data,
            "copilot_cache" => &self.copilot_cache,
            "rtok_cache" => &self.rtok_cache,
            _ => return PathBuf::from(template),
        };
        match rest.strip_prefix('/') {
            Some(tail) => base.join(tail),
            None => base.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute path on every OS (`/c` is not absolute on Windows, so `Roots::new` would drop it).
    fn abs(name: &str) -> PathBuf {
        std::env::temp_dir().join("rtok-junk-map").join(name)
    }

    fn roots(env: &[(&str, PathBuf)]) -> Roots {
        let env: Vec<(String, PathBuf)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        Roots::new(abs("h"), move |k| {
            env.iter().find(|(n, _)| n == k).map(|(_, v)| v.into())
        })
    }

    #[test]
    fn defaults_sit_under_home() {
        let r = roots(&[]);
        let h = abs("h");
        assert_eq!(r.resolve("{claude}/debug"), h.join(".claude").join("debug"));
        assert_eq!(r.resolve("{codex}"), h.join(".codex"));
        assert_eq!(
            r.resolve("{xdg_cache}/opencode"),
            h.join(".cache").join("opencode")
        );
        assert_eq!(
            r.resolve("{xdg_data}/opencode/log"),
            h.join(".local").join("share").join("opencode").join("log")
        );
    }

    #[test]
    fn environment_overrides_move_every_path_under_them() {
        let r = roots(&[
            ("CLAUDE_CONFIG_DIR", abs("c")),
            ("CODEX_HOME", abs("x")),
            ("XDG_CACHE_HOME", abs("xc")),
            ("COPILOT_CACHE_HOME", abs("cc")),
        ]);
        assert_eq!(
            r.resolve("{claude}/paste-cache"),
            abs("c").join("paste-cache")
        );
        assert_eq!(r.resolve("{codex}/log"), abs("x").join("log"));
        assert_eq!(r.resolve("{xdg_cache}/zed"), abs("xc").join("zed"));
        assert_eq!(r.resolve("{copilot_cache}"), abs("cc"));
    }

    #[test]
    fn a_relative_or_empty_override_is_ignored() {
        let r = roots(&[
            ("CLAUDE_CONFIG_DIR", PathBuf::from("rel")),
            ("CODEX_HOME", PathBuf::new()),
        ]);
        assert_eq!(r.resolve("{claude}"), abs("h").join(".claude"));
        assert_eq!(r.resolve("{codex}"), abs("h").join(".codex"));
    }

    #[test]
    fn only_documented_rows_come_from_the_section_22_table_and_cursor_has_none() {
        assert!(specs("cursor").iter().all(|s| !s.documented));
        assert!(specs("claude").iter().all(|s| s.documented));
        assert!(specs("claude").iter().any(|s| s.role == Role::Logs));
    }

    #[test]
    fn every_host_has_a_row_or_is_listed_as_having_none() {
        for host in crate::agents::HOSTS {
            let none = NO_ROWS.contains(host);
            assert_eq!(
                none,
                specs(host).is_empty(),
                "{host}: add a row to `specs`, or list it in NO_ROWS"
            );
        }
    }

    #[test]
    fn only_vscode_code_cache_is_per_commit_and_documented() {
        let per: Vec<_> = crate::agents::HOSTS
            .iter()
            .flat_map(|h| specs(h).into_iter().map(move |s| (*h, s)))
            .filter(|(_, s)| s.per_commit)
            .collect();
        assert_eq!(per.len(), 2);
        assert!(per.iter().all(|(h, s)| *h == "vscode"
            && s.documented
            && s.role == Role::Cache
            && s.path.ends_with("/Code/CachedData")));
        // Cursor shares the layout only by unverified report (§22.2): it stays list-only.
        assert!(
            specs("cursor")
                .iter()
                .all(|s| !s.documented && !s.per_commit)
        );
    }

    #[test]
    fn every_template_starts_with_a_known_token() {
        let r = roots(&[]);
        for host in crate::agents::HOSTS {
            for s in specs(host) {
                assert!(
                    r.resolve(&s.path).is_absolute(),
                    "{host}: `{}` has no known token",
                    s.path
                );
            }
        }
    }
}

// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! A throwaway home with every host's files under it, and the binary run against it.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rtok")
}

/// `s` with every `\` as `/`. [`write_cfg`] hands rtok `/`-joined paths and rtok joins what it
/// derives with the OS separator, so Windows output mixes both; compare both sides through this
/// (T83.4).
pub fn slash(s: impl AsRef<str>) -> String {
    s.as_ref().replace('\\', "/")
}

/// A fresh empty directory, unique per test and process.
pub fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rtok-agents-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// One config pointing every host at files inside `home`; the host dirs exist so `present`
/// says yes for each of them.
pub fn write_cfg(home: &Path) -> PathBuf {
    for sub in [
        ".claude",
        ".cursor",
        ".codex",
        ".config/opencode",
        ".config/kilo",
        ".pi/agent",
        ".omp/agent",
        ".zcode/cli",
        ".kimi-code",
        ".grok",
        "Library/Application Support/Code/User",
        "Library/Application Support/Code - Insiders/User",
        ".copilot/hooks",
        ".commandcode",
        ".codeium/windsurf",
        ".config/zed",
        "Documents/Cline/Hooks",
        ".cline/data/settings",
        ".gemini",
        ".codewhale",
        ".config/mimocode",
        ".gemini/config/plugins",
        ".gemini/antigravity-cli/plugins",
        "devin",
        "Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings",
        ".qwen",
        ".codebuddy",
        ".workbuddy",
    ] {
        fs::create_dir_all(home.join(sub)).unwrap();
    }
    let cfg = home.join("config.toml");
    let h = home.display().to_string().replace('\\', "/");
    fs::write(
        &cfg,
        format!(
            "[doctor]\nclaude_json = \"{h}/.claude.json\"\n\
             [setup.claude]\nsettings_path = \"{h}/.claude/settings.json\"\n\
             [setup.cursor]\nhooks_path = \"{h}/.cursor/hooks.json\"\n\
             [setup.codex]\nconfig_path = \"{h}/.codex/config.toml\"\n\
             [setup.opencode]\nconfig_path = \"{h}/.config/opencode/opencode.json\"\n\
             [setup.kilo]\nconfig_path = \"{h}/.config/kilo/kilo.json\"\n\
             [setup.pi]\nextensions_path = \"{h}/.pi/agent/extensions\"\n\
             [setup.omp]\nextensions_path = \"{h}/.omp/agent/extensions\"\n\
             mcp_path = \"{h}/.omp/agent/mcp.json\"\n\
             [setup.zcode]\nconfig_path = \"{h}/.zcode/cli/config.json\"\n\
             [setup.kimi]\nconfig_path = \"{h}/.kimi-code/config.toml\"\n\
             [setup.grok]\nconfig_path = \"{h}/.grok/config.toml\"\n\
             [setup.vscode]\ncode_user_dir = \"{h}/Library/Application Support/Code/User\"\n\
             insiders_user_dir = \"{h}/Library/Application Support/Code - Insiders/User\"\n\
              [setup.copilot]\ndir = \"{h}/.copilot\"\n\
              [setup.commandcode]\ndir = \"{h}/.commandcode\"\n\
             [setup.aider]\nconfig_path = \"{h}/.aider.conf.yml\"\n\
              [setup.windsurf]\nconfig_path = \"{h}/.codeium/windsurf/mcp_config.json\"\n\
              [setup.zed]\nconfig_path = \"{h}/.config/zed/settings.json\"\n\
             [setup.cline]\nhooks_path = \"{h}/Documents/Cline/Hooks\"\n\
             mcp_path = \"{h}/.cline/data/settings/cline_mcp_settings.json\"\n\
              [setup.gemini]\ndir = \"{h}/.gemini\"\n\
              [setup.codewhale]\ndir = \"{h}/.codewhale\"\n\
              [setup.mimo]\nconfig_path = \"{h}/.config/mimocode/mimocode.json\"\n\
              [setup.antigravity]\nplugins_path = \"{h}/.gemini/config/plugins\"\n\
              cli_plugins_path = \"{h}/.gemini/antigravity-cli/plugins\"\n\
              [setup.devin]\nconfig_path = \"{h}/devin/config.json\"\n\
              [setup.roo]\nmcp_path = \"{h}/Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json\"\n\
              [setup.qwen]\ndir = \"{h}/.qwen\"\n\
              [setup.codebuddy]\ndir = \"{h}/.codebuddy\"\nworkbuddy_dir = \"{h}/.workbuddy\"\n"
        ),
    )
    .unwrap();
    cfg
}

/// `rtok --config <cfg> <args>` with `home` as HOME, USERPROFILE and APPDATA, so every
/// platform's home-relative path lands inside the temp dir.
///
/// `agents install|remove|update` calls (and their `agent` / `setup` / `uninstall` aliases) get
/// `--no-restart` appended (T141): these tests must never
/// shell out to a real `osascript`/`pgrep`/`tasklist` to probe whether some app on the test
/// machine happens to be running, let alone quit or reopen one.
pub fn raw(args: &[&str], cfg: &Path, home: &Path) -> Output {
    raw_with_path(args, cfg, home, fake_claude_path(home))
}

/// [`raw`], but every host CLI on PATH is a [`fake_hosts`] stub that fails anything past
/// `--version` — the host is installed (T426) yet `claude plugin install` and its kin fail, so
/// `rtok agents install claude` falls back to the settings-file surfaces instead of the plugin
/// (T139).
pub fn raw_without_claude(args: &[&str], cfg: &Path, home: &Path) -> Output {
    let path = std::env::join_paths([fake_hosts(home)]).unwrap();
    raw_with_path(args, cfg, home, path)
}

fn raw_with_path(args: &[&str], cfg: &Path, home: &Path, path: std::ffi::OsString) -> Output {
    let mut full: Vec<&str> = args.to_vec();
    if matches!(
        args,
        [
            "agents" | "agent",
            "install" | "setup" | "remove" | "uninstall" | "update",
            ..
        ]
    ) {
        full.push("--no-restart");
    }
    Command::new(bin())
        .args(["--config", cfg.to_str().unwrap()])
        .args(&full)
        .env("PATH", path)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("APPDATA", home)
        .env("RTOK_HOME", home.join(".rtok"))
        // T279: `plugin_version::default_receipt_path`'s own fallback for each OS, pinned
        // explicitly rather than left to whatever the test machine happens to have set — a
        // real `XDG_STATE_HOME`/`LOCALAPPDATA` on the runner would otherwise send the T279
        // receipt outside `home` and into that user's actual state directory.
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("LOCALAPPDATA", home.join("AppData/Local"))
        .output()
        .expect("rtok")
}

/// A fake `claude` (T115) and a fake `codex` (T140) first on PATH, so no test ever runs
/// either real CLI: `claude` answers the detection probe (`--version`), appends every other
/// argv to `<home>/claude.log` and keeps `<config dir>/plugins/installed_plugins.json` the way
/// `claude plugin install` / `uninstall` / `update` do (`update` fails while
/// `<home>/fake-claude-fail-update` exists, T242.3; `install` fails while
/// `<home>/fake-claude-fail-install` exists, T279 step 3/6 "Failure"); `codex` answers
/// `--version` and edits
/// `${CODEX_HOME:-$HOME/.codex}/config.toml`'s `[marketplaces.rtok]` / `[plugins."rtok@rtok"]`
/// tables the way `codex plugin marketplace add|remove` / `plugin add|remove` do, including the
/// real CLI's "already added from a different source" error on a second `marketplace add` with
/// a different source; `marketplace upgrade rtok` rewrites the installed cache's `.mcp.json`
/// (fails while `<home>/fake-codex-fail-upgrade` exists, T242.4). A shell script on Unix; on Windows a `.cmd` shim (the same shape npm
/// installs the real CLI as), which `agents::run_cli`'s `cmd /C` wrapper (T139 windows fix)
/// resolves the way it resolves the real thing.
/// A fake `copilot` beside the fake `claude`, so `rtok()`'s PATH picks it up: logs every
/// call to `$HOME/copilot.log` and mirrors `plugin install` / `plugin uninstall` into the
/// `installed-plugins/` marker layout the real CLI writes (T116). `--version` prints
/// deterministic wrapper noise ahead of the version (T168): the real npm wrapper leaks
/// `Package extraction …` lines into `--version`, so the shim carries a fixed noise line
/// and the byte-comparing tables stay hermetic without caring what npm prints.
pub fn fake_copilot(home: &Path) {
    let dir = home.join(".fake-bin");
    fs::create_dir_all(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("copilot");
        if !bin.exists() {
            fs::write(
                &bin,
                r#"#!/bin/sh
[ "$1" = --version ] && { echo "Package extraction took 1234ms"; echo "0.1.0 (fake copilot)"; exit 0; }
echo "$*" >> "$HOME/copilot.log"
plugins="${COPILOT_HOME:-$HOME/.copilot}/installed-plugins"
case "$*" in
  "plugin install "*) mkdir -p "$plugins/_direct/x"
    printf '{"name":"rtok","version":"0.0.1"}' > "$plugins/_direct/x/plugin.json" ;;
  "plugin uninstall rtok") rm -rf "$plugins/_direct/x" ;;
esac
"#,
            )
            .unwrap();
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    #[cfg(windows)]
    {
        let bin = dir.join("copilot.cmd");
        if !bin.exists() {
            fs::write(
                &bin,
                r#"@echo off
if "%~1"=="--version" (
  echo Package extraction took 1234ms
  echo 0.1.0 ^(fake copilot^)
  exit /b 0
)
set "ALLARGS=%*"
echo %ALLARGS%>>"%HOME%\copilot.log"
if defined COPILOT_HOME (set "PLUGINS=%COPILOT_HOME%\installed-plugins") else (set "PLUGINS=%HOME%\.copilot\installed-plugins")
echo %ALLARGS%| findstr /b /c:"plugin install " >nul && (
  mkdir "%PLUGINS%\_direct\x" 2>nul
  >"%PLUGINS%\_direct\x\plugin.json" echo {"name":"rtok","version":"0.0.1"}
)
if "%ALLARGS%"=="plugin uninstall rtok" rmdir /s /q "%PLUGINS%\_direct\x" 2>nul
"#,
            )
            .unwrap();
        }
    }
}

/// A fake `gemini` (T118.3): `--version` answers deterministically, every other call logs to
/// `$HOME/gemini.log`, and `extensions link <path>`/`extensions uninstall rtok` mirror the
/// real CLI's `~/.gemini/extensions/<name>/gemini-extension.json` marker — a fixed manifest
/// naming `rtok`, not an actual copy of `<path>`, the same shortcut `fake_copilot` takes for
/// `plugin install`, since `plugin_installed` only reads the manifest's `name`.
pub fn fake_gemini(home: &Path) {
    let dir = home.join(".fake-bin");
    fs::create_dir_all(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("gemini");
        if !bin.exists() {
            fs::write(
                &bin,
                r#"#!/bin/sh
[ "$1" = --version ] && { echo "0.1.0 (fake gemini)"; exit 0; }
echo "$*" >> "$HOME/gemini.log"
ext="${GEMINI_CLI_HOME:-$HOME/.gemini}/extensions"
case "$*" in
  "extensions link "*) mkdir -p "$ext/rtok"
    printf '{"name":"rtok","version":"0.0.1"}' > "$ext/rtok/gemini-extension.json" ;;
  "extensions uninstall rtok") rm -rf "$ext/rtok" ;;
esac
"#,
            )
            .unwrap();
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    #[cfg(windows)]
    {
        let bin = dir.join("gemini.cmd");
        if !bin.exists() {
            fs::write(
                &bin,
                r#"@echo off
if "%~1"=="--version" (
  echo 0.1.0 ^(fake gemini^)
  exit /b 0
)
set "ALLARGS=%*"
echo %ALLARGS%>>"%HOME%\gemini.log"
if defined GEMINI_CLI_HOME (set "EXT=%GEMINI_CLI_HOME%\extensions") else (set "EXT=%HOME%\.gemini\extensions")
echo %ALLARGS%| findstr /b /c:"extensions link " >nul && (
  mkdir "%EXT%\rtok" 2>nul
  >"%EXT%\rtok\gemini-extension.json" echo {"name":"rtok","version":"0.0.1"}
)
if "%ALLARGS%"=="extensions uninstall rtok" rmdir /s /q "%EXT%\rtok" 2>nul
"#,
            )
            .unwrap();
        }
    }
}

/// Every host variant installed under `home` (T426: setup refuses a host whose app or binary
/// is missing): a stub per CLI in `<home>/.fake-hosts` that answers `--version` and fails
/// anything else, as a host CLI rtok cannot drive; it goes on PATH after `.fake-bin` so the
/// scripted fakes above win. App-only variants get an empty bundle at their first absolute app
/// path, where the host sandbox looks for it.
pub fn fake_hosts(home: &Path) -> PathBuf {
    let dir = home.join(".fake-hosts");
    fs::create_dir_all(&dir).unwrap();
    for v in rtok::agents::HOSTS
        .iter()
        .filter_map(|id| rtok::agents::host(id))
        .flat_map(|a| a.variants())
    {
        for bin in v.bins {
            fake_stub(&dir, bin);
        }
        if v.bins.is_empty()
            && let Some(app) = v.apps.iter().find_map(|a| a.strip_prefix('/'))
        {
            fs::create_dir_all(home.join(app)).unwrap();
        }
    }
    dir
}

fn fake_stub(dir: &Path, name: &str) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join(name);
        if !bin.exists() {
            fs::write(&bin, "#!/bin/sh\n[ \"$1\" = --version ] && { echo 0.0.0; exit 0; }\necho \"fake host: unsupported\" >&2\nexit 1\n").unwrap();
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    #[cfg(windows)]
    {
        let bin = dir.join(format!("{name}.cmd"));
        if !bin.exists() {
            fs::write(
                &bin,
                "@echo off\r\nif \"%~1\"==\"--version\" (\r\n  echo 0.0.0\r\n  exit /b 0\r\n)\r\necho fake host: unsupported 1>&2\r\nexit /b 1\r\n",
            )
            .unwrap();
        }
    }
}

pub fn fake_claude_path(home: &Path) -> std::ffi::OsString {
    // T168: the copilot shim lives beside claude/codex so every `raw`/`rtok` probe is
    // hermetic — without it `app_version` reached the real npm wrapper, whose
    // `Package extraction …` noise flakes the byte-compared `agents list` tables.
    fake_copilot(home);
    let hosts = fake_hosts(home);
    let path = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = home.join(".fake-bin");
        fs::create_dir_all(&dir).unwrap();
        let claude = dir.join("claude");
        if !claude.exists() {
            // T132: a real `claude plugin install` copies the plugin tree into its cache
            // (`plugins/claude/README.md`), `agents/` included — mirror that here with the
            // repo's actual shipped file, so an install/removal e2e can assert on it without
            // hardcoding the agent's contents twice.
            let scout_src =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/claude/agents/rtok-scout.md");
            let script = r#"#!/bin/sh
[ "$1" = --version ] && { echo "2.0.0 (Claude Code)"; exit 0; }
echo "$*" >> "$HOME/claude.log"
plugins="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/plugins"
case "$*" in
  "plugin install rtok@rtok")
    [ -f "$HOME/fake-claude-fail-install" ] && { echo "install failed" >&2; exit 1; }
    mkdir -p "$plugins/cache/rtok/agents"
    cp "__SCOUT_SRC__" "$plugins/cache/rtok/agents/rtok-scout.md"
    printf '{"version":2,"plugins":{"rtok@rtok":[{"scope":"user"}]}}' > "$plugins/installed_plugins.json" ;;
  "plugin uninstall rtok@rtok") rm -f "$plugins/installed_plugins.json"
    rm -rf "$plugins/cache/rtok" ;;
  "plugin update rtok@rtok")
    [ -f "$HOME/fake-claude-fail-update" ] && { echo "update failed" >&2; exit 1; }
    mkdir -p "$plugins"
    printf '{"version":2,"plugins":{"rtok@rtok":[{"scope":"user","version":"latest"}]}}' > "$plugins/installed_plugins.json" ;;
esac
"#
            .replace("__SCOUT_SRC__", &scout_src.display().to_string());
            fs::write(&claude, script).unwrap();
            fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let codex = dir.join("codex");
        if !codex.exists() {
            fs::write(&codex, FAKE_CODEX_SH).unwrap();
            fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
        }
        // `app_version` runs `cursor`, `cursor-agent`, and `agent` with `--version` once
        // the desktop bundle is present. These shims answer and exit so the real CLIs
        // never start.
        for name in ["cursor", "cursor-agent", "agent"] {
            let bin = dir.join(name);
            if !bin.exists() {
                fs::write(
                    &bin,
                    "#!/bin/sh\n[ \"$1\" = --version ] && echo 0.0.0\nexit 0\n",
                )
                .unwrap();
                fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let mut dirs = vec![dir, hosts];
        dirs.extend(std::env::split_paths(&path));
        return std::env::join_paths(dirs).unwrap();
    }
    #[cfg(windows)]
    {
        let dir = home.join(".fake-bin");
        fs::create_dir_all(&dir).unwrap();
        let claude = dir.join("claude.cmd");
        if !claude.exists() {
            fs::write(
                &claude,
                r#"@echo off
if "%~1"=="--version" (
  echo 2.0.0 Claude Code
  exit /b 0
)
set "ALLARGS=%*"
echo %ALLARGS%>>"%HOME%\claude.log"
if defined CLAUDE_CONFIG_DIR (
  set "PLUGINS=%CLAUDE_CONFIG_DIR%\plugins"
) else (
  set "PLUGINS=%HOME%\.claude\plugins"
)
if "%ALLARGS%"=="plugin install rtok@rtok" (
  rem `exit`, not `exit /b`: cmd /C loses a nested `exit /b` code and reports 0.
  if exist "%HOME%\fake-claude-fail-install" (
    echo install failed 1>&2
    exit 1
  )
  mkdir "%PLUGINS%" 2>nul
  >"%PLUGINS%\installed_plugins.json" echo {"version":2,"plugins":{"rtok@rtok":[{"scope":"user"}]}}
)
if "%ALLARGS%"=="plugin uninstall rtok@rtok" (
  del /f /q "%PLUGINS%\installed_plugins.json" 2>nul
)
if "%ALLARGS%"=="plugin update rtok@rtok" (
  rem `exit`, not `exit /b`: cmd /C loses a nested `exit /b` code and reports 0.
  if exist "%HOME%\fake-claude-fail-update" (
    echo update failed 1>&2
    exit 1
  )
  mkdir "%PLUGINS%" 2>nul
  >"%PLUGINS%\installed_plugins.json" echo {"version":2,"plugins":{"rtok@rtok":[{"scope":"user","version":"latest"}]}}
)
"#,
            )
            .unwrap();
        }
        let codex = dir.join("codex.cmd");
        if !codex.exists() {
            fs::write(&codex, FAKE_CODEX_CMD).unwrap();
        }
        for name in ["cursor.cmd", "cursor-agent.cmd", "agent.cmd"] {
            let bin = dir.join(name);
            if !bin.exists() {
                fs::write(
                    &bin,
                    "@echo off\r\nif \"%~1\"==\"--version\" echo 0.0.0\r\nexit /b 0\r\n",
                )
                .unwrap();
            }
        }
        let mut dirs = vec![dir, hosts];
        dirs.extend(std::env::split_paths(&path));
        return std::env::join_paths(dirs).unwrap();
    }
    #[allow(unreachable_code)]
    path
}

/// `codex`'s config: `${CODEX_HOME:-$HOME/.codex}/config.toml` (matches `agents::codex::plugin`'s
/// `CODEX_HOME` override, which is only set when `config_path` is not the default).
#[cfg(unix)]
const FAKE_CODEX_SH: &str = r#"#!/bin/sh
[ "$1" = --version ] && { echo "codex-cli 0.155.1"; exit 0; }
echo "$*" >> "$HOME/codex.log"
cfg="${CODEX_HOME:-$HOME/.codex}/config.toml"
mkdir -p "$(dirname "$cfg")"
touch "$cfg"
case "$*" in
  "plugin marketplace add pyrlyn/rtok")
    if grep -q '^source = "https://github.com/pyrlyn/rtok.git"$' "$cfg" 2>/dev/null; then
      exit 0
    fi
    if grep -q '^\[marketplaces.rtok\]$' "$cfg" 2>/dev/null; then
      echo "rtok: already added from a different source" >&2
      exit 1
    fi
    printf '\n[marketplaces.rtok]\nsource_type = "git"\nsource = "https://github.com/pyrlyn/rtok.git"\n' >> "$cfg"
    ;;
  "plugin marketplace remove rtok")
    grep -q '^\[marketplaces.rtok\]$' "$cfg" 2>/dev/null || { echo "rtok: no such marketplace" >&2; exit 1; }
    awk '/^\[marketplaces\.rtok\]$/{skip=1;next} /^\[/{skip=0} !skip' "$cfg" > "$cfg.tmp" && mv "$cfg.tmp" "$cfg"
    ;;
  "plugin add rtok@rtok")
    grep -q '^\[plugins\."rtok@rtok"\]$' "$cfg" 2>/dev/null || printf '\n[plugins."rtok@rtok"]\nenabled = true\n' >> "$cfg"
    ;;
  "plugin marketplace upgrade rtok")
    [ -e "$HOME/fake-codex-fail-upgrade" ] && { echo "rtok: upgrade failed" >&2; exit 1; }
    d="$(dirname "$cfg")/plugins/cache/rtok/rtok/0.0.1"
    mkdir -p "$d" && echo upgraded > "$d/.mcp.json"
    ;;
  "plugin remove rtok@rtok")
    awk '/^\[plugins\."rtok@rtok"\]$/{skip=1;next} /^\[/{skip=0} !skip' "$cfg" > "$cfg.tmp" && mv "$cfg.tmp" "$cfg"
    ;;
esac
"#;

/// [`FAKE_CODEX_SH`]'s Windows counterpart.
#[cfg(windows)]
const FAKE_CODEX_CMD: &str = r#"@echo off
if "%~1"=="--version" (
  echo codex-cli 0.155.1
  exit /b 0
)
if defined CODEX_HOME (set "CFG=%CODEX_HOME%\config.toml") else (set "CFG=%HOME%\.codex\config.toml")
for %%F in ("%CFG%") do if not exist "%%~dpF" mkdir "%%~dpF"
type nul >> "%CFG%"
set "ALLARGS=%*"
echo %ALLARGS%>>"%HOME%\codex.log"
if "%ALLARGS%"=="plugin marketplace add pyrlyn/rtok" (
  findstr /c:"source = \"https://github.com/pyrlyn/rtok.git\"" "%CFG%" >nul 2>&1 && exit /b 0
  findstr /c:"[marketplaces.rtok]" "%CFG%" >nul 2>&1 && (echo rtok: already added from a different source 1>&2 & exit /b 1)
  >>"%CFG%" echo([marketplaces.rtok]
  >>"%CFG%" echo source_type = "git"
  >>"%CFG%" echo source = "https://github.com/pyrlyn/rtok.git"
)
if "%ALLARGS%"=="plugin marketplace remove rtok" (
  findstr /c:"[marketplaces.rtok]" "%CFG%" >nul 2>&1 || (echo rtok: no such marketplace 1>&2 & exit /b 1)
  findstr /v /c:"[marketplaces.rtok]" /c:"source_type = \"git\"" /c:"source = \"https://github.com/pyrlyn/rtok.git\"" "%CFG%" > "%CFG%.tmp"
  move /y "%CFG%.tmp" "%CFG%" >nul
)
if "%ALLARGS%"=="plugin add rtok@rtok" (
  findstr /c:"[plugins.\"rtok@rtok\"]" "%CFG%" >nul 2>&1 || (
    >>"%CFG%" echo([plugins."rtok@rtok"]
    >>"%CFG%" echo enabled = true
  )
)
if "%ALLARGS%"=="plugin marketplace upgrade rtok" (
  rem `exit`, not `exit /b`: cmd /C loses a nested `exit /b` code and reports 0.
  if exist "%HOME%\fake-codex-fail-upgrade" (
    echo rtok: upgrade failed 1>&2
    exit 1
  )
  for %%F in ("%CFG%") do (
    if not exist "%%~dpFplugins\cache\rtok\rtok\0.0.1" mkdir "%%~dpFplugins\cache\rtok\rtok\0.0.1"
    >"%%~dpFplugins\cache\rtok\rtok\0.0.1\.mcp.json" echo upgraded
  )
)
if "%ALLARGS%"=="plugin remove rtok@rtok" (
  findstr /v /c:"[plugins.\"rtok@rtok\"]" /c:"enabled = true" "%CFG%" > "%CFG%.tmp"
  move /y "%CFG%.tmp" "%CFG%" >nul
)
"#;

/// The fake `claude`'s calls so far, one argv per line (`\n`, even from the Windows `.cmd`).
pub fn claude_log(home: &Path) -> String {
    fs::read_to_string(home.join("claude.log"))
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

/// The fake `codex`'s calls so far, one argv per line.
pub fn codex_log(home: &Path) -> String {
    fs::read_to_string(home.join("codex.log"))
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

/// [`raw`] that must succeed; returns stdout.
pub fn rtok(args: &[&str], cfg: &Path, home: &Path) -> String {
    let out = raw(args, cfg, home);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "rtok {args:?} failed: {stderr}\n{stdout}"
    );
    stdout
}

/// [`rtok`] over [`raw_without_claude`].
pub fn rtok_without_claude(args: &[&str], cfg: &Path, home: &Path) -> String {
    let out = raw_without_claude(args, cfg, home);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "rtok {args:?} failed: {stderr}\n{stdout}"
    );
    stdout
}

pub fn json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// Does `text` install our hook for `event`? `rtok_command`'s Windows fallback (no bare `rtok`
/// on the sandboxed PATH) writes the absolute `…\rtok.exe` instead of the bare name, so a
/// literal `"rtok hook <event>"` search would miss it; dropping `.exe` first collapses that
/// back to the same shape the test expects on every platform.
pub fn contains_hook(text: &str, event: &str) -> bool {
    text.replace(".exe", "")
        .contains(&format!("rtok hook {event}"))
}

/// The copies of `path` in its sibling `_backup/` directory, oldest name first.
pub fn backups(path: &Path) -> Vec<PathBuf> {
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let dir = parent.join("_backup");
    if !dir.is_dir() {
        return Vec::new();
    }
    let name = format!("{}.bak-", path.file_name().unwrap().to_string_lossy());
    let mut found: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(&name))
        .collect();
    found.sort();
    found
}

/// Where Claude Desktop's config lands under `home` on this platform (the binary's rule).
pub fn claude_desktop_config(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Claude/claude_desktop_config.json")
    } else if cfg!(windows) {
        home.join("Claude/claude_desktop_config.json")
    } else {
        home.join(".config/Claude/claude_desktop_config.json")
    }
}

// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Close a running desktop app before `agents install|uninstall` writes its config, reopen it
//! after (T141; replaces T76's interactive y/N prompt with an automatic, no-prompt flow).
//!
//! Only [`Kind::Desktop`] apps are ever quit/reopened, and only when both are true: the app is
//! currently running, and the write would actually change something for that host (checked with
//! a dry-run via [`super::would_change`]). CLI-only hosts are never killed — a running CLI
//! binary just gets a one-line reminder to restart its session.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use rtok_sys::proc::ProgressRunner;

use super::{Agent, Kind, Request, Variant, locate};
use crate::config::Config;

/// Process control seam (T141): the real impl shells out; tests fake it and record calls.
pub trait Procs {
    /// True if a desktop app bundle/window named `name` is currently running. Must not launch
    /// it as a side effect. Not for CLI binaries — on macOS `osascript`'s app-name lookup is
    /// case-insensitive and can match the wrong app (or pop a "Choose Application" dialog for
    /// an unknown name), so a CLI binary is asked about through [`Self::bin_running`] instead.
    fn app_running(&self, name: &str) -> bool;
    /// True if a CLI binary named `bin` is currently running (`pgrep -x` / `tasklist`, never
    /// `osascript`'s app-name check).
    fn bin_running(&self, bin: &str) -> bool;
    /// Ask a running desktop app to quit.
    fn quit(&self, name: &str) -> Result<()>;
    /// Launch a desktop app again.
    fn open(&self, name: &str, path: &Path) -> Result<()>;
}

/// `tasklist /FI "IMAGENAME eq <name>.exe"` (Windows): used for both an app's exe and a CLI
/// binary — unlike `osascript`'s app-name lookup, it matches an exact image name only.
#[cfg(target_os = "windows")]
fn tasklist_running(name: &str) -> bool {
    let exe = image_name(name);
    ProgressRunner::quiet("tasklist", ["/FI", &format!("IMAGENAME eq {exe}")])
        .run()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .to_ascii_lowercase()
                .contains(&exe.to_ascii_lowercase())
        })
        .unwrap_or(false)
}

/// `pgrep -x <name>` (macOS/Linux): exact process-name match, used for CLI binaries everywhere
/// and for desktop apps outside macOS (where there is no app-bundle concept to ask `osascript`
/// about).
#[cfg(not(target_os = "windows"))]
fn pgrep_running(name: &str) -> bool {
    // `pgrep` prints matching PIDs; only the exit status is wanted, and the runner keeps them
    // out of rtok's own stdout.
    ProgressRunner::quiet("pgrep", ["-x", name])
        .run()
        .is_ok_and(|o| o.status.success())
}

/// Real OS process control: macOS uses `osascript` (the same name resolution `tell application
/// … to quit` and `open -a` already rely on) for [`Procs::app_running`] only; Windows uses
/// `tasklist`/`taskkill` for both; everything else uses `pgrep`/`killall` for both.
pub struct RealProcs;

impl Procs for RealProcs {
    fn app_running(&self, name: &str) -> bool {
        #[cfg(target_os = "macos")]
        {
            ProgressRunner::quiet(
                "osascript",
                ["-e", &format!("application \"{name}\" is running")],
            )
            .run()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        }
        #[cfg(target_os = "windows")]
        {
            tasklist_running(name)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            pgrep_running(name)
        }
    }

    fn bin_running(&self, bin: &str) -> bool {
        #[cfg(target_os = "windows")]
        {
            tasklist_running(bin)
        }
        #[cfg(not(target_os = "windows"))]
        {
            pgrep_running(bin)
        }
    }

    fn quit(&self, name: &str) -> Result<()> {
        // Defense in depth: the desktop `apps` lists we act on never name rtok itself, but
        // never let this path kill the process it runs under.
        if name.eq_ignore_ascii_case("rtok") {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            run_ok(
                ProgressRunner::new(
                    &closing(name),
                    "osascript",
                    ["-e", &format!("tell application \"{name}\" to quit")],
                ),
                &format!("osascript quit {name}"),
            )
        }
        #[cfg(target_os = "windows")]
        {
            run_ok(
                ProgressRunner::new(&closing(name), "taskkill", ["/IM", &image_name(name), "/F"]),
                &format!("taskkill {name}"),
            )
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            run_ok(
                ProgressRunner::new(&closing(name), "killall", [name]),
                &format!("killall {name}"),
            )
        }
    }

    fn open(&self, name: &str, path: &Path) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let _ = path;
            run_ok(
                ProgressRunner::new(&reopening(name), "open", ["-a", name]),
                &format!("open -a {name}"),
            )
        }
        #[cfg(target_os = "windows")]
        {
            run_ok(
                ProgressRunner::new(
                    &reopening(name),
                    "cmd",
                    ["/C", "start", "", path.to_str().unwrap_or(name)],
                ),
                &format!("start {name}"),
            )
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            // A desktop app outlives this command, so it gets no pipes to fill and no spinner.
            let (program, args): (&OsStr, &[&str]) = if path.exists() {
                (path.as_os_str(), &[])
            } else {
                (OsStr::new("xdg-open"), &[name])
            };
            ProgressRunner::new("", program, args)
                .output(rtok_sys::proc::Output::Detached)
                .spawn()?;
            Ok(())
        }
    }
}

/// Run `cmd` to completion and fail on a non-zero exit, carrying its stderr: `open -a` on an
/// app that is still shutting down exits non-zero, and checking only that it started hid that
/// failure — the app stayed closed with no warning (T434).
fn run_ok(cmd: ProgressRunner, what: &str) -> Result<()> {
    let out = cmd.run().with_context(|| what.to_string())?;
    if !out.status.success() {
        anyhow::bail!(
            "{what}: {} {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn closing(name: &str) -> String {
    format!("closing {name}…")
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn reopening(name: &str) -> String {
    format!("reopening {name}…")
}

/// The Windows image name for an app/binary `name`: `name` plus `.exe`, unless it already ends
/// in one (`apps[]` entries such as `.../Code.exe` resolve to `Code.exe`, not `Code.exe.exe`).
#[cfg(any(target_os = "windows", test))]
fn image_name(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".exe") {
        name.to_string()
    } else {
        format!("{name}.exe")
    }
}

/// The display/process name our path parsing reads from one app path ([`locate::app_paths`]:
/// the user's, then each `apps[]` entry) — the same string asked of [`Procs::app_running`] and
/// handed to [`Procs::quit`]/[`Procs::open`].
fn desktop_app_name(path: PathBuf, fallback: &str) -> (String, PathBuf) {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .map(|n| n.strip_suffix(".app").unwrap_or(n).to_string())
        .unwrap_or_else(|| fallback.to_string());
    (name, path)
}

/// The first app path of a desktop variant that `procs` reports as currently running.
fn running_desktop_app(
    cfg: &Config,
    host: &str,
    v: &Variant,
    procs: &dyn Procs,
) -> Option<(String, PathBuf)> {
    locate::app_paths(cfg, host, v)
        .into_iter()
        .map(|path| desktop_app_name(path, v.name))
        .find(|(name, _)| procs.app_running(name))
}

/// The first CLI binary of a host that `procs` reports as currently running.
fn running_cli_bin(agent: &dyn Agent, procs: &dyn Procs) -> Option<&'static str> {
    agent
        .variants()
        .iter()
        .filter(|v| v.kind == Kind::Cli)
        .find_map(|v| v.bins.iter().copied().find(|b| procs.bin_running(b)))
}

/// Whether a CLI binary or an installed desktop app of `agent` runs now. Only installed apps
/// are asked: on macOS `osascript` may open a "Choose Application" dialog for an unknown name.
pub fn host_running(cfg: &Config, agent: &dyn Agent, procs: &dyn Procs) -> bool {
    let apps = agent.variants().iter().filter(|v| v.kind == Kind::Desktop);
    running_cli_bin(agent, procs).is_some()
        || apps
            .filter(|v| super::present(cfg, agent.id(), v))
            .any(|v| running_desktop_app(cfg, agent.id(), v, procs).is_some())
}

/// How long a quit app gets to exit. Electron apps often need well over 5 s, and reopening one
/// that is still shutting down only activates the dying instance (T434).
const QUIT_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(30)
};

/// Poll `procs.app_running(name)` until it goes false or `timeout` elapses; true if it exited.
fn wait_until_not_running(procs: &dyn Procs, name: &str, timeout: Duration) -> bool {
    let started = std::time::Instant::now();
    loop {
        if !procs.app_running(name) {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Print a restart warning and log it; restart problems never fail the command.
fn warn(cfg: &Config, msg: &str) {
    crate::log::stderr_ln(&crate::ui::style::warn(&format!("warning: {msg}")));
    crate::log::append(cfg, "warn", "agents", "restart", msg);
}

/// `rtok agents install|uninstall`: quit a running desktop app before the write if it would
/// actually change that host's config, write, then reopen it. Never touches a CLI-only host's
/// process; if one is running it just prints a reminder. Fails open — a quit/reopen failure is
/// a warning, never a command failure.
pub fn run(cfg: &mut Config, req: &Request, no_restart: bool) -> Result<String> {
    with_restart(
        cfg,
        req,
        no_restart,
        &RealProcs,
        |c, a| super::would_change(c, a, req),
        |c| super::run(c, req),
    )
}

/// Test seam for [`run`]: `procs`, `would_change` and `write` are injected so orchestration
/// tests never shell out or touch a real host's files.
fn with_restart(
    cfg: &mut Config,
    req: &Request,
    no_restart: bool,
    procs: &dyn Procs,
    would_change: impl Fn(&Config, &'static dyn Agent) -> Result<bool>,
    write: impl FnOnce(&mut Config) -> Result<String>,
) -> Result<String> {
    let mut out = String::new();
    let mut to_reopen: Vec<(String, PathBuf)> = Vec::new();
    if !no_restart {
        let agents = super::resolve(&req.hosts)?;
        // Plan every host before closing any: a failing `would_change` returns early, and an app
        // closed by then would stay closed (T629).
        let mut plan = Vec::new();
        for &agent in &agents {
            let target = agent
                .variants()
                .iter()
                .filter(|v| v.kind == Kind::Desktop)
                .find_map(|v| running_desktop_app(cfg, agent.id(), v, procs));
            let bin = running_cli_bin(agent, procs);
            // Nothing of this host's is running: skip the dry-run entirely (T141 review).
            if target.is_none() && bin.is_none() {
                continue;
            }
            plan.push((agent, target, bin, would_change(cfg, agent)?));
        }
        for (agent, target, bin, changed) in plan {
            if let Some((name, path)) = target
                && changed
            {
                if cfg.setup.dry_run {
                    out.push_str(&format!("{}: would close/reopen {name}\n", agent.id()));
                } else if let Err(e) = procs.quit(&name) {
                    let msg = format!(
                        "could not quit {name}: {e:#}; restart it manually to load the new config"
                    );
                    warn(cfg, &msg);
                } else if wait_until_not_running(procs, &name, QUIT_TIMEOUT) {
                    to_reopen.push((name, path));
                } else {
                    // Still up — likely a quit prompt. Reopening now would only focus it, and
                    // the app would close after the prompt is answered.
                    let msg = format!(
                        "{name} did not quit within {} s; restart it manually to load the new config",
                        QUIT_TIMEOUT.as_secs()
                    );
                    warn(cfg, &msg);
                }
            }
            if changed && let Some(bin) = bin {
                out.push_str(&format!(
                    "{}: restart your {bin} session to load the new config\n",
                    agent.id()
                ));
            }
        }
    }
    // Always reopen what was quit — even when the write itself fails — before propagating
    // the write's error (T141 review): a write failure must never leave a closed app closed.
    let written = write(cfg);
    for (name, path) in to_reopen {
        if let Err(e) = procs.open(&name, &path) {
            let msg = format!("could not reopen {name}: {e:#}; open it manually");
            warn(cfg, &msg);
        }
    }
    out.push_str(&written?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Mode;
    use std::cell::RefCell;
    use std::collections::HashSet;

    /// T328: a resolved app name that is already an `.exe` (`Code.exe`) must not become `.exe.exe`.
    #[test]
    fn windows_image_name_adds_exe_once() {
        assert_eq!(image_name("Cursor"), "Cursor.exe");
        assert_eq!(image_name("Code.exe"), "Code.exe");
        assert_eq!(image_name("CODE.EXE"), "CODE.EXE");
    }

    #[derive(Default)]
    struct FakeProcs {
        running: RefCell<HashSet<String>>,
        calls: RefCell<Vec<String>>,
        fail_quit: bool,
        /// `quit` succeeds but the app keeps running, as when it shows a quit prompt.
        stuck_quit: bool,
        fail_open: bool,
    }

    impl FakeProcs {
        fn running_names(names: &[&str]) -> Self {
            Self {
                running: RefCell::new(names.iter().map(|s| s.to_string()).collect()),
                ..Default::default()
            }
        }
    }

    impl Procs for FakeProcs {
        fn app_running(&self, name: &str) -> bool {
            self.running.borrow().contains(name)
        }
        fn bin_running(&self, bin: &str) -> bool {
            self.running.borrow().contains(bin)
        }
        fn quit(&self, name: &str) -> Result<()> {
            self.calls.borrow_mut().push(format!("quit:{name}"));
            if self.fail_quit {
                anyhow::bail!("boom");
            }
            if !self.stuck_quit {
                self.running.borrow_mut().remove(name);
            }
            Ok(())
        }
        fn open(&self, name: &str, _path: &Path) -> Result<()> {
            self.calls.borrow_mut().push(format!("open:{name}"));
            if self.fail_open {
                anyhow::bail!("open -a {name}: exit status: 1");
            }
            Ok(())
        }
    }

    fn req(host: &str) -> Request {
        Request {
            hosts: vec![host.to_string()],
            mode: Mode::Install,
            cli: false,
            desktop: false,
            all: true,
        }
    }

    /// [`with_restart`] for the "windsurf" host with a fixed `would_change` answer and a
    /// no-op write — every test below that does not need to observe the write's position in
    /// the call order goes through here instead of repeating the same call shape.
    fn call(
        cfg: &mut Config,
        procs: &FakeProcs,
        no_restart: bool,
        changed: bool,
    ) -> Result<String> {
        with_restart(
            cfg,
            &req("windsurf"),
            no_restart,
            procs,
            |_, _| Ok(changed),
            |_| Ok(String::new()),
        )
    }

    /// A changed install for `windsurf` whose write closure pushes `write` into the same call
    /// log as quit/open, so one assertion on the log proves the order of all three.
    fn call_logging_write(procs: &FakeProcs) -> String {
        with_restart(
            &mut Config::default(),
            &req("windsurf"),
            false,
            procs,
            |_, _| Ok(true),
            |_| {
                procs.calls.borrow_mut().push("write".into());
                Ok(String::new())
            },
        )
        .unwrap()
    }

    /// Quit before write, write before reopen.
    #[test]
    fn running_and_changed_quits_writes_then_reopens_in_order() {
        let procs = FakeProcs::running_names(&["Windsurf"]);
        call_logging_write(&procs);
        assert_eq!(
            procs.calls.borrow().as_slice(),
            [
                "quit:Windsurf".to_string(),
                "write".to_string(),
                "open:Windsurf".to_string()
            ]
        );
    }

    #[test]
    fn running_but_unchanged_does_nothing() {
        let procs = FakeProcs::running_names(&["Windsurf"]);
        call(&mut Config::default(), &procs, false, false).unwrap();
        assert!(procs.calls.borrow().is_empty());
    }

    #[test]
    fn not_running_writes_only() {
        let procs = FakeProcs::default();
        call(&mut Config::default(), &procs, false, true).unwrap();
        assert!(procs.calls.borrow().is_empty());
    }

    #[test]
    fn dry_run_prints_would_close_and_makes_no_calls() {
        let procs = FakeProcs::running_names(&["Windsurf"]);
        let mut cfg = Config::default();
        cfg.setup.dry_run = true;
        let out = call(&mut cfg, &procs, false, true).unwrap();
        assert!(procs.calls.borrow().is_empty());
        assert!(out.contains("would close/reopen Windsurf"), "{out}");
    }

    #[test]
    fn no_restart_flag_writes_only() {
        let procs = FakeProcs::running_names(&["Windsurf"]);
        call(&mut Config::default(), &procs, true, true).unwrap();
        assert!(procs.calls.borrow().is_empty());
    }

    #[test]
    fn quit_failure_still_writes_and_warns_instead_of_erroring() {
        let procs = FakeProcs {
            fail_quit: true,
            ..FakeProcs::running_names(&["Windsurf"])
        };
        let out = call_logging_write(&procs);
        // No reopen call: a failed quit still writes, but nothing was closed to reopen.
        assert_eq!(
            procs.calls.borrow().as_slice(),
            ["quit:Windsurf".to_string(), "write".to_string()]
        );
        assert!(out.is_empty());
    }

    /// T434: an app still running after the quit timeout (a quit prompt) is not reopened —
    /// `open -a` would only focus it, and it would close once the prompt is answered.
    #[test]
    fn app_that_never_quits_is_written_but_not_reopened() {
        let procs = FakeProcs {
            stuck_quit: true,
            ..FakeProcs::running_names(&["Windsurf"])
        };
        let out = call_logging_write(&procs);
        assert_eq!(
            procs.calls.borrow().as_slice(),
            ["quit:Windsurf".to_string(), "write".to_string()]
        );
        assert!(out.is_empty());
    }

    /// T434: a failed reopen warns and the install still succeeds.
    #[test]
    fn reopen_failure_warns_instead_of_erroring() {
        let procs = FakeProcs {
            fail_open: true,
            ..FakeProcs::running_names(&["Windsurf"])
        };
        let out = call_logging_write(&procs);
        assert_eq!(
            procs.calls.borrow().as_slice(),
            [
                "quit:Windsurf".to_string(),
                "write".to_string(),
                "open:Windsurf".to_string()
            ]
        );
        assert!(out.is_empty());
    }

    /// T434: a command that starts but exits non-zero is an error carrying its stderr.
    #[cfg(unix)]
    #[test]
    fn run_ok_fails_on_a_non_zero_exit() {
        let err = run_ok(
            ProgressRunner::quiet("sh", ["-c", "echo nope >&2; exit 3"]),
            "probe",
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("probe") && msg.contains("nope"), "{msg}");
        run_ok(ProgressRunner::quiet("true", [] as [&str; 0]), "probe").unwrap();
    }

    #[test]
    fn write_failure_still_reopens_then_propagates_the_error() {
        let procs = FakeProcs::running_names(&["Windsurf"]);
        let mut cfg = Config::default();
        let err = with_restart(
            &mut cfg,
            &req("windsurf"),
            false,
            &procs,
            |_, _| Ok(true),
            |_| anyhow::bail!("disk full"),
        )
        .unwrap_err();
        // Quit, then reopen despite the write error, then the error propagates — a write
        // failure must never leave the app closed (T141 review).
        assert_eq!(
            procs.calls.borrow().as_slice(),
            ["quit:Windsurf".to_string(), "open:Windsurf".to_string()]
        );
        assert!(err.to_string().contains("disk full"), "{err:#}");
    }

    fn req_hosts(hosts: &[&str]) -> Request {
        Request {
            hosts: hosts.iter().map(|h| h.to_string()).collect(),
            ..req("windsurf")
        }
    }

    /// T629: a host that cannot be planned must be found out before any app is closed, or the
    /// apps closed so far stay closed when the error returns early.
    #[test]
    fn a_failing_preflight_on_any_host_closes_nothing_and_writes_nothing() {
        let hosts = ["windsurf", "cursor", "devin"];
        for bad in hosts {
            let procs = FakeProcs::running_names(&["Windsurf", "Cursor", "Devin"]);
            let wrote = RefCell::new(false);
            let err = with_restart(
                &mut Config::default(),
                &req_hosts(&hosts),
                false,
                &procs,
                |_, a| {
                    if a.id() == bad {
                        anyhow::bail!("cannot read {bad} config")
                    }
                    Ok(true)
                },
                |_| {
                    *wrote.borrow_mut() = true;
                    Ok(String::new())
                },
            )
            .unwrap_err();
            assert!(err.to_string().contains(bad), "{err:#}");
            assert!(procs.calls.borrow().is_empty(), "{bad}: {:?}", procs.calls);
            assert!(!*wrote.borrow(), "{bad}: the write must not run");
        }
    }

    /// T629 control: with every host planned, each running app is closed, the write runs once,
    /// then each is reopened.
    #[test]
    fn two_running_desktop_hosts_are_both_closed_then_both_reopened() {
        let procs = FakeProcs::running_names(&["Windsurf", "Cursor"]);
        with_restart(
            &mut Config::default(),
            &req_hosts(&["windsurf", "cursor"]),
            false,
            &procs,
            |_, _| Ok(true),
            |_| {
                procs.calls.borrow_mut().push("write".into());
                Ok(String::new())
            },
        )
        .unwrap();
        let calls = procs.calls.borrow();
        assert_eq!(calls.len(), 5, "{calls:?}");
        assert!(
            calls[..2].iter().all(|c| c.starts_with("quit:")),
            "{calls:?}"
        );
        assert_eq!(calls[2], "write");
        assert!(
            calls[3..].iter().all(|c| c.starts_with("open:")),
            "{calls:?}"
        );
    }

    #[test]
    fn cli_only_host_running_prints_message_and_is_never_killed() {
        let procs = FakeProcs::running_names(&["aider"]);
        let mut cfg = Config::default();
        let out = with_restart(
            &mut cfg,
            &req("aider"),
            false,
            &procs,
            |_, _| Ok(true),
            |_| Ok(String::new()),
        )
        .unwrap();
        assert!(procs.calls.borrow().is_empty());
        assert!(out.contains("aider: restart your aider session"), "{out}");
    }

    /// One row per host with a `Kind::Desktop` variant, naming what `apps[0]` (always the
    /// macOS entry in this tree) should resolve to. A host added here without a row panics
    /// instead of silently skipping restart coverage.
    fn expected_desktop_name(variant: &str) -> &'static str {
        match variant {
            "Antigravity" => "Antigravity",
            "Claude Desktop" => "Claude",
            "GitHub Copilot" => "GitHub Copilot",
            "Cursor" => "Cursor",
            "Devin" => "Devin",
            "Codex App" => "ChatGPT",
            "Cline for VS Code" => "Visual Studio Code",
            "Kilo Code for VS Code" => "Visual Studio Code",
            "Kimi Code Desktop" => "Kimi Code",
            "Kiro IDE" => "Kiro",
            "MiMo Desktop" => "Xiaomi MiMo AI",
            "OpenCode Desktop" => "OpenCode",
            "Roo Code" => "Visual Studio Code",
            "Goose" => "Goose",
            "Continue" => "Visual Studio Code",
            "VS Code" => "Visual Studio Code",
            "VS Code - Insiders" => "Visual Studio Code - Insiders",
            "Warp" => "Warp",
            "Windsurf" => "Windsurf",
            "WorkBuddy" => "WorkBuddy",
            "ZCode" => "ZCode",
            "Zed" => "Zed",
            other => panic!("desktop variant `{other}` has no expected-name row (T141) — add one"),
        }
    }

    #[test]
    fn every_desktop_variant_of_every_host_resolves_its_known_app_name() {
        for &id in crate::agents::HOSTS {
            let agent = crate::agents::host(id).unwrap();
            let mut had_desktop = false;
            for v in agent.variants().iter().filter(|v| v.kind == Kind::Desktop) {
                had_desktop = true;
                assert!(!v.apps.is_empty(), "{id}/{} has no apps[]", v.name);
                let (name, _) = desktop_app_name(crate::agents::expand_spec(v.apps[0]), v.name);
                assert_eq!(name, expected_desktop_name(v.name), "host {id}");
            }
            if !had_desktop {
                // CLI-only host: pretend every bin is running and confirm it never yields a
                // quit/open call — running_desktop_app always returns None for it.
                let procs = FakeProcs::running_names(
                    &agent
                        .variants()
                        .iter()
                        .flat_map(|v| v.bins.iter().copied())
                        .collect::<Vec<_>>(),
                );
                assert!(
                    agent
                        .variants()
                        .iter()
                        .filter(|v| v.kind == Kind::Desktop)
                        .find_map(|v| running_desktop_app(&Config::default(), id, v, &procs))
                        .is_none(),
                    "CLI-only host {id} must never produce a desktop restart target"
                );
            }
        }
    }
}

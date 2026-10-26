// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T943: what `rtok completions <shell> --install` writes completes in the real shell, over a
//! temporary HOME. A test whose shell is not installed returns early and passes; no agent is
//! started.

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t943-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A working `name` on the outer `PATH`; a shim that cannot run it (mise without a version)
/// counts as missing.
fn find(name: &str, probe: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let bin = std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())?;
    let ok = Command::new(&bin)
        .args(probe)
        .output()
        .is_ok_and(|o| o.status.success());
    ok.then_some(bin)
}

/// The test's `rtok` first, so the wrappers run this build and not an installed one.
fn path() -> String {
    let bin = Path::new(env!("CARGO_BIN_EXE_rtok")).parent().unwrap();
    format!("{}:/usr/bin:/bin", bin.display())
}

fn install(home: &Path, shell: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_rtok"))
        .args(["completions", shell, "--install"])
        .env_clear()
        .env("HOME", home)
        .env("RTOK_HOME", home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    text.lines()
        .next()
        .and_then(|l| l.strip_prefix("wrote "))
        .unwrap_or_else(|| panic!("{text}"))
        .to_string()
}

/// An interactive shell on a pseudo-terminal: wait for text, then type.
struct Term {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    seen: Arc<Mutex<String>>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
}

impl Term {
    fn spawn(cmd: CommandBuilder) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 200,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("a pseudo-terminal");
        let child = pair.slave.spawn_command(cmd).expect("shell starts");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let writer = pair.master.take_writer().expect("writer");
        let seen = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&seen);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        });
        Self {
            child,
            writer,
            seen,
            _master: pair.master,
        }
    }

    /// Keys typed before the line editor is up can be flushed, so every step waits first.
    fn wait_for(&self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.seen.lock().unwrap().contains(text) {
            assert!(
                Instant::now() < deadline,
                "never saw {text:?} in:\n{}",
                self.seen.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn send(&mut self, keys: &str) {
        self.writer.write_all(keys.as_bytes()).expect("keys");
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// The creator's failure: `.zfunc` off `fpath` and a `compinit -C` dump made before `_rtok`
/// existed. After one install, with no `.zshrc` edit, Tab completes a subcommand.
#[test]
fn zsh_tab_completes_with_a_stale_compinit_dump_and_no_zshrc_edit() {
    let Some(zsh) = find("zsh", &["-fc", "exit 0"]) else {
        return;
    };
    let home = home("zsh");
    // A cleared environment, like the shell below: an exported `FPATH` can carry the
    // developer's own `_rtok` into the dump.
    let made = Command::new(&zsh)
        .args([
            "-fc",
            "autoload -Uz compinit && compinit -u -d \"$HOME/dump\"",
        ])
        .env_clear()
        .env("HOME", &home)
        .env("PATH", path())
        .status()
        .unwrap();
    assert!(made.success());
    install(&home, "zsh");
    std::fs::write(
        home.join(".zshrc"),
        "autoload -Uz compinit && compinit -C -d \"$HOME/dump\"\nPS1='rtok-test> '\n",
    )
    .unwrap();
    let dump = std::fs::read_to_string(home.join("dump")).unwrap();
    assert!(!dump.contains("_rtok"), "the dump must predate _rtok");

    let mut cmd = CommandBuilder::new(&zsh);
    cmd.arg("-i");
    cmd.env_clear();
    cmd.env("HOME", &home);
    cmd.env("RTOK_HOME", &home);
    cmd.env("PATH", path());
    cmd.env("TERM", "xterm");
    cmd.cwd(&home);
    let mut term = Term::spawn(cmd);
    term.wait_for("rtok-test> ");
    term.send("rtok work\t");
    term.wait_for("worktree");
    // Ctrl-A puts `print` in front of the completed line, so the shell echoes the buffer.
    term.send("\x01print -r -- RESULT: \r");
    term.wait_for("RESULT: rtok worktree");
    drop(term);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn bash_wrapper_registers_the_generated_completion() {
    let Some(bash) = find("bash", &["-c", "exit 0"]) else {
        return;
    };
    let home = home("bash");
    let script = install(&home, "bash");
    let out = Command::new(bash)
        .args(["--norc", "--noprofile", "-c"])
        .arg(format!(". '{script}' && complete -p rtok"))
        .env_clear()
        .env("HOME", &home)
        .env("PATH", path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("-F _rtok") && text.trim_end().ends_with(" rtok"),
        "{out:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn fish_wrapper_completes_a_subcommand() {
    let Some(fish) = find("fish", &["--no-config", "-c", "exit 0"]) else {
        return;
    };
    let home = home("fish");
    let script = install(&home, "fish");
    let out = Command::new(fish)
        .args(["--no-config", "-c"])
        .arg(format!("source '{script}'; complete -C 'rtok work'"))
        .env_clear()
        .env("HOME", &home)
        .env("PATH", path())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("worktree"),
        "{out:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn powershell_wrapper_completes_a_subcommand() {
    let Some(pwsh) = find("pwsh", &["-NoProfile", "-Command", "exit 0"]) else {
        return;
    };
    let home = home("pwsh");
    let script = install(&home, "powershell");
    let outer = std::env::var("PATH").unwrap_or_default();
    // pwsh needs more of the environment than a cleared one, so only PATH and HOME change.
    let out = Command::new(pwsh)
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            ". '{script}'; (TabExpansion2 'rtok work' 9).CompletionMatches.CompletionText"
        ))
        .env("HOME", &home)
        .env("PATH", format!("{}:{outer}", path()))
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("worktree"),
        "{out:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

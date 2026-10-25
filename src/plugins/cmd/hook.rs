// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! PreToolUse(Bash) rewrite to `rtok run --` (plan T3.4).

use rtok_plugin_sdk::{Ctx, PreToolDecision, PreToolUse};
use serde_json::json;

fn skip_wrap(cmd: &str, cfg: &crate::config::Cmd) -> bool {
    skip_wrap_host(cfg!(windows), cmd, &cfg.never_wrap, &cfg.interactive_stems)
}

/// True when `-i` on this command should skip wrapping (REPL / TTY stems only).
fn interactive_i_skip(stem: &str, args: &[&str], interactive_stems: &[String]) -> bool {
    if !interactive_stems
        .iter()
        .any(|s| s.eq_ignore_ascii_case(stem))
    {
        return false;
    }
    match stem.to_ascii_lowercase().as_str() {
        "docker" => args
            .iter()
            .any(|t| t.eq_ignore_ascii_case("run") || t.eq_ignore_ascii_case("exec")),
        "kubectl" => args.iter().any(|t| t.eq_ignore_ascii_case("exec")),
        _ => true,
    }
}

/// `windows` is a parameter so both host contracts stay tested on one toolchain
/// (T55.12): a PowerShell `''` rewrite must never reach a POSIX shell, where
/// `'echo it''s fine'` concatenates to `echo its fine` and silently drops the
/// apostrophe — so any command containing `'` stays unwrapped there.
fn skip_wrap_host(
    windows: bool,
    cmd: &str,
    never_wrap: &[String],
    interactive_stems: &[String],
) -> bool {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    let first = tokens.first().copied().unwrap_or("");
    // Same stem rules as formatters::cmd_stem / run::shell_kind: Windows argv may
    // be `C:\…\sudo.exe` while never_wrap lists bare `sudo`.
    let base = super::formatters::cmd_stem(first);
    if never_wrap.iter().any(|w| w.eq_ignore_ascii_case(base)) {
        return true;
    }
    if cmd.contains("<<") {
        return true;
    }
    // T55.12: the PowerShell `''` form is lossy if the rewritten command reaches a
    // POSIX shell — Claude Code on Windows runs Bash through Git Bash, where
    // `'echo it''s fine'` concatenates to `echo its fine`. Nothing containing an
    // apostrophe is wrapped there; staying whole is the safe direction.
    if windows && cmd.contains('\'') {
        return true;
    }
    if tokens.contains(&"&") {
        return true;
    }
    if tokens.contains(&"--interactive") {
        return true;
    }
    if tokens.contains(&"-i") && interactive_i_skip(base, &tokens[1..], interactive_stems) {
        return true;
    }
    // AGENTS: trailing `&` (background) — also `sleep 10&` with no space before `&`.
    // Do not treat `&&` as background.
    let t = cmd.trim_end();
    if t.ends_with('&') && !t.ends_with("&&") {
        return true;
    }
    false
}

/// T127: only bare `[A-Za-z0-9_-]`, 1–64 bytes. Anything else is dropped rather than
/// embedded — a malformed or hostile `agent_id` must never reach the rewritten argv, and
/// missing/unknown stays exactly today's un-scoped dispatch (fail open).
fn is_valid_agent_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `--agent <id> ` when `cx` carries a validly-shaped sub-agent id, else empty — spliced
/// before the quoted command so `rtok run` can scope its dedup pointer to the same window
/// this dispatch came from (T127).
fn agent_flag(cx: &Ctx) -> String {
    match cx.agent_id() {
        Some(id) if is_valid_agent_id(id) => format!("--agent {id} "),
        _ => String::new(),
    }
}

/// A builtin whose effect outlives the command (cwd, environment, aliases) would be lost
/// inside `rtok run`'s child shell, so such a command is never wrapped.
fn changes_shell_state(cmd: &str) -> bool {
    super::bounded::stages(cmd).iter().any(|stage| {
        matches!(
            stage.first().map(String::as_str),
            Some(
                "cd" | "pushd"
                    | "popd"
                    | "export"
                    | "source"
                    | "."
                    | "unset"
                    | "alias"
                    | "unalias"
                    | "set"
                    | "shopt"
                    | "umask"
                    | "trap"
                    | "declare"
                    | "typeset"
                    | "readonly"
            )
        ) || is_bare_assignment(stage)
    })
}

/// `A=1` (or `A=1 B=2`) with no command word sets a variable in the calling shell; with a
/// command word (`A=1 cargo test`) the assignment is scoped to that command and wraps fine.
fn is_bare_assignment(stage: &[String]) -> bool {
    !stage.is_empty()
        && stage.iter().all(|w| {
            w.split_once('=').is_some_and(|(name, _)| {
                name.chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
}

/// T437: true when `git` is a word anywhere in the command text. Matches the raw text
/// rather than parsed stages because the host guard reads that same text: a stage or
/// stem match would still wrap `bash -c 'git status'`, `$(git rev-parse HEAD)` or
/// `env X=1 git status`, which the guard refuses just the same. A word is a run of
/// alphanumerics and `_`, so `/usr/bin/git`, `git.exe` and `git-lfs` match, `github`
/// does not. Over-matching only costs compression, and any doubt leaves the command
/// unwrapped.
fn runs_git(cmd: &str) -> bool {
    cmd.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|w| w.eq_ignore_ascii_case("git"))
}

/// Wrap a Bash command unless the skip rules fire.
pub fn pre_tool(ev: &PreToolUse<'_>, cx: &Ctx) -> Option<PreToolDecision> {
    let cfg = cx.plugin_config::<crate::config::Cmd>("cmd");
    if ev.tool_name != "Bash" || !cfg.rewrite {
        return None;
    }
    let full = ev.tool_input.get("command")?.as_str()?;
    if skip_wrap(full, &cfg) {
        return None;
    }
    // T437: Claude Code runs sub-agents in isolated worktrees behind a guard that must
    // prove every git command stays inside the worktree; it cannot see through the
    // `rtok run` wrapper and refuses the command, so no sub-agent could commit. The main
    // session has no such guard and keeps its wrapped git.
    if cx.agent_id().is_some() && runs_git(full) {
        return None;
    }
    // The host keeps its shell's cwd between calls, so leading `cd <dir> &&` hops stay in
    // that shell and only the rest runs inside `rtok run`'s child.
    let mut cmd = full;
    while let Some((_, rest)) = crate::plugins::guard::strip_cd_hop(cmd) {
        cmd = &full[full.len() - rest.len()..];
    }
    // T444: `skip_wrap` above only saw `cd` as the first word; the command that really
    // runs behind the hops (`cd x && sudo ls`, `cd x && rtok expand id`) needs the same
    // never_wrap / interactive checks.
    if cmd.len() != full.len() && skip_wrap(cmd, &cfg) {
        return None;
    }
    if changes_shell_state(cmd) {
        return None;
    }
    let mut input = ev.tool_input.clone();
    // One argv so the outer shell cannot split on `&&`, `|`, `;`, or redirects. The
    // `--agent` token (alnum/`_`/`-` only, validated above) sits before `--` and never
    // touches the quoting that protects `cmd` on either the POSIX or Windows path.
    input["command"] = json!(format!(
        "{}{}rtok run {}-- {}",
        &full[..full.len() - cmd.len()],
        crate::names::run_guard(),
        agent_flag(cx),
        super::run::wrap_quote(cmd)
    ));
    Some(PreToolDecision::Rewrite {
        input,
        reason: "wrapped by rtok".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtok_plugin_sdk::Ctx;
    fn decide(command: &str) -> Option<PreToolDecision> {
        let cx = crate::plugin::Runtime::in_memory("wrap").unwrap();
        let input = json!({"command": command, "description": "t"});
        let ev = PreToolUse {
            tool_name: "Bash",
            tool_input: &input,
        };
        pre_tool(&ev, &Ctx::new(&cx))
    }

    fn decide_as(agent_id: Option<&str>, command: &str) -> Option<PreToolDecision> {
        let cx = crate::plugin::Runtime::in_memory("wrap").unwrap();
        let input = json!({"command": command, "description": "t"});
        let ev = PreToolUse {
            tool_name: "Bash",
            tool_input: &input,
        };
        pre_tool(&ev, &Ctx::with_agent(&cx, agent_id))
    }

    /// The line exactly as the hook writes it, T922 guard included.
    fn rewritten(d: &PreToolDecision) -> &str {
        match d {
            PreToolDecision::Rewrite { input, reason } => {
                assert_eq!(reason, "wrapped by rtok");
                input["command"].as_str().unwrap()
            }
            _ => panic!("{d:?}"),
        }
    }

    /// The same line minus the T922 guard, so the shape tests below keep reading `rtok run …`.
    fn wrapped(d: &PreToolDecision) -> String {
        rewritten(d).replacen(crate::names::run_guard(), "", 1)
    }

    #[test]
    fn git_status_is_wrapped() {
        let d = decide("git status").unwrap();
        assert_eq!(wrapped(&d), "rtok run -- 'git status'");
    }

    #[test]
    fn heredoc_and_sudo_untouched() {
        assert!(decide("cat <<EOF").is_none());
        assert!(decide("sudo ls").is_none());
    }

    #[test]
    fn metacharacters_stay_inside_one_quoted_argument() {
        for cmd in [
            "true && false",
            "git status | head",
            "foo; bar",
            "echo x >/tmp/x",
        ] {
            let d = decide(cmd).unwrap();
            assert_eq!(
                wrapped(&d),
                format!("rtok run -- {}", super::super::run::wrap_quote(cmd))
            );
        }
    }

    /// A foreground `sleep` chained with `;` and a pipe is not background: it is
    /// wrapped whole, like any other compound command.
    #[test]
    fn sleep_then_piped_cat_is_wrapped() {
        let cmd = "sleep 90; cat /tmp/tasks/x.output | tail -30";
        let d = decide(cmd).unwrap();
        assert_eq!(
            wrapped(&d),
            "rtok run -- 'sleep 90; cat /tmp/tasks/x.output | tail -30'"
        );
    }

    /// The host keeps the shell's cwd between Bash calls, so a leading `cd` must run in
    /// the host shell, not inside `rtok run`'s child: only the rest is wrapped.
    #[test]
    fn leading_cd_hops_stay_outside_the_wrap() {
        let d = decide("cd crates/rtok && cargo test").unwrap();
        assert_eq!(wrapped(&d), "cd crates/rtok && rtok run -- 'cargo test'");
        // Double quotes: on Windows any apostrophe keeps the command unwrapped (T55.12).
        let d = decide(r#"cd "a && b" && cd c && git status | head"#).unwrap();
        assert_eq!(
            wrapped(&d),
            r#"cd "a && b" && cd c && rtok run -- 'git status | head'"#
        );
        let d = decide_as(Some("sub_1"), "cd x && ls").unwrap();
        assert_eq!(wrapped(&d), "cd x && rtok run --agent sub_1 -- 'ls'");
    }

    /// Anything else that changes the calling shell's state stays unwrapped whole.
    #[test]
    fn shell_state_builtins_are_not_wrapped() {
        for cmd in [
            "cd /repo",
            "cd /repo; ls",
            "ls && cd /repo",
            "cd a && cargo test && cd b",
            "export A=1",
            "export A=1 && cargo test",
            "source .venv/bin/activate && pytest",
            ". ./env.sh",
            "unset A",
            "pushd x && ls",
            "popd",
            "alias g=git",
        ] {
            assert!(decide(cmd).is_none(), "{cmd} must stay unwrapped");
        }
        assert!(decide("git checkout -b export").is_some());
    }

    /// T444: the skip rules also apply to the command behind `cd` hops.
    #[test]
    fn skip_rules_apply_behind_cd_hops() {
        for cmd in [
            "cd x && rtok expand abc",
            "cd x && cd y && rtok expand abc",
            "cd x && sudo ls",
            "cd x && python -i",
            "cd x && sleep 10 &",
        ] {
            assert!(decide(cmd).is_none(), "{cmd} must stay unwrapped");
        }
        let d = decide("cd x && cargo test").unwrap();
        assert_eq!(wrapped(&d), "cd x && rtok run -- 'cargo test'");
    }

    /// T444: an escaped quote must not hide a later `cd` from the state check.
    #[test]
    fn escaped_quote_does_not_hide_a_cd() {
        assert!(decide(r"echo it\'s && cd sub").is_none());
        assert!(decide(r#"echo "a\" b" && cd sub"#).is_none());
        // Windows leaves every apostrophe command whole (T55.12).
        assert_eq!(decide(r"echo it\'s fine").is_some(), !cfg!(windows));
    }

    #[test]
    fn more_state_builtins_and_bare_assignments_stay_unwrapped() {
        for cmd in [
            "set -e",
            "shopt -s globstar",
            "umask 022",
            "trap 'rm x' EXIT",
            "declare -x A=1",
            "typeset A=1",
            "readonly A=1",
            "A=1",
            "A=1 B=2 && cargo test",
            "A='x y'",
        ] {
            assert!(decide(cmd).is_none(), "{cmd} must stay unwrapped");
        }
        let d = decide("A=1 cargo test").unwrap();
        assert_eq!(wrapped(&d), "rtok run -- 'A=1 cargo test'");
        assert!(decide("echo a=b").is_some());
    }

    #[test]
    fn trailing_ampersand_untouched() {
        assert!(decide("sleep 10 &").is_none());
        assert!(decide("sleep 10&").is_none());
        assert!(decide("true && false").is_some(), "&& is not background");
    }

    #[test]
    fn windows_path_and_exe_honor_never_wrap() {
        assert!(decide(r"C:\Windows\System32\sudo.exe ls").is_none());
        assert!(decide(r"C:\tools\rtok.exe run -- true").is_none());
        assert!(decide("sudo.exe ls").is_none());
        // Still wrap a normal command with a Windows-looking path.
        assert!(decide(r"C:\Program Files\Git\cmd\git.exe status").is_some());
    }

    #[test]
    fn sudo_exe_case_insensitive_never_wrap() {
        assert!(decide("Sudo.exe ls").is_none());
        assert!(decide("SUDO.EXE ls").is_none());
        assert!(decide("RTOK.EXE run -- true").is_none());
        assert!(decide("Sudo ls").is_none());
    }

    #[test]
    fn wrap_keeps_apostrophe_host_safe() {
        let cmd = "echo it's fine";
        match decide(cmd) {
            // Windows (T55.12): nothing is wrapped, so no PowerShell `''` form
            // can reach a POSIX host shell. The host contracts are pinned by
            // `windows_apostrophe_commands_stay_unwrapped`; here the wrapped
            // POSIX path only has to round-trip through the real quoter.
            None => {}
            Some(d) => {
                let w = wrapped(&d);
                assert!(w.starts_with("rtok run -- "), "{w}");
                let q = &w["rtok run -- ".len()..];
                assert_eq!(q, &super::super::run::wrap_quote(cmd));
            }
        }
    }

    #[test]
    fn windows_apostrophe_commands_stay_unwrapped() {
        let stems = crate::config::Cmd::default().interactive_stems;
        assert!(skip_wrap_host(true, "echo it's fine", &[], &stems));
        assert!(skip_wrap_host(true, "git commit -m 'fix it'", &[], &stems));
        // The rule fires regardless of position; other skips still apply first.
        assert!(skip_wrap_host(true, "jq '.' data.json", &[], &stems));
        // A POSIX host keeps wrapping apostrophe commands: sh quoting round-trips.
        assert!(!skip_wrap_host(false, "echo it's fine", &[], &stems));
        assert!(!skip_wrap_host(
            false,
            "git commit -m 'fix it'",
            &[],
            &stems
        ));
    }

    #[test]
    fn per_stem_interactive_i_table() {
        let stems = crate::config::Cmd::default().interactive_stems;
        let nw = &[];
        // Non-interactive `-i` stems get wrapped.
        assert!(!skip_wrap_host(false, "ffmpeg -i x", nw, &stems));
        assert!(decide("ffmpeg -i x").is_some());
        assert!(!skip_wrap_host(false, "ssh -i key host", nw, &stems));
        assert!(decide("ssh -i key host").is_some());
        // REPL / TTY stems stay unwrapped.
        assert!(skip_wrap_host(false, "python -i", nw, &stems));
        assert!(decide("python -i").is_none());
        assert!(skip_wrap_host(false, "docker run -i alpine sh", nw, &stems));
        assert!(decide("docker run -i alpine sh").is_none());
        // `--interactive` always skips, even on non-REPL stems.
        assert!(skip_wrap_host(false, "ffmpeg --interactive x", nw, &stems));
        assert!(decide("ffmpeg --interactive x").is_none());
    }

    /// Append a POSIX single-quoted span to `word`, up to the closing `'`.
    fn sh_single_quoted(chars: &mut std::str::Chars<'_>, word: &mut String) {
        word.extend(chars.by_ref().take_while(|&q| q != '\''));
    }

    /// A minimal double-quote pass so the POSIX `'"'"'` embedding (an apostrophe inside
    /// `"'"`) parses the way sh reads it: `\` escapes the next char, `"` closes.
    fn sh_double_quoted(chars: &mut std::str::Chars<'_>, word: &mut String) {
        while let Some(q) = chars.next() {
            match q {
                '"' => break,
                '\\' => word.extend(chars.next()),
                q => word.push(q),
            }
        }
    }

    /// Split `input` into words the way POSIX sh would (quotes only, no expansion).
    fn sh_words(input: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut word = String::new();
        let mut in_word = false;
        let mut chars = input.chars();
        while let Some(c) = chars.next() {
            match c {
                '\'' => {
                    in_word = true;
                    sh_single_quoted(&mut chars, &mut word);
                }
                '"' => {
                    in_word = true;
                    sh_double_quoted(&mut chars, &mut word);
                }
                c if c.is_whitespace() => {
                    if in_word {
                        words.push(std::mem::take(&mut word));
                        in_word = false;
                    }
                }
                c => {
                    in_word = true;
                    word.push(c);
                }
            }
        }
        if in_word {
            words.push(word);
        }
        words
    }

    /// Parse-simulation of the loss the fix removes: the PowerShell `''` form,
    /// split as POSIX sh words, concatenates the adjacent quotes into one word
    /// and the apostrophe is gone — the command that runs is not the command
    /// the model asked for.
    #[test]
    fn ps_quoting_does_not_round_trip_under_sh() {
        let cmd = "echo it's fine";
        let ps_form = format!("'{}'", cmd.replace('\'', "''"));
        assert_eq!(sh_words(&ps_form), ["echo its fine"]);
        assert_ne!(sh_words(&ps_form), [cmd]);
        // The POSIX form does round-trip, which is why only the Windows path skips.
        assert_eq!(sh_words(&super::super::run::sh_quote(cmd)), [cmd]);
    }

    /// T127: a sub-agent's `agent_id` rides along in the rewrite so `rtok run` can scope
    /// its dedup pointer to the same window that dispatched the command.
    #[test]
    fn sub_agent_dispatch_embeds_its_agent_id() {
        let d = decide_as(Some("agent-a1"), "cargo test").unwrap();
        assert_eq!(wrapped(&d), "rtok run --agent agent-a1 -- 'cargo test'");
    }

    /// The main window carries no `agent_id`; the rewrite is unchanged from before T127.
    #[test]
    fn main_window_dispatch_has_no_agent_flag() {
        let d = decide_as(None, "git status").unwrap();
        assert_eq!(wrapped(&d), "rtok run -- 'git status'");
    }

    /// A shape outside `[A-Za-z0-9_-]{1,64}` is dropped rather than embedded — fail open,
    /// never a chance to inject into the rewritten argv.
    #[test]
    fn malformed_agent_id_is_dropped() {
        for bad in ["has space", "semi;colon", "", &"a".repeat(65), "quote'here"] {
            let d = decide_as(Some(bad), "cargo test").unwrap();
            assert_eq!(wrapped(&d), "rtok run -- 'cargo test'", "bad id: {bad:?}");
        }
    }

    /// T437: the host guard of an isolated sub-agent refuses `rtok run -- 'git ...'`, so
    /// every shape of a git command from a sub-agent must reach the host unwrapped.
    #[test]
    fn sub_agent_git_commands_are_never_wrapped() {
        for cmd in [
            "git status",
            "cd /repo/.claude/worktrees/agent-a1 && git status",
            "/usr/bin/git add -A",
            "git -C /repo/.claude/worktrees/agent-a1 commit -m x",
            "cargo test && git status",
            "C:\\Program Files\\Git\\cmd\\git.exe status",
            "bash -c 'git status'",
            "echo $(git rev-parse HEAD)",
            "env GIT_DIR=x GIT_PAGER=cat git log",
        ] {
            assert!(decide_as(Some("agent-a1"), cmd).is_none(), "wrapped: {cmd}");
        }
    }

    /// T933: the exact commands the host guard refused for three weft sub-agents on
    /// 2026-10-16 (an installed binary older than T437 had wrapped them, `--agent` and all).
    /// Each must reach the host unwrapped with a real sub-agent id, and the same commands
    /// from the main session, which has no guard, keep their wrap.
    #[test]
    fn weft_2026_10_16_refused_commands_are_never_wrapped() {
        let wt = "/work/_worktrees/weft-agent-a10131a73e855515f";
        let cmds = [
            "git fetch origin -q && git checkout -q -B t105.17-winui-keywords origin/main && git log --oneline -3 && ls && grep -n \"T105.17\" -A12 plan.md | head -40".to_string(),
            "git fetch origin -q && git checkout -q -B t105.17-winui-keywords origin/main && git log --oneline -3".to_string(),
            "git fetch origin -q && git checkout -B t903-macos-save-as origin/main 2>&1 | tail -2 && ls && grep -n \"T903\" -A15 plan.md | head -60".to_string(),
            "git fetch origin -q && git checkout -B t105.15-winui-tokens origin/main 2>&1 | tail -2 && git log --oneline -3 && ls".to_string(),
            format!("git -C {wt} fetch origin -q"),
            "git status".to_string(),
            "git status --short --branch".to_string(),
            "git fetch origin -q".to_string(),
            "git fetch origin -q && echo ok".to_string(),
            "/usr/bin/git fetch origin -q && echo ok".to_string(),
            "/usr/bin/git status | head -3".to_string(),
            "/usr/bin/env git log --oneline -S\"@class\" | cat; grep -n \"keyword\\|@class\\|verbatim\" research.md | head -20; grep -n \"account\" crates/weft-winui/tests/common/mod.rs".to_string(),
        ];
        for cmd in &cmds {
            assert!(
                decide_as(Some("a10131a73e855515f"), cmd).is_none(),
                "sub-agent git wrapped: {cmd}"
            );
            let d = decide_as(None, cmd).unwrap();
            assert!(
                wrapped(&d).starts_with("rtok run -- '"),
                "main session: {cmd}"
            );
        }
    }

    /// Only git is exempt: the sub-agent still gets compressed output for everything
    /// else, including words that merely contain the letters.
    #[test]
    fn sub_agent_non_git_commands_stay_wrapped() {
        for cmd in ["cargo test", "cargo test github", "ls my_git digit"] {
            let d = decide_as(Some("agent-a1"), cmd).unwrap();
            assert_eq!(wrapped(&d), format!("rtok run --agent agent-a1 -- '{cmd}'"));
        }
    }

    /// The main session has no guard, so its git output keeps being compressed.
    #[test]
    fn main_session_git_commands_stay_wrapped() {
        let d = decide_as(None, "cd /repo && git status").unwrap();
        assert_eq!(wrapped(&d), "cd /repo && rtok run -- 'git status'");
    }

    /// T922: the line runs in the host's Bash, whose PATH may lack `rtok` even when the hook
    /// itself found it; the original command must still run, with its own exit code.
    #[cfg(unix)]
    fn run_line(shell: &str, line: &str, path: &str) -> (String, i32) {
        let out = std::process::Command::new(shell)
            .args(["-c", line])
            .env_clear()
            .env("PATH", path)
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap(),
        )
    }

    #[cfg(unix)]
    #[test]
    fn rewrite_runs_the_original_command_when_rtok_is_not_on_path() {
        for shell in ["/bin/sh", "/bin/bash", "/bin/zsh"] {
            if !std::path::Path::new(shell).exists() {
                continue;
            }
            for (cmd, want) in [
                ("echo \"it's fine\"; exit 3", ("it's fine\n", 3)),
                ("cd / && pwd", ("/\n", 0)),
            ] {
                let d = decide(cmd).unwrap();
                let got = run_line(shell, rewritten(&d), "/usr/bin:/bin");
                assert_eq!((got.0.as_str(), got.1), want, "{shell}: {}", rewritten(&d));
            }
            let d = decide_as(Some("agent-a1"), "echo hi").unwrap();
            let got = run_line(shell, rewritten(&d), "/usr/bin:/bin");
            assert_eq!(got, ("hi\n".to_string(), 0), "{shell}: {}", rewritten(&d));
        }
    }

    /// The guard only fills a gap: with `rtok` on PATH the real binary gets the exact argv.
    #[cfg(unix)]
    #[test]
    fn rewrite_calls_rtok_untouched_when_it_is_on_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rtok-t922-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("rtok");
        std::fs::write(&fake, "#!/bin/sh\nfor a; do echo \"[$a]\"; done\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let d = decide("echo hi").unwrap();
        let path = format!("{}:/usr/bin:/bin", dir.display());
        let got = run_line("/bin/sh", rewritten(&d), &path);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(got, ("[run]\n[--]\n[echo hi]\n".to_string(), 0));
    }
}

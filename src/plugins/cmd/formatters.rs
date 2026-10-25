// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Per-family stdout compactors (plan T3.3). `None` → fall back to rules.

use super::rules;

/// Compact `output`. Kind is `formatter`, `rule`, `raw` (below the size gate, or a T176
/// bounded passthrough — both by design), or `unmatched` (a rule/formatter ran and
/// shrank nothing — T177's actionable share).
pub fn compress(
    settings: &rules::Settings,
    argv: &[String],
    output: &str,
    exit: i32,
    archive_id: &str,
) -> (String, &'static str) {
    // T677: mise's install chatter sits in front of whatever the task prints, so it goes
    // before the inner command's formatter or rule looks at the text. Dropping it is the
    // mise formatter's work, even if nothing further shrinks.
    if bin(&family_argv(argv)) == "mise"
        && let Some(clean) = super::mise::tidy(output)
    {
        let (s, kind) = compress_text(settings, argv, &clean, exit, archive_id);
        return (
            s,
            if matches!(kind, "raw" | "unmatched") {
                "formatter"
            } else {
                kind
            },
        );
    }
    // T678: a polling loop is mostly one line repeated. A loop over two programs is a mixed
    // chain that `[script]` would otherwise take, so the collapse runs before the route is chosen
    // and the rule or formatter then sees the short text.
    if matches!(bin(&family_argv(argv)), "for" | "while" | "until")
        && let Some(clean) = super::loops::collapse(output)
    {
        let (s, kind) = compress_text(settings, argv, &clean, exit, archive_id);
        return (
            s,
            if matches!(kind, "raw" | "unmatched") {
                "formatter"
            } else {
                kind
            },
        );
    }
    compress_text(settings, argv, output, exit, archive_id)
}

fn compress_text(
    settings: &rules::Settings,
    argv: &[String],
    output: &str,
    exit: i32,
    archive_id: &str,
) -> (String, &'static str) {
    // T176: `sed -n 1,620p`, `| tail -300`, `grep -A3` already printed what was asked
    // for; a second cut here only sent the agent to `expand` for the same bytes. This
    // stays ahead of the mix check: `cargo test | tail -300` is bounded even though it
    // names two programs.
    if output.len() <= super::bounded::MAX_BYTES && super::bounded::is_bounded(&argv.join(" ")) {
        return (output.to_string(), "raw");
    }
    // T177: a single Bash string chaining 2+ DISTINCT programs (`&&`, `|`, `;`, or a newline)
    // is not one family's output; matching only argv[0] picks the wrong rule, or none at
    // all. A same-program chain (`cargo build && cargo test`, `cd x && cargo test`) keeps
    // its own family's formatter/rule — only a genuine mix routes to `[script]`.
    // T386: `mise exec -- cargo test` is still argv[0] `mise` until the inner command is
    // peeled; `mixed_chain` uses the same peel so two inner programs stay a mix.
    let matched = resolve(settings, argv, output);
    let stem = if matched.multi {
        "script"
    } else {
        bin(&matched.argv)
    };
    // The outer line is `mise`, which is not a bounded command; the peeled `sed -n` is.
    if !matched.multi
        && output.len() <= super::bounded::MAX_BYTES
        && super::bounded::is_bounded(&matched.argv.join(" "))
    {
        return (output.to_string(), "raw");
    }
    let rule = settings.pick(stem);
    // T65.2: JSON bodies skip table formatters so kubectl -o json / gh --json
    // reach the compact pass instead of a NAME/STATUS parser.
    if let Some(json) = rules::parse_json_body(output) {
        let s = rules::apply_parsed(settings, output, Some(json), exit, &rule, archive_id);
        let kind = if s.len() < output.len() {
            "rule"
        } else {
            "unmatched"
        };
        return (s, kind);
    }
    if !matched.multi
        && let Some(s) = format(&matched.argv, output)
    {
        return (s, "formatter");
    }
    let s = rules::apply(settings, output, exit, &rule, archive_id);
    let kind = if stem == "skill" {
        "skill"
    } else if s.len() < output.len() {
        "rule"
    } else {
        "unmatched"
    };
    (s, kind)
}

/// The argv a family is matched against. The hook quotes the whole command into one argv
/// (`rtok run -- 'git status'`, `cmd/hook.rs`), while a hand-typed `rtok run -- git status`
/// arrives split; both must read as `["git", "status"]`, or no formatter and no `[rule]`
/// matches the path that actually runs. Only the first word of a snippet is considered —
/// `cmd/AGENTS.md` forbids parsing shell syntax beyond it.
pub fn family_argv(argv: &[String]) -> Vec<String> {
    match argv {
        [one] => one.split_whitespace().map(str::to_string).collect(),
        many => many.to_vec(),
    }
}

/// `argv[0]`'s basename — the family a `Measurement` names. `other` when there is none.
/// A pipe stays the first command (`git status | head` is `git`); [`family_of`] is what
/// sees a `mise run` / `just` echo.
pub fn family(argv: &[String]) -> String {
    // A pipe stays the first command. A wrapper around two different programs is
    // `[script]` — the same route `compress` takes — rather than the first inner stem.
    if let [one] = argv
        && super::bounded::mixed_chain(one)
    {
        let raw_argv = family_argv(argv);
        let peeled_argv = visible_argv(&raw_argv);
        let raw = bin(&raw_argv);
        let peeled = bin(&peeled_argv);
        if peeled != raw && wrapper_stem(raw) {
            return "script".to_string();
        }
        return named(&raw_argv);
    }
    // `command_argv` sees through `cd && mise exec -- cargo`, which `visible_argv`
    // on the whole line does not (argv[0] is still `cd`).
    named(&command_argv(argv))
}

/// [`family`] once `output` is available, so a wrapper whose inner command is only on
/// an echoed line (`[task] $ cargo test`, a just recipe line) is named for that command.
pub fn family_of(settings: &rules::Settings, argv: &[String], output: &str) -> String {
    let base = family(argv);
    if base != "mise" && base != "just" {
        return base;
    }
    match echoed(settings, &base, output) {
        Some(resolved) if resolved.multi => "script".to_string(),
        Some(resolved) => named(&resolved.argv),
        None => base,
    }
}

fn named(argv: &[String]) -> String {
    match bin(argv) {
        "" => "other".to_string(),
        found => found.to_string(),
    }
}

/// Basename of argv[0], splitting on `/` and `\` and dropping a trailing `.exe`
/// (case-insensitive). Re-export of [`crate::agents::cmd_stem`] — one definition,
/// shared with `measure::stats::bash_family` and `agents::is_rtok_bin` (T55.10).
pub(crate) use crate::names::cmd_stem;

/// Stems with a Rust formatter (any subcommand). `rtok stats` labels the whole stem.
const FORMATTER_STEMS: &[&str] = &[
    "cargo", "git", "gh", "pytest", "python", "python3", "py", "jest", "vitest", "tree", "go",
    "docker", "podman", "kubectl", "ps", "df", "sqlite3", "wc", "mise", "for", "while", "until",
];

/// T50.1: how `rtok stats` labels a Bash family — `formatter`, named `rule`, or `default`.
pub fn filter_kind(settings: &rules::Settings, stem: &str) -> &'static str {
    if FORMATTER_STEMS.contains(&stem) {
        return "formatter";
    }
    if settings.pick(stem).match_cmd == stem {
        return "rule";
    }
    "default"
}

fn bin(argv: &[String]) -> &str {
    argv.first().map(|a| cmd_stem(a)).unwrap_or("")
}

fn sub(argv: &[String]) -> &str {
    argv.get(1).map(String::as_str).unwrap_or("")
}

struct Resolved {
    argv: Vec<String>,
    /// Two or more distinct programs: use `[script]`, not the first family's formatter.
    multi: bool,
}

fn script_match() -> Resolved {
    Resolved {
        argv: vec!["script".to_string()],
        multi: true,
    }
}

fn resolve(settings: &rules::Settings, argv: &[String], output: &str) -> Resolved {
    if matches!(argv, [one] if super::bounded::mixed_chain(one)) {
        return script_match();
    }
    let vis = command_argv(argv);
    let stem = bin(&vis);
    if matches!(stem, "mise" | "just")
        && let Some(echoed) = echoed(settings, stem, output)
    {
        return echoed;
    }
    Resolved {
        argv: vis,
        multi: false,
    }
}

/// Argv after wrappers the host puts in front of the real command: `mise exec`,
/// `just -c`, a leading `FOO=1`, and `uv`/`npx`/`python -m`/`cmd /c`. At most four
/// peels, so `uv run python -m pytest` lands on `pytest` and a deeper nest stays put.
/// A recipe stays as written.
pub(crate) fn visible_argv(argv: &[String]) -> Vec<String> {
    let mut cur = argv.to_vec();
    for _ in 0..4 {
        let Some(inner) = peel_mise_exec(&cur)
            .or_else(|| peel_just_command(&cur))
            .or_else(|| peel_leading_env(&cur))
            .or_else(|| peel_tool_wrapper(&cur))
        else {
            break;
        };
        if inner == cur {
            break;
        }
        cur = inner;
    }
    cur
}

/// Same inner command on every wrapper stage. Peeling `cargo build && cargo test` would keep only `cargo test`'s needles, so that chain stays on [`family_argv`].
fn command_argv(argv: &[String]) -> Vec<String> {
    let line = match argv {
        [one] => one.clone(),
        many => many.join(" "),
    };
    let stages: Vec<Vec<String>> = super::bounded::stages(&line)
        .into_iter()
        .filter(|s| !s.is_empty() && !silent_stage(s))
        .collect();
    if stages.iter().any(|stage| is_wrapper_stage(stage)) {
        let peeled: Vec<Vec<String>> = stages.iter().map(|s| visible_argv(s)).collect();
        if let Some(first) = peeled.first() {
            let stem = bin(first).to_string();
            if !stem.is_empty() && peeled.iter().all(|p| bin(p) == stem) {
                let sub0 = sub(first).to_string();
                if peeled.iter().all(|p| sub(p) == sub0) {
                    return first.clone();
                }
                return vec![stem];
            }
        }
    }
    visible_argv(&family_argv(argv))
}

fn silent_stage(stage: &[String]) -> bool {
    matches!(stage.first().map(String::as_str), Some("cd" | "export"))
}

fn is_wrapper_stage(stage: &[String]) -> bool {
    peel_mise_exec(stage).is_some()
        || peel_just_command(stage).is_some()
        || peel_leading_env(stage).is_some()
        || peel_tool_wrapper(stage).is_some()
}

fn wrapper_stem(stem: &str) -> bool {
    matches!(
        stem,
        "mise"
            | "just"
            | "uv"
            | "uvx"
            | "npx"
            | "pnpm"
            | "yarn"
            | "poetry"
            | "pipenv"
            | "hatch"
            | "python"
            | "python3"
            | "py"
            | "cmd"
    ) || is_env_assign(stem)
}

fn peel_mise_exec(argv: &[String]) -> Option<Vec<String>> {
    if cmd_stem(argv.first()?) != "mise" {
        return None;
    }
    let mut i = skip_mise_flags(argv, 1)?;
    if i >= argv.len() || !matches!(argv[i].as_str(), "exec" | "x") {
        return None;
    }
    i += 1;
    while i < argv.len() {
        let tok = argv[i].as_str();
        if tok == "--" {
            let cmd = argv[i + 1..].to_vec();
            return (!cmd.is_empty()).then_some(cmd);
        }
        if tok == "-c" || tok == "--command" {
            let words: Vec<String> = argv
                .get(i + 1)?
                .split_whitespace()
                .map(str::to_string)
                .collect();
            return (!words.is_empty()).then_some(words);
        }
        if let Some(n) = mise_flag_width(tok) {
            if i + n > argv.len() {
                return None;
            }
            i += n;
            continue;
        }
        if tok.starts_with('-') {
            return None;
        }
        i += 1;
    }
    None
}

/// Leading `FOO=1 BAR=baz cmd`: the assignments are not the program.
fn peel_leading_env(argv: &[String]) -> Option<Vec<String>> {
    let mut i = 0;
    while i < argv.len() && is_env_assign(&argv[i]) {
        i += 1;
    }
    (i > 0 && i < argv.len()).then(|| argv[i..].to_vec())
}

fn is_env_assign(tok: &str) -> bool {
    let Some((key, value)) = tok.split_once('=') else {
        return false;
    };
    // A quote in the value is an unclosed `FOO='x cd y`. `bash_family` leaves that
    // token alone; peeling it would name `cd` instead.
    !value.is_empty()
        && !value.contains(['\'', '"'])
        && key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `uv run`, `uvx`, `npx`, `pnpm exec|dlx`, `yarn dlx`, `poetry|pipenv|hatch run`,
/// `python|python3|py -m`, `cmd /c`. A flag we do not know stays on the wrapper.
fn peel_tool_wrapper(argv: &[String]) -> Option<Vec<String>> {
    let stem = cmd_stem(argv.first()?);
    let rest_at = match stem {
        "uv" if argv.get(1).map(String::as_str) == Some("run") => 2,
        "uvx" | "npx" => 1,
        "pnpm" if matches!(argv.get(1).map(String::as_str), Some("exec" | "dlx")) => 2,
        "yarn" if argv.get(1).map(String::as_str) == Some("dlx") => 2,
        "poetry" | "pipenv" | "hatch" if argv.get(1).map(String::as_str) == Some("run") => 2,
        "python" | "python3" | "py" if argv.get(1).map(String::as_str) == Some("-m") => 2,
        "cmd" if argv.get(1).is_some_and(|s| s.eq_ignore_ascii_case("/c")) => 2,
        _ => return None,
    };
    command_after(argv, rest_at)
}

fn command_after(argv: &[String], at: usize) -> Option<Vec<String>> {
    let mut rest = argv.get(at..)?;
    if rest.first().map(String::as_str) == Some("--") {
        rest = rest.get(1..)?;
    }
    if rest.is_empty() || rest[0].starts_with('-') {
        return None;
    }
    if rest.len() == 1 && rest[0].chars().any(char::is_whitespace) {
        let words: Vec<String> = rest[0].split_whitespace().map(str::to_string).collect();
        return (!words.is_empty()).then_some(words);
    }
    Some(rest.to_vec())
}

/// `just -c` / `--command` names the program on the argv. A recipe name does not: the body is in the justfile, so only an echoed line can name it.
fn peel_just_command(argv: &[String]) -> Option<Vec<String>> {
    if cmd_stem(argv.first()?) != "just" {
        return None;
    }
    let mut i = 1;
    while i < argv.len() {
        let tok = argv[i].as_str();
        if tok == "-c" || tok == "--command" {
            let cmd = argv[i + 1..].to_vec();
            return (!cmd.is_empty()).then_some(cmd);
        }
        if tok.starts_with('-') {
            let extra = just_flag_values(tok);
            if i + extra >= argv.len() {
                return None;
            }
            i += 1 + extra;
            continue;
        }
        return None;
    }
    None
}

fn just_flag_values(tok: &str) -> usize {
    if tok.contains('=') {
        return 0;
    }
    match tok {
        "--set" => 2,
        "-f"
        | "--justfile"
        | "-d"
        | "--working-directory"
        | "--color"
        | "--shell"
        | "--shell-arg"
        | "--command-color"
        | "--dotenv-filename"
        | "--dotenv-command"
        | "--chooser"
        | "--ceiling"
        | "--justfile-name" => 1,
        _ => 0,
    }
}

fn skip_mise_flags(argv: &[String], mut i: usize) -> Option<usize> {
    while i < argv.len() {
        let tok = argv[i].as_str();
        if matches!(tok, "exec" | "x" | "run" | "r" | "--") {
            break;
        }
        if let Some(n) = mise_flag_width(tok) {
            if i + n > argv.len() {
                return None;
            }
            i += n;
            continue;
        }
        if tok.starts_with('-') {
            return None;
        }
        break;
    }
    Some(i)
}

fn mise_flag_width(tok: &str) -> Option<usize> {
    if let Some((name, _)) = tok.split_once('=')
        && name.starts_with("--")
    {
        return mise_flag_width(name).map(|_| 1);
    }
    const VALUE: &[&str] = &[
        "-C",
        "--cd",
        "-E",
        "--env",
        "-j",
        "--jobs",
        "--allow-env",
        "--allow-net",
        "--allow-read",
        "--allow-write",
    ];
    const BOOL: &[&str] = &[
        "-q",
        "-v",
        "-y",
        "--quiet",
        "--verbose",
        "--yes",
        "--locked",
        "--silent",
        "--raw",
        "--no-deps",
        "--fresh-env",
        "--deny-all",
        "--deny-env",
        "--deny-net",
        "--deny-read",
        "--deny-write",
    ];
    if VALUE.contains(&tok) {
        return Some(2);
    }
    if BOOL.contains(&tok) || is_verbose_short(tok) {
        return Some(1);
    }
    None
}

fn is_verbose_short(tok: &str) -> bool {
    tok.len() > 2 && tok.bytes().all(|b| b == b'-' || b == b'v') && tok.ends_with('v')
}

/// Inner command from a wrapper's own echo. Absent when the run was quiet or the
/// line is the tool's output — those stay on the wrapper (fail open).
fn echoed(settings: &rules::Settings, wrapper: &str, output: &str) -> Option<Resolved> {
    let mut first: Option<Vec<String>> = None;
    let mut stem = String::new();
    let mut first_sub = String::new();
    let mut sub_differs = false;
    for line in output.lines() {
        let Some(cmd) = echo_command(settings, wrapper, line) else {
            continue;
        };
        if super::bounded::mixed_chain(&cmd) {
            return Some(script_match());
        }
        let vis = visible_argv(&family_argv(&[cmd]));
        let got = bin(&vis);
        if got.is_empty() {
            continue;
        }
        if stem.is_empty() {
            stem = got.to_string();
            first_sub = sub(&vis).to_string();
            first = Some(vis);
            continue;
        }
        if got != stem {
            return Some(script_match());
        }
        if sub(&vis) != first_sub {
            sub_differs = true;
        }
    }
    let argv = first?;
    if sub_differs {
        return Some(Resolved {
            argv: vec![stem],
            multi: false,
        });
    }
    Some(Resolved { argv, multi: false })
}

fn echo_command(settings: &rules::Settings, wrapper: &str, line: &str) -> Option<String> {
    let stripped = strip_ansi(line);
    if stripped.starts_with([' ', '\t']) {
        return None;
    }
    let t = stripped.trim();
    // A long line that starts with a known stem is tool output, not the echoed command.
    if t.is_empty() || t.len() > 400 {
        return None;
    }
    if let Some(cmd) = task_dollar(t) {
        return Some(cmd.to_string());
    }
    if wrapper != "just" {
        return None;
    }
    let words: Vec<String> = t.split_whitespace().map(str::to_string).collect();
    let stem = cmd_stem(words.first()?);
    if matches!(stem, "mise" | "just") {
        return Some(t.to_string());
    }
    if filter_kind(settings, stem) != "default" && looks_like_invocation(&words) {
        return Some(t.to_string());
    }
    None
}

fn task_dollar(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let (name, cmd) = rest.split_once("] $ ")?;
    if name.is_empty() || cmd.is_empty() {
        return None;
    }
    Some(cmd)
}

fn looks_like_invocation(words: &[String]) -> bool {
    let Some(second) = words.get(1) else {
        return true;
    };
    let s = second.as_str();
    if s.contains("ERR") || s.eq_ignore_ascii_case("error") || s.eq_ignore_ascii_case("warning") {
        return false;
    }
    !s.contains(':')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub(super) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn format(argv: &[String], output: &str) -> Option<String> {
    match (bin(argv), sub(argv)) {
        ("cargo", "test") => Some(
            super::compact::cargo_test(output)
                .unwrap_or_else(|| keep(output, &["FAILED", "test result:", "error[", "panicked"])),
        ),
        ("cargo", "build") | ("cargo", "clippy") => {
            Some(super::compact::cargo_build(output).unwrap_or_else(|| {
                keep(output, &["error[", "error:", "-->", "Finished", "warning:"])
            }))
        }
        ("git", "status") => {
            Some(super::compact::git_status(output).unwrap_or_else(|| git_status(output)))
        }
        ("git", "diff") => Some(
            super::diffs::tiered(output)
                .unwrap_or_else(|| super::compact::git_diff(&git_diff(output))),
        ),
        // `git show <rev>:<file>` prints a file, not a diff: only a diff stream is touched.
        ("git", "show") => super::diffs::tiered(output),
        ("ls", _) if super::compact::ls_long(argv) => super::compact::ls_l(output),
        ("cat", _) => super::catfile::outline_of(argv, output),
        ("grep", _) | ("rg", _) | ("git", "grep") => super::grep::group(argv, output),
        ("git", "log") => Some(
            git_log(output)
                .unwrap_or_else(|| output.lines().take(20).collect::<Vec<_>>().join("\n")),
        ),
        ("gh", _) => super::gh::format(argv, output),
        ("pytest", _) => Some(
            super::python::pytest(output)
                .unwrap_or_else(|| keep(output, &["FAILED", "ERROR", "passed", "failed", "error"])),
        ),
        ("python" | "python3" | "py", _) => super::python::script(output),
        ("jest", _) | ("vitest", _) => Some(keep(output, &["FAIL", "PASS", "Tests:", "● "])),
        ("go", "test") => Some(keep(output, &["FAIL", "PASS", "ok  ", "--- FAIL"])),
        ("wc", _) => Some(wc_table(output)),
        ("for" | "while" | "until", _) => super::loops::collapse(output),
        ("tree", _) => Some(output.lines().take(40).collect::<Vec<_>>().join("\n")),
        // `podman ps` prints the same table (T905). Other podman subcommands stay on the rule path.
        ("docker" | "podman", "ps") => docker_ps(output),
        ("kubectl", "get") => kubectl_get(output),
        ("ps", "aux") => ps_aux(output),
        ("df", _) => df_table(output),
        ("sqlite3", _) => sqlite3_table(output),
        ("mise", _) => super::mise::tidy(output),
        _ => None,
    }
}

/// Commits shown by [`git_log`] before the rest is counted as `… +K more`.
const GIT_LOG_MAX_COMMITS: usize = 100;

struct Commit<'a> {
    sha: &'a str,
    author: &'a str,
    date: String,
    year: &'a str,
    subject: &'a str,
    stat: Vec<&'a str>,
}

/// `(insertions, deletions)` of a `--stat` summary line (`3 files changed, 5 insertions(+),
/// 1 deletion(-)`); `None` for a per-file line, whose `|` a summary never has.
fn stat_totals(line: &str) -> Option<(usize, usize)> {
    if line.contains(" | ") || !line.contains(" changed") {
        return None;
    }
    let count = |word: &str| {
        line.split(',')
            .find(|part| part.contains(word))
            .and_then(|part| part.split_whitespace().next()?.parse().ok())
            .unwrap_or(0)
    };
    Some((count("insertion"), count("deletion")))
}

/// `git log` in its default (medium) format, one line per commit: short hash, date,
/// subject. A first line gives `[N commits, +A/-D]` (totals only when `--stat` printed them)
/// and names the most frequent author once (alone, when all are equal); the others are named per line. `--stat` file lines
/// and the `N files changed` summary are kept under their commit. Every commit up to
/// [`GIT_LOG_MAX_COMMITS`] is listed, where a head cut kept only the first four.
/// `None` for any other shape (`--oneline`, `--format=…`, `-p`), which keeps the old cut.
fn git_log(output: &str) -> Option<String> {
    let mut commits: Vec<Commit> = Vec::new();
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("commit ") {
            let sha = rest.split_whitespace().next()?;
            if sha.len() < 7 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            commits.push(Commit {
                sha: &sha[..7],
                author: "",
                date: String::new(),
                year: "",
                subject: "",
                stat: Vec::new(),
            });
            continue;
        }
        let c = commits.last_mut()?;
        if let Some(a) = line.strip_prefix("Author:") {
            let a = a.trim();
            c.author = a.split(" <").next().unwrap_or(a);
        } else if let Some(d) = line.strip_prefix("Date:") {
            // `Wed Oct 14 23:25:53 2026 +0300` → `Oct 14`, year kept aside.
            let parts: Vec<&str> = d.split_whitespace().collect();
            if parts.len() >= 5 {
                c.date = format!("{} {}", parts[1], parts[2]);
                c.year = parts[4];
            }
        } else if line.starts_with("Merge:") || line.trim().is_empty() {
        } else if let Some(body) = line.strip_prefix("    ") {
            if c.subject.is_empty() {
                c.subject = body.trim();
            }
        } else if line.starts_with(' ') && (line.contains(" | ") || line.contains(" changed")) {
            c.stat.push(line.trim());
        } else if line.starts_with("diff --git") || line.starts_with("@@") {
            return None;
        }
    }
    if commits.is_empty() || commits.iter().any(|c| c.date.is_empty()) {
        return None;
    }
    // The most frequent author goes in the header once; only the others are named per line.
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for c in &commits {
        match counts.iter_mut().find(|(a, _)| *a == c.author) {
            Some((_, n)) => *n += 1,
            None => counts.push((c.author, 1)),
        }
    }
    let main_author = counts
        .iter()
        .fold(
            ("", 0),
            |best, &(a, n)| if n > best.1 { (a, n) } else { best },
        )
        .0;
    let one_year = commits.iter().all(|c| c.year == commits[0].year);
    let mut out = Vec::new();
    let mut head: Vec<String> = Vec::new();
    if !main_author.is_empty() {
        head.push(if counts.len() == 1 {
            main_author.to_string()
        } else {
            format!("{main_author} unless named")
        });
    }
    if one_year {
        head.push(commits[0].year.to_string());
    }
    // T682: the count and the `--stat` totals lead, so "how big is this range" needs no
    // counting of lines; the totals are claimed only when `--stat` printed them.
    let (added, removed, has_totals) =
        commits
            .iter()
            .flat_map(|c| &c.stat)
            .fold(
                (0usize, 0usize, false),
                |(a, d, seen), line| match stat_totals(line) {
                    Some((ins, del)) => (a + ins, d + del, true),
                    None => (a, d, seen),
                },
            );
    let n = commits.len();
    let mut counts_line = format!("[{n} commit{}", if n == 1 { "" } else { "s" });
    if has_totals {
        counts_line.push_str(&format!(", +{added}/-{removed}"));
    }
    counts_line.push(']');
    if !head.is_empty() {
        counts_line.push(' ');
        counts_line.push_str(&head.join(" · "));
    }
    out.push(counts_line);
    for c in commits.iter().take(GIT_LOG_MAX_COMMITS) {
        let date = if one_year {
            c.date.clone()
        } else {
            format!("{} {}", c.date, c.year)
        };
        let mut l = format!("{} {date} {}", c.sha, c.subject);
        if c.author != main_author {
            l.push_str(&format!(" — {}", c.author));
        }
        out.push(l);
        out.extend(c.stat.iter().map(|s| format!("  {s}")));
    }
    if commits.len() > GIT_LOG_MAX_COMMITS {
        out.push(format!("… +{} more", commits.len() - GIT_LOG_MAX_COMMITS));
    }
    Some(out.join("\n"))
}

/// T684: `wc` right-aligns its counts in padded columns; the numbers, names and the
/// `total` row stay, only the padding goes (leading too, which the default rule keeps).
fn wc_table(output: &str) -> String {
    output
        .lines()
        .map(|l| rules::collapse_columns(l.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `git diff`: the changed lines *are* the answer, so only the blob-hash bookkeeping goes.
/// A needle list here (`"+++"`, `"--- a/"`) matched the file headers and dropped every
/// `+`/`-` line, i.e. the whole change.
fn git_diff(output: &str) -> String {
    output
        .lines()
        .filter(|l| !l.starts_with("index "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `git status`: the branch line, what changed, and the untracked paths listed under their
/// header. Porcelain codes are read on the raw line — `trim_start` used to eat the first
/// column, so `" M x"` could never match — and an indented path belongs to the section
/// header above it. Action hints and section boilerplate are dropped; empty output falls
/// back to the first 15 lines rather than to nothing.
fn git_status(output: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut untracked = false;
    for line in output.lines() {
        let head = line.trim_start();
        if head.starts_with("Untracked files:") {
            untracked = true;
            out.push(head.to_string());
        } else if is_status_entry(head) {
            untracked = false;
            out.push(head.to_string());
        } else if is_porcelain(line) {
            untracked = false;
            out.push(line.to_string());
        } else if untracked && line.starts_with(['\t', ' ']) && !head.starts_with("(use ") {
            // An untracked path, indented under its header in the long format.
            out.push(head.to_string());
        }
    }
    if out.is_empty() {
        output.lines().take(15).collect::<Vec<_>>().join("\n")
    } else {
        out.join("\n")
    }
}

/// Long-format entries and the one-line summaries worth keeping.
fn is_status_entry(head: &str) -> bool {
    const VERBS: &[&str] = &[
        "modified:",
        "new file:",
        "deleted:",
        "renamed:",
        "copied:",
        "both modified:",
        "both added:",
        "both deleted:",
        "added by us:",
        "unmerged:",
    ];
    const LINES: &[&str] = &[
        "On branch ",
        "Your branch ",
        "HEAD detached ",
        "nothing to commit",
        "no changes added to commit",
        "nothing added to commit",
    ];
    VERBS.iter().any(|v| head.starts_with(v)) || LINES.iter().any(|l| head.starts_with(l))
}

/// `XY path` in the short/porcelain format: two status columns, then a space. Both columns
/// are meaningful (` M` modified in the worktree, `??` untracked), so the line is never
/// trimmed before this test.
fn is_porcelain(line: &str) -> bool {
    let b = line.as_bytes();
    let column = |c: u8| {
        matches!(
            c,
            b' ' | b'M' | b'A' | b'D' | b'R' | b'C' | b'U' | b'?' | b'!'
        )
    };
    b.len() > 3 && column(b[0]) && column(b[1]) && b[2] == b' '
}

/// `docker ps` and `podman ps`: one compact row per container. Padding, bind-all, container-side
/// port/proto, registry host and `N days`/`N hours` units are alignment noise; the
/// name, status, host port and image tag are the objects the rule's head/tail cut drops.
fn docker_ps(output: &str) -> Option<String> {
    let mut lines = output.lines().filter(|l| !l.is_empty());
    let header = lines.next()?;
    let u = header.to_ascii_uppercase();
    if !u.contains("CONTAINER ID") && !(u.contains("IMAGE") && u.contains("NAMES")) {
        return None;
    }
    let rows: Vec<String> = lines
        .map(compact_docker_row)
        .filter(|r| !r.is_empty())
        .collect();
    if rows.is_empty() {
        None
    } else {
        Some(rows.join("\n"))
    }
}

fn compact_docker_row(line: &str) -> String {
    let joined = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let no_bind = joined.replace("0.0.0.0:", "");
    shorten_ago(&drop_registry_host(&drop_arrow_port(&no_bind)))
}

fn drop_arrow_port(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '-' && chars.peek() == Some(&'>') {
            chars.next();
            while matches!(chars.peek(), Some(d) if d.is_ascii_digit()) {
                chars.next();
            }
            if chars.peek() == Some(&'/') {
                chars.next();
                while matches!(chars.peek(), Some(d) if d.is_ascii_alphabetic()) {
                    chars.next();
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn drop_registry_host(s: &str) -> String {
    s.split_whitespace()
        .map(|tok| match tok.split_once('/') {
            Some((host, rest)) if host.contains('.') && !host.contains(':') => rest,
            _ => tok,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `2 hours` → `2h`. Only a number followed by a unit shortens: docker's prose forms
/// (`About an hour`, `Less than a second`) have no number and stay whole words.
fn shorten_ago(s: &str) -> String {
    let unit = |w: &str| match w {
        "day" | "days" => Some('d'),
        "hour" | "hours" => Some('h'),
        "minute" | "minutes" => Some('m'),
        "second" | "seconds" => Some('s'),
        "week" | "weeks" => Some('w'),
        _ => None,
    };
    let words: Vec<&str> = s.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let number = !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit());
        if let Some(u) = words.get(i + 1).copied().and_then(unit).filter(|_| number) {
            out.push(format!("{w}{u}"));
            i += 2;
        } else {
            out.push(w.to_string());
            i += 1;
        }
    }
    out.join(" ")
}

/// `kubectl get`: one row per object, dropping wide columns (AGE, NODE, RESTARTS)
/// that the default rule keeps in the head/tail while omitting the middle objects.
fn kubectl_get(output: &str) -> Option<String> {
    let mut lines = output.lines().filter(|l| !l.is_empty());
    let header = lines.next()?;
    let cols: Vec<&str> = header.split_whitespace().collect();
    if cols.first().copied() != Some("NAME") {
        return None;
    }
    if !cols
        .iter()
        .any(|c| matches!(*c, "STATUS" | "READY" | "AGE"))
    {
        return None;
    }
    const KEEP: &[&str] = &["NAME", "READY", "STATUS", "IP"];
    let idx: Vec<usize> = cols
        .iter()
        .enumerate()
        .filter(|(_, c)| KEEP.contains(c))
        .map(|(i, _)| i)
        .collect();
    if idx.is_empty() {
        return None;
    }
    // kubectl aligns columns, and a cell can hold spaces (RESTARTS `1 (3m ago)` since
    // 1.22), so a cell is the text under its header, not the n-th whitespace token.
    let starts: Vec<usize> = header
        .char_indices()
        .filter(|&(i, c)| c != ' ' && (i == 0 || header.as_bytes()[i - 1] == b' '))
        .map(|(i, _)| i)
        .collect();
    let cell = |line: &str, col: usize| -> Option<String> {
        let from = starts.get(col).copied()?;
        let to = starts.get(col + 1).copied().unwrap_or(usize::MAX);
        let bytes = line.get(from..)?;
        let text = bytes.get(..to - from).unwrap_or(bytes).trim();
        (!text.is_empty()).then(|| text.to_string())
    };
    let mut out = vec![idx.iter().map(|&i| cols[i]).collect::<Vec<_>>().join(" ")];
    for line in lines {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 {
            continue;
        }
        // A row that does not line up with the header (proportional output, a cell
        // wider than its column) falls back to token order.
        let aligned = starts.len() == cols.len()
            && starts.iter().skip(1).all(|&at| {
                at <= line.len() && line.is_char_boundary(at) && line[..at].ends_with(' ')
            });
        let picked: Vec<String> = if aligned {
            idx.iter().filter_map(|&i| cell(line, i)).collect()
        } else {
            idx.iter()
                .filter_map(|&i| fields.get(i).map(|f| f.to_string()))
                .collect()
        };
        if !picked.is_empty() {
            out.push(picked.join(" "));
        }
    }
    if out.len() < 2 {
        None
    } else {
        Some(out.join("\n"))
    }
}

/// `ps aux`: one row per process. TTY/TIME padding is noise; PID plus the
/// command basename (and its last arg when that is a distinct worker id) is the object.
fn ps_aux(output: &str) -> Option<String> {
    let mut lines = output.lines().filter(|l| !l.is_empty());
    let header = lines.next()?;
    let u = header.to_ascii_uppercase();
    if !u.contains("PID") || !(u.contains("CMD") || u.contains("COMMAND")) {
        return None;
    }
    let mut rows = Vec::new();
    // The PID column is not always first: BSD `ps aux` prints `USER PID …`,
    // the macOS `ps aux` shape in the goldens prints `PID …` first.
    let pid_at = header
        .split_whitespace()
        .position(|h| h.eq_ignore_ascii_case("PID"))
        .unwrap_or(0);
    for line in lines {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // One bad line (a repeated header, a localized string, a wrapped row)
        // skips itself — returning None here would drop every good row with it.
        let pid_ok = fields
            .get(pid_at)
            .is_some_and(|pid| pid.bytes().all(|b| b.is_ascii_digit()));
        if !pid_ok {
            continue;
        }
        let pid = fields[pid_at];
        let cmd = fields
            .iter()
            .copied()
            .find(|t| t.starts_with('/') || t.starts_with("./"))
            .or_else(|| fields.last().copied())
            .unwrap_or("");
        let base = cmd.rsplit('/').next().unwrap_or(cmd);
        let tail = fields.last().copied().unwrap_or("");
        if tail != base && tail != pid {
            rows.push(format!("{pid} {base} {tail}"));
        } else {
            rows.push(format!("{pid} {base}"));
        }
    }
    if rows.is_empty() {
        None
    } else {
        Some(rows.join("\n"))
    }
}

/// `df`: one row per mount. Inode columns are noise beside used/size; a body that is not a df table falls through to [`Rule::default`].
fn df_table(output: &str) -> Option<String> {
    let mut lines = output.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next()?;
    let u = header.to_ascii_uppercase();
    if !u.contains("FILESYSTEM")
        || !(u.contains("MOUNTED") || u.contains("CAPACITY") || u.contains("USE%"))
    {
        return None;
    }
    let rows: Vec<String> = lines.filter_map(compact_df_row).collect();
    // Past the default cap, head/tail of the raw table is smaller than one short
    // row per mount. Stay on `Rule::default()` there.
    if rows.is_empty() || rows.len() > 40 {
        None
    } else {
        Some(rows.join("\n"))
    }
}

fn compact_df_row(line: &str) -> Option<String> {
    let t: Vec<&str> = line.split_whitespace().collect();
    let p = t.iter().position(|tok| tok.ends_with('%'))?;
    if p < 3 {
        return None;
    }
    let fs = t[..p - 3].join(" ");
    let size = t[p - 3];
    let used = t[p - 2];
    let pct = t[p];
    let mut i = p + 1;
    while i < t.len() && !t[i].contains('/') && df_inode_token(t[i]) {
        i += 1;
    }
    if fs.is_empty() || i >= t.len() {
        return None;
    }
    Some(format!("{fs} {used}/{size} {pct} {}", t[i..].join(" ")))
}

/// `sqlite3` rule lines (`----`, `+---+`) are not cells. Past [`Rule::default`]'s line cap, keeping every row is larger than that rule's head/tail cut.
fn sqlite3_table(output: &str) -> Option<String> {
    let lines: Vec<&str> = output.lines().collect();
    let dropped = lines.iter().filter(|l| sqlite_rule_line(l)).count();
    if dropped == 0 {
        return None;
    }
    // Collapse padding: dropping only the rule line can still be larger than `Rule::default`.
    let kept: Vec<String> = lines
        .into_iter()
        .filter(|l| !sqlite_rule_line(l))
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect();
    if kept.is_empty() || kept.len() > 40 {
        return None;
    }
    Some(kept.join("\n"))
}

fn sqlite_rule_line(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3
        && t.bytes()
            .all(|b| matches!(b, b'-' | b'=' | b' ' | b'+' | b'|'))
        && t.bytes().any(|b| b == b'-' || b == b'=')
}

fn df_inode_token(tok: &str) -> bool {
    if tok == "-" || tok.ends_with('%') {
        return true;
    }
    let core = tok.trim_end_matches(|c: char| {
        matches!(
            c,
            'k' | 'K' | 'm' | 'M' | 'g' | 'G' | 't' | 'T' | 'b' | 'B' | 'i' | 'I'
        )
    });
    !core.is_empty() && core.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

fn keep(output: &str, needles: &[&str]) -> String {
    let lines: Vec<&str> = output
        .lines()
        .filter(|l| needles.iter().any(|n| l.contains(n)))
        .collect();
    if lines.is_empty() {
        return output
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Only `<number> <unit>` shortens; docker's prose forms stay readable words.
    #[test]
    fn shorten_ago_leaves_docker_prose_durations_whole() {
        assert_eq!(shorten_ago("Up 2 hours"), "Up 2h");
        assert_eq!(shorten_ago("Exited (0) 3 days ago"), "Exited (0) 3d ago");
        assert_eq!(shorten_ago("Up 1 week (healthy)"), "Up 1w (healthy)");
        assert_eq!(shorten_ago("Up About an hour"), "Up About an hour");
        assert_eq!(shorten_ago("Up About a minute"), "Up About a minute");
        assert_eq!(
            shorten_ago("Up Less than a second"),
            "Up Less than a second"
        );
    }

    /// kubectl ≥1.22 prints RESTARTS as `1 (3m ago)`: three tokens in one column, so
    /// later columns are found by the header's offsets, not by token index.
    #[test]
    fn kubectl_get_wide_keeps_ip_after_a_restart_with_age() {
        let out = kubectl_get(
            "NAME    READY   STATUS    RESTARTS     AGE   IP         NODE\n\
             p       1/1     Running   1 (3m ago)   10d   10.0.0.5   n1\n\
             q       0/1     Pending   0            1m    <none>     <none>\n",
        )
        .expect("table");
        assert_eq!(
            out,
            "NAME READY STATUS IP\np 1/1 Running 10.0.0.5\nq 0/1 Pending <none>"
        );
    }

    /// T176 audit repro: `cargo nextest run … | tail -300` came back as 4.7 KB of 20 KB and
    /// lost the failing test's panic. Bounded, the failure block survives whole; the same
    /// log unbounded is still cut.
    #[test]
    fn a_bounded_failing_nextest_log_keeps_its_failure_block() {
        let settings = rules::Settings::builtin();
        let mut log: Vec<String> = (0..150)
            .map(|i| format!("        PASS [   0.01s] t{i}"))
            .collect();
        log.splice(
            70..70,
            [
                "        FAIL [   0.02s] rtok store::tests::lock",
                "--- STDERR:              rtok store::tests::lock ---",
                "thread 'store::tests::lock' panicked at src/store/mod.rs:88:5:",
                "assertion `left == right` failed",
            ]
            .map(String::from),
        );
        let log = log.join("\n");
        let bounded = ["cargo nextest run --workspace 2>&1 | tail -300".to_string()];
        assert_eq!(
            compress(&settings, &bounded, &log, 0, "id"),
            (log.clone(), "raw")
        );
        let (cut, _) = compress(&settings, &["cargo nextest run".into()], &log, 0, "id");
        assert!(cut.len() < log.len(), "unbounded output is still filtered");
    }

    /// T931: a chain whose every segment is an explicit `sed -n a,bp` window printed what
    /// was asked for, however it is joined or labelled; one unbounded segment still cuts.
    #[test]
    fn a_chain_of_bounded_sed_windows_passes_through_whole() {
        let settings = rules::Settings::builtin();
        let body = (0..240)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        for cmd in [
            "sed -n 1,80p a; sed -n 1,40p b",
            "sed -n 1,80p a && sed -n 1,40p b",
            "git show x | sed -n 1,100p",
            "sed -n 1,80p a; echo ---; sed -n 1,40p b",
            "sed -n 1,80p a\necho \"=== b ===\"\nsed -n 1,40p b",
            "cd /repo && sed -n 1,80p a; printf '\\n--\\n'; sed -n 1,40p b",
        ] {
            let (got, kind) = compress(&settings, &[cmd.to_string()], &body, 0, "id");
            assert_eq!((got.as_str(), kind), (body.as_str(), "raw"), "{cmd}");
        }
        for cmd in [
            "sed -n 1,80p a; cargo test",
            "sed -n 1,80p a; echo ---; cargo test",
            "echo ---; cargo test",
            "sed -n 1,80p a; echo $(cat big)",
        ] {
            let (got, _) = compress(&settings, &[cmd.to_string()], &body, 0, "id");
            assert!(got.len() < body.len(), "{cmd} must still be cut");
        }
    }

    fn goldens() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cmd_golden")
    }

    /// `min_saving: <percent>` (T238): the floor a family's saving must clear, checked in
    /// `ten_families_and_aws_key_unredacted`. `None` when the header line is absent — the
    /// golden test then fails so every `.in` must declare one.
    ///
    /// T297: `argv: 'a && b'` (single-quoted) parses as ONE argv element, the shape the
    /// hook actually sends (`rtok run -- '<cmd>'`, see `family_argv`'s doc comment). Only
    /// that shape can reach `compress`'s mixed-chain (`[script]`) routing, since `multi`
    /// requires a one-element argv — plain `argv: a b c` still splits on whitespace like
    /// every other golden.
    fn parse_in(s: &str) -> (Vec<String>, i32, Option<u32>, String) {
        let mut argv = Vec::new();
        let mut exit = 0;
        let mut min_saving = None;
        let mut rest = s;
        for line in s.lines() {
            if let Some(a) = line.strip_prefix("argv: ") {
                argv = match a.strip_prefix('\'').and_then(|a| a.strip_suffix('\'')) {
                    Some(quoted) => vec![quoted.to_string()],
                    None => a.split_whitespace().map(str::to_string).collect(),
                };
            } else if let Some(e) = line.strip_prefix("exit: ") {
                exit = e.parse().unwrap_or(0);
            } else if let Some(m) = line.strip_prefix("min_saving: ") {
                min_saving = m.trim().parse().ok();
            } else if line == "---" {
                rest = s.split_once("---\n").map(|(_, r)| r).unwrap_or("");
                break;
            }
        }
        (argv, exit, min_saving, rest.to_string())
    }

    /// Fixed rates (T238), matching `tests/mode_bench.rs` and `tokens::tests::RATES` — not
    /// `Estimator::default()`, so a config default change can't silently shift the goldens'
    /// floors.
    /// AWS's documented example access key id, planted in the `cat.in` golden. It must reach the
    /// output untouched: the formatters compress, they do not redact.
    const AWS_EXAMPLE_KEY_ID: &str = "AKIAIOSFODNN7EXAMPLE";

    const SAVING_RATES: crate::config::Estimator = crate::config::Estimator {
        code: 3.5,
        prose: 4.2,
        json: 3.0,
        cjk: 1.0,
    };

    /// Saving % of shrinking `before` to `after`, both estimated as `Class::Code` — the same
    /// class `cmd/run.rs` and `cmd/filter.rs` record every `Measurement` with, so the floor
    /// tracks what a real run actually reports instead of a duplicated estimate path.
    fn saving_pct(before: &str, after: &str) -> (u32, u32, i64) {
        let b = crate::tokens::estimate(before, rtok_plugin_sdk::Class::Code, &SAVING_RATES);
        let a = crate::tokens::estimate(after, rtok_plugin_sdk::Class::Code, &SAVING_RATES);
        let pct = if b == 0 {
            0
        } else {
            (b as i64 - a as i64) * 100 / b as i64
        };
        (b, a, pct)
    }

    #[test]
    fn ten_families_and_aws_key_unredacted() {
        let settings = rules::Settings::builtin();
        let dir = goldens();
        let mut n = 0u32;
        let mut files: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        for p in files {
            if p.extension().and_then(|e| e.to_str()) != Some("in") {
                continue;
            }
            n += 1;
            let raw = fs::read_to_string(&p).unwrap();
            let (argv, exit, min_saving, output) = parse_in(&raw);
            let (got, _) = compress(&settings, &argv, &output, exit, "deadbeef");
            let outp = p.with_extension("out");
            let want = fs::read_to_string(&outp).unwrap();
            assert_eq!(got.trim_end(), want.trim_end(), "{}", p.display());
            // T238: guard the size of the saving, not only the bytes — a re-blessed `.out`
            // that keeps twice as much must fail even though it still matches itself.
            let floor = min_saving
                .unwrap_or_else(|| panic!("{}: missing `min_saving:` header", p.display()));
            let (before, after, pct) = saving_pct(&output, &got);
            assert!(
                after <= before,
                "{}: output ({after} tok) is larger than input ({before} tok)",
                p.display()
            );
            assert!(
                pct >= floor as i64,
                "{}: saving {pct}% below floor {floor}% (before {before} tok, after {after} tok)",
                p.display()
            );
        }
        assert!(n >= 10, "need 10 families, got {n}");
        let secret = fs::read_to_string(dir.join("cat.in")).unwrap();
        assert!(secret.contains(AWS_EXAMPLE_KEY_ID));
        let (got, _) = compress(
            &settings,
            &["cat".into(), "secrets.env".into()],
            &parse_in(&secret).3,
            0,
            "id",
        );
        // Never echo `got` here: it carries the credential-shaped fixture, and a failing assert
        // would print it to the test log.
        assert!(
            got.contains(AWS_EXAMPLE_KEY_ID),
            "compressed `cat secrets.env` output dropped or altered the AWS key line"
        );
    }

    #[test]
    fn rule_is_picked_by_the_command_not_an_argument() {
        let settings = rules::Settings::builtin();
        let argv = |s: &[&str]| s.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        assert_eq!(
            settings
                .pick(bin(&family_argv(&argv(&["/usr/bin/grep", "-rn", "x"]))))
                .match_cmd,
            "grep"
        );
        assert_eq!(
            settings
                .pick(bin(&family_argv(&argv(&[
                    "git", "commit", "-m", "fix grep"
                ]))))
                .match_cmd,
            ""
        );
        assert_eq!(
            settings
                .pick(bin(&family_argv(&argv(&["docker", "run", "node"]))))
                .match_cmd,
            ""
        );
    }

    /// T177: a single Bash string joining 2+ commands is filtered by `[script]`, not by
    /// the first command's family — `git`'s formatter would otherwise run on `cargo`'s
    /// mixed-in output too.
    #[test]
    fn newline_joined_script_uses_the_script_rule_not_the_first_command() {
        let settings = rules::Settings::builtin();
        let body = (0..40)
            .map(|i| format!("line {i} of mixed output"))
            .collect::<Vec<_>>()
            .join("\n");
        let script = format!("git status\ncat file.txt\n{body}");
        let (got, kind) = compress(&settings, std::slice::from_ref(&script), &script, 0, "id");
        assert_eq!(kind, "rule", "{got}");
        assert!(got.len() < script.len(), "{got}");
        assert!(got.contains("omitted (expand id)"), "{got}");
        // A single command with no `&&`/`;`/newline chain keeps using its own family.
        let (_, single_kind) = compress(&settings, &["git status".into()], "clean\n", 0, "id");
        assert_ne!(
            single_kind, "rule",
            "one-line git status should not hit [script]"
        );
    }

    /// T177 (revised): a chain of the SAME program (`cargo build && cargo test`, or with a
    /// leading `cd`) must keep using `cargo`'s own formatter/rule, not the generic
    /// `[script]` — only a genuinely mixed chain loses the specialised path.
    #[test]
    fn same_program_chain_keeps_its_own_family_not_script() {
        let settings = rules::Settings::builtin();
        let diag = (0..40)
            .map(|i| format!("warning: unused variable `x{i}`"))
            .collect::<Vec<_>>()
            .join("\n");
        for cmd in [
            "cargo build && cargo test".to_string(),
            "cd crates/rtok && cargo test".to_string(),
        ] {
            let (got, kind) = compress(&settings, std::slice::from_ref(&cmd), &diag, 0, "id");
            assert_ne!(kind, "rule", "{cmd}: {got}");
        }
    }

    /// T177: `unmatched` (not `raw`) once a family rule ran and shrank nothing — `raw`
    /// stays reserved for a tiny body below the trailer gate or a T176 bounded passthrough.
    #[test]
    fn a_picked_rule_that_shrinks_nothing_is_unmatched_not_raw() {
        let settings = rules::Settings::builtin();
        // `cat` has a named rule (`max_lines = 80`); 5 lines never reach the cut.
        let body = "one\ntwo\nthree\nfour\nfive";
        let (got, kind) = compress(&settings, &["cat".into(), "f".into()], body, 0, "id");
        assert_eq!(got, body);
        assert_eq!(kind, "unmatched");
    }

    /// T177: `cat -n` of several named files is not one bounded read (T176) — it must
    /// still hit `[cat]`'s head/tail cut once it is large.
    #[test]
    fn unbounded_multi_file_cat_gets_the_cat_rule() {
        let settings = rules::Settings::builtin();
        let body = (0..200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let cmd = "cat -n a.rs b.rs c.rs".to_string();
        let (got, kind) = compress(&settings, &[cmd], &body, 0, "id");
        assert_eq!(kind, "rule", "{got}");
        assert!(got.len() < body.len(), "{got}");
    }

    /// T177: a recursive grep/rg with a context flag but no `-m`/`--max-count` can return
    /// an unbounded hit list — it must still hit `[grep]`'s cut once it is large.
    #[test]
    fn large_recursive_grep_hit_list_gets_the_grep_rule() {
        let settings = rules::Settings::builtin();
        let body = (0..200)
            .map(|i| format!("src/f{i}.rs:{i}:needle found here"))
            .collect::<Vec<_>>()
            .join("\n");
        let cmd = "grep -rn -A3 needle src".to_string();
        let (got, kind) = compress(&settings, &[cmd], &body, 0, "id");
        assert_eq!(kind, "rule", "{got}");
        assert!(got.len() < body.len(), "{got}");
    }

    /// The hook wraps Bash as `rtok run -- '<cmd>'`, so the command reaches `compress` as
    /// one argv. Every golden must filter identically in that shape — this is the shape that
    /// runs in production, and the split-argv one only in this test file.
    #[test]
    fn one_quoted_argv_filters_like_a_split_argv() {
        let mut checked = 0;
        for p in fs::read_dir(goldens())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("in"))
        {
            let raw = fs::read_to_string(&p).unwrap();
            let (argv, exit, _, output) = parse_in(&raw);
            if argv.is_empty() {
                continue;
            }
            checked += 1;
            let joined = vec![argv.join(" ")];
            let settings = rules::Settings::builtin();
            assert_eq!(
                compress(&settings, &joined, &output, exit, "id"),
                compress(&settings, &argv, &output, exit, "id"),
                "{}: one quoted argv filtered differently",
                p.display()
            );
        }
        assert!(checked >= 10, "expected the golden families, saw {checked}");
    }

    #[test]
    fn family_names_the_command_not_the_whole_snippet() {
        let argv = |s: &[&str]| s.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        assert_eq!(family(&argv(&["git status | head"])), "git");
        assert_eq!(family(&argv(&["/usr/bin/git", "status"])), "git");
        assert_eq!(family(&argv(&[])), "other");
    }

    #[test]
    fn wrappers_use_the_inner_formatter_when_the_command_is_visible() {
        let settings = rules::Settings::builtin();
        let body = "test a::bad ... FAILED\ntest result: FAILED. 1 failed\n";
        let run = |cmd: &str, output: &str| compress(&settings, &[cmd.into()], output, 0, "id");
        for cmd in [
            "mise exec -- cargo test",
            "mise x node@20 -- cargo test",
            "mise exec -c 'cargo test'",
            "just --command cargo test",
            "just -c cargo test",
            "cd /tmp && mise exec -- cargo test",
        ] {
            let (got, kind) = run(cmd, body);
            assert_eq!(kind, "formatter", "{cmd}: {got}");
            assert!(got.contains("FAILED"), "{cmd}: {got}");
        }
        let echoed = format!("[test] $ cargo test\n{body}");
        let (got, kind) = run("mise run test", &echoed);
        assert_eq!(kind, "formatter", "{got}");
        let echoed = format!("mise exec -- cargo test\n{body}");
        let (got, kind) = run("just test", &echoed);
        assert_eq!(kind, "formatter", "{got}");
        // Quiet runs print no command line, so the wrapper stem stays.
        let (got, kind) = run("mise run test", body);
        assert_ne!(kind, "formatter", "{got}");
        let (got, kind) = run("just test", body);
        assert_ne!(kind, "formatter", "{got}");
    }

    #[test]
    fn two_echoed_programs_use_the_script_rule() {
        let settings = rules::Settings::builtin();
        let body = (0..40)
            .map(|i| format!("line {i} of mixed output"))
            .collect::<Vec<_>>()
            .join("\n");
        let output = format!("mise exec -- git status\n{body}\nmise exec -- cargo test\n{body}");
        let (got, kind) = compress(&settings, &["just check".into()], &output, 0, "id");
        assert_eq!(
            family_of(&settings, &["just check".into()], &output),
            "script"
        );
        assert_eq!(kind, "rule", "{got}");
        assert!(got.contains("omitted"), "{got}");
    }

    #[test]
    fn family_names_windows_argv() {
        let argv = |s: &[&str]| s.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        assert_eq!(family(&argv(&[r"C:\tools\git.exe", "status"])), "git");
        assert_eq!(family(&argv(&["cargo.exe", "test"])), "cargo");
    }

    #[test]
    fn table_formatter_beats_default_rule_and_keeps_every_object() {
        let settings = rules::Settings::builtin();
        let dir = goldens();
        for (file, argv0, prefix) in [
            ("docker_ps.in", ["docker", "ps"], "web-"),
            ("podman_ps.in", ["podman", "ps"], "web-"),
            ("kubectl_get.in", ["kubectl", "get"], "web-deploy-"),
            ("ps_aux.in", ["ps", "aux"], "worker-"),
        ] {
            let raw = fs::read_to_string(dir.join(file)).unwrap();
            let (_, exit, _, output) = parse_in(&raw);
            let argv = argv0.iter().map(|w| w.to_string()).collect::<Vec<_>>();
            let (got, kind) = compress(&settings, &argv, &output, exit, "deadbeef");
            assert_eq!(kind, "formatter", "{file}");
            let rule_out = rules::apply(
                &settings,
                &output,
                exit,
                &rules::Rule::default(),
                "deadbeef",
            );
            assert!(
                got.len() < rule_out.len(),
                "{file}: formatter {} vs rule {}",
                got.len(),
                rule_out.len()
            );
            for i in 0..40 {
                let name = match prefix {
                    "web-" => format!("web-{i:02}"),
                    "web-deploy-" => format!("web-deploy-{i:02}-abcd"),
                    _ => format!("worker-{i}"),
                };
                assert!(got.contains(&name), "{file} dropped {name}");
            }
        }
    }

    #[test]
    fn table_formatter_stands_down_on_unrecognized() {
        let settings = rules::Settings::builtin();
        let argv = |s: &[&str]| s.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        let (got, kind) = compress(
            &settings,
            &argv(&["docker", "ps"]),
            "Cannot connect to the Docker daemon\n",
            0,
            "deadbeef",
        );
        assert_ne!(kind, "formatter", "{got}");
        assert!(got.contains("Cannot connect"), "{got}");
        let (got, kind) = compress(
            &settings,
            &argv(&["podman", "ps"]),
            "Cannot connect to the Podman daemon\n",
            0,
            "deadbeef",
        );
        assert_ne!(kind, "formatter", "{got}");
        assert!(got.contains("Cannot connect"), "{got}");
        let (got, kind) = compress(
            &settings,
            &argv(&["kubectl", "get"]),
            "error: the server doesn't have a resource type \"pods\"\n",
            1,
            "deadbeef",
        );
        assert_ne!(kind, "formatter", "{got}");
        assert!(got.contains("doesn't have a resource type"), "{got}");
        let (got, kind) = compress(
            &settings,
            &argv(&["ps", "aux"]),
            "ps: invalid option -- z\n",
            1,
            "deadbeef",
        );
        assert_ne!(kind, "formatter", "{got}");
        assert!(got.contains("invalid option"), "{got}");
        let (got, kind) = compress(
            &settings,
            &argv(&["df", "-h"]),
            "df: /nope: No such file or directory\n",
            1,
            "deadbeef",
        );
        assert_ne!(kind, "formatter", "{got}");
        let (got, kind) = compress(
            &settings,
            &argv(&["sqlite3", "db"]),
            "1|alpha\n2|beta\n",
            0,
            "deadbeef",
        );
        assert_ne!(kind, "formatter", "{got}");
    }

    #[test]
    fn df_sqlite_and_awk_beat_the_default_rule() {
        let settings = rules::Settings::builtin();
        let dir = goldens();
        for file in ["df.in", "sqlite3.in", "awk.in"] {
            let raw = fs::read_to_string(dir.join(file)).unwrap();
            let (argv, exit, _, output) = parse_in(&raw);
            let (got, _) = compress(&settings, &argv, &output, exit, "deadbeef");
            let rule_out = rules::apply(
                &settings,
                &output,
                exit,
                &rules::Rule::default(),
                "deadbeef",
            );
            assert!(
                got.len() < rule_out.len(),
                "{file}: {} B vs default {} B\n{got}",
                got.len(),
                rule_out.len()
            );
        }
    }

    #[test]
    fn group_goldens_beat_the_same_rule_without_group() {
        use super::rules::Group;
        let settings = rules::Settings::builtin();
        let dir = goldens();
        for file in [
            "ls.in",
            "find.in",
            "rg.in",
            "tsc_dup.in",
            "eslint_dup.in",
            "cargo_check.in",
            "dotnet_dup.in",
            "ruff_check.in",
            "mypy_run.in",
            "rubocop_run.in",
            "golangci_lint_run.in",
        ] {
            let raw = fs::read_to_string(dir.join(file)).unwrap();
            let (argv, exit, _, output) = parse_in(&raw);
            let (got, _) = compress(&settings, &argv, &output, exit, "deadbeef");
            let mut off = settings.pick(bin(&family_argv(&argv)));
            off.group = Group::Off;
            let ungrouped = rules::apply(&settings, &output, exit, &off, "deadbeef");
            assert!(
                got.len() < ungrouped.len(),
                "{file}: grouped {} B vs ungrouped {} B\n{got}",
                got.len(),
                ungrouped.len()
            );
        }
    }

    #[test]
    fn ps_aux_skips_a_bad_line_instead_of_dropping_every_row() {
        let output = "USER PID %CPU COMMAND\n\
            root 1 0.0 /sbin/init\n\
            USER PID %CPU COMMAND\n\
            root 42 0.1 /usr/bin/worker-7\n";
        let got = ps_aux(output).expect("good rows survive a repeated header");
        assert!(got.contains("1 init"), "{got}");
        assert!(got.contains("42 worker-7"), "{got}");
        assert!(!got.contains("PID"), "{got}");
    }

    #[test]
    fn git_log_lists_every_commit_with_authors_when_mixed() {
        let raw = "commit 1111111111111111111111111111111111111111\nAuthor: Ann <a@x>\nDate:   Wed Oct 14 10:00:00 2026 +0300\n\n    first\n\n    body line\n\ncommit 2222222222222222222222222222222222222222\nAuthor: Bob <b@x>\nDate:   Tue Oct 13 10:00:00 2025 +0300\n\n    second\n";
        let out = git_log(raw).unwrap();
        assert_eq!(
            out,
            "[2 commits] Ann unless named\n1111111 Oct 14 2026 first\n2222222 Oct 13 2025 second — Bob"
        );
    }

    #[test]
    fn git_log_keeps_stat_lines_under_their_commit() {
        let raw = "commit 1111111111111111111111111111111111111111\nAuthor: Ann <a@x>\nDate:   Wed Oct 14 10:00:00 2026 +0300\n\n    first\n\n src/a.rs | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n";
        let out = git_log(raw).unwrap();
        assert!(out.contains("1111111 Oct 14 first"), "{out}");
        assert!(out.contains("  src/a.rs | 2 +-"), "{out}");
        assert!(out.contains("  1 file changed"), "{out}");
    }

    /// T682: the first line counts the commits and, when `--stat` printed totals, sums them.
    #[test]
    fn git_log_header_counts_commits_and_stat_totals() {
        let c = |sha: char, stat: &str| {
            format!(
                "commit {}\nAuthor: Ann <a@x>\nDate:   Wed Oct 14 10:00:00 2026 +0300\n\n    s{sha}\n\n src/a.rs | 3 ++-\n{stat}\n\n",
                sha.to_string().repeat(40)
            )
        };
        let raw = format!(
            "{}{}",
            c('1', " 1 file changed, 2 insertions(+), 1 deletion(-)"),
            c('2', " 1 file changed, 3 insertions(+)")
        );
        let out = git_log(&raw).unwrap();
        assert_eq!(out.lines().next(), Some("[2 commits, +5/-1] Ann · 2026"));
        // Without stat totals there is nothing to sum, so only the count is claimed.
        let plain = "commit 1111111111111111111111111111111111111111\nAuthor: Ann <a@x>\nDate:   Wed Oct 14 10:00:00 2026 +0300\n\n    one\n";
        assert_eq!(
            git_log(plain).unwrap().lines().next(),
            Some("[1 commit] Ann · 2026")
        );
    }

    /// T682: one author is named once in the header, never on a commit line.
    #[test]
    fn git_log_drops_the_author_when_all_are_equal() {
        let one = |sha: char| {
            format!(
                "commit {}\nAuthor: Ann <a@x>\nDate:   Wed Oct 14 10:00:00 2026 +0300\n\n    s{sha}\n\n",
                sha.to_string().repeat(40)
            )
        };
        let out = git_log(&format!("{}{}", one('1'), one('2'))).unwrap();
        assert_eq!(out.matches("Ann").count(), 1, "{out}");
        assert!(!out.contains(" — "), "{out}");
    }

    #[test]
    fn git_log_other_shapes_keep_the_head_cut() {
        assert!(git_log("1111111 first\n2222222 second\n").is_none());
        let patch = "commit 1111111111111111111111111111111111111111\nAuthor: Ann <a@x>\nDate:   Wed Oct 14 10:00:00 2026 +0300\n\n    first\n\ndiff --git a/x b/x\n";
        assert!(git_log(patch).is_none());
    }

    #[test]
    fn git_log_caps_the_listing() {
        let one = "commit 1111111111111111111111111111111111111111\nAuthor: Ann <a@x>\nDate:   Wed Oct 14 10:00:00 2026 +0300\n\n    c\n\n";
        let out = git_log(&one.repeat(GIT_LOG_MAX_COMMITS + 5)).unwrap();
        assert!(out.ends_with("… +5 more"), "{out}");
        assert_eq!(out.lines().count(), GIT_LOG_MAX_COMMITS + 2);
    }

    /// T296: every `(bin, sub)` arm dispatched by `format()` above must have a golden `.in`
    /// whose argv reaches that arm and for which `format(...)` actually returns `Some`. This
    /// list must mirror the match arms in `format()` — add a row here whenever a new arm is
    /// added there. `""` stands for a `_` (wildcard) sub-command.
    #[test]
    fn every_formatter_arm_has_a_golden() {
        const ARMS: &[(&str, &str)] = &[
            ("cargo", "test"),
            ("cargo", "build"),
            ("cargo", "clippy"),
            ("git", "status"),
            ("git", "diff"),
            ("git", "show"),
            ("git", "log"),
            ("gh", ""),
            ("pytest", ""),
            ("python", ""),
            ("python3", ""),
            ("py", ""),
            ("jest", ""),
            ("vitest", ""),
            ("go", "test"),
            ("tree", ""),
            ("docker", "ps"),
            ("podman", "ps"),
            ("kubectl", "get"),
            ("ps", "aux"),
            ("df", ""),
            ("sqlite3", ""),
            ("wc", ""),
            ("mise", ""),
            ("for", ""),
            ("while", ""),
            ("until", ""),
        ];
        // A stem added to `FORMATTER_STEMS` without a row here fails too.
        for stem in FORMATTER_STEMS {
            assert!(
                ARMS.iter().any(|(b, _)| b == stem),
                "{stem} missing from ARMS"
            );
        }
        let parsed: Vec<(Vec<String>, String)> = fs::read_dir(goldens())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("in"))
            .map(|p| fs::read_to_string(&p).unwrap())
            .map(|raw| {
                let (argv, _, _, output) = parse_in(&raw);
                (argv, output)
            })
            .collect();
        let covered = |b: &str, s: &str| {
            parsed.iter().any(|(argv, output)| {
                if argv.is_empty() {
                    return false;
                }
                let split = family_argv(argv);
                if bin(&split) != b || (!s.is_empty() && sub(&split) != s) {
                    return false;
                }
                format(&split, output).is_some()
            })
        };
        let missing: Vec<String> = ARMS
            .iter()
            .filter(|(b, s)| !covered(b, s))
            .map(|(b, s)| format!("({b:?}, {s:?})"))
            .collect();
        assert!(
            missing.is_empty(),
            "formatter arms without a golden: {missing:?}"
        );
    }

    /// T240: every family a builtin rule names in `rules/default.toml` must have a golden
    /// `.in` whose argv actually picks that rule via `family_argv`/`bin` — the same path
    /// `compress` uses in production. `[script]` is reached by mixed-chain detection, not
    /// by an argv[0] match, so it is exempt (covered by the chain tests above instead).
    #[test]
    fn every_builtin_rule_family_has_a_golden() {
        let in_files: Vec<String> = fs::read_dir(goldens())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("in"))
            .map(|p| fs::read_to_string(&p).unwrap())
            .collect();
        let covered = |family: &str| {
            in_files.iter().any(|raw| {
                let (argv, ..) = parse_in(raw);
                !argv.is_empty() && bin(&family_argv(&argv)) == family
            })
        };
        let missing: Vec<String> = rules::defaults()
            .into_iter()
            .map(|r| r.match_cmd)
            .filter(|family| family != "script" && !covered(family))
            .collect();
        assert!(missing.is_empty(), "families without a golden: {missing:?}");
    }

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    /// T386: the inner command is used only when it is on the argv line or on the
    /// wrapper's own echo. A flag we do not know, or a quiet run, stays on the wrapper.
    #[test]
    fn wrappers_use_the_inner_command_when_it_is_visible() {
        let settings = rules::Settings::builtin();
        let failed = "test a::bad ... FAILED\ntest result: FAILED. 1 failed\n";
        assert_eq!(
            family_of(&settings, &words("mise exec -- cargo test"), failed),
            "cargo"
        );
        assert_eq!(
            family_of(
                &settings,
                &words("mise x -C /tmp -- git status"),
                "On branch\n"
            ),
            "git"
        );
        assert_eq!(
            family_of(&settings, &words("mise exec node@20 -- node -v"), "v20\n"),
            "node"
        );
        assert_eq!(
            family_of(&settings, &words("just -c cargo test"), failed),
            "cargo"
        );
        assert_eq!(family_of(&settings, &words("mise exec -c"), failed), "mise");
        let run = format!("[test] $ cargo test\n{failed}");
        assert_eq!(family_of(&settings, &words("mise run test"), &run), "cargo");
        assert_eq!(
            family_of(&settings, &words("mise run test"), failed),
            "mise"
        );
        assert_eq!(
            family_of(&settings, &words("mise exec cargo test"), failed),
            "mise"
        );
        let just = "git status\nOn branch main\n";
        assert_eq!(family_of(&settings, &words("just status"), just), "git");
        let colored = "\u{1b}[32mgit status\u{1b}[0m\nOn branch main\n";
        assert_eq!(family_of(&settings, &words("just status"), colored), "git");
        assert_eq!(
            family_of(
                &settings,
                &words("just check"),
                "Checking foo v0.1.0\nFinished dev\n"
            ),
            "just"
        );
        let mixed = "mise exec -- cargo test\nmise exec -- git status\n";
        assert_eq!(family_of(&settings, &words("just check"), mixed), "script");
        let nested = format!("[test] $ cargo test\n{failed}");
        assert_eq!(
            family_of(&settings, &words("mise exec -- just test"), &nested),
            "cargo"
        );
    }

    #[test]
    fn family_names_peeled_wrappers() {
        let settings = rules::Settings::builtin();
        let body = "ok\n";
        let family = |cmd: &str| family_of(&settings, &words(cmd), body);
        assert_eq!(family("uv run cargo test"), "cargo");
        assert_eq!(family("FOO=1 cargo test"), "cargo");
        assert_eq!(family(".venv/bin/pytest -q"), "pytest");
        assert_eq!(family("uv run python -m pytest -q"), "pytest");
        assert_eq!(family("mise exec -- cargo test"), "cargo");
        assert_eq!(
            family_of(&settings, &["git commit -m \"a|b\"".into()], body),
            "git"
        );
        assert_eq!(family("mise exec --"), "mise");
    }
}

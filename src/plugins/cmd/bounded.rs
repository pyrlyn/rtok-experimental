// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T176: a command the agent already bounded — `sed -n 1,80p`, `head`/`tail -n`,
//! `grep -A/-B/-C/-m`, `cat -n` of named files — printed what was asked for. Cutting it
//! again only buys an `expand` round trip. The lexer reads quotes, backslash escapes and the
//! `|`, `|&`, `&&`, `||`, `;` separators and nothing else of the shell grammar (`cmd/AGENTS.md`).

/// Past this the host truncates a Bash result on its own (Claude Code: 30 000 chars),
/// and a cut that names the archive beats one that does not.
pub const MAX_BYTES: usize = 30_000;

/// Unicode scalars the host keeps of a Bash result (Claude Code: 30 000 chars).
/// A `Measurement` estimates this prefix: bytes past it never reach the model.
pub const HOST_VISIBLE_CHARS: usize = 30_000;

/// The prefix the host receives: at most [`HOST_VISIBLE_CHARS`] scalars, on a char boundary.
pub fn host_visible_prefix(text: &str) -> &str {
    match text.char_indices().nth(HOST_VISIBLE_CHARS) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

/// Whether every command in `snippet` ends in a bounding stage. A leading `cd`/`export`
/// prints nothing and does not unbound the rest; at least one stage must bound.
/// T931: a literal `echo`/`printf` label between windows (`sed -n 1,80p a; echo ---;
/// sed -n 1,40p b`) prints a few bytes and does not unbound the chain either.
pub fn is_bounded(snippet: &str) -> bool {
    let mut any = false;
    for pipeline in lex(snippet) {
        let Some(last) = pipeline.last() else {
            continue;
        };
        let silent = pipeline.len() == 1
            && (matches!(last.first().map(String::as_str), Some("cd" | "export"))
                || is_label(last));
        if silent {
            continue;
        }
        if !bounds(last) {
            return false;
        }
        any = true;
    }
    any
}

/// `echo`/`printf` of literal words. The lexer keeps `$(…)`, backticks and `$var` inside the
/// word, so any expansion (whose output could be unbounded) disqualifies the label.
fn is_label(stage: &[String]) -> bool {
    matches!(
        stage.first().map(|c| super::formatters::cmd_stem(c)),
        Some("echo" | "printf")
    ) && !stage[1..].iter().any(|w| w.contains(['$', '`']))
}

/// T177: true only when `snippet` chains 2+ DISTINCT programs — `a && b`, `a; b`,
/// `a\nb` and `a | b` all chain, but `cargo build && cargo test` and `cd x && cargo test`
/// stay one program end to end. Reused by `formatters::compress` to route a genuinely mixed
/// Bash string to the `[script]` rule instead of misreading it as (or losing) one
/// family's specialised formatter/rule. A leading `cd`/`export` stage is skipped, same
/// as `is_bounded`.
pub(crate) fn mixed_chain(snippet: &str) -> bool {
    let mut programs = std::collections::HashSet::new();
    for pipeline in lex(snippet) {
        let Some(last) = pipeline.last() else {
            continue;
        };
        let silent = pipeline.len() == 1
            && matches!(last.first().map(String::as_str), Some("cd" | "export"));
        if silent {
            continue;
        }
        // Every stage: `git log | grep foo` is two programs. Counting only the
        // first stage left the pipe on `git`'s formatter.
        for stage in pipeline {
            // T386: `mise exec -- cargo && mise exec -- git` is two programs. Counting
            // argv[0] left both stages on `mise` and kept cargo's formatter off the mix.
            let vis = super::formatters::visible_argv(&stage);
            if let Some(program) = vis.first() {
                programs.insert(super::formatters::cmd_stem(program).to_string());
            }
        }
    }
    programs.len() >= 2
}

/// Every stage of every pipeline in `snippet`, as words with quotes removed.
pub(crate) fn stages(snippet: &str) -> Vec<Vec<String>> {
    lex(snippet).into_iter().flatten().collect()
}

/// Pipelines of stages of words, quotes removed.
fn lex(s: &str) -> Vec<Vec<Vec<String>>> {
    let mut lists = vec![vec![Vec::new()]];
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if q == '"' && c == '\\' {
                // Inside double quotes only these four are escapable (and `\`-newline is
                // a continuation); any other `\` stays literal. Single quotes have no
                // escapes, so they never get here.
                match chars.peek().copied() {
                    Some(n @ ('"' | '\\' | '$' | '`')) => {
                        chars.next();
                        word.get_or_insert_default().push(n);
                    }
                    Some('\n') => {
                        chars.next();
                    }
                    _ => word.get_or_insert_default().push(c),
                }
            } else {
                word.get_or_insert_default().push(c);
            }
            continue;
        }
        let sep = match c {
            // A backslash-newline outside quotes is shell line continuation, not a
            // separator — `cargo test \` + newline + `  --all` is one pipeline.
            '\\' if chars.peek() == Some(&'\n') => {
                chars.next();
                None
            }
            // Any other escaped char is a plain word char: `it\'s` must not open a quote
            // that swallows a later `&& cd sub`.
            '\\' => {
                word.get_or_insert_default()
                    .push(chars.next().unwrap_or('\\'));
                continue;
            }
            '\'' | '"' => {
                quote = Some(c);
                word.get_or_insert_default();
                continue;
            }
            '|' if chars.peek() == Some(&'|') => Some(true),
            '&' if chars.peek() == Some(&'&') => Some(true),
            ';' | '\n' => Some(true),
            '|' => Some(false),
            c if c.is_whitespace() => None,
            c => {
                word.get_or_insert_default().push(c);
                continue;
            }
        };
        let stages = lists.last_mut().expect("never empty");
        if let Some(w) = word.take() {
            stages.last_mut().expect("never empty").push(w);
        }
        if let Some(new_list) = sep {
            if matches!(chars.peek(), Some('|' | '&')) {
                chars.next();
            }
            if new_list {
                lists.push(vec![Vec::new()]);
            } else {
                stages.push(Vec::new());
            }
        }
    }
    if let Some(w) = word {
        lists
            .last_mut()
            .and_then(|s| s.last_mut())
            .expect("never empty")
            .push(w);
    }
    lists
        .into_iter()
        .filter(|p| p.iter().any(|s| !s.is_empty()))
        .collect()
}

fn bounds(stage: &[String]) -> bool {
    let Some((cmd, args)) = stage.split_first() else {
        return false;
    };
    match super::formatters::cmd_stem(cmd) {
        "head" => counts(args, |n| n.bytes().all(|b| b.is_ascii_digit())),
        "tail" => {
            !args
                .iter()
                .any(|a| matches!(a.as_str(), "-f" | "-F" | "--follow"))
                && counts(args, |n| !n.starts_with('+'))
        }
        "sed" => sed_range(args),
        // T177: `-A/-B/-C` bound the context *per match*, not the number of matches — a
        // recursive `grep` can still return an unbounded hit list, so it also needs an
        // explicit `-m`/`--max-count`. `rg` is recursive by default and has no
        // `-r`/`-R` recursive flag (`-r` is `--replace`), so this extra check is
        // `grep`-only (`cmd/AGENTS.md` notes `rg` is not covered).
        stem @ ("grep" | "rg") => {
            if stem == "grep" {
                // `-r`/`-R` bundle with other short flags (`-rn`), so this checks
                // membership, not an exact match; `--recursive` is matched exactly.
                let recursive = args.iter().any(|a| {
                    a == "--recursive"
                        || (a.starts_with('-') && !a.starts_with("--") && a.contains(['r', 'R']))
                });
                let max_count = args.iter().any(|a| is_max_count(a));
                if recursive && !max_count {
                    return false;
                }
            }
            has_bounding_context(args)
        }
        // T177: `-n` only numbers lines, it does not bound how many files print — require
        // exactly one named file, or a `cat a.rs b.rs c.rs` dump passes through whole.
        "cat" => {
            args.iter().any(|a| a == "-n")
                && args.iter().filter(|a| !a.starts_with('-')).count() == 1
        }
        _ => false,
    }
}

/// `grep`/`rg` context or max-count flags (`-A/-B/-C/-m N`, `--context=N`, …) that bound
/// how much a match prints. Shared by both families — `rg`'s check has no recursive gate.
fn has_bounding_context(args: &[String]) -> bool {
    args.iter().any(|a| {
        let short = a.strip_prefix('-').filter(|s| !s.starts_with('-'));
        short.is_some_and(|s| {
            s.trim_end_matches(|c: char| c.is_ascii_digit())
                .ends_with(['A', 'B', 'C', 'm'])
        }) || [
            "--context",
            "--after-context",
            "--before-context",
            "--max-count",
        ]
        .iter()
        .any(|l| a.split('=').next() == Some(l))
    })
}

fn is_max_count(a: &str) -> bool {
    let short = a.strip_prefix('-').filter(|s| !s.starts_with('-'));
    short.is_some_and(|s| {
        s.trim_end_matches(|c: char| c.is_ascii_digit())
            .ends_with('m')
    }) || a.split('=').next() == Some("--max-count")
}

/// `head`/`tail` counts (`-n N`, `-nN`, `--lines=N`, `-c N`, `-N`) all pass `ok`.
/// No count is the default ten lines — bounded too.
fn counts(args: &[String], ok: impl Fn(&str) -> bool) -> bool {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let val = match a.as_str() {
            "-n" | "-c" | "--lines" | "--bytes" => it.next().map(String::as_str),
            _ => a
                .strip_prefix("--lines=")
                .or_else(|| a.strip_prefix("--bytes="))
                .or_else(|| a.strip_prefix("-n"))
                .or_else(|| a.strip_prefix("-c"))
                .or_else(|| {
                    a.strip_prefix('-')
                        .filter(|n| n.starts_with(|c: char| c.is_ascii_digit()))
                }),
        };
        if let Some(v) = val
            && (v.is_empty() || !ok(v))
        {
            return false;
        }
    }
    true
}

/// `sed -n` whose every script is numeric `a[,b]p` addresses, `;`-joined.
fn sed_range(args: &[String]) -> bool {
    let quiet = args
        .iter()
        .any(|a| matches!(a.as_str(), "-n" | "--quiet" | "--silent"));
    let mut scripts = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-e" | "--expression" => scripts.extend(it.next()),
            s if s.starts_with('-') => {}
            _ if scripts.is_empty() => scripts.push(a),
            _ => {}
        }
    }
    let range = |s: &str| {
        let Some(addr) = s.trim().strip_suffix('p') else {
            return false;
        };
        let mut ends = addr.split(',');
        let num = |e: Option<&str>| {
            e.is_some_and(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit()))
        };
        num(ends.next()) && ends.next().is_none_or(|e| num(Some(e))) && ends.next().is_none()
    };
    quiet && !scripts.is_empty() && scripts.iter().all(|s| s.split(';').all(range))
}

#[cfg(test)]
mod tests {
    use super::{HOST_VISIBLE_CHARS, host_visible_prefix, is_bounded};
    use rstest::rstest;

    #[test]
    fn host_visible_prefix_stops_at_thirty_thousand_scalars() {
        let over: String = "x".repeat(HOST_VISIBLE_CHARS + 1);
        let prefix = host_visible_prefix(&over);
        assert_eq!(prefix.chars().count(), HOST_VISIBLE_CHARS);
        assert!(prefix.bytes().all(|b| b == b'x'));

        let exact: String = "é".repeat(HOST_VISIBLE_CHARS);
        let whole = host_visible_prefix(&exact);
        assert_eq!(whole, exact);
        assert!(std::str::from_utf8(whole.as_bytes()).is_ok());
    }

    #[rstest]
    #[case("sed -n '1,620p' src/hooks/types.rs")]
    #[case("sed -n 10p a; sed -n -e 1,5p -e 9p b")]
    #[case("cargo nextest run --workspace 2>&1 | tail -300")]
    #[case("cargo nextest run |& tail -n 50")]
    #[case("git log --oneline | head")]
    #[case("head -n 40 a.rs b.rs")]
    #[case("grep -rn -A3 -m 5 'a|b' src")]
    #[case("rg -C 2 needle")]
    #[case("grep -m 5 x f")]
    #[case("grep -A3 x f")]
    #[case("cd /repo && sed -n '1,80p' Cargo.toml")]
    #[case("cat -n src/main.rs")]
    // T931: windows joined by `;`/`&&`/newline, a pipe into `sed`, and literal label lines.
    #[case("sed -n 1,80p a; sed -n 1,40p b")]
    #[case("sed -n 1,80p a && sed -n 1,40p b")]
    #[case("git show x | sed -n 1,100p")]
    #[case("sed -n 1,80p a; echo ---; sed -n 1,40p b")]
    #[case("sed -n 1,80p a\necho \"=== b ===\"\nsed -n 1,40p b")]
    #[case("printf '\\n--\\n'; sed -n 1,9p a")]
    // T177: `rg` is recursive by default and `-r` is `--replace`, not a recursive flag —
    // only `grep`'s recursive-without-`-m` gate applies; the context flag still bounds it.
    #[case("rg -r -C 2 needle src")]
    fn bounded_forms(#[case] cmd: &str) {
        assert!(is_bounded(cmd), "{cmd}");
    }

    #[rstest]
    #[case("cargo nextest run")]
    #[case("sed -n '/fn main/,$p' a.rs")]
    #[case("sed 's/a/b/' a.rs")]
    #[case("tail -n +5 log")]
    #[case("tail -f log")]
    #[case("head -n -5 log")]
    #[case("grep -rn needle src")]
    #[case("sed -n 1,5p a | grep x")]
    #[case("make && tail -5 build.log")]
    #[case("cat a.rs")]
    #[case("cd /repo")]
    #[case("echo 'x | head'")]
    // T931: a label never bounds on its own, and an expansion is not a literal label.
    #[case("echo ---")]
    #[case("echo ---; cargo test")]
    #[case("sed -n 1,5p a; cargo test")]
    #[case("sed -n 1,5p a; echo $(cat big)")]
    #[case("sed -n 1,5p a; echo `cat big`")]
    #[case("sed -n 1,5p a; echo $BIG")]
    // T177: `-A/-B/-C` bound context per match, not match count; a recursive `grep`
    // without an explicit `-m`/`--max-count` can still return an unbounded hit list.
    // (`rg` has no such gate — see `bounded_forms` above.)
    #[case("grep -rn -A3 'a|b' src")]
    // T177: `-n` only numbers lines; more than one named file is still an unbounded dump.
    #[case("cat -n a.rs b.rs")]
    fn unbounded_forms(#[case] cmd: &str) {
        assert!(!is_bounded(cmd), "{cmd}");
    }

    #[rstest]
    #[case("git status")]
    // Same program end to end, chained or piped — not a mix.
    #[case("cargo build && cargo test")]
    #[case("cd x && cargo test")]
    #[case("cargo test; cargo clippy")]
    // Backslash-newline continuation is not a chain separator.
    #[case("cargo test \\\n  --all")]
    #[case("mise exec -- cargo build && mise exec -- cargo test")]
    fn same_program_forms(#[case] cmd: &str) {
        assert!(!super::mixed_chain(cmd), "{cmd}");
    }

    #[rstest]
    #[case("cargo test && cargo clippy && git status")]
    #[case("mise exec -- cargo test && mise exec -- git status")]
    #[case("git log | grep foo")]
    #[case("echo one; cat two")]
    #[case("git status\ncat file.txt\nls")]
    fn mixed_program_forms(#[case] cmd: &str) {
        assert!(super::mixed_chain(cmd), "{cmd}");
    }

    #[test]
    fn backslash_newline_continuation_does_not_split_a_pipeline() {
        assert_eq!(super::lex("cargo test \\\n  --all").len(), 1);
    }

    /// T444: `\` outside quotes makes the next char literal; inside double quotes it
    /// escapes only `"`, `\`, `$`, `` ` ``; single quotes have no escapes.
    #[rstest]
    #[case(r"echo it\'s && cd sub", r#"[["echo", "it's"], ["cd", "sub"]]"#)]
    #[case(r#"echo "a\" b" && x"#, r#"[["echo", "a\" b"], ["x"]]"#)]
    #[case(r#"echo "a\\" && x"#, r#"[["echo", "a\\"], ["x"]]"#)]
    #[case(r#"echo "a\n" && x"#, r#"[["echo", "a\\n"], ["x"]]"#)]
    #[case(r"echo 'a\' && x", r#"[["echo", "a\\"], ["x"]]"#)]
    #[case(r"echo a\;b c", r#"[["echo", "a;b", "c"]]"#)]
    #[case("cargo test \\\n --all", r#"[["cargo", "test", "--all"]]"#)]
    fn lexer_backslash_escapes(#[case] cmd: &str, #[case] want: &str) {
        assert_eq!(format!("{:?}", super::stages(cmd)), want, "{cmd}");
    }
}

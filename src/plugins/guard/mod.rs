// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Deny duplicate Read/Bash when a prior archive id exists (plan T2.6).

use rtok_plugin_sdk::{
    Ctx, DashboardPage, Injection, Manifest, Measurement, Plugin, PostToolUse, PreCompact,
    PreToolDecision, PreToolUse, SessionStart, Surface,
};
use serde_json::Value;

use crate::names::strip_wrap;

#[cfg(feature = "graph")]
pub(crate) mod grep_symbol;
mod sed;
mod skill;

/// T201: above this, `post_tool`'s sha256 + synchronous disk write (inside [`Ctx::put_archive`])
/// would blow the hook's ≤ 10 ms budget on a large tool result — the `tests/latency.rs` 5 MB
/// PostToolUse gate. Skipping the archive for a body this size just leaves the key uncached,
/// so `pre_tool` can never deny with a pointer to an archive nothing wrote (lossless by
/// omission, never by truncation).
const ARCHIVE_CAP_BYTES: usize = 256 * 1024;

pub struct GuardPlugin;

impl Plugin for GuardPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "guard",
            surfaces: &[Surface::Hook, Surface::Cli],
            default_on: true,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "Guard",
            "Deny duplicate reads and commands inside a sliding window.",
            true,
        )
    }

    fn pre_tool(&self, ev: &PreToolUse, cx: &Ctx) -> Option<PreToolDecision> {
        if ev.tool_name == "Skill" {
            return skill::digest(ev, cx);
        }
        // T369: answer a symbol-shaped Grep before `native_redirect` would only point elsewhere.
        #[cfg(feature = "graph")]
        if ev.tool_name == "Grep"
            && let Some(d) = grep_symbol::answer(ev, cx)
        {
            return Some(d);
        }
        if let Some(d) = native_redirect(ev.tool_name, cx) {
            return Some(d);
        }
        let key = cache_key(ev.tool_name, ev.tool_input, cx.agent_id(), cx.cwd())?;
        let (id, ts) = cx.get_read_cache(&key).ok().flatten()?;
        let id = id?;
        let n = cx.calls_since(ts).unwrap_or(0);
        let window = cx
            .plugin_config::<crate::config::Guard>("guard")
            .window_turns;
        if n > i64::from(window) {
            return None;
        }
        let shown = crate::names::short_id(&id);
        let reason = format!("duplicate; rtok expand {shown}");
        // AGENTS: denial Measurement carries the avoided result size (archive bytes).
        // Never deny without a retrievable original (lossless + fail open). Metadata only:
        // reading a possibly-megabyte body would break the ≤ 10 ms hook budget (T55.16).
        let avoided = cx.archive_size(&id).ok().flatten()?;
        if avoided == 0 {
            return None;
        }
        // The same bytes/4 heuristic `record_context_path` and the semantic-cache
        // measurement use — the body is not loaded to estimate it.
        let est = (avoided / 4).max(1) as u32;
        let _ = cx.record(&Measurement {
            plugin: "guard",
            kind: "guard",
            before_bytes: avoided,
            after_bytes: 0,
            est_before: est,
            est_after: 0,
            ref_id: Some(id.clone()),
            call_id: None,
        });
        Some(PreToolDecision::Deny { reason })
    }

    // T392: loaded bodies are gone after a compaction. `SessionStart` with `compact` covers hosts
    // that send no `PreCompact`.
    fn pre_compact(&self, _ev: &PreCompact, cx: &Ctx) {
        skill::forget_loads(cx);
    }

    fn session_start(&self, ev: &SessionStart, cx: &Ctx) -> Option<Injection> {
        if ev.source == "compact" {
            skill::forget_loads(cx);
        }
        None
    }

    fn post_tool(&self, ev: &PostToolUse, cx: &Ctx) -> Option<String> {
        if ev.tool_name == "Skill" {
            return skill::note_load(ev, cx);
        }
        match cache_key(ev.tool_name, ev.tool_input, cx.agent_id(), cx.cwd()) {
            Some(key) => {
                // A kept `cd` hop moves the shell for every later call: no earlier Bash
                // answer was taken from the directory the next command will run in.
                if ev.tool_name == "Bash"
                    && ev
                        .tool_input
                        .get("command")
                        .and_then(Value::as_str)
                        .is_some_and(has_cd_hop)
                {
                    let _ = cx.clear_read_cache("bash");
                }
                let body = payload(ev.tool_response);
                if body.len() <= ARCHIVE_CAP_BYTES {
                    let id = cx.put_archive(&body).ok()?;
                    let _ = cx.put_read_cache(&key, &id, Some(&id));
                } else {
                    // An earlier, smaller result under this key must not answer the repeat.
                    let _ = cx.clear_read_cache(&key);
                }
            }
            // A mutating Bash, Edit or Write can change what any earlier command printed
            // or any earlier Read returned. The guard owns its keys (T55.8): a mutating
            // Bash drops every `bash\t…` and `read\t…` key (prefix clears); an Edit or
            // Write drops the bash keys plus its own path's `read\t{path}` key — no
            // dependency on the `read` plugin's invalidation.
            None if matches!(ev.tool_name, "Bash" | "Edit" | "Write") => {
                let _ = cx.clear_read_cache("bash");
                let mutated_path = file_arg(ev.tool_input).and_then(|p| path_identity(p, cx.cwd()));
                match (ev.tool_name, mutated_path) {
                    ("Bash", _) => {
                        let _ = cx.clear_read_cache("read");
                    }
                    (_, Some(p)) => {
                        let _ = cx.clear_read_cache(&format!("read\t{p}"));
                    }
                    // An Edit/Write whose path is missing, or has no stable identity
                    // (relative without a cwd, `..`), can name any file: drop them all.
                    (_, None) => {
                        let _ = cx.clear_read_cache("read");
                    }
                }
            }
            None => {}
        }
        None
    }
}

/// `rtok guard check` — same allow/deny `pre_tool` returns, as a JSON line.
pub fn check(tool: &str, raw_input: &str, cx: &crate::plugin::Runtime) -> String {
    let tool = crate::names::canonical_tool_name(tool);
    let mut input: Value = serde_json::from_str(raw_input).unwrap_or(Value::Null);
    if let Some(obj) = input.as_object_mut()
        && let Some(fp) = obj.remove("filePath")
    {
        obj.entry("file_path").or_insert(fp);
    }
    let ev = PreToolUse {
        tool_name: &tool,
        tool_input: &input,
    };
    match GuardPlugin.pre_tool(&ev, &Ctx::new(cx)) {
        Some(PreToolDecision::Deny { reason }) if !reason.is_empty() => {
            serde_json::json!({ "allow": false, "reason": reason }).to_string()
        }
        _ => serde_json::json!({ "allow": true }).to_string(),
    }
}

/// T50.4: opt-in deny of native `Grep`/`Glob` pointing at MCP `search`/`tree`.
/// Fail open: off by default, and silent while the `read` plugin is disabled
/// (no `search`/`tree` to point at). The knob is per-host opt-in, so a host
/// without `rtok mcp` never turns it on; the hook path does no filesystem
/// reads to check the host config, the read-plugin flag is the guard.
fn native_redirect(tool: &str, cx: &Ctx) -> Option<PreToolDecision> {
    let target = match tool {
        "Grep" => "search",
        "Glob" => "tree",
        _ => return None,
    };
    if !cx
        .plugin_config::<crate::config::Guard>("guard")
        .deny_grep_glob
    {
        return None;
    }
    if !cx.plugin_config::<crate::config::Read>("read").enabled {
        return None;
    }
    let reason = format!("native {tool} is disabled here; use rtok {target} (MCP) instead");
    // Countable but claims no saving: the denied output was never seen (D3).
    let _ = cx.record(&Measurement {
        plugin: "guard",
        kind: "native_deny",
        before_bytes: 0,
        after_bytes: 0,
        est_before: 0,
        est_after: 0,
        ref_id: None,
        call_id: None,
    });
    Some(PreToolDecision::Deny { reason })
}

fn file_arg(input: &Value) -> Option<&str> {
    input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(Value::as_str)
}

/// One spelling for a file, shared by the Read key and the Edit/Write invalidation (T556): a
/// relative path is resolved against the hook cwd, `.` and repeated separators are dropped.
/// Lexical only — no file system access. `None` (no key, or clear every read) when the
/// path is blank, relative without a cwd, or holds `..`, which a symlink makes unresolvable.
fn path_identity(p: &str, cwd: Option<&str>) -> Option<String> {
    use std::path::{Component, Path, PathBuf};
    if p.trim().is_empty() {
        return None;
    }
    let p = Path::new(p);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        Path::new(cwd.filter(|d| !d.is_empty())?).join(p)
    };
    if !joined.is_absolute() || joined.components().any(|c| c == Component::ParentDir) {
        return None;
    }
    Some(
        joined
            .components()
            .collect::<PathBuf>()
            .to_string_lossy()
            .into_owned(),
    )
}

pub(crate) fn cache_key(
    tool: &str,
    input: &Value,
    agent: Option<&str>,
    cwd: Option<&str>,
) -> Option<String> {
    let key = match tool {
        // Claude Code sends `file_path`; Copilot's `read_file`/`view` are adapted to the
        // tool name `Read` but keep their own input key `path` — either names the file.
        // The `read\t` prefix is what the mutating arm below clears in one store call
        // (`clear_read_cache` deletes `x` and every `x\t…`).
        "Read" => {
            let p = path_identity(file_arg(input)?, cwd)?;
            // T322: a slice (`offset`/`limit`/`pages`) is a different body. The slice is a
            // suffix after the path, so the `read\t{path}` prefix clear still reaches it.
            let slice = ["offset", "limit", "pages"].map(|f| match input.get(f) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => String::new(),
                Some(v) => v.to_string(),
            });
            Some(if slice.iter().all(String::is_empty) {
                format!("read\t{p}")
            } else {
                format!("read\t{p}\t{}", slice.join(":"))
            })
        }
        "Bash" => {
            // T322/T555: `None` when no safe identity exists (an unterminated quote).
            let c = norm_cmd(input.get("command")?.as_str()?)?;
            // Only read-only commands are keyed: a repeat of `cargo test` after an Edit is
            // new information, not a duplicate.
            // A relative hop resolves against a directory the key cannot name: PostToolUse
            // already sees the cwd after the command's own `cd`, so the repeat would match
            // and run one level deeper. Over-invalidating is safe, a false deny is not.
            if relative_hop(&c) {
                return None;
            }
            // The starting directory is part of the key: the host keeps the shell's cwd
            // between calls, so the same text run elsewhere prints something else.
            read_only(&c).then(|| match cwd.filter(|d| !d.is_empty()) {
                Some(d) => format!("bash\t{d}\t{c}"),
                None => format!("bash\t{c}"),
            })
        }
        _ => None,
    }?;
    // T129: a context window is `(session_id, agent_id)`. A sub-agent's key carries the id
    // as a suffix, so a body one window has seen is only a duplicate to that window — and
    // the prefix clears below (`read`, `read\t{path}`) still reach every window's key.
    Some(match agent {
        Some(id) if !id.is_empty() => format!("{key}\t{id}"),
        _ => key,
    })
}

/// Stems whose output only changes when something else ran in between. Looks past the
/// `cd <dir> &&` prefix [`norm_cmd`] keeps: only the command after it is keyed.
/// T57.1: first-word + marker scan, no shell grammar. Keyed only when every `|`
/// segment's stem is read-only and no writer marker is present (`>`/`>>`, `| tee`,
/// pipe into a non-read-only stem, `find -delete`/`-exec`, `sed -i`, `tail -f`).
fn read_only(cmd: &str) -> bool {
    // T322: a substitution runs a command the segment scan never sees (over-detecting
    // is safe: a false writer only skips a dedup).
    !cmd.contains("$(")
        && !cmd.contains('`')
        && segments(after_cd_prefix(cmd))
            .into_iter()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .all(segment_read_only)
}

/// Splits at every unquoted `|`, `;` and `&` (so also `||`, `&&`, `|&`, a lone `&`:
/// the empty pieces between doubled separators are filtered by the caller).
fn segments(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quote) = (0, None);
    for (i, b) in s.bytes().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'\'' || b == b'"' => quote = Some(b),
            None if matches!(b, b'|' | b';' | b'&') => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            None => {}
        }
    }
    out.push(&s[start..]);
    out
}

fn segment_read_only(seg: &str) -> bool {
    stem_read_only(seg) && !writer_marker(seg)
}

fn stem_read_only(seg: &str) -> bool {
    let mut w = seg.split_whitespace();
    match super::cmd::formatters::cmd_stem(w.next().unwrap_or("")) {
        "ls" | "cat" | "head" | "tail" | "grep" | "rg" | "find" | "tree" | "wc" | "jq" => true,
        // T557: a script can write (`w`) or run (`e`) from inside its quotes.
        "sed" => shlex::split(seg).is_some_and(|w| {
            w.split_first()
                .is_some_and(|(_, args)| sed::read_only(args))
        }),
        // T322: an awk program can write from inside its quotes (`system(`, `print >`,
        // `| "cmd"`), which the quote-aware redirect scan does not see.
        "awk" => !seg.contains("system(") && !seg.contains(['>', '|']),
        "git" => match w.next() {
            // T557: `--output=<file>` (and its unambiguous abbreviations) writes a file.
            Some("status" | "log" | "diff" | "show" | "rev-parse") => shlex::split(seg)
                .is_some_and(|w| {
                    !w.iter().any(|a| {
                        let name = a.split_once('=').map_or(a.as_str(), |(n, _)| n);
                        name.len() >= 3 && "--output".starts_with(name)
                    })
                }),
            // Only the listing forms: `-D`, `-m`, `-c` and `newname` mutate refs.
            Some("branch") => w.all(|a| {
                matches!(
                    a,
                    "-a" | "-r"
                        | "-v"
                        | "-vv"
                        | "--all"
                        | "--remotes"
                        | "--verbose"
                        | "--list"
                        | "--show-current"
                )
            }),
            _ => false,
        },
        "cargo" => matches!(w.next(), Some("metadata")),
        _ => false,
    }
}

/// Writer markers take the mutating path and clear `bash` keys (fail-open vs false deny).
fn writer_marker(seg: &str) -> bool {
    let toks: Vec<&str> = seg.split_whitespace().collect();
    let stem = super::cmd::formatters::cmd_stem(toks.first().copied().unwrap_or(""));
    toks.iter().copied().any(is_redirect)
        || has_unquoted_redirect(seg)
        || (stem == "find"
            && shlex::split(seg).is_none_or(|w| {
                w.iter().any(|t| {
                    // `-fprint*`/`-fls` write a file, `-ok*` asks to run a command.
                    t == "-delete"
                        || t == "-fls"
                        || ["-exec", "-ok", "-fprint"].iter().any(|p| t.starts_with(p))
                })
            }))
        || (stem == "tail"
            && toks
                .iter()
                .any(|t| *t == "-f" || *t == "--follow" || t.starts_with("--follow=")))
}

fn is_redirect(t: &str) -> bool {
    let t = t.trim_start_matches(|c: char| c.is_ascii_digit());
    t.starts_with('>') || t.starts_with("&>")
}

/// A `>`/`>>` glued to a word (`echo x>file`, `cat a>a`): the whitespace-token
/// scan in [`writer_marker`] never sees it as its own token, so the write is
/// keyed as read-only and a repeat after it denies with stale data. Quote-aware
/// like [`super::skip_word`]: `echo "a>b"` is not a redirect.
fn has_unquoted_redirect(seg: &str) -> bool {
    // Over-detecting is the safe direction: a false writer only skips a dedup,
    // while a missed one denies with stale data.
    let mut quote = None;
    for b in seg.bytes() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'\'' || b == b'"' => quote = Some(b),
            None if b == b'>' => return true,
            None => {}
        }
    }
    false
}

/// Normalized Bash key body: whitespace collapsed, the `rtok run --` wrap stripped, every
/// leading `cd … &&` hop kept verbatim and in order. Hops are relative to the shell's
/// current directory, so `cd sub` twice is `sub/sub`, not `sub` — folding them to the last
/// hop would key two different directories as one and deny a first-time listing.
/// Quote-aware: `cd 'a && b' && ls` is one hop to `'a && b'`.
fn norm_cmd(s: &str) -> Option<String> {
    let (hops, rest) = split_cd(&unwrap_flat(s)?)?;
    Some(
        hops.into_iter()
            .map(|d| format!("cd {d} && "))
            .chain([rest])
            .collect(),
    )
}

/// Splits the leading `cd <dir> &&` hops from the command. Each remainder re-runs
/// `unwrap_flat` because PostToolUse re-wraps the command the model actually ran.
fn split_cd(s: &str) -> Option<(Vec<String>, String)> {
    let mut hops = Vec::new();
    let mut t = s.to_string();
    while let Some((d, rest)) = strip_cd_hop(&t) {
        hops.push(d);
        t = unwrap_flat(&rest)?;
    }
    Some((hops, t))
}

/// The wrapper is removed before the inner command is flattened: inside the wrapper's own
/// quotes every byte looks like argument text, so flattening first would keep the model's
/// insignificant whitespace and split the Pre/Post keys.
fn unwrap_flat(s: &str) -> Option<String> {
    flatten(&strip_wrap(&flatten(s)?))
}

/// Whether any leading `cd` hop of the normalized command is relative (`cd sub`, `cd -`,
/// `cd $HOME/x`). Absolute means a rooted path, `~…` or a Windows drive.
fn relative_hop(cmd: &str) -> bool {
    let Some((hops, _)) = split_cd(cmd) else {
        return true;
    };
    hops.iter().any(|t| {
        let t = t
            .strip_prefix('\'')
            .and_then(|r| r.strip_suffix('\''))
            .or_else(|| t.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
            .unwrap_or(t);
        let b = t.as_bytes();
        let drive = b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'\\' | b'/');
        !(t.starts_with(['/', '~']) || drive)
    })
}

/// Whether the command moves the host's persistent shell, so every cached listing was
/// taken from a different directory.
fn has_cd_hop(command: &str) -> bool {
    unwrap_flat(command).is_some_and(|t| strip_cd_hop(&t).is_some())
}

/// One `cd <dir> &&` prefix: the target word (quotes kept as typed) and the remainder
/// after `&&`. `None` unless `&&` follows the target at top level — a quoted path with
/// `&&` inside is one word, not a split point.
pub(crate) fn strip_cd_hop(s: &str) -> Option<(String, String)> {
    let after = s.strip_prefix("cd ")?;
    let rest = crate::plugins::skip_word(after)?;
    let target = after[..after.len() - rest.len()].trim_end();
    if target.is_empty() {
        return None;
    }
    let rest = rest.strip_prefix("&&")?.trim_start();
    Some((target.to_string(), rest.to_string()))
}

/// Strips every `cd <dir> &&` hop [`norm_cmd`] kept, so the read-only stem check sees the
/// command that actually runs (`cd a && cd b && ls` → `ls`).
fn after_cd_prefix(mut s: &str) -> &str {
    while let Some(rest) = s
        .strip_prefix("cd ")
        .and_then(crate::plugins::skip_word)
        .and_then(|r| r.strip_prefix("&&"))
    {
        s = rest.trim_start();
    }
    s
}

/// Collapses whitespace runs to one space and turns each unquoted newline into a `;` (T322:
/// it would otherwise hide a command separator, `ls\nrm x` → `ls rm x`). Whitespace inside
/// quotes or behind a backslash is argument text (T555) and stays byte-for-byte. `None` for
/// an unterminated quote: no safe identity, so no key.
fn flatten(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let (mut quote, mut gap) = (None::<char>, false);
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some('\'') => {
                out.push(c);
                if c == '\'' {
                    quote = None;
                }
            }
            Some(q) => {
                out.push(c);
                if c == '\\' {
                    out.push(chars.next()?);
                } else if c == q {
                    quote = None;
                }
            }
            None if c == '\n' || c == '\r' => {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push(';');
                gap = true;
            }
            None if c.is_whitespace() => gap = true,
            None => {
                if gap && !out.is_empty() {
                    out.push(' ');
                }
                gap = false;
                out.push(c);
                match c {
                    '\'' | '"' => quote = Some(c),
                    '\\' => out.extend(chars.next()),
                    _ => {}
                }
            }
        }
    }
    quote.is_none().then_some(out)
}

fn payload(v: &Value) -> Vec<u8> {
    if let Some(s) = v.as_str() {
        return s.as_bytes().to_vec();
    }
    for k in ["stdout", "content"] {
        if let Some(s) = v.get(k).and_then(Value::as_str) {
            return s.as_bytes().to_vec();
        }
    }
    serde_json::to_vec(v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::native_abs;
    use rtok_plugin_sdk::Ctx;
    use serde_json::json;

    fn setup() -> crate::plugin::Runtime {
        crate::testutil::runtime("guard").0
    }

    /// T445: every `cd` hop stays in the key, verbatim and in order, and the starting
    /// directory is part of the key. A relative hop is never keyed at all.
    #[test]
    fn bash_key_keeps_every_absolute_cd_hop_and_the_cwd() {
        let k = |c: &str| cache_key("Bash", &json!({"command": c}), None, None);
        assert_eq!(k("ls"), Some("bash\tls".to_string()));
        assert_eq!(k("cd /a && ls"), Some("bash\tcd /a && ls".to_string()));
        assert_eq!(k("cd /b && ls"), Some("bash\tcd /b && ls".to_string()));
        assert_eq!(
            k("cd /a && cd /b && ls"),
            Some("bash\tcd /a && cd /b && ls".to_string())
        );
        assert_ne!(k("cd /a && cd /a && ls"), k("cd /a && ls"));
        // A quoted path with `&&` inside is one target word, not a split point.
        assert_eq!(
            k("cd '/a && b' && ls"),
            Some("bash\tcd '/a && b' && ls".to_string())
        );
        for abs in ["~", "~/x", "~u/x", "'/a b'", "\"/a b\"", "C:\\x", "d:/x"] {
            assert!(k(&format!("cd {abs} && ls")).is_some(), "{abs}");
        }
        // Whitespace normalization and the `rtok run` wrap behind hops do not split a key:
        // PreToolUse sees the typed command, PostToolUse the rewritten one.
        assert_eq!(k("cd   /a   &&   ls"), k("cd /a && ls"));
        assert_eq!(k("cd /a && rtok run -- 'ls'"), k("cd /a && ls"));
        assert_eq!(
            k("cd /a && cd /b && rtok run -- 'ls'"),
            k("cd /a && cd /b && ls")
        );
        // T457: the sub-agent form carries `--agent <id>` before `--`.
        assert_eq!(k("rtok run --agent sub_1 -- 'ls'"), k("ls"));
        assert_eq!(
            k("cd /a && rtok run --agent sub_1 -- 'ls'"),
            k("cd /a && ls")
        );
        // T922: the POSIX `command -v rtok` guard leads the wrapper and is not part of the key.
        let guard = crate::names::run_guard();
        assert_eq!(k(&format!("{guard}rtok run -- 'ls'")), k("ls"));
        assert_eq!(
            k(&format!("cd /a && {guard}rtok run --agent sub_1 -- 'ls'")),
            k("cd /a && ls")
        );
        assert_eq!(k("rtok run --agent ../x -- 'ls'"), None);
        assert_eq!(k("rtok run --agent -- 'ls'"), None);

        let at = |cwd, agent| cache_key("Bash", &json!({"command": "ls"}), agent, cwd);
        assert_eq!(at(Some("/r"), None), Some("bash\t/r\tls".to_string()));
        assert_ne!(at(Some("/r"), None), at(Some("/s"), None));
        assert_ne!(at(Some("/r"), None), at(None, None));
        assert_eq!(at(Some(""), None), at(None, None));
        // The agent suffix still trails the key, so the `bash` prefix clear reaches it.
        assert_eq!(
            at(Some("/r"), Some("ag")),
            Some("bash\t/r\tls\tag".to_string())
        );
    }

    /// A hop that resolves against the current directory has no stable key: the same text
    /// runs somewhere else once the shell has moved.
    #[test]
    fn bash_behind_a_relative_cd_hop_has_no_key() {
        for c in [
            "cd a && ls",
            "cd - && ls",
            "cd .. && ls",
            "cd $HOME/x && ls",
            "cd 'a b' && ls",
            "cd /r && cd sub && ls",
            "cd sub && cd /r && ls",
            "cd a && rtok run -- 'ls'",
        ] {
            for cwd in [None, Some("/s")] {
                assert_eq!(
                    cache_key("Bash", &json!({"command": c}), None, cwd),
                    None,
                    "{c}"
                );
            }
        }
    }

    /// T555: whitespace inside quotes or behind a backslash is part of an argument, so two
    /// commands that differ only there read different files and must not share a key.
    #[test]
    fn bash_key_keeps_whitespace_inside_arguments() {
        let k = |c: &str| cache_key("Bash", &json!({"command": c}), None, Some("/r"));
        for (a, b) in [
            ("cat 'a  b'", "cat 'a b'"),
            ("cat \"a  b\"", "cat \"a b\""),
            ("cat a\\ \\ b", "cat a\\ b"),
            ("cat 'a\tb'", "cat 'a b'"),
            ("cat 'a\nb'", "cat 'a b'"),
            ("cat \"a\nb\"", "cat \"a b\""),
            ("rtok run -- 'cat '\"'\"'a  b'\"'\"''", "cat 'a b'"),
        ] {
            assert!(k(a).is_some(), "{a}");
            assert_ne!(k(a), k(b), "{a} vs {b}");
        }
        // The wrapper PostToolUse sees keys the same as the command PreToolUse sees.
        assert_eq!(k("rtok run -- 'cat '\"'\"'a  b'\"'\"''"), k("cat 'a  b'"));
        assert_eq!(k("cat 'a\nb'"), k("cat 'a\nb'"));
        // Insignificant whitespace still collapses, and a newline outside quotes stays a `;`.
        assert_eq!(k("cat   'a b'\t x"), k("cat 'a b' x"));
        assert_eq!(k("rtok run -- 'cat   x'"), k("cat x"));
        assert_ne!(k("ls\nrm x"), k("ls rm x"));
        assert_eq!(k("ls\nrm x"), k("ls ; rm x"));
        // No safe identity: an unterminated quote is never keyed.
        assert_eq!(k("cat 'a  b"), None);
    }

    /// T555 at the hook layer: a cached result for one file never answers the other.
    #[test]
    fn quoted_whitespace_variant_is_not_denied_as_a_duplicate() {
        let sh = Shell::new();
        sh.ran("cat 'a  b'");
        assert!(sh.denied("cat 'a  b'"));
        assert!(sh.denied("cat   'a  b'"));
        assert!(!sh.denied("cat 'a b'"));
    }

    /// A redirect glued to a word (`echo x>file`) is still a writer: the
    /// whitespace-token scan never sees it, so without this the write is keyed
    /// as read-only and a repeat after it denies with stale data.
    #[test]
    fn redirect_without_spaces_is_not_read_only() {
        for w in [
            "echo x>file",
            "cat a>a",
            "ls>>out",
            "echo x > file",
            "grep foo bar 2>err",
        ] {
            assert!(!read_only(w), "{w}");
            assert!(cache_key("Bash", &json!({"command": w}), None, None).is_none());
        }
        for r in ["ls", "cat file", "grep foo bar", "grep 'a>b' file"] {
            assert!(read_only(r), "{r}");
        }
    }

    /// T445: the key is only a duplicate when the shell sits where it sat before.
    struct Shell {
        cx: crate::plugin::Runtime,
    }

    impl Shell {
        fn new() -> Self {
            Self { cx: setup() }
        }

        fn at(&mut self, cwd: &str) {
            self.cx.cwd = Some(cwd.to_string());
        }

        fn ran(&self, command: &str) {
            let input = json!({"command": command});
            let resp = json!({"stdout": "listing"});
            let ev = PostToolUse {
                tool_name: "Bash",
                tool_input: &input,
                tool_response: &resp,
            };
            assert!(GuardPlugin.post_tool(&ev, &Ctx::new(&self.cx)).is_none());
        }

        fn denied(&self, command: &str) -> bool {
            let input = json!({"command": command});
            let ev = PreToolUse {
                tool_name: "Bash",
                tool_input: &input,
            };
            matches!(
                GuardPlugin.pre_tool(&ev, &Ctx::new(&self.cx)),
                Some(PreToolDecision::Deny { .. })
            )
        }
    }

    /// The shell is persistent, so by PostToolUse the host cwd is already past the
    /// command's own `cd`: a relative repeat would match that key and run one level deeper.
    #[test]
    fn relative_cd_repeat_is_not_denied() {
        let mut sh = Shell::new();
        sh.at("/s/a");
        sh.ran("cd a && ls");
        assert!(!sh.denied("cd a && ls"), "would run in /s/a/a");
        sh.ran("cd a && ls");
        assert!(!sh.denied("cd a && ls"));
        assert!(!sh.denied("ls"));
    }

    /// An absolute hop is safe: the post cwd is the hop target, and the repeat lists it again.
    #[test]
    fn absolute_cd_repeat_is_denied() {
        let mut sh = Shell::new();
        sh.at("/s");
        sh.ran("ls");
        sh.at("/r/sub");
        sh.ran("cd /r && cd /r/sub && ls -la");
        assert!(
            !sh.denied("ls"),
            "the shell moved, the parent listing is stale"
        );
        assert!(sh.denied("cd /r && cd /r/sub && ls -la"));
        assert!(
            !sh.denied("cd /r/sub && ls -la"),
            "one hop fewer is another key"
        );
    }

    /// `cd /r && ls` is a read-only listing, yet it moves the shell: a plain `ls` after it
    /// runs in `/r`, and a relative `cd a && ls` (unkeyed) moves it just the same.
    #[test]
    fn cd_hop_drops_earlier_bash_answers() {
        for hop in ["cd /r && ls", "cd a && ls"] {
            let mut sh = Shell::new();
            sh.at("/repo");
            sh.ran("ls");
            sh.ran("git status");
            sh.ran(hop);
            assert!(
                !sh.denied("ls"),
                "the parent listing is stale after `{hop}`"
            );
            assert!(!sh.denied("git status"));
            // No hop, no flush: ordinary read-only commands keep each other's answers.
            sh.ran("ls");
            sh.ran("git status");
            assert!(sh.denied("ls"));
            assert!(sh.denied("git status"));
        }
    }

    /// The same text from another starting directory prints something else.
    #[test]
    fn bash_repeat_from_another_cwd_is_a_new_key() {
        let mut sh = Shell::new();
        sh.at("/repo/a");
        sh.ran("ls");
        sh.at("/repo/b");
        assert!(!sh.denied("ls"));
        sh.at("/repo/a");
        assert!(sh.denied("ls"));
    }

    /// `cargo test` is never keyed, and a mutating Bash, Edit or Write drops every Bash key.
    #[test]
    fn mutation_between_commands_allows_the_repeat() {
        let cx = setup();
        let g = GuardPlugin;
        let resp = json!({"stdout": "out"});
        let post = |name: &'static str, input: &Value| {
            let ev = PostToolUse {
                tool_name: name,
                tool_input: input,
                tool_response: &resp,
            };
            assert!(g.post_tool(&ev, &Ctx::new(&cx)).is_none());
        };
        let denied = |input: &Value| {
            let ev = PreToolUse {
                tool_name: "Bash",
                tool_input: input,
            };
            matches!(
                g.pre_tool(&ev, &Ctx::new(&cx)),
                Some(PreToolDecision::Deny { .. })
            )
        };
        let test = json!({"command": "cargo test"});
        post("Bash", &test);
        assert!(!denied(&test), "cargo test is not read-only");
        let cat = json!({"command": "cat a.txt"});
        post("Bash", &cat);
        assert!(denied(&cat));
        post("Bash", &test);
        assert!(!denied(&cat), "a mutating Bash clears bash keys");
        post("Bash", &cat);
        assert!(denied(&cat));
        post("Edit", &json!({"file_path": "/proj/b.rs"}));
        assert!(!denied(&cat), "an Edit clears bash keys");
        post("Bash", &json!({"command": "git status"}));
        assert!(denied(&json!({"command": "git status"})));
        assert!(!denied(&json!({"command": "git add ."})));
    }

    #[test]
    fn two_identical_reads_second_denies_naming_archive() {
        let cx = setup();
        let g = GuardPlugin;
        let path = json!({"file_path": native_abs("/Users/dev/proj/src/main.rs")});
        let other = json!({"file_path": native_abs("/Users/dev/proj/src/lib.rs")});
        let read = PreToolUse {
            tool_name: "Read",
            tool_input: &path,
        };
        assert!(g.pre_tool(&read, &Ctx::new(&cx)).is_none());
        let resp = json!({"content": "fn main() {}"});
        let post = PostToolUse {
            tool_name: "Read",
            tool_input: &path,
            tool_response: &resp,
        };
        assert!(g.post_tool(&post, &Ctx::new(&cx)).is_none());
        match g.pre_tool(&read, &Ctx::new(&cx)) {
            Some(PreToolDecision::Deny { reason }) => {
                assert!(reason.contains("rtok expand "), "{reason}");
                let id = reason.rsplit(' ').next().unwrap();
                assert!(reason.contains(id));
            }
            other => panic!("{other:?}"),
        }
        let diff = PreToolUse {
            tool_name: "Read",
            tool_input: &other,
        };
        assert!(g.pre_tool(&diff, &Ctx::new(&cx)).is_none());
        assert!(cx.store.measurement_count("guard").unwrap() >= 1);
        let rows = cx.store.list_measurements("guard").unwrap();
        assert!(rows.iter().any(|r| r.before_bytes > 0), "{rows:?}");
    }

    /// T299: the denial row claims exactly the payload the repeat would have returned:
    /// `before_bytes` is the archived body's length and `est_before` the `bytes / 4`
    /// heuristic (the hook never loads the body back to estimate it — T55.16).
    #[test]
    fn deny_measurement_before_bytes_and_est_before_match_the_avoided_payload() {
        let cx = setup();
        let g = GuardPlugin;
        let path = json!({"file_path": native_abs("/Users/dev/proj/src/inventory.rs")});
        let read = PreToolUse {
            tool_name: "Read",
            tool_input: &path,
        };
        assert!(g.pre_tool(&read, &Ctx::new(&cx)).is_none());
        // Not a multiple of 4, so the truncating division is exercised.
        let body = "x".repeat(4097);
        let resp = json!({"content": body.clone()});
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Read",
                    tool_input: &path,
                    tool_response: &resp,
                },
                &Ctx::new(&cx),
            )
            .is_none()
        );
        assert!(matches!(
            g.pre_tool(&read, &Ctx::new(&cx)),
            Some(PreToolDecision::Deny { .. })
        ));
        let rows = cx.store.list_measurements("guard").unwrap();
        let row = rows
            .iter()
            .find(|r| r.kind == "guard" && r.before_bytes > 0)
            .expect("deny row");
        // `payload()` for `{"content": body}` is `body.as_bytes()`, archived verbatim by
        // `post_tool` — so `before_bytes` is exactly the denied payload's byte length.
        assert_eq!(row.before_bytes as usize, body.len());
        let heuristic_est = (body.len() / 4).max(1) as i32;
        assert_eq!(row.est_before, heuristic_est);
        assert_eq!(row.after_bytes, 0);
        assert_eq!(row.est_after, 0);
    }

    /// T201: a body over `ARCHIVE_CAP_BYTES` is never archived, so the key stays uncached —
    /// the repeat must be allowed, not denied with a pointer nothing wrote.
    #[test]
    fn post_tool_over_cap_skips_archive_and_the_repeat_is_allowed() {
        let cx = setup();
        let g = GuardPlugin;
        let path = json!({"file_path": native_abs("/Users/dev/proj/src/big.rs")});
        let read = PreToolUse {
            tool_name: "Read",
            tool_input: &path,
        };
        assert!(g.pre_tool(&read, &Ctx::new(&cx)).is_none());
        // A small first result is cached; the file then grows past the cap.
        let small = json!({"content": "fn main() {}"});
        let post_small = PostToolUse {
            tool_name: "Read",
            tool_input: &path,
            tool_response: &small,
        };
        assert!(g.post_tool(&post_small, &Ctx::new(&cx)).is_none());
        assert!(
            g.pre_tool(&read, &Ctx::new(&cx)).is_some(),
            "small body cached"
        );
        let big = "x".repeat(ARCHIVE_CAP_BYTES + 1);
        let resp = json!({"content": big});
        let post = PostToolUse {
            tool_name: "Read",
            tool_input: &path,
            tool_response: &resp,
        };
        assert!(g.post_tool(&post, &Ctx::new(&cx)).is_none());
        // Neither the big body nor the stale small one answers the repeat.
        assert!(g.pre_tool(&read, &Ctx::new(&cx)).is_none());
        // Only the small body's deny was measured.
        assert_eq!(cx.store.measurement_count("guard").unwrap(), 1);
    }

    #[test]
    fn edit_clears_guard_read_so_the_next_read_is_allowed() {
        let cx = setup();
        let g = GuardPlugin;
        let path = json!({"file_path": native_abs("/proj/src/main.rs")});
        let resp = json!({"content": "fn main() {}"});
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Read",
                    tool_input: &path,
                    tool_response: &resp,
                },
                &Ctx::new(&cx),
            )
            .is_none()
        );
        // The guard owns its keys (T55.8): with `read.delta` on, the read plugin keeps
        // its cache so a re-read can answer with a diff, so the guard's own Edit
        // invalidation is what has to clear `read\t{path}` here.
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Edit",
                    tool_input: &path,
                    tool_response: &json!({}),
                },
                &Ctx::new(&cx),
            )
            .is_none()
        );
        let read = PreToolUse {
            tool_name: "Read",
            tool_input: &path,
        };
        assert!(g.pre_tool(&read, &Ctx::new(&cx)).is_none());
    }

    fn read_at(cx: &crate::plugin::Runtime, input: &Value) -> bool {
        let ev = PreToolUse {
            tool_name: "Read",
            tool_input: input,
        };
        matches!(
            GuardPlugin.pre_tool(&ev, &Ctx::new(cx)),
            Some(PreToolDecision::Deny { .. })
        )
    }

    fn done(cx: &crate::plugin::Runtime, tool: &'static str, input: &Value) {
        let resp = json!({"content": "body"});
        let ev = PostToolUse {
            tool_name: tool,
            tool_input: input,
            tool_response: &resp,
        };
        assert!(GuardPlugin.post_tool(&ev, &Ctx::new(cx)).is_none());
    }

    /// T556: a relative path names another file once the hook cwd differs.
    #[test]
    fn relative_read_is_keyed_by_its_cwd() {
        let mut cx = setup();
        let rel = json!({"file_path": "file.txt"});
        cx.cwd = Some(native_abs("/A"));
        done(&cx, "Read", &rel);
        assert!(read_at(&cx, &rel));
        cx.cwd = Some(native_abs("/B"));
        assert!(!read_at(&cx, &rel), "another directory's file.txt");
        cx.cwd = Some(native_abs("/A"));
        assert!(read_at(&cx, &rel));
        assert!(read_at(&cx, &json!({"file_path": "./file.txt"})));
        assert!(read_at(
            &cx,
            &json!({"file_path": native_abs("/A/file.txt")})
        ));
        // No cwd, no identity for a relative path: never cached.
        cx.cwd = None;
        assert!(!read_at(&cx, &rel));
        assert_eq!(cache_key("Read", &rel, None, None), None);
        // `..` cannot be resolved without the file system.
        let up = json!({"file_path": native_abs("/A/x/../file.txt")});
        assert_eq!(
            cache_key("Read", &up, None, Some(native_abs("/A").as_str())),
            None
        );
    }

    /// T556: every spelling of a path invalidates together, slices included; other files keep
    /// their cache.
    #[test]
    fn edit_through_an_alias_invalidates_every_spelling() {
        let mut cx = setup();
        cx.cwd = Some(native_abs("/A"));
        let rel = json!({"file_path": "./file.txt"});
        let slice = json!({"file_path": "file.txt", "offset": 10, "limit": 5});
        let other = json!({"file_path": "other.txt"});
        for i in [&rel, &slice, &other] {
            done(&cx, "Read", i);
            assert!(read_at(&cx, i));
        }
        done(
            &cx,
            "Edit",
            &json!({"file_path": native_abs("/A/file.txt")}),
        );
        assert!(!read_at(&cx, &rel));
        assert!(!read_at(&cx, &slice));
        assert!(read_at(&cx, &other), "unrelated file keeps its cache");
        // A relative edit with no cwd cannot name its file: drop every read.
        cx.cwd = None;
        done(&cx, "Edit", &json!({"file_path": "other.txt"}));
        cx.cwd = Some(native_abs("/A"));
        assert!(!read_at(&cx, &other));
    }

    #[test]
    fn missing_archive_fails_open() {
        let cx = setup();
        let g = GuardPlugin;
        let path = json!({"file_path": native_abs("/proj/gone.rs")});
        let resp = json!({"content": "old"});
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Read",
                    tool_input: &path,
                    tool_response: &resp,
                },
                &Ctx::new(&cx),
            )
            .is_none()
        );
        let key = cache_key("Read", &path, None, None).unwrap();
        let (id, _) = cx.store.get_read_cache(&cx.session, &key).unwrap().unwrap();
        let id = id.unwrap();
        std::fs::remove_file(cx.config.core.archive_dir.join(&id)).unwrap();
        let read = PreToolUse {
            tool_name: "Read",
            tool_input: &path,
        };
        assert!(g.pre_tool(&read, &Ctx::new(&cx)).is_none());
    }

    /// T129: a context window is `(session_id, agent_id)`. The parent reads P; a sub-agent's
    /// first Read of P is new to its window and must be allowed with no `guard` Measurement
    /// row (the session-scoped key denied a body that window never saw); the sub-agent's
    /// repeat denies; the parent's repeat still denies.
    #[test]
    fn a_sub_agent_window_scopes_the_read_cache() {
        let cx = setup();
        let g = GuardPlugin;
        let path = json!({"file_path": native_abs("/proj/p.rs")});
        let resp = json!({"content": "body"});
        let read = PreToolUse {
            tool_name: "Read",
            tool_input: &path,
        };
        let post = PostToolUse {
            tool_name: "Read",
            tool_input: &path,
            tool_response: &resp,
        };
        assert!(g.pre_tool(&read, &Ctx::new(&cx)).is_none());
        assert!(g.post_tool(&post, &Ctx::new(&cx)).is_none());

        let sub = Ctx::with_agent(&cx, Some("a1"));
        let rows = cx.store.measurement_count("guard").unwrap();
        assert!(
            g.pre_tool(&read, &sub).is_none(),
            "the sub-agent's first Read of P is new to its window"
        );
        assert_eq!(
            cx.store.measurement_count("guard").unwrap(),
            rows,
            "the allowed path records no guard Measurement row"
        );
        assert!(g.post_tool(&post, &sub).is_none());
        match g.pre_tool(&read, &sub) {
            Some(PreToolDecision::Deny { reason }) => {
                assert!(reason.contains("expand"), "{reason}")
            }
            other => panic!("the sub-agent's repeat must deny: {other:?}"),
        }
        match g.pre_tool(&read, &Ctx::new(&cx)) {
            Some(PreToolDecision::Deny { reason }) => {
                assert!(reason.contains("expand"), "{reason}")
            }
            other => panic!("the parent's repeat must deny: {other:?}"),
        }
    }

    /// T50.4: off by default, Grep points at `search` and Glob at `tree` when on,
    /// and a disabled `read` plugin fails open (no MCP tools to point at).
    #[test]
    fn native_grep_glob_deny_is_opt_in_and_points_at_mcp() {
        let g = GuardPlugin;
        let empty = json!({});
        let pre = |tool: &'static str| PreToolUse {
            tool_name: tool,
            tool_input: &empty,
        };
        // Default config: the knob is off, everything passes through.
        let cx = setup();
        assert!(g.pre_tool(&pre("Grep"), &Ctx::new(&cx)).is_none());
        assert!(g.pre_tool(&pre("Glob"), &Ctx::new(&cx)).is_none());
        // Knob on: Grep and Glob deny naming their MCP replacement.
        let (mut c, _dir) = crate::testutil::config("grep-glob");
        c.plugins.guard.deny_grep_glob = true;
        let cx = crate::plugin::Runtime::open(c, "grep-glob").unwrap();
        match g.pre_tool(&pre("Grep"), &Ctx::new(&cx)) {
            Some(PreToolDecision::Deny { reason }) => {
                assert!(reason.contains("search"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
        match g.pre_tool(&pre("Glob"), &Ctx::new(&cx)) {
            Some(PreToolDecision::Deny { reason }) => assert!(reason.contains("tree"), "{reason}"),
            other => panic!("{other:?}"),
        }
        // Other tools are untouched, and the deny is counted without claiming bytes.
        assert!(g.pre_tool(&pre("Read"), &Ctx::new(&cx)).is_none());
        let rows = cx.store.list_measurements("guard").unwrap();
        assert!(
            rows.iter()
                .any(|r| r.kind == "native_deny" && r.before_bytes == 0 && r.after_bytes == 0),
            "{rows:?}"
        );
        // Read plugin disabled: no search/tree to point at, so allow (fail open).
        let (mut c, _dir) = crate::testutil::config("grep-noread");
        c.plugins.guard.deny_grep_glob = true;
        c.plugins.read.enabled = false;
        let cx = crate::plugin::Runtime::open(c, "grep-noread").unwrap();
        assert!(g.pre_tool(&pre("Grep"), &Ctx::new(&cx)).is_none());
        assert!(g.pre_tool(&pre("Glob"), &Ctx::new(&cx)).is_none());
    }

    /// T55.13: Copilot's adapted `Read` keeps its own input key `path`; the deny must
    /// fire for it exactly as it does for Claude Code's `file_path`.
    #[test]
    fn copilot_path_key_dedups_like_file_path() {
        let cx = setup();
        let g = GuardPlugin;
        let resp = json!({"content": "fn main() {}"});
        for key in ["path", "file_path"] {
            let input = json!({ key: native_abs("/Users/dev/proj/src/main.rs") });
            assert!(
                g.post_tool(
                    &PostToolUse {
                        tool_name: "Read",
                        tool_input: &input,
                        tool_response: &resp,
                    },
                    &Ctx::new(&cx),
                )
                .is_none()
            );
            match g.pre_tool(
                &PreToolUse {
                    tool_name: "Read",
                    tool_input: &input,
                },
                &Ctx::new(&cx),
            ) {
                Some(PreToolDecision::Deny { reason }) => {
                    assert!(reason.contains("rtok expand "), "{key}: {reason}")
                }
                other => panic!("{key}: {other:?}"),
            }
        }
    }

    /// T55.8: a mutating Bash between two identical Reads must not serve the stale
    /// archive — the guard drops its own `read\t…` keys, no `read` plugin involved.
    #[test]
    fn bash_mutation_allows_the_next_read() {
        let cx = setup();
        let g = GuardPlugin;
        let path = json!({"file_path": native_abs("/proj/src/main.rs")});
        let resp = json!({"content": "fn main() {}"});
        let post = |name: &'static str, input: &Value| {
            assert!(
                g.post_tool(
                    &PostToolUse {
                        tool_name: name,
                        tool_input: input,
                        tool_response: &resp,
                    },
                    &Ctx::new(&cx),
                )
                .is_none()
            );
        };
        post("Read", &path);
        let read = PreToolUse {
            tool_name: "Read",
            tool_input: &path,
        };
        assert!(matches!(
            g.pre_tool(&read, &Ctx::new(&cx)),
            Some(PreToolDecision::Deny { .. })
        ));
        post("Bash", &json!({"command": "cargo fmt"}));
        assert!(
            g.pre_tool(&read, &Ctx::new(&cx)).is_none(),
            "a mutating Bash must drop the guard read key"
        );
    }

    /// T55.8: Edit drops that path's guard read key even with the `read` plugin off.
    #[test]
    fn edit_with_read_plugin_off_allows_the_next_read() {
        let (mut c, _dir) = crate::testutil::config("guard-edit-noread");
        c.plugins.read.enabled = false;
        let cx = crate::plugin::Runtime::open(c, "guard-edit-noread").unwrap();
        let g = GuardPlugin;
        let path = json!({"file_path": native_abs("/proj/src/main.rs")});
        let resp = json!({"content": "fn main() {}"});
        for name in ["Read", "Edit"] {
            assert!(
                g.post_tool(
                    &PostToolUse {
                        tool_name: name,
                        tool_input: &path,
                        tool_response: &resp,
                    },
                    &Ctx::new(&cx),
                )
                .is_none()
            );
        }
        let read = PreToolUse {
            tool_name: "Read",
            tool_input: &path,
        };
        assert!(
            g.pre_tool(&read, &Ctx::new(&cx)).is_none(),
            "guard's own Edit invalidation must not need the read plugin"
        );
    }

    /// T55.16: the deny reads metadata only — an unreadable body file still denies
    /// (the old body read would have failed open), so megabytes never cross the
    /// ≤ 10 ms hook path.
    #[cfg(unix)]
    #[test]
    fn deny_does_not_read_the_archive_body() {
        use std::os::unix::fs::PermissionsExt;
        let cx = setup();
        let g = GuardPlugin;
        let path = json!({"file_path": "/proj/src/hot.rs"});
        let resp = json!({"content": "fn main() {}"});
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Read",
                    tool_input: &path,
                    tool_response: &resp,
                },
                &Ctx::new(&cx),
            )
            .is_none()
        );
        let key = cache_key("Read", &path, None, None).unwrap();
        let (id, _) = cx.store.get_read_cache(&cx.session, &key).unwrap().unwrap();
        let id = id.unwrap();
        let file = cx.config.core.archive_dir.join(&id);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        let read = PreToolUse {
            tool_name: "Read",
            tool_input: &path,
        };
        assert!(
            matches!(
                g.pre_tool(&read, &Ctx::new(&cx)),
                Some(PreToolDecision::Deny { .. })
            ),
            "an unreadable body must still deny — only its metadata was consulted"
        );
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    /// T322: compound commands are keyed only when every segment is read-only; a writer
    /// behind `;`, `&&`, `||`, `&`, a newline or a substitution must take the mutating path.
    #[test]
    fn compound_commands_with_a_writer_are_not_keyed() {
        let k = |c: &str| cache_key("Bash", &json!({"command": c}), None, None);
        for w in [
            "ls && rm -rf build",
            "cat a; rm a",
            "ls || rm x",
            "ls & rm x",
            "ls\nrm x",
            "cat $(rm x)",
            "cat `rm x`",
            "cd d && ls && rm x",
        ] {
            assert!(k(w).is_none(), "{w:?}");
        }
        for r in [
            "ls && cat a",
            "cat a; ls",
            "grep 'a;b' f",
            "cd /d && ls; wc f",
        ] {
            assert!(k(r).is_some(), "{r:?}");
        }
    }

    /// T322: only the plain listing forms of `git branch` are read-only.
    #[test]
    fn git_branch_is_read_only_only_when_listing() {
        let k = |c: &str| cache_key("Bash", &json!({"command": c}), None, None);
        for w in ["git branch -D x", "git branch -m a b", "git branch newname"] {
            assert!(k(w).is_none(), "{w}");
        }
        for r in [
            "git branch",
            "git branch -a",
            "git branch -vv",
            "git branch --show-current",
        ] {
            assert!(k(r).is_some(), "{r}");
        }
    }

    /// T322: an awk program can write (`system(`, `print >`, `| cmd`) inside its quotes.
    #[test]
    fn awk_writers_are_not_read_only() {
        let k = |c: &str| cache_key("Bash", &json!({"command": c}), None, None);
        assert!(k(r#"awk 'BEGIN{system("rm x")}'"#).is_none());
        assert!(k(r#"awk '{print > "f"}' g"#).is_none());
        assert!(k(r#"awk '{print | "sh"}' g"#).is_none());
        assert!(k("awk '{print $1}' f").is_some());
    }

    /// T322: each Read slice is its own key; a plain Read keeps the bare path key.
    #[test]
    fn read_slices_get_distinct_keys() {
        let k = |v: Value| cache_key("Read", &v, None, None).unwrap();
        let (f, pdf) = (native_abs("/f"), native_abs("/f.pdf"));
        let plain = k(json!({"file_path": f}));
        assert_eq!(plain, format!("read\t{f}"));
        let a = k(json!({"file_path": f, "offset": 1, "limit": 50}));
        let b = k(json!({"file_path": f, "offset": 300, "limit": 50}));
        assert_ne!(a, b);
        assert_ne!(a, plain);
        assert_eq!(a, k(json!({"file_path": f, "offset": 1, "limit": 50})));
        assert!(
            a.starts_with(&format!("read\t{f}\t")),
            "prefix clear must reach slices"
        );
        assert_ne!(
            k(json!({"file_path": pdf, "pages": "1-3"})),
            k(json!({"file_path": pdf, "pages": "4-6"}))
        );
    }

    /// T57.1: flag-aware read-only keys — writer markers take the mutating path.
    #[test]
    fn flag_aware_read_only_keys() {
        let k = |c: &str| cache_key("Bash", &json!({"command": c}), None, None);
        assert!(k("sed -n 1,40p f").is_some(), "sed -n is read-only");
        assert!(k("sed -i s/a/b/ f").is_none(), "sed -i is mutating");
        assert!(
            k("find . -name x -delete").is_none(),
            "find -delete is mutating"
        );
        assert!(k("cat a > b").is_none(), "redirect is mutating");
        assert!(k("tail -f log").is_none(), "tail -f is never keyed");
        assert!(k("cat a | grep b").is_some(), "pipe of readers is keyed");
        assert!(
            k("ls | xargs rm").is_none(),
            "pipe into a writer is mutating"
        );
        assert!(k("jq . f").is_some());
        assert!(k("awk '{print $1}' f").is_some());
        assert!(k("git rev-parse HEAD").is_some());
        assert!(k("cargo metadata").is_some());
        assert!(k("cargo test").is_none());
    }

    /// T557: options and scripts that write a file or run a command are not read-only, even
    /// when the stem is; the common listing forms keep their key.
    #[test]
    fn side_effect_variants_of_read_only_stems_have_no_key() {
        let k = |c: &str| cache_key("Bash", &json!({"command": c}), None, None);
        for w in [
            "sed 'w output' input",
            "sed -n 'w output' input",
            "sed -e 'w output' input",
            "sed --expression='w output' input",
            "sed -ne 'w output' input",
            "sed 's/a/b/w output' input",
            "sed 's/a/b/e' input",
            "sed '1e touch x' input",
            "sed 'r other' input",
            "sed -f script input",
            "sed --file=script input",
            "sed --in-place s/a/b/ input",
            "sed 's/[/]/x/w out' input",
            "find . -fprint output",
            "find . -fprint0 output",
            "find . -fprintf output '%p'",
            "find . -fls output",
            "find . \"-fprint\" output",
            "find . -ok rm {} ;",
            "find . -okdir rm {} ;",
            "git diff --output=output",
            "git diff --output output",
            "git log --out=output",
            "git show '--output=output'",
        ] {
            assert!(k(w).is_none(), "{w}");
        }
        for r in [
            "sed -n 1,40p f",
            "sed -n '10,20p' f",
            "sed -n '/foo/p' f",
            "sed 's/a/b/g' f",
            "sed -E 's/(a|b)+/x/gI;5q' f",
            "sed -e 's/a/b/' -e '$d' f",
            "sed -ne '2p' f",
            "sed '/^#/d' f",
            "find . -name '*.rs' -type f",
            "find . -name x -print",
            "git diff --stat",
            "git log --oneline -5",
            "git diff --name-only HEAD~1",
        ] {
            assert!(k(r).is_some(), "{r}");
        }
    }

    /// T557 at the hook layer: a writer in disguise drops the earlier Read result and is not
    /// itself remembered, so repeating it executes again.
    #[test]
    fn a_disguised_writer_invalidates_an_earlier_read() {
        let sh = Shell::new();
        let read = json!({"file_path": native_abs("/r/input")});
        done(&sh.cx, "Read", &read);
        assert!(read_at(&sh.cx, &read));
        sh.ran("sed 'w output' input");
        assert!(!read_at(&sh.cx, &read), "the writer may have changed it");
        assert!(!sh.denied("sed 'w output' input"));
        sh.ran("sed -n 1,2p input");
        assert!(sh.denied("sed -n 1,2p input"));
    }

    /// T57.1 false-deny Check: a writer that shares a read-only stem must clear bash keys.
    #[test]
    fn find_delete_allows_the_next_ls() {
        let cx = setup();
        let g = GuardPlugin;
        let resp = json!({"stdout": "out"});
        let post = |cmd: &str| {
            let input = json!({"command": cmd});
            assert!(
                g.post_tool(
                    &PostToolUse {
                        tool_name: "Bash",
                        tool_input: &input,
                        tool_response: &resp,
                    },
                    &Ctx::new(&cx),
                )
                .is_none()
            );
        };
        let denied = |cmd: &str| {
            let input = json!({"command": cmd});
            matches!(
                g.pre_tool(
                    &PreToolUse {
                        tool_name: "Bash",
                        tool_input: &input,
                    },
                    &Ctx::new(&cx),
                ),
                Some(PreToolDecision::Deny { .. })
            )
        };
        post("ls");
        assert!(denied("ls"));
        post("find . -delete");
        assert!(
            !denied("ls"),
            "find -delete must drop bash keys so the next ls is allowed"
        );
    }

    #[test]
    fn mutating_bash_clears_then_allows_repeat_ls() {
        let cx = setup();
        let g = GuardPlugin;
        let ctx = || Ctx::new(&cx);
        let ls = json!({"command": "ls"});
        let ls_resp = json!({"stdout": "a"});
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Bash",
                    tool_input: &ls,
                    tool_response: &ls_resp,
                },
                &ctx(),
            )
            .is_none()
        );
        let mutating = json!({"command": "find . -delete"});
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Bash",
                    tool_input: &mutating,
                    tool_response: &json!({"stdout": ""}),
                },
                &ctx(),
            )
            .is_none()
        );
        assert!(
            g.pre_tool(
                &PreToolUse {
                    tool_name: "Bash",
                    tool_input: &ls,
                },
                &ctx(),
            )
            .is_none()
        );
    }

    /// T457: `rtok run --agent <id> -- '…'` used to leave the stem as `rtok`, so
    /// `cache_key` was `None` and PostToolUse cleared every `bash` and `read` key.
    #[test]
    fn agent_wrap_matches_the_inner_command_and_keeps_other_keys() {
        let cx = setup();
        let g = GuardPlugin;
        let path = json!({"file_path": native_abs("/proj/src/main.rs")});
        let read = PreToolUse {
            tool_name: "Read",
            tool_input: &path,
        };
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Read",
                    tool_input: &path,
                    tool_response: &json!({"content": "fn main() {}"}),
                },
                &Ctx::new(&cx),
            )
            .is_none()
        );
        let ls = json!({"command": "ls"});
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Bash",
                    tool_input: &ls,
                    tool_response: &json!({"stdout": "a\n"}),
                },
                &Ctx::new(&cx),
            )
            .is_none()
        );
        let quoted = format!(
            "rtok run --agent sub_1 -- {}",
            crate::plugins::cmd::run::sh_quote("git status")
        );
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Bash",
                    tool_input: &json!({"command": quoted}),
                    tool_response: &json!({"stdout": "clean"}),
                },
                &Ctx::new(&cx),
            )
            .is_none()
        );
        assert!(
            matches!(
                g.pre_tool(&read, &Ctx::new(&cx)),
                Some(PreToolDecision::Deny { .. })
            ),
            "a read-only sub-agent wrap must not drop read keys"
        );
        assert!(
            matches!(
                g.pre_tool(
                    &PreToolUse {
                        tool_name: "Bash",
                        tool_input: &ls,
                    },
                    &Ctx::new(&cx),
                ),
                Some(PreToolDecision::Deny { .. })
            ),
            "a read-only sub-agent wrap must not drop bash keys"
        );
        assert!(matches!(
            g.pre_tool(
                &PreToolUse {
                    tool_name: "Bash",
                    tool_input: &json!({"command": "git status"}),
                },
                &Ctx::new(&cx),
            ),
            Some(PreToolDecision::Deny { .. })
        ));
        // A writer behind the same wrap still clears, as an unwrapped writer does.
        let writer = format!(
            "rtok run --agent sub_1 -- {}",
            crate::plugins::cmd::run::sh_quote("echo x>f")
        );
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Bash",
                    tool_input: &json!({"command": writer}),
                    tool_response: &json!({"stdout": ""}),
                },
                &Ctx::new(&cx),
            )
            .is_none()
        );
        assert!(g.pre_tool(&read, &Ctx::new(&cx)).is_none());
    }

    #[test]
    fn wrapped_bash_post_matches_unwrapped_pre() {
        let cx = setup();
        let g = GuardPlugin;
        let quoted = format!(
            "rtok run -- {}",
            crate::plugins::cmd::run::sh_quote("git status")
        );
        let wrapped = json!({"command": quoted});
        let resp = json!({"stdout": "ok"});
        assert!(
            g.post_tool(
                &PostToolUse {
                    tool_name: "Bash",
                    tool_input: &wrapped,
                    tool_response: &resp,
                },
                &Ctx::new(&cx),
            )
            .is_none()
        );
        let pre = json!({"command": "git status"});
        assert!(matches!(
            g.pre_tool(
                &PreToolUse {
                    tool_name: "Bash",
                    tool_input: &pre,
                },
                &Ctx::new(&cx),
            ),
            Some(PreToolDecision::Deny { .. })
        ));
    }

    /// T350: `~/.rtok/errors.log` showed `PreToolUse panicked: start byte index N is not a
    /// char boundary` on `–…` / `⌘…` commands. Every input shape the guard inspects, with a
    /// multi-byte char at every offset a byte-sliced helper could cut at, must not panic.
    #[test]
    fn non_ascii_input_never_panics_in_pre_or_post_tool() {
        let cx = setup();
        let words = [
            "–",
            "⌘",
            "a–bc",
            "⌘ab",
            "x–",
            "–x",
            "é.exe",
            "ab–.exe",
            "cd –",
            "cd ⌘ && ls",
            "cd '⌘' && ls –",
            "'⌘'",
            "'–",
            "rtok run -- '⌘'",
            "rtok run -- –",
            "ls –a | ⌘c",
            "echo ⌘>–",
            "git ⌘",
            "/usr/bin/–",
            "\\⌘\\é",
        ];
        for w in words {
            let inputs = [
                ("Bash", json!({ "command": w })),
                ("Read", json!({ "file_path": w })),
                ("Read", json!({ "path": w, "offset": w })),
                ("Edit", json!({ "file_path": w })),
                ("Write", json!({ "path": w })),
                ("Grep", json!({ "pattern": w, "path": w })),
                ("Glob", json!({ "pattern": w })),
                ("Skill", json!({ "skill": w })),
                ("Skill", json!({ "skill": format!("{w}:{w}") })),
            ];
            for (tool, input) in &inputs {
                let _ = GuardPlugin.pre_tool(
                    &PreToolUse {
                        tool_name: tool,
                        tool_input: input,
                    },
                    &Ctx::new(&cx),
                );
                let _ = GuardPlugin.post_tool(
                    &PostToolUse {
                        tool_name: tool,
                        tool_input: input,
                        tool_response: &json!({ "content": w }),
                    },
                    &Ctx::new(&cx),
                );
            }
        }
    }
}

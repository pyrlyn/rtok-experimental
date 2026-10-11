// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `read` — MCP `read` / `search` / `tree` with modes, size caps and re-read dedup (plan P4).
//!
//! Spec: the catalogue in `plan.md` §1 names the tools this replaces; none is a
//! dependency (D6) — the behaviour is re-implemented here.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use rtok_plugin_sdk::{
    Class, Ctx, DashboardPage, Manifest, Measurement, Plugin, PostToolUse, PreToolDecision,
    PreToolUse, Surface, ToolDef,
};

// `ReadFs` and the lexical `normalize` live at crate level since T154: `project` needs them
// with this plugin compiled out.
pub use crate::fs;
use crate::fs::normalize;

pub mod cache;
pub mod hook;
pub(crate) mod outline;
pub mod roots;
pub mod search;
#[cfg(test)]
pub(crate) mod walk;

pub struct Read;

impl Plugin for Read {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "read",
            surfaces: &[Surface::Mcp, Surface::Hook],
            default_on: true,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "Read",
            "Outline, map, and search instead of dumping full files into context.",
            true,
        )
    }

    fn pre_tool(&self, ev: &PreToolUse, cx: &Ctx) -> Option<PreToolDecision> {
        hook::pre_tool(ev, cx)
    }

    fn post_tool(&self, ev: &PostToolUse, cx: &Ctx) -> Option<String> {
        cache::invalidate(ev, cx);
        None
    }

    fn mcp_tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "read",
                description: "Read a file; mode full|lines|map|signatures|diff|stripped; range a-b for full|lines|stripped.",
                input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"mode":{"type":"string"},"range":{"type":"string"}},"required":["path"]}),
            },
            ToolDef {
                name: "search",
                description: "Regex search files; path:line: snippet, max hits.",
                input_schema: json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"max":{"type":"integer"}},"required":["pattern"]}),
            },
            ToolDef {
                name: "tree",
                description: "Compact directory listing with sizes; depth cap.",
                input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"depth":{"type":"integer"}}}),
            },
        ]
    }
}

pub fn read(cx: &Ctx, path: &str, mode: &str, range: Option<&str>) -> Result<String> {
    let cwd = std::env::current_dir()?;
    read_with(cx, &fs::HostFs, &cwd, path, mode, range)
}

/// The file a Read/Edit/Write input names: Claude Code's `file_path`, or Copilot's
/// `path` (`read_file`/`view` are adapted to the tool name `Read` but keep their key).
pub(crate) fn path_arg(input: &Value) -> Option<&str> {
    input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(Value::as_str)
}

/// `read` over an arbitrary [`fs::ReadFs`] (host disk or [`crate::testutil::Vfs`]).
pub(crate) fn read_with(
    cx: &Ctx,
    fs: &impl fs::ReadFs,
    cwd: &Path,
    path: &str,
    mode: &str,
    range: Option<&str>,
) -> Result<String> {
    let cfg = cx.plugin_config::<crate::config::Read>("read");
    let abs = resolve_with(fs, cwd, Path::new(path), &cfg.allow_paths)?;
    // The bare io error (`No such file or directory (os error 2)`) never says which file; an
    // agent reading several paths in one turn cannot tell which one failed (T353).
    let raw = fs.read_to_string(&abs).map_err(|e| {
        let msg = e.to_string();
        let msg = msg.split(" (os error").next().unwrap_or(&msg);
        anyhow::anyhow!("{msg}: {path}")
    })?;
    let mode = if mode.is_empty() {
        cfg.default_mode.as_str()
    } else {
        mode
    };
    let force_delta = mode == "diff";
    let mode = if force_delta { "full" } else { mode };
    // T50.3: comments-stripped when a grammar exists; unknown language / parse fail → `full`.
    let stripped_src = if mode == "stripped" {
        outline::stripped(&abs, &raw)
    } else {
        None
    };
    let body = if mode == "map" || mode == "signatures" {
        outline::render(&abs, &raw, mode)?
    } else {
        // Same grammar as `expand --lines` (`a`, `a-b`, `a-`, `-b`); a malformed range is an
        // error, not the whole file. Outline modes carry their own line numbers per definition.
        let view = stripped_src.as_deref().unwrap_or(raw.as_str());
        let lines: Vec<&str> = view.lines().collect();
        let (a, b) = match range {
            Some(spec) => crate::expand::parse_range(spec, lines.len())?,
            None => (1, lines.len()),
        };
        crate::expand::slice_lines(lines, a, b)
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}:{l}", a + i))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let key = cache::key(abs.to_string_lossy().as_ref(), mode, range);
    // T122: a ranged read is keyed on the bytes it returns, never the whole file.
    let payload =
        if mode == "map" || mode == "signatures" || stripped_src.is_some() || range.is_some() {
            body.as_bytes()
        } else {
            raw.as_bytes()
        };
    if let Some(hit) = cache::hit(
        cx,
        &key,
        abs.to_string_lossy().as_ref(),
        payload,
        body.lines().count(),
        force_delta,
    ) {
        return Ok(hit);
    }
    // A same-session content-hash hit on those bytes is a pointer. T127: this call runs
    // only inside the MCP `read` tool (`src/mcp.rs`), which has no per-request signal for
    // which context — parent or sub-agent — is asking; `context` stays `None` here until
    // the creator picks between "no pointer on MCP" and today's session-only behaviour.
    if let Some(msg) = crate::plugin::identical_result(&**cx, "read", payload, None) {
        let _ = cache::remember(cx, &key, payload);
        return Ok(msg);
    }
    let _ = cache::remember(cx, &key, payload);
    let out = cap(cx, body)?;
    // T300: `map`/`signatures` measure the same way `stripped` already does — raw file vs
    // the text actually returned — but only on this fresh-render path; `cache::hit` and
    // `identical_result` above already recorded their own row (kind `dedup`/`delta`) and
    // returned early, so at most one row is written per call.
    let render_kind = match mode {
        "map" => Some("map"),
        "signatures" => Some("signatures"),
        _ if stripped_src.is_some() => Some("stripped"),
        _ => None,
    };
    if let Some(kind) = render_kind
        && (out.len() as u64) < (raw.len() as u64)
    {
        let _ = cx.record(&Measurement {
            plugin: "read",
            kind,
            before_bytes: raw.len() as u64,
            after_bytes: out.len() as u64,
            est_before: cx.estimate(&raw, Class::Code),
            est_after: cx.estimate(&out, Class::Code),
            ref_id: None,
            call_id: None,
        });
    }
    Ok(out)
}

pub(crate) fn resolve(cwd: &Path, path: &Path, extra: &[PathBuf]) -> Result<PathBuf> {
    resolve_with(&fs::HostFs, cwd, path, extra)
}

/// Resolve + allow-check using [`fs::ReadFs::canonicalize`] (symlink follow on host / Vfs).
pub(crate) fn resolve_with(
    fs: &impl fs::ReadFs,
    cwd: &Path,
    path: &Path,
    extra: &[PathBuf],
) -> Result<PathBuf> {
    // Canonicalise roots first. Relative paths are then joined onto the
    // canonical cwd so a missing file (canonicalize fails) still shares the
    // same case as the root `under` compares against.
    let roots: Vec<PathBuf> = std::iter::once(cwd.to_path_buf())
        .chain(extra.iter().cloned())
        .map(|r| fs.canonicalize(&r).unwrap_or(r))
        .collect();
    let abs = if path.is_absolute() {
        normalize(Path::new("/"), path)
    } else {
        normalize(&roots[0], path)
    };
    // Lexical allow, then (when the path exists) reject symlink escapes past the root.
    let check = fs.canonicalize(&abs).unwrap_or_else(|| abs.clone());
    if roots.iter().any(|r| under(&check, r)) {
        return Ok(abs);
    }
    // T351 (`rtok mcp` only): sibling worktrees and client roots, looked up after the cheap
    // checks failed so an ordinary in-cwd call never pays for them.
    if roots::dynamic(cwd)
        .iter()
        .any(|r| under(&check, &fs.canonicalize(r).unwrap_or_else(|| r.clone())))
    {
        return Ok(abs);
    }
    bail!(
        "path outside cwd: {} (outside this project and its worktrees; read it with the host's own file tool)",
        path.display()
    )
}

/// `Path::starts_with("")` is true for every path, so an empty root (`allow_paths = [""]`)
/// would open the whole filesystem; it grants nothing instead.
///
/// On Windows, `Path::starts_with` is case-sensitive even though the filesystem
/// is not. When canonicalize succeeds on the root but fails on a missing path
/// (or the other way around), the two sides can differ only in ASCII case and
/// a lexical `starts_with` wrongly rejects an in-tree file.
fn under(path: &Path, root: &Path) -> bool {
    crate::fs::path_starts_with(path, root)
}

/// T263: refuse to walk `/` or the home directory (Claude.app launches `rtok mcp` in `/`);
/// such a walk times out instead of answering. T356: graph indexing uses it too.
pub(crate) fn walk_root_ok(root: &Path) -> Result<()> {
    if crate::fs::is_unwalkable_root(root, std::env::home_dir().as_deref()) {
        let root = crate::fs::canon(root);
        bail!(
            "no project root: rtok mcp runs in {}; pass an absolute path inside the repository or start rtok mcp there",
            root.display()
        );
    }
    Ok(())
}

/// Shared body of [`cap`] / [`cap_recording`]: cap at `plugins.read.max_chars`, archiving an
/// oversized text so the cut carries its id. `kind` is `Some` only for callers whose entire
/// "before" is this cap (`search`/`tree` have no other honest before), so a `Measurement` row
/// is written exactly when the cap actually cuts something.
fn cap_impl(cx: &Ctx, text: String, kind: Option<&'static str>) -> Result<String> {
    let max = cx.plugin_config::<crate::config::Read>("read").max_chars as usize;
    if text.chars().count() <= max {
        return Ok(text);
    }
    let id = cx.put_archive(text.as_bytes())?;
    let out = crate::expand::cut(&text, &format!("\n… archived {id} …\n"), max);
    if let Some(kind) = kind {
        let _ = cx.record(&Measurement {
            plugin: "read",
            kind,
            before_bytes: text.len() as u64,
            after_bytes: out.len() as u64,
            est_before: cx.estimate(&text, Class::Code),
            est_after: cx.estimate(&out, Class::Code),
            ref_id: Some(id),
            call_id: None,
        });
    }
    Ok(out)
}

/// Cap at `plugins.read.max_chars`; an oversized text is archived and the cut carries its id.
pub(crate) fn cap(cx: &Ctx, text: String) -> Result<String> {
    cap_impl(cx, text, None)
}

/// `cap`, plus a `Measurement` row (`kind`) when it actually truncated. For `search`/`tree`:
/// there is no honest "before" beyond their own full output, so nothing is recorded unless
/// `cap` cut something.
pub(crate) fn cap_recording(cx: &Ctx, text: String, kind: &'static str) -> Result<String> {
    cap_impl(cx, text, Some(kind))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;

    /// A runtime whose fresh temp dir is also the one `allow_paths` root; shared with `cache.rs`.
    pub(crate) fn cx(name: &str) -> (crate::plugin::Runtime, PathBuf) {
        let (mut c, dir) = crate::testutil::config(name);
        c.plugins.read.allow_paths = vec![dir.clone()];
        (crate::plugin::Runtime::open(c, name).unwrap(), dir)
    }

    // Disk e2e kept; Vfs twins below call `read_with` / `resolve_with` (T56.5 ReadFs).
    #[test]
    fn three_lines_are_numbered() {
        let (cx, dir) = cx("three");
        let p = dir.join("a.txt");
        fs::write(&p, "alpha\nbeta\ngamma\n").unwrap();
        let out = read(&Ctx::new(&cx), p.to_str().unwrap(), "full", None).unwrap();
        assert_eq!(out, "1:alpha\n2:beta\n3:gamma");
        let _ = fs::remove_dir_all(dir);
    }

    /// `range` follows `expand --lines` in every line mode; a malformed one is an error.
    #[test]
    fn range_applies_to_full_and_lines_and_rejects_junk() {
        let (cx, dir) = cx("range");
        let p = dir.join("r.txt");
        fs::write(&p, "a\nb\nc\nd\n").unwrap();
        let path = p.to_str().unwrap();
        let cx = Ctx::new(&cx);
        assert_eq!(read(&cx, path, "full", Some("2-3")).unwrap(), "2:b\n3:c");
        assert_eq!(read(&cx, path, "lines", Some("3")).unwrap(), "3:c\n4:d");
        assert_eq!(read(&cx, path, "lines", Some("-1")).unwrap(), "1:a");
        assert!(read(&cx, path, "lines", Some("x-y")).is_err());
        assert!(read(&cx, path, "full", Some("3-2")).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    /// A missing file or a directory fails naming the path, without the `(os error N)` tail.
    #[test]
    fn file_errors_name_the_path() {
        let (cx, dir) = cx("file-errors");
        let cx = Ctx::new(&cx);
        // Canonical: a missing path is only matched lexically against the canonical root.
        let canon = dir.canonicalize().unwrap();
        let missing = canon.join("nope.rs");
        let missing = missing.to_str().unwrap();
        // The io wording is the OS's own (Windows: "The system cannot find …", "Access is
        // denied."), so only the path suffix and the dropped tail are asserted.
        let err = read(&cx, missing, "full", None).unwrap_err().to_string();
        assert!(err.ends_with(&format!(": {missing}")), "{err}");
        assert!(!err.contains("os error"), "{err}");
        let sub = canon.join("sub");
        fs::create_dir(&sub).unwrap();
        let sub = sub.to_str().unwrap();
        let err = read(&cx, sub, "map", None).unwrap_err().to_string();
        assert!(err.ends_with(&format!(": {sub}")), "{err}");
        assert!(!err.contains("os error"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }

    #[rstest]
    fn hundred_kb_is_capped_with_archive_id() {
        let (cx, dir) = cx("big");
        let max = cx.config.plugins.read.max_chars as usize;
        let p = dir.join("big.txt");
        let blob = "x".repeat(100 * 1024);
        fs::write(&p, &blob).unwrap();
        let out = read(&Ctx::new(&cx), p.to_str().unwrap(), "full", None).unwrap();
        assert!(out.contains("archived"), "{out}");
        assert!(
            out.chars().count() <= max,
            "cap includes marker: {}/{}",
            out.chars().count(),
            max
        );
        assert!(out.chars().count() < blob.len(), "capped");
        let _ = fs::remove_dir_all(dir);
    }

    fn archive_id(out: &str) -> &str {
        const PRE: &str = "… archived ";
        const SUF: &str = " …";
        let start = out.find(PRE).expect("archive marker") + PRE.len();
        let rest = &out[start..];
        &rest[..rest.find(SUF).expect("archive marker end")]
    }

    #[rstest]
    #[case(500)]
    #[case(200)]
    fn cap_includes_marker_in_max_chars(#[case] max_chars: u32) {
        let (mut c, dir) = crate::testutil::config("cap-marker");
        c.plugins.read.allow_paths = vec![dir.clone()];
        c.plugins.read.max_chars = max_chars;
        let cx = crate::plugin::Runtime::open(c, "cap-marker").unwrap();
        let blob = "abcdefghij".repeat(200);
        let p = dir.join("cap.txt");
        fs::write(&p, &blob).unwrap();
        let expected = format!("1:{blob}");
        let out = read(&Ctx::new(&cx), p.to_str().unwrap(), "full", None).unwrap();
        let max = max_chars as usize;
        assert!(
            out.chars().count() <= max,
            "output {} chars exceeds max {}",
            out.chars().count(),
            max
        );
        let id = archive_id(&out);
        let archived = String::from_utf8(cx.store.get_archive(id, None).unwrap().unwrap()).unwrap();
        assert_eq!(archived, expected);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn symlink_escape_is_err() {
        let (_cx, dir) = cx("symlink");
        let cwd = dir.join("cwd");
        fs::create_dir_all(&cwd).unwrap();
        // Target lives beside the allow_paths root, not under it.
        let outside = dir
            .parent()
            .unwrap()
            .join(format!("rtok-read-symlink-out-{}", std::process::id()));
        fs::write(&outside, "secret\n").unwrap();
        #[cfg(unix)]
        {
            let link = cwd.join("escape");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            // `read` passes the process cwd to `resolve`; calling `resolve` with `cwd` and
            // no allow_paths checks the same guard without moving the cwd every parallel
            // test shares (moving it failed `map_src_main_lists_fn_main` on ubuntu CI,
            // 2026-09-11).
            let err = resolve(&cwd, Path::new("escape"), &[])
                .unwrap_err()
                .to_string();
            let _ = fs::remove_file(&outside);
            assert!(err.contains("outside cwd"), "{err}");
        }
        #[cfg(not(unix))]
        let _ = fs::remove_file(&outside);
        let _ = fs::remove_dir_all(dir);
    }

    /// T263: `/` is refused; an ordinary project directory is not.
    #[test]
    fn walk_root_ok_rejects_filesystem_root_and_accepts_a_project_dir() {
        assert!(walk_root_ok(Path::new("/")).is_err());
        let dir = std::env::temp_dir().join(format!("rtok-walk-root-ok-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert!(walk_root_ok(&dir).is_ok());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn normalize_absolute_unix_path_still_roots() {
        let got = normalize(Path::new("/unused"), Path::new("/tmp/a/../b"));
        assert_eq!(got, PathBuf::from("/tmp/b"));
    }

    #[cfg(windows)]
    #[test]
    fn normalize_keeps_windows_drive_prefix() {
        let got = normalize(Path::new(r"C:\unused"), Path::new(r"D:\Users\x\..\y"));
        assert_eq!(got, PathBuf::from(r"D:\Users\y"));
    }

    /// The lexical twin of `symlink_escape_is_err`: `..` climbs past every allowed root.
    #[test]
    fn dot_dot_escape_is_err() {
        let (cx, dir) = cx("dotdot");
        let deep = "../".repeat(32) + "etc/passwd";
        assert!(read(&Ctx::new(&cx), &deep, "full", None).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    /// T529: agents asked `read` for a host's own skill and instruction files; a bare refusal
    /// left them guessing, so it names the way out and keeps the prefix callers match on.
    #[test]
    fn a_refusal_points_to_the_host_s_own_file_tool() {
        let err = resolve(Path::new("/nonexistent-cwd"), Path::new("/etc/hosts"), &[])
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("path outside cwd: /etc/hosts"), "{err}");
        assert!(err.contains("host's own file tool"), "{err}");
    }

    /// `RTOK_PLUGINS_READ_ALLOW_PATHS=` once became `[""]`, and `starts_with("")` is always true.
    #[test]
    fn empty_allow_root_grants_nothing() {
        let err = resolve(
            Path::new("/nonexistent-cwd"),
            Path::new("/etc/passwd"),
            &[PathBuf::new()],
        );
        assert!(err.is_err(), "{err:?}");
    }

    /// Missing file + canonical root with different ASCII case: `Path::starts_with`
    /// alone rejects; resolve must still allow (Windows CI path; also covers the
    /// relative rebase onto a canonical cwd on every host).
    #[test]
    fn resolve_allows_missing_path_when_root_case_differs() {
        let base = std::env::temp_dir().join(format!("rtok-read-case-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let proj = base.join("Proj");
        fs::create_dir_all(&proj).unwrap();
        let disk = dunce::canonicalize(&proj).unwrap();
        let mut alt = disk.clone();
        if let Some(name) = alt.file_name().map(|n| n.to_string_lossy().into_owned()) {
            let flipped: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_uppercase() {
                        c.to_ascii_lowercase()
                    } else if c.is_ascii_lowercase() {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    }
                })
                .collect();
            alt.pop();
            alt.push(flipped);
        }
        // Relative path through the alt-cased cwd string (canonicalize of missing fails).
        let got = resolve(&alt, Path::new("missing.txt"), &[]).expect("under alt cwd");
        assert!(got.ends_with("missing.txt"), "{got:?}");
        // Absolute path with alt case against a canonical root via allow_paths.
        let abs_missing = alt.join("also-missing.txt");
        let got2 = resolve(
            Path::new("/nonexistent-cwd-rtok"),
            &abs_missing,
            std::slice::from_ref(&disk),
        );
        if cfg!(windows) {
            assert!(got2.is_ok(), "{got2:?}");
        } else {
            // On case-sensitive Linux the alt path is a different directory; on
            // macOS the relative rebase above already covers the asymmetry.
            let _ = got2;
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(windows)]
    #[test]
    fn under_ascii_case_insensitive_matches_windows_prefix() {
        let path = Path::new(r"\\?\C:\Users\Me\proj\file.txt");
        let root = Path::new(r"c:\users\me\proj");
        assert!(under(path, root));
        assert!(!under(path, Path::new(r"c:\users\me\project")));
    }

    /// T56.2: line numbering / range from Vfs bytes — same grammar as `read` full|lines, no host disk.
    fn numbered_from_vfs(raw: &str, range: Option<&str>) -> String {
        let lines: Vec<&str> = raw.lines().collect();
        let (a, b) = match range {
            Some(spec) => crate::expand::parse_range(spec, lines.len()).unwrap(),
            None => (1, lines.len()),
        };
        crate::expand::slice_lines(lines, a, b)
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}:{l}", a + i))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn three_lines_numbered_from_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("a.txt", b"alpha\nbeta\ngamma\n");
        let out = numbered_from_vfs(vfs.read_str("a.txt").unwrap(), None);
        assert_eq!(out, "1:alpha\n2:beta\n3:gamma");
    }

    #[test]
    fn range_from_vfs_matches_expand_grammar() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("r.txt", b"a\nb\nc\nd\n");
        let raw = vfs.read_str("r.txt").unwrap();
        assert_eq!(numbered_from_vfs(raw, Some("2-3")), "2:b\n3:c");
        assert_eq!(numbered_from_vfs(raw, Some("3")), "3:c\n4:d");
        assert_eq!(numbered_from_vfs(raw, Some("-1")), "1:a");
    }

    #[test]
    fn range_from_vfs_rejects_junk_like_disk_twin() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("r.txt", b"a\nb\nc\nd\n");
        let raw = vfs.read_str("r.txt").unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert!(crate::expand::parse_range("x-y", lines.len()).is_err());
        assert!(crate::expand::parse_range("3-2", lines.len()).is_err());
    }

    #[test]
    fn empty_file_from_vfs_numbers_nothing() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("empty.txt", b"");
        assert_eq!(
            numbered_from_vfs(vfs.read_str("empty.txt").unwrap(), None),
            ""
        );
    }

    #[test]
    fn spaced_path_content_from_vfs_numbers_lines() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("My Docs/notes.txt", b"one\ntwo\n");
        let out = numbered_from_vfs(vfs.read_str("My Docs/notes.txt").unwrap(), Some("1-2"));
        assert_eq!(out, "1:one\n2:two");
    }

    /// T56.5: full `read_with` on Vfs — disk `three_lines_are_numbered` twin (kept).
    #[test]
    fn three_lines_are_numbered_via_read_fs_vfs() {
        let (mut c, dir) = crate::testutil::config("three-vfs");
        c.plugins.read.allow_paths = vec![PathBuf::from("ws")];
        let cx = crate::plugin::Runtime::open(c, "three-vfs").unwrap();
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("ws/a.txt", b"alpha\nbeta\ngamma\n");
        let out = read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None).unwrap();
        assert_eq!(out, "1:alpha\n2:beta\n3:gamma");
        let _ = fs::remove_dir_all(dir);
    }

    /// T56.5: range grammar through `read_with` + Vfs (disk twin kept).
    #[test]
    fn range_applies_via_read_fs_vfs() {
        let (mut c, dir) = crate::testutil::config("range-vfs");
        c.plugins.read.allow_paths = vec![PathBuf::from("ws")];
        let cx = crate::plugin::Runtime::open(c, "range-vfs").unwrap();
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("ws/r.txt", b"a\nb\nc\nd\n");
        let ctx = Ctx::new(&cx);
        assert_eq!(
            read_with(&ctx, &vfs, Path::new("ws"), "r.txt", "full", Some("2-3")).unwrap(),
            "2:b\n3:c"
        );
        assert_eq!(
            read_with(&ctx, &vfs, Path::new("ws"), "r.txt", "lines", Some("3")).unwrap(),
            "3:c\n4:d"
        );
        assert_eq!(
            read_with(&ctx, &vfs, Path::new("ws"), "r.txt", "lines", Some("-1")).unwrap(),
            "1:a"
        );
        assert!(read_with(&ctx, &vfs, Path::new("ws"), "r.txt", "lines", Some("x-y")).is_err());
        assert!(read_with(&ctx, &vfs, Path::new("ws"), "r.txt", "full", Some("3-2")).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    /// T56.5: cap + archive id through `read_with` + Vfs (disk twin kept).
    #[test]
    fn hundred_kb_is_capped_via_read_fs_vfs() {
        let (mut c, dir) = crate::testutil::config("big-vfs");
        c.plugins.read.allow_paths = vec![PathBuf::from("ws")];
        let cx = crate::plugin::Runtime::open(c, "big-vfs").unwrap();
        let max = cx.config.plugins.read.max_chars as usize;
        let mut vfs = crate::testutil::Vfs::new();
        let blob = "x".repeat(100 * 1024);
        vfs.write("ws/big.txt", blob.as_bytes());
        let out = read_with(
            &Ctx::new(&cx),
            &vfs,
            Path::new("ws"),
            "big.txt",
            "full",
            None,
        )
        .unwrap();
        assert!(out.contains("archived"), "{out}");
        assert!(
            out.chars().count() <= max,
            "cap includes marker: {}/{}",
            out.chars().count(),
            max
        );
        assert!(out.chars().count() < blob.len(), "capped");
        let _ = fs::remove_dir_all(dir);
    }

    #[rstest]
    #[case(500)]
    #[case(200)]
    fn cap_includes_marker_via_read_fs_vfs(#[case] max_chars: u32) {
        let (mut c, dir) = crate::testutil::config("cap-marker-vfs");
        c.plugins.read.allow_paths = vec![PathBuf::from("ws")];
        c.plugins.read.max_chars = max_chars;
        let cx = crate::plugin::Runtime::open(c, "cap-marker-vfs").unwrap();
        let blob = "abcdefghij".repeat(200);
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("ws/cap.txt", blob.as_bytes());
        let expected = format!("1:{blob}");
        let out = read_with(
            &Ctx::new(&cx),
            &vfs,
            Path::new("ws"),
            "cap.txt",
            "full",
            None,
        )
        .unwrap();
        let max = max_chars as usize;
        assert!(
            out.chars().count() <= max,
            "output {} chars exceeds max {}",
            out.chars().count(),
            max
        );
        let id = archive_id(&out);
        let archived = String::from_utf8(cx.store.get_archive(id, None).unwrap().unwrap()).unwrap();
        assert_eq!(archived, expected);
        let _ = fs::remove_dir_all(dir);
    }

    /// T56.5: symlink escape via `resolve_with` + Vfs (disk twin kept).
    #[test]
    fn symlink_escape_via_read_fs_vfs() {
        let mut vfs = crate::testutil::Vfs::new();
        vfs.write("/outside/secret", b"secret\n");
        vfs.symlink("ws/escape", "/outside/secret");
        let err = resolve_with(&vfs, Path::new("ws"), Path::new("escape"), &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("outside cwd"), "{err}");
    }

    #[test]
    fn map_src_main_lists_fn_main() {
        let (cx, dir) = crate::testutil::runtime("mapmain");
        let out = read(&Ctx::new(&cx), "src/main.rs", "map", None).unwrap();
        assert!(out.contains("fn main"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn stripped_drops_comments_keeps_body_and_unknown_is_full() {
        let (cx, dir) = cx("stripped");
        let rs = dir.join("a.rs");
        fs::write(&rs, "/// gone\nfn keep() {}\n").unwrap();
        let out = read(&Ctx::new(&cx), rs.to_str().unwrap(), "stripped", None).unwrap();
        assert!(!out.contains("gone"), "{out}");
        assert!(out.contains("fn keep"), "{out}");
        assert!(cx.store.measurement_count("read").unwrap() >= 1);
        let txt = dir.join("a.txt");
        fs::write(&txt, "// gone\nkeep\n").unwrap();
        let out = read(&Ctx::new(&cx), txt.to_str().unwrap(), "stripped", None).unwrap();
        assert!(out.contains("gone"), "{out}");
        assert!(out.contains("keep"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T122: a whole-file archive (the native Read hook's) never answers a ranged read.
    #[test]
    fn ranged_read_is_not_a_pointer_to_the_whole_file() {
        let (cx, dir) = cx("t122_ranges");
        let file = dir.join("multiline.txt");
        let content: String = (1..=100).map(|i| format!("Line {i}\n")).collect();
        fs::write(&file, &content).unwrap();
        cx.store
            .put_archive(&cx.session, content.as_bytes(), &cx.config.core.archive_dir)
            .unwrap();
        let path = file.to_str().unwrap();
        let out1 = read(&Ctx::new(&cx), path, "lines", Some("10-20")).unwrap();
        let out2 = read(&Ctx::new(&cx), path, "lines", Some("30-40")).unwrap();
        assert!(!out1.contains("identical to"), "{out1}");
        assert!(
            out1.contains("10:Line 10") && !out1.contains("Line 30"),
            "{out1}"
        );
        assert!(
            out2.contains("30:Line 30") && !out2.contains("Line 10"),
            "{out2}"
        );
        let _ = fs::remove_dir_all(dir);
    }
}

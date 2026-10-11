// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Re-read dedup (plan T4.4): same session/path/mode/range + sha256 → short hit.

use std::path::Path;

use rtok_plugin_sdk::{Class, Ctx, Measurement, PostToolUse};

pub fn key(path: &str, mode: &str, range: Option<&str>) -> String {
    format!("{path}\t{mode}\t{}", range.unwrap_or(""))
}

/// Unchanged re-read → one line; changed re-read → unified diff when it is below
/// `read.delta_max_ratio` of the file (T58.1). `force_delta` is MCP `mode=diff`.
pub fn hit(
    cx: &Ctx,
    key: &str,
    path: &str,
    body: &[u8],
    lines: usize,
    force_delta: bool,
) -> Option<String> {
    let (Some(id), _) = cx.get_read_cache(key).ok().flatten()? else {
        return None;
    };
    let prev = cx.get_archive(&id).ok().flatten()?;
    if prev.as_slice() == body {
        return Some(dedup_line(cx, &id, body, lines));
    }
    let cfg = cx.plugin_config::<crate::config::Read>("read");
    if !cfg.delta && !force_delta {
        return None;
    }
    let before = std::str::from_utf8(&prev).ok()?;
    let after = std::str::from_utf8(body).ok()?;
    let diff = change_preview::unified_diff(Path::new(path), before, after);
    if diff.is_empty() || body.is_empty() {
        return None;
    }
    let ratio = diff.len() as f32 / body.len() as f32;
    if ratio >= cfg.delta_max_ratio {
        return None;
    }
    let new_id = remember(cx, key, body).ok()?;
    let mut msg = format!("{diff}previous {id}\nexpand {new_id}");
    let max = cfg.max_chars as usize;
    if msg.chars().count() > max {
        msg = crate::expand::cut(&msg, &format!("\n… archived {new_id} …\n"), max);
    }
    let _ = cx.record(&Measurement {
        plugin: "read",
        kind: "delta",
        before_bytes: body.len() as u64,
        after_bytes: msg.len() as u64,
        est_before: cx.estimate(after, Class::Code),
        est_after: cx.estimate(&msg, Class::Code),
        ref_id: Some(new_id),
        call_id: None,
    });
    Some(msg)
}

fn dedup_line(cx: &Ctx, id: &str, body: &[u8], lines: usize) -> String {
    let msg = format!("unchanged since {id} ({lines} lines)");
    let msg = if msg.len() < 80 {
        msg
    } else {
        format!("unchanged since {:.8} ({lines} lines)", id)
    };
    let _ = cx.record(&Measurement {
        plugin: "read",
        kind: "dedup",
        before_bytes: body.len() as u64,
        after_bytes: msg.len() as u64,
        est_before: cx.estimate(std::str::from_utf8(body).unwrap_or(""), Class::Code),
        est_after: cx.estimate(&msg, Class::Code),
        ref_id: Some(id.to_string()),
        call_id: None,
    });
    msg
}

pub fn remember(cx: &Ctx, key: &str, body: &[u8]) -> anyhow::Result<String> {
    let id = cx.put_archive(body)?;
    cx.put_read_cache(key, &hex_sha256(body), Some(&id))?;
    Ok(id)
}

pub fn invalidate(ev: &PostToolUse<'_>, cx: &Ctx) {
    if cx.plugin_config::<crate::config::Read>("read").delta {
        return;
    }
    if ev.tool_name != "Edit" && ev.tool_name != "Write" {
        return;
    }
    let Some(path) = super::path_arg(ev.tool_input) else {
        return;
    };
    let _ = cx.clear_read_cache(path);
    let _ = cx.clear_read_cache(&format!("read\t{path}"));
}

fn hex_sha256(bytes: &[u8]) -> String {
    crate::store::hex_sha256(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::read::read;
    use crate::plugins::read::read_with;
    use crate::plugins::read::tests::cx;
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// T56.2: runtime + in-memory workspace for the dedup twins — the store stays real
    /// (it is not a filesystem), only the read paths run on `Vfs` via `read_with`.
    fn vfs_cx(name: &str) -> (crate::plugin::Runtime, crate::testutil::Vfs, PathBuf) {
        let (mut c, dir) = crate::testutil::config(name);
        c.plugins.read.allow_paths = vec![PathBuf::from("ws")];
        let cx = crate::plugin::Runtime::open(c, name).unwrap();
        (cx, crate::testutil::Vfs::new(), dir)
    }

    #[test]
    fn two_identical_reads_second_is_short() {
        let (cx, dir) = cx("same");
        let p = dir.join("a.txt");
        fs::write(&p, "alpha\nbeta\n").unwrap();
        let path = p.to_str().unwrap();
        let first = read(&Ctx::new(&cx), path, "full", None).unwrap();
        assert!(first.contains("1:alpha"), "{first}");
        let second = read(&Ctx::new(&cx), path, "full", None).unwrap();
        assert!(second.len() < 80, "{second}");
        assert!(second.contains("unchanged"), "{second}");
        assert!(cx.store.measurement_count("read").unwrap() >= 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn edit_fixture_between_reads_is_full() {
        let (cx, dir) = cx("edit");
        let p = dir.join("a.txt");
        fs::write(&p, "alpha\n").unwrap();
        let path = p.to_str().unwrap();
        let _ = read(&Ctx::new(&cx), path, "full", None).unwrap();
        fs::write(&p, "omega\n").unwrap();
        let second = read(&Ctx::new(&cx), path, "full", None).unwrap();
        assert!(second.contains("1:omega"), "{second}");
        assert!(!second.contains("unchanged"), "{second}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn post_tool_edit_clears_hit() {
        let (cx, dir) = cx("hook");
        let p = dir.join("a.txt");
        fs::write(&p, "alpha\n").unwrap();
        let path = p.to_str().unwrap();
        let _ = read(&Ctx::new(&cx), path, "full", None).unwrap();
        let input = json!({"file_path": path});
        invalidate(
            &PostToolUse {
                tool_name: "Edit",
                tool_input: &input,
                tool_response: &json!({}),
            },
            &Ctx::new(&cx),
        );
        let second = read(&Ctx::new(&cx), path, "full", None).unwrap();
        assert!(second.contains("unchanged"), "{second}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T56.2: identical re-read through `read_with` + Vfs is the short hit (disk twin kept).
    #[test]
    fn two_identical_reads_second_is_short_from_vfs() {
        let (cx, mut vfs, dir) = vfs_cx("same-vfs");
        vfs.write("ws/a.txt", b"alpha\nbeta\n");
        let first =
            read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None).unwrap();
        assert!(first.contains("1:alpha"), "{first}");
        let second =
            read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None).unwrap();
        assert!(second.len() < 80, "{second}");
        assert!(second.contains("unchanged"), "{second}");
        assert!(cx.store.measurement_count("read").unwrap() >= 1);
        let _ = fs::remove_dir_all(dir);
    }

    /// T56.2: an edited file between reads is the full body again (disk twin kept).
    #[test]
    fn edit_fixture_between_reads_is_full_from_vfs() {
        let (cx, mut vfs, dir) = vfs_cx("edit-vfs");
        vfs.write("ws/a.txt", b"alpha\n");
        let _ = read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None);
        vfs.write("ws/a.txt", b"omega\n");
        let second =
            read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None).unwrap();
        assert!(second.contains("1:omega"), "{second}");
        assert!(!second.contains("unchanged"), "{second}");
        let _ = fs::remove_dir_all(dir);
    }

    /// T56.2: an `Edit` clears the hit under the same key `read_with` stored (disk twin kept).
    #[test]
    fn post_tool_edit_clears_hit_from_vfs() {
        let (cx, mut vfs, dir) = vfs_cx("hook-vfs");
        vfs.write("ws/a.txt", b"alpha\n");
        let _ = read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None);
        let input = json!({"file_path": "ws/a.txt"});
        invalidate(
            &PostToolUse {
                tool_name: "Edit",
                tool_input: &input,
                tool_response: &json!({}),
            },
            &Ctx::new(&cx),
        );
        let second =
            read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None).unwrap();
        assert!(second.contains("unchanged"), "{second}");
        let _ = fs::remove_dir_all(dir);
    }

    fn many_lines(changed: Option<usize>) -> String {
        (0..80)
            .map(|i| {
                if changed == Some(i) {
                    format!("chg{i}\n")
                } else {
                    format!("line{i}\n")
                }
            })
            .collect()
    }

    #[test]
    fn vfs_small_change_is_hunks_large_is_full_missing_archive_is_full() {
        let (cx, mut vfs, dir) = vfs_cx("delta-vfs");
        vfs.write("ws/a.txt", many_lines(None).as_bytes());
        let _ = read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None);
        vfs.write("ws/a.txt", many_lines(Some(3)).as_bytes());
        let small =
            read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None).unwrap();
        assert!(small.contains("@@"), "{small}");
        assert!(small.contains("+chg3"), "{small}");
        assert!(small.contains("expand "), "{small}");
        assert!(cx.store.measurement_count("read").unwrap() >= 1);

        let (cx, mut vfs, dir2) = vfs_cx("delta-large");
        let old: String = (0..40).map(|i| format!("old{i}\n")).collect();
        let new: String = (0..40).map(|i| format!("new{i}\n")).collect();
        vfs.write("ws/b.txt", old.as_bytes());
        let _ = read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "b.txt", "full", None);
        vfs.write("ws/b.txt", new.as_bytes());
        let large =
            read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "b.txt", "full", None).unwrap();
        assert!(large.contains("1:new0"), "{large}");
        assert!(!large.contains("previous "), "{large}");

        let (cx, mut vfs, dir3) = vfs_cx("delta-miss");
        vfs.write("ws/c.txt", b"keep\n");
        let _ = read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "c.txt", "full", None);
        // The key `read_with` builds: its resolved path, `ws\c.txt` on Windows (T83.12).
        let abs =
            crate::plugins::read::resolve_with(&vfs, Path::new("ws"), Path::new("c.txt"), &[])
                .unwrap();
        let key = key(abs.to_string_lossy().as_ref(), "full", None);
        Ctx::new(&cx)
            .put_read_cache(&key, "dead", Some("no-such-id"))
            .unwrap();
        let miss = read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "c.txt", "full", None).unwrap();
        assert_eq!(miss, "1:keep");
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(dir2);
        let _ = fs::remove_dir_all(dir3);
    }

    #[test]
    fn vfs_crlf_delta_archives_crlf_and_unchanged_is_at_most_13_tokens() {
        let (cx, mut vfs, dir) = vfs_cx("delta-crlf");
        let old: String = (0..80).map(|i| format!("line{i}\r\n")).collect();
        let new: String = (0..80)
            .map(|i| {
                if i == 3 {
                    "chg3\r\n".to_string()
                } else {
                    format!("line{i}\r\n")
                }
            })
            .collect();
        vfs.write("ws/a.txt", old.as_bytes());
        let first =
            read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None).unwrap();
        vfs.write("ws/a.txt", new.as_bytes());
        let second =
            read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None).unwrap();
        assert!(second.contains("+chg3"), "{second}");
        let id = second
            .rsplit("expand ")
            .next()
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        let archived = cx.store.get_archive(id, None).unwrap().unwrap();
        assert_eq!(archived, new.as_bytes());
        let third =
            read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None).unwrap();
        assert!(third.contains("unchanged"), "{third}");
        let est = Ctx::new(&cx).estimate(&third, Class::Code);
        assert!(est <= 13, "{est} tokens for {third}");
        let again =
            read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None).unwrap();
        assert_eq!(again, third, "byte-stable unchanged line");
        let _ = first;
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mode_diff_reads_the_full_cache() {
        let (cx, mut vfs, dir) = vfs_cx("mode-diff");
        vfs.write("ws/a.txt", many_lines(None).as_bytes());
        let _ = read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "full", None);
        vfs.write("ws/a.txt", many_lines(Some(3)).as_bytes());
        let out = read_with(&Ctx::new(&cx), &vfs, Path::new("ws"), "a.txt", "diff", None).unwrap();
        assert!(out.contains("+chg3"), "{out}");
        let _ = fs::remove_dir_all(dir);
    }
}

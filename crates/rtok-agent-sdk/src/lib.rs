//! What every `rtok agents install <host>` installer does, once.
//!
//! An agent host — Claude Code, Cursor, Codex, OpenCode, pi — is a config file rtok edits and,
//! for some of them, a plugin directory rtok installs. The *shapes* differ (JSON hooks, a TOML
//! `[mcp_servers]` table, an `env` map, a symlink or a copy); the contract around them does not:
//!
//! - **Reversible.** Every file rtok touches is copied to `<name>.bak-<unix-seconds>` first, so
//!   one `.bak-*` per file is the whole undo.
//! - **Idempotent.** A second apply reports [`NO_CHANGES`] and writes nothing.
//! - **Dry-runnable.** `--dry-run` produces the same report and touches nothing at all.
//! - **Offered, never forced.** A host plugin is a prompt on a terminal, a no anywhere else
//!   (CI, a pipe, a host running setup for the user) unless `--yes` says otherwise.
//!
//! Every installer returns one report line per change. The report is also the write gate:
//! [`Apply`] refuses to write when the report is [`NO_CHANGES`], so "we said nothing changed"
//! and "we changed nothing" cannot drift apart.
//!
//! ```
//! use rtok_agent_sdk::{Apply, NO_CHANGES, register_mcp};
//! let dir = std::env::temp_dir().join(format!("rtok-agent-sdk-doc-{}", std::process::id()));
//! std::fs::create_dir_all(&dir).unwrap();
//! let path = dir.join("mcp.json");
//! let apply = Apply { dry_run: false, backup: true, backup_files: 0, yes: false };
//! let first = register_mcp(&apply, &path, "rtok", "rtok", &["mcp"]).unwrap();
//! assert_eq!(first, "mcpServers.rtok: rtok mcp");
//! assert_eq!(register_mcp(&apply, &path, "rtok", "rtok", &["mcp"]).unwrap(), NO_CHANGES);
//! std::fs::remove_dir_all(&dir).ok();
//! ```

use std::ffi::OsStr;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{Value, json};

/// The report an installer returns when it found nothing to do. Also the write gate: a run
/// carrying this report never touches the disk.
pub const NO_CHANGES: &str = "no changes";

/// How to install rtok when rtok itself is missing (D21 (5)). Named in every plugin offer,
/// because the offer is the one place a host's user reads before rtok exists for them.
pub const KETCH_INSTALL: &str = "ketch install pyrlyn/rtok";

/// Marker file written into a Windows (non-unix) plugin *copy* so remove can
/// `remove_dir_all` only trees rtok created — never a foreign directory.
pub const OWNED_MARKER: &str = ".rtok-owned";

/// How much an installer is allowed to do to the disk on this run.
#[derive(Clone, Copy, Debug, Default)]
pub struct Apply {
    /// Describe the change and write nothing.
    pub dry_run: bool,
    /// Copy each file to `_backup/<name>.bak-<unix-seconds>` before the first write to it.
    pub backup: bool,
    /// Newest `.bak-*` generations kept per file after a backup; `0` keeps all.
    pub backup_files: usize,
    /// Accept every offer without asking. The only way to say yes without a terminal.
    pub yes: bool,
}

impl Apply {
    /// True when this run may write a file for `report`. A report of only `leave …` / `? …`
    /// lines changed nothing, so it writes nothing either (T246).
    pub fn writes(&self, report: &str) -> bool {
        !self.dry_run
            && report != NO_CHANGES
            && !report
                .lines()
                .all(|l| l.starts_with("leave ") || l.starts_with("? "))
    }
}

/// Directory name next to a host config that holds undo copies.
pub const BACKUP_DIR: &str = "_backup";

/// Copy `path` to `_backup/<name>.bak-<unix-seconds>` beside it. `None` when there is no file
/// yet, or when any file in that folder already holds the same bytes — a second install or
/// uninstall of unchanged content is the same undo, whatever the copy is named.
///
/// After a copy is taken, [`prune_backups`] keeps the newest `keep` generations of that name.
pub fn backup(path: &Path, keep: usize) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let bak = backup_at(path, ts)?;
    if let Some(b) = &bak {
        prune_backups(b, keep);
    }
    Ok(bak)
}

/// Delete the oldest `_backup/<name>.bak-<ts>[-<n>]` generations beside `kept` — a copy
/// [`backup`] returned — until `keep` remain. `kept` itself is never deleted; `0` keeps all.
/// Only regular files with that exact name shape inside a `_backup` folder are candidates.
/// Best effort: an fs error leaves the rest in place and is not reported.
pub fn prune_backups(kept: &Path, keep: usize) {
    let (Some(dir), Some(file)) = (kept.parent(), kept.file_name().and_then(OsStr::to_str)) else {
        return;
    };
    if keep == 0 || dir.file_name().is_none_or(|d| d != BACKUP_DIR) {
        return;
    }
    let Some((name, _)) = file.rsplit_once(".bak-") else {
        return;
    };
    let mut older = generations(dir, name);
    older.retain(|(_, p)| p != kept);
    older.sort();
    let excess = (older.len() + 1).saturating_sub(keep);
    for (_, p) in older.into_iter().take(excess) {
        let _ = fs::remove_file(p);
    }
}

/// The generations in the `_backup` folder `dir` that a cap of `keep` has no room for: per base
/// name, all but the newest `keep`, oldest first. Read-only, the listing twin of
/// [`prune_backups`] for a cap lowered after the copies were taken; `0` keeps all.
pub fn stale_backups(dir: &Path, keep: usize) -> Vec<PathBuf> {
    if keep == 0 || dir.file_name().is_none_or(|d| d != BACKUP_DIR) {
        return Vec::new();
    }
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let f = e.ok()?.file_name().into_string().ok()?;
            Some(f.rsplit_once(".bak-")?.0.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    let mut out = Vec::new();
    for name in names {
        let mut all = generations(dir, &name);
        all.sort();
        let excess = all.len().saturating_sub(keep);
        out.extend(all.into_iter().take(excess).map(|(_, p)| p));
    }
    out
}

/// Regular files in `dir` named `<name>.bak-<ts>[-<n>]`, with `(ts, n)` to sort them by.
fn generations(dir: &Path, name: &str) -> Vec<((u64, u64), PathBuf)> {
    let prefix = format!("{name}.bak-");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let f = e.file_name().into_string().ok()?;
            Some((generation(f.strip_prefix(&prefix)?)?, e.path()))
        })
        .collect()
}

/// `<ts>` or `<ts>-<n>`, the suffix [`backup`] writes, as a sortable pair; anything else is `None`.
fn generation(suffix: &str) -> Option<(u64, u64)> {
    let (ts, n) = suffix.split_once('-').unwrap_or((suffix, "0"));
    let num = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    Some((num(ts)?, num(n)?))
}

fn backup_dir(path: &Path) -> Option<PathBuf> {
    path.parent().map(|p| p.join(BACKUP_DIR))
}

/// `backup` with the clock passed in, so a test can pin the second instead of racing it.
fn backup_at(path: &Path, ts: u64) -> Result<Option<PathBuf>> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let body = fs::read(path).with_context(|| path.display().to_string())?;
    let Some(dir) = backup_dir(path) else {
        return Ok(None);
    };
    if identical_backup_exists(&dir, &body) {
        return Ok(None);
    }
    fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    // Past the highest `-<n>` of this second, never a slot pruning freed: the new copy must
    // sort newest, or the next prune would delete it first.
    let mut n = generations(&dir, &name)
        .iter()
        .filter(|((t, _), _)| *t == ts)
        .map(|((_, n), _)| n + 1)
        .max()
        .unwrap_or(0);
    let slot = |n: u64| match n {
        0 => dir.join(format!("{name}.bak-{ts}")),
        n => dir.join(format!("{name}.bak-{ts}-{n}")),
    };
    let mut bak = slot(n);
    while bak.exists() {
        n += 1;
        bak = slot(n);
    }
    fs::copy(path, &bak).with_context(|| bak.display().to_string())?;
    Ok(Some(bak))
}

/// True when any regular file in `dir` is byte-equal to `body`. Size first; names are ignored.
fn identical_backup_exists(dir: &Path, body: &[u8]) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .any(|e| {
            e.metadata().is_ok_and(|m| m.len() == body.len() as u64)
                && fs::read(e.path()).is_ok_and(|b| b == body)
        })
}

/// A host's JSON config as a value to edit. A missing or empty file is an empty object, not an
/// error: `agent setup` on a host the user has never configured is the common case.
///
/// Strict JSON only: a JSONC file (comments, trailing commas — the shape Zed and VS Code
/// write) is refused, never rewritten — a rewrite from a parsed value would delete the
/// user's comments (T79 option b). The error names the file and the offending line, so the
/// entry can be pasted in by hand.
pub fn read_json(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let raw = fs::read_to_string(path).with_context(|| path.display().to_string())?;
    parse_strict_json(path, &raw)
}

/// [`read_json`] on text a caller already holds (rtok-mcp reads through its own `Fs`): an empty
/// document is an empty object, anything that is not strict JSON gets the same refusal message.
pub fn parse_strict_json(path: &Path, raw: &str) -> Result<Value> {
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(raw).map_err(|e| {
        let line = e.line();
        let src = raw.lines().nth(line.saturating_sub(1)).unwrap_or("").trim();
        anyhow::anyhow!(
            "{}:{}: not strict JSON: {e}. rtok refuses to rewrite this file (a rewrite would \
             delete its comments); add the entry by hand at that line and re-run. The line: {src}",
            path.display(),
            e.column()
        )
    })
}

/// Every JSON host installer: [`read_json`], let `edit` change the document and report what it
/// did, then [`write_json`] under `apply`. Each host spelled this out on its own.
pub fn edit_json(
    apply: &Apply,
    path: &Path,
    edit: impl FnOnce(&mut Value) -> String,
) -> Result<String> {
    let mut root = read_json(path)?;
    let report = edit(&mut root);
    write_json(apply, path, &root, &report)?;
    Ok(report)
}

/// `parent[key]` as an object, created when absent. A `parent` that is not an object becomes
/// `{}`, and so does a `key` of another shape: a host config is user data, and setup must not
/// panic on it. Each host installer used to repeat this dance (and its `unwrap`s) per key.
pub fn object_at<'a>(parent: &'a mut Value, key: &str) -> &'a mut Value {
    slot(parent, key, Value::is_object, || json!({}))
}

/// [`object_at`] for an array.
pub fn array_at<'a>(parent: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    slot(parent, key, Value::is_array, || json!([]))
        .as_array_mut()
        .expect("slot keeps an array")
}

fn slot<'a>(
    parent: &'a mut Value,
    key: &str,
    fits: fn(&Value) -> bool,
    empty: fn() -> Value,
) -> &'a mut Value {
    if !parent.is_object() {
        *parent = json!({});
    }
    let v = parent
        .as_object_mut()
        .expect("just made an object")
        .entry(key)
        .or_insert_with(empty);
    if !fits(v) {
        *v = empty();
    }
    v
}

/// Write `body` at `path`, gated by `apply` and `report`: a dry run and a [`NO_CHANGES`] report
/// write nothing, and the previous file is backed up first when asked.
///
/// Writes atomically: `body` lands in a sibling temp file first, then an `fs::rename` swaps it
/// over the target, so a kill/crash/full-disk between the two steps never leaves the host's
/// config empty or half-written — the target is either the old file or the fully-written new
/// one, never a partial write. When `path` is a symlink (dotfile managers replace host config
/// files with one), the temp file lands beside, and the rename lands on, the file it points to,
/// so the symlink itself survives.
pub fn write(apply: &Apply, path: &Path, body: &str, report: &str) -> Result<()> {
    if !apply.writes(report) {
        return Ok(());
    }
    if apply.backup {
        backup(path, apply.backup_files)?;
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).ok();
    }
    let target = link_target(path);
    write_atomic(&target, body).with_context(|| target.display().to_string())
}

/// The file `path` resolves to. `canonicalize` fails on a dangling link (a dotfile manager's
/// link whose target is not there yet), and the rename then replaced the link with a plain
/// file; following the links by hand writes the target and keeps the link. A target whose
/// directory is gone fails the write instead of being created.
fn link_target(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    // 40: the kernel's own ELOOP limit, so a cycle ends.
    for _ in 0..40 {
        if let Ok(real) = fs::canonicalize(&p) {
            return real;
        }
        match fs::read_link(&p) {
            Ok(next) => p = p.parent().unwrap_or(Path::new("")).join(next),
            Err(_) => break,
        }
    }
    p
}

/// [`write`]'s atomic swap: write `body` to a sibling temp file, copy `target`'s
/// permissions onto it when `target` exists (a 0600 `~/.claude.json` must stay
/// 0600 on Unix), then `fs::rename` the temp file over `target` — atomic on one
/// filesystem. Any failed step cleans up the temp file before returning the error.
///
/// On Windows, `MoveFileExW(REPLACE_EXISTING)` fails when the destination has the
/// read-only attribute. Copying that bit onto the temp file (as Unix mode copy
/// would) then made every update of a read-only host config fail; clear it on
/// the destination instead. Temp names include a nanos suffix so two writes in
/// the same process cannot share one leftover `.rtok-tmp-*` file.
pub fn write_atomic(target: &Path, body: &str) -> Result<()> {
    let dir = target.parent().unwrap_or(Path::new("."));
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{name}.rtok-tmp-{}-{nanos}", std::process::id()));
    let result: Result<()> = (|| -> Result<()> {
        fs::write(&tmp, body)?;
        #[cfg(unix)]
        if let Ok(meta) = fs::metadata(target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        #[cfg(windows)]
        clear_readonly(target);
        fs::rename(&tmp, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// `MOVEFILE_REPLACE_EXISTING` cannot replace a read-only file; drop that bit.
#[cfg(windows)]
// Windows-only, so the world-writable risk the lint names for Unix cannot arise here.
#[allow(clippy::permissions_set_readonly_false)]
fn clear_readonly(path: &Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    let mut perms = meta.permissions();
    if !perms.readonly() {
        return;
    }
    perms.set_readonly(false);
    let _ = fs::set_permissions(path, perms);
}

/// [`write`] for a JSON document: pretty-printed with the trailing newline host files carry.
pub fn write_json(apply: &Apply, path: &Path, root: &Value, report: &str) -> Result<()> {
    if !apply.writes(report) {
        return Ok(());
    }
    let body = serde_json::to_string_pretty(root)? + "\n";
    write(apply, path, &body, report)
}

/// Register a stdio MCP server under `mcpServers.<name>` in a host's JSON config — the shape
/// Claude Code (`~/.claude.json`) and Cursor (`~/.cursor/mcp.json`) both read.
pub fn register_mcp(
    apply: &Apply,
    path: &Path,
    name: &str,
    command: &str,
    args: &[&str],
) -> Result<String> {
    let summary = format!("{command} {}", args.join(" "));
    register_server(
        apply,
        path,
        "mcpServers",
        name,
        mcp_entry(command, args),
        &summary,
    )
}

/// The `mcpServers.<name>` entry [`register_mcp`] writes.
pub fn mcp_entry(command: &str, args: &[&str]) -> Value {
    json!({"type": "stdio", "command": command, "args": args})
}

/// [`register_mcp`] for a host whose server map or entry has another shape: OpenCode keeps
/// `mcp.<name> = {type: "local", command: [..]}`, Copilot adds `tools` to `mcpServers`.
/// `summary` is what the report prints after `<key>.<name>: `. A dotted `key` walks nested
/// objects (ZCode's `mcp.servers`).
pub fn register_server(
    apply: &Apply,
    path: &Path,
    key: &str,
    name: &str,
    entry: Value,
    summary: &str,
) -> Result<String> {
    edit_json(apply, path, |root| {
        let servers = key.split('.').fold(root, |o, k| object_at(o, k));
        if servers.get(name).map(without_default_type) == Some(without_default_type(&entry)) {
            return NO_CHANGES.into();
        }
        servers[name] = entry;
        format!("{key}.{name}: {summary}")
    })
}

/// Drop the `<name>` entry from a host's `mcpServers` map. Foreign servers are left alone, and a
/// map that ends up empty goes with it so the file reads as it did before rtok arrived.
pub fn unregister_mcp(apply: &Apply, path: &Path, name: &str) -> Result<String> {
    unregister_server(apply, path, "mcpServers", name)
}

/// [`unregister_mcp`] under another map key (OpenCode's `mcp`), dotted for a nested one
/// (ZCode's `mcp.servers`); only the last level is dropped when it ends up empty. Private
/// (T246.5): it drops an entry by name alone, with no ownership check, so every caller outside
/// this module goes through [`unregister_owned`] instead.
fn unregister_server(apply: &Apply, path: &Path, key: &str, name: &str) -> Result<String> {
    edit_json(apply, path, |root| {
        let mut parts: Vec<&str> = key.split('.').collect();
        let last = parts.pop().unwrap_or(key);
        let mut parent = root;
        for k in parts {
            let Some(next) = parent.get_mut(k) else {
                return NO_CHANGES.into();
            };
            parent = next;
        }
        let Some(servers) = parent.get_mut(last).and_then(Value::as_object_mut) else {
            return NO_CHANGES.into();
        };
        if servers.remove(name).is_none() {
            return NO_CHANGES.into();
        }
        if servers.is_empty() {
            parent.as_object_mut().map(|p| p.remove(last));
        }
        format!("- {key}.{name}")
    })
}

/// [`unregister_server`] that takes back only what rtok wrote (T246). `ours` is the entry the
/// installer writes now; `is_bin` says whether a string names the rtok binary, so any rtok
/// path counts as the same. The ownership call is [`judge_owned`]; a slot it clears goes.
pub fn unregister_owned(
    apply: &Apply,
    path: &Path,
    key: &str,
    name: &str,
    ours: &Value,
    is_bin: fn(&str) -> bool,
) -> Result<String> {
    let have = key
        .split('.')
        .try_fold(&read_json(path)?, |o, k| o.get(k))
        .and_then(|s| s.get(name))
        .cloned();
    let Some(have) = have else {
        return Ok(NO_CHANGES.into());
    };
    let at = format!("{key}.{name} in {}", path.display());
    if let Some(leave) = judge_owned(apply, &at, &have, ours, is_bin) {
        return Ok(leave);
    }
    unregister_server(apply, path, key, name)
}

/// The three-step ownership check every removal path runs on an entry named `rtok` (T246,
/// T246.5): not a string naming the rtok binary anywhere in `have` → not ours,
/// `Some("leave … (not rtok's; remove by hand)")`; runs rtok but `have` differs from `ours`
/// once every rtok string in both is folded to one placeholder and a top-level
/// `"type": "stdio"` is dropped (T265) → the user changed it,
/// `Some(`[`keep_edited`]`)` (`?` on a dry run, `leave …` once declined, `None` on `--yes` or a
/// yes); otherwise unchanged, `None` — go ahead and remove it. `pub` so a host whose config
/// [`unregister_owned`] cannot read directly (Zed's JSONC, Grok's TOML) runs the same check on
/// a value it converted itself, instead of copying the three steps.
pub fn judge_owned(
    apply: &Apply,
    at: &str,
    have: &Value,
    ours: &Value,
    is_bin: fn(&str) -> bool,
) -> Option<String> {
    if !runs_bin(have, is_bin) {
        return Some(format!("leave {at} (not rtok's; remove by hand)"));
    }
    (rtok_as_one(&without_default_type(have), is_bin)
        != rtok_as_one(&without_default_type(ours), is_bin))
    .then(|| keep_edited(apply, at))
    .flatten()
}

/// For an rtok entry the user changed, `at` naming it: `None` removes it (`--yes`, or the
/// user said yes); `Some(report)` keeps it — `? …` on a dry run, `leave …` once declined.
pub fn keep_edited(apply: &Apply, at: &str) -> Option<String> {
    if apply.dry_run && !apply.yes {
        return Some(format!("? {at} (changed by you; remove asks)"));
    }
    (!confirmed(apply, &format!("remove {at}? you changed it")))
        .then(|| format!("leave {at} (changed by you; remove by hand)"))
}

/// True when `v` (or anything nested in it) is a string naming the rtok binary — [`judge_owned`]'s
/// "not rtok's" check, and the doctor's way to tell rtok's own MCP entry from any other server.
pub fn runs_bin(v: &Value, is_bin: fn(&str) -> bool) -> bool {
    match v {
        Value::String(s) => is_bin(s),
        Value::Array(a) => a.iter().any(|x| runs_bin(x, is_bin)),
        Value::Object(m) => m.values().any(|x| runs_bin(x, is_bin)),
        _ => false,
    }
}

/// `v` without a top-level `"type": "stdio"`, the MCP default [`mcp_entry`] spells out:
/// Claude.app drops it when it re-saves its config (T265). Anything else is unchanged.
pub fn without_default_type(v: &Value) -> Value {
    match v {
        Value::Object(m) if m.get("type").and_then(Value::as_str) == Some("stdio") => {
            let mut m = m.clone();
            m.remove("type");
            Value::Object(m)
        }
        _ => v.clone(),
    }
}

/// The elements of an argv without the `--host <id>` pair that follows `mcp`: every host's
/// `rtok mcp` entry carries it since T283.2, and an entry written before that has none. Both
/// are rtok's own, so the ownership check must not call either one an edit.
fn without_host_arg(argv: &[Value]) -> impl Iterator<Item = &Value> {
    let mut after_mcp = false;
    let mut skip = false;
    argv.iter().filter(move |x| {
        if std::mem::take(&mut skip) {
            return false;
        }
        if after_mcp && *x == "--host" {
            skip = true;
            return false;
        }
        after_mcp = *x == "mcp";
        true
    })
}

/// `v` with every string naming the rtok binary replaced by one placeholder — [`judge_owned`]'s
/// "did the user change it" comparison.
fn rtok_as_one(v: &Value, is_bin: fn(&str) -> bool) -> Value {
    match v {
        Value::String(s) if is_bin(s) => Value::Null,
        Value::Array(a) => without_host_arg(a)
            .map(|x| rtok_as_one(x, is_bin))
            .collect(),
        Value::Object(m) => m
            .iter()
            .map(|(k, x)| (k.clone(), rtok_as_one(x, is_bin)))
            .collect(),
        _ => v.clone(),
    }
}

/// [`accepted`] for a question whose default is no: `[y/N]`, and Enter keeps what is there.
/// `--yes` still accepts; no terminal is a no.
pub fn confirmed(apply: &Apply, question: &str) -> bool {
    ask(apply, question, false)
}

/// Ask, unless the answer is already known. `--yes` accepts without asking; a terminal gets
/// one plain line on stdout (`? {question} [Y/n] `) — no raw mode, no hidden cursor, and a
/// redirected or busy stderr can never hide the question (T81). Enter takes the default
/// (yes); EOF, a read error, or anything but y/yes is a no, so an unanswered question never
/// acts. Anywhere without a terminal an unanswered question is a no, so `agents install`
/// stays non-interactive by default.
pub fn accepted(apply: &Apply, question: &str) -> bool {
    ask(apply, question, true)
}

fn ask(apply: &Apply, question: &str, default: bool) -> bool {
    if apply.yes {
        return true;
    }
    if !std::io::stdin().is_terminal() {
        return false;
    }
    use std::io::Write;
    let mut out = std::io::stdout();
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    let _ = write!(out, "? {question} {hint} ");
    let _ = out.flush();
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    let _ = writeln!(out);
    match read {
        // 0 bytes is EOF: never act on a question nobody answered.
        Ok(0) => false,
        Ok(_) => answer(&line, default),
        Err(_) => false,
    }
}

/// Enter takes `default`; explicit y/yes accept; anything else declines.
fn answer(line: &str, default: bool) -> bool {
    match line.trim().to_ascii_lowercase().as_str() {
        "" => default,
        a => matches!(a, "y" | "yes"),
    }
}

/// A host plugin directory this repo ships, and where that host loads it from (D21 (6)).
///
/// On Unix the install is a symlink so the host and the repo see the same tree. On Windows
/// Cursor may reject external junctions, so the install is a directory copy marked with
/// [`OWNED_MARKER`] — remove undoes only what rtok installed.
pub struct PluginLink<'a> {
    /// Repo-relative source (`plugins/cursor`), named in every report so the user can find it.
    pub src_rel: &'a str,
    /// Absolute source directory in this repo.
    pub src: PathBuf,
    /// Where the host expects the plugin.
    pub dest: PathBuf,
    /// How the destination reads to a person (`~/.cursor/plugins/local`); a dry run prints
    /// the real path beside it, and every other line the label alone. `None` spells the
    /// destination path itself, with the source tree beside it on a dry run.
    pub label: Option<&'a str>,
    /// The host's name as its users spell it, for the question.
    pub host: &'a str,
}

impl PluginLink<'_> {
    /// True when something already sits at the destination — a link, a directory, anything.
    /// A foreign directory counts: rtok does not overwrite what it did not put there.
    pub fn linked(&self) -> bool {
        self.dest.symlink_metadata().is_ok()
    }

    /// True when what sits at the destination is rtok's to remove (T75): a link or a
    /// plain file — either unlinks — or a directory rtok can prove holds its own plugin:
    /// the [`OWNED_MARKER`] a copy leaves, or every byte of `src` present unchanged (a
    /// host may materialize the linked tree into a copy). A foreign directory is not
    /// ours however it got there, so the installed mark never outlives an uninstall
    /// that refused to touch it.
    pub fn ours(&self) -> bool {
        let Ok(meta) = self.dest.symlink_metadata() else {
            return false;
        };
        if meta.file_type().is_symlink() || meta.file_type().is_file() {
            return true;
        }
        meta.is_dir()
            && (self.dest.join(OWNED_MARKER).is_file() || tree_copies(&self.src, &self.dest))
    }

    /// The destination as the question and a declined offer spell it: the label, or the
    /// path itself when the host has no shorthand for where its plugins live.
    fn main_desc(&self) -> String {
        match self.label {
            Some(label) => label.to_string(),
            None => self.dest.display().to_string(),
        }
    }

    /// The destination as a dry run spells it: the label (or the path) with the concrete
    /// path beside it — the destination when a label stands in for it, the source tree the
    /// link will carry otherwise.
    fn dest_desc(&self) -> String {
        match self.label {
            Some(label) => format!("{label} ({})", self.dest.display()),
            None => format!("{} ({})", self.dest.display(), self.src.display()),
        }
    }

    /// Offer, link, or unlink. Returns the one-line report; a dry run and a declined offer both
    /// describe the offer and touch nothing.
    pub fn run(&self, apply: &Apply, remove: bool) -> Result<String> {
        self.run_with(apply, remove, keep_bytes)
    }

    /// [`run`](Self::run) whose owned copy (the non-Unix install) passes every file through
    /// `fix`; the up-to-date check compares against the fixed bytes, so a fixed copy is not
    /// reinstalled on every run (T250.3: Cursor's POSIX hook lines go back to bare `rtok`).
    pub fn run_with(&self, apply: &Apply, remove: bool, fix: CopyFix) -> Result<String> {
        if apply.dry_run {
            return Ok(format!(
                "offer {} → {} {KETCH_INSTALL}",
                self.src_rel,
                self.dest_desc()
            ));
        }
        if remove {
            if !self.linked() {
                return Ok(NO_CHANGES.into());
            }
            // Install refuses to overwrite a foreign directory; remove must not wipe one
            // either.
            if !self.ours() {
                return Ok(format!(
                    "leave {} (not an rtok link; remove by hand)",
                    self.dest.display()
                ));
            }
            self.unlink()?;
            return Ok(format!("- plugin {}", self.dest.display()));
        }
        if self.linked() {
            if self.up_to_date(fix) {
                return Ok(NO_CHANGES.into());
            }
            // Something else is at the destination: never overwrite ground we did not
            // put there, whatever the answer to the question below would be (T164).
            if !self.ours() {
                return Ok(format!(
                    "offer {} → {} (accept with --yes) {KETCH_INSTALL}",
                    self.src_rel,
                    self.main_desc()
                ));
            }
            // Ours but stale — an older ketch version, or a link whose target is gone.
            // Drop it so a fresh install can take its place (T164).
            self.unlink()?;
        }
        let question = format!(
            "install {} into {} for {}?",
            self.src_rel,
            self.main_desc(),
            self.host
        );
        if !accepted(apply, &question) {
            return Ok(format!(
                "offer {} → {} (accept with --yes) {KETCH_INSTALL}",
                self.src_rel,
                self.main_desc()
            ));
        }
        if let Some(dir) = self.dest.parent() {
            fs::create_dir_all(dir).ok();
        }
        install_plugin(&self.src, &self.dest, fix)?;
        let label = self.label.map(|l| format!(" {l}")).unwrap_or_default();
        Ok(format!(
            "+ plugin {} → {}{}",
            self.src_rel,
            self.dest.display(),
            label
        ))
    }

    /// Unlink whatever sits at the destination outright: a symlink or plain file removed,
    /// an owned directory copy wiped whole. Callers check [`ours`] first — this only runs
    /// once that already said yes, whether for a genuine uninstall or to clear a stale
    /// version before a relink (T164).
    fn unlink(&self) -> Result<()> {
        unlink_at(&self.dest)
    }

    /// True when the destination already carries exactly this build's plugin: the same
    /// symlink target on Unix (and it still resolves), or a byte-identical owned copy
    /// elsewhere. A link into a different — typically older — ketch store version, or a
    /// dangling one, is not up to date: [`run`] relinks it like a fresh install instead of
    /// reporting [`NO_CHANGES`] forever (T164).
    fn up_to_date(&self, fix: CopyFix) -> bool {
        let Ok(meta) = self.dest.symlink_metadata() else {
            return false;
        };
        if meta.file_type().is_symlink() {
            return self.dest.exists()
                && fs::read_link(&self.dest).is_ok_and(|target| target == self.src);
        }
        if meta.file_type().is_file() {
            return self.src.is_file() && fs::read(&self.dest).ok() == fs::read(&self.src).ok();
        }
        meta.is_dir() && tree_copies_with(&self.src, &self.dest, fix, Path::new(""))
    }
}

/// What [`SkillCopy::run`] will do given destination state. Pure so `Vfs` tests
/// can drive the same decision as the installer without touching the host disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillPlan {
    /// Copy the hub tree and write [`OWNED_MARKER`]. Replaces a destination [`SkillView::ours`]
    /// already holds when the bytes are not the current hub.
    Copy,
    /// Already installed, already gone, or a foreign tree on remove.
    NoChanges,
    /// Destination exists but is not an rtok-owned skill tree.
    LeaveForeign,
    /// Delete the owned skill directory.
    Remove,
}

/// Destination facts for [`skill_plan`]. A symlink is never followed: its target is not proof
/// and is not deleted (T380).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillView {
    pub present: bool,
    pub symlink: bool,
    /// Last path component starts with `rtok`. That name is ours, beside the marker and a
    /// byte-for-byte copy, because an unprefixed name collides with a user's own skill.
    pub prefixed: bool,
    pub marked: bool,
    pub same_bytes: bool,
}

impl SkillView {
    /// Marker, byte-for-byte copy, or a `rtok` directory name. A symlink counts only when the
    /// link's own name is prefixed — the target is never rtok's.
    pub fn ours(self) -> bool {
        if !self.present {
            return false;
        }
        if self.symlink {
            return self.prefixed;
        }
        self.prefixed || self.marked || self.same_bytes
    }
}

/// Decide install / reinstall / remove from [`SkillView`].
pub fn skill_plan(remove: bool, view: SkillView) -> SkillPlan {
    if !view.present {
        return if remove {
            SkillPlan::NoChanges
        } else {
            SkillPlan::Copy
        };
    }
    if remove {
        return if view.ours() {
            SkillPlan::Remove
        } else {
            SkillPlan::NoChanges
        };
    }
    if view.symlink {
        return if view.prefixed {
            SkillPlan::Copy
        } else {
            SkillPlan::LeaveForeign
        };
    }
    if view.same_bytes {
        return SkillPlan::NoChanges;
    }
    // The prefix is the slot: a foreign or stale tree under a `rtok` name is replaced.
    // An unprefixed marked copy stays, so an edit there is not clobbered on install.
    if view.prefixed {
        return SkillPlan::Copy;
    }
    if view.marked {
        return SkillPlan::NoChanges;
    }
    SkillPlan::LeaveForeign
}

/// Hub skill directory (`skills/rtok/`) copied into a host's documented skill root.
///
/// A directory whose name starts with `rtok` is ours. Anything else is ours only with
/// [`OWNED_MARKER`] or a byte-for-byte copy of `src` ([`tree_copies`]), the same two proofs
/// a plugin directory uses. A symlink is unlinked and never followed.
pub struct SkillCopy {
    /// Absolute source directory in this repo (`skills/rtok`).
    pub src: PathBuf,
    /// Where the host loads skills from (`~/.cursor/skills/rtok`, …).
    pub dest: PathBuf,
    /// How the destination reads to a person (`~/.cursor/skills/rtok`); dry runs print the
    /// concrete path beside it.
    pub label: Option<String>,
}

impl SkillCopy {
    fn prefixed(&self) -> bool {
        self.dest
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rtok"))
    }

    /// `symlink_metadata` only. Following a link would treat the user's target as ours.
    fn view(&self) -> SkillView {
        let Ok(meta) = fs::symlink_metadata(&self.dest) else {
            return SkillView {
                present: false,
                symlink: false,
                prefixed: self.prefixed(),
                marked: false,
                same_bytes: false,
            };
        };
        if meta.file_type().is_symlink() {
            return SkillView {
                present: true,
                symlink: true,
                prefixed: self.prefixed(),
                marked: false,
                same_bytes: false,
            };
        }
        let dir = meta.is_dir();
        SkillView {
            present: true,
            symlink: false,
            prefixed: self.prefixed(),
            marked: dir && self.dest.join(OWNED_MARKER).is_file(),
            same_bytes: dir && tree_copies(&self.src, &self.dest),
        }
    }

    fn dest_desc(&self) -> String {
        match &self.label {
            Some(label) => format!("{label} ({})", self.dest.display()),
            None => self.dest.display().to_string(),
        }
    }

    /// Copy or remove the hub skill tree. Returns one report line; a dry run describes the
    /// change and touches nothing.
    pub fn run(&self, apply: &Apply, remove: bool) -> Result<String> {
        let view = self.view();
        match skill_plan(remove, view) {
            SkillPlan::NoChanges => Ok(NO_CHANGES.into()),
            SkillPlan::LeaveForeign => Ok(format!(
                "leave {} (not an rtok skill; remove by hand)",
                self.dest.display()
            )),
            SkillPlan::Remove => {
                // A symlink has no marker of its own; do not stat the target.
                if !view.symlink
                    && edited_since_marked(&self.dest)
                    && let Some(leave) = keep_edited(apply, &self.dest.display().to_string())
                {
                    return Ok(leave);
                }
                let report = if apply.dry_run {
                    format!("- skill {}", self.dest_desc())
                } else {
                    format!("- skill {}", self.dest.display())
                };
                if apply.writes(&report) {
                    unlink_at(&self.dest)?;
                }
                Ok(report)
            }
            SkillPlan::Copy => {
                let report = format!("+ skill → {}", self.dest_desc());
                if apply.writes(&report) {
                    if view.present {
                        unlink_at(&self.dest)?;
                    }
                    if let Some(dir) = self.dest.parent() {
                        fs::create_dir_all(dir).ok();
                    }
                    copy_owned(&self.src, &self.dest)?;
                }
                Ok(report)
            }
        }
    }
}

/// Remove `dest` itself. A symlink is `remove_file`d so its target is left untouched.
fn unlink_at(dest: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(dest)?;
    if meta.file_type().is_symlink() || meta.file_type().is_file() {
        fs::remove_file(dest).with_context(|| format!("unlink {}", dest.display()))?;
    } else if meta.is_dir() {
        fs::remove_dir_all(dest).with_context(|| format!("remove {}", dest.display()))?;
    }
    Ok(())
}

/// True when something under the owned copy `dest` is newer than its [`OWNED_MARKER`] (T246.4):
/// [`copy_owned`] writes the marker last, so a later write is the user's — an edited or an
/// added file. An older rtok's copy is not an edit. An unreadable time proves nothing.
fn edited_since_marked(dest: &Path) -> bool {
    fn mtime(p: &Path) -> Option<SystemTime> {
        fs::metadata(p).and_then(|m| m.modified()).ok()
    }
    fn newer(dir: &Path, marked: SystemTime) -> bool {
        let Ok(entries) = fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|e| {
            let p = e.path();
            e.file_name() != OWNED_MARKER
                && (mtime(&p).is_some_and(|t| t > marked) || (p.is_dir() && newer(&p, marked)))
        })
    }
    mtime(&dest.join(OWNED_MARKER)).is_some_and(|marked| newer(dest, marked))
}

/// True when every file under `src` sits in `dest` with the same bytes — extra files in
/// `dest` are allowed, a host may put its own beside ours. This is what lets remove take
/// back a directory a host materialized from our symlink, while a foreign directory
/// (which fails the first differing byte) still stands. An empty or unreadable `src`
/// proves nothing.
fn tree_copies(src: &Path, dest: &Path) -> bool {
    tree_copies_with(src, dest, keep_bytes, Path::new(""))
}

/// [`tree_copies`] against `fix`ed source bytes; `rel` is `src`'s path under the plugin root.
fn tree_copies_with(src: &Path, dest: &Path, fix: CopyFix, rel: &Path) -> bool {
    let Ok(entries) = fs::read_dir(src) else {
        return false;
    };
    let mut seen = 0;
    for entry in entries.flatten() {
        seen += 1;
        let Ok(ft) = entry.file_type() else {
            return false;
        };
        let there = dest.join(entry.file_name());
        let rel = rel.join(entry.file_name());
        if ft.is_dir() {
            if !there.is_dir() || !tree_copies_with(&entry.path(), &there, fix, &rel) {
                return false;
            }
        } else {
            let (Ok(a), Ok(b)) = (fs::read(entry.path()), fs::read(&there)) else {
                return false;
            };
            if fix(&rel, a) != b {
                return false;
            }
        }
    }
    seen > 0
}

/// Rewrites one plugin file on its way into an owned copy: `(path under the plugin root,
/// source bytes) → bytes written`. A symlinked install carries the source unchanged.
pub type CopyFix = fn(&Path, Vec<u8>) -> Vec<u8>;

/// The [`CopyFix`] that changes nothing.
pub fn keep_bytes(_: &Path, bytes: Vec<u8>) -> Vec<u8> {
    bytes
}

/// Install the plugin tree at `dest`: symlink on Unix, owned copy elsewhere.
fn install_plugin(src: &Path, dest: &Path, fix: CopyFix) -> Result<()> {
    #[cfg(unix)]
    {
        let _ = fix;
        std::os::unix::fs::symlink(src, dest)
            .with_context(|| format!("symlink {} → {}", src.display(), dest.display()))
    }
    #[cfg(not(unix))]
    {
        copy_owned_with(src, dest, fix)
    }
}

/// Recursively copy `src` into `dest` and leave [`OWNED_MARKER`] so remove can undo it.
/// A single-file plugin (OpenCode's `rtok.ts`) is one copy; remove already unlinks a plain
/// file. Compiled on every target so unit tests cover the Windows install path on Unix CI too.
fn copy_owned(src: &Path, dest: &Path) -> Result<()> {
    copy_owned_with(src, dest, keep_bytes)
}

/// [`copy_owned`] with every tree file passed through `fix`.
fn copy_owned_with(src: &Path, dest: &Path, fix: CopyFix) -> Result<()> {
    if src.is_file() {
        fs::copy(src, dest)
            .with_context(|| format!("copy plugin {} → {}", src.display(), dest.display()))?;
        return Ok(());
    }
    copy_dir(src, dest, fix, Path::new(""))
        .with_context(|| format!("copy plugin {} → {}", src.display(), dest.display()))?;
    fs::write(dest.join(OWNED_MARKER), b"")
        .with_context(|| format!("mark owned {}", dest.display()))?;
    Ok(())
}

fn copy_dir(src: &Path, dest: &Path, fix: CopyFix, rel: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let rel = rel.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir(&from, &to, fix, &rel)?;
        } else {
            // Plain files and symlink targets we can read: plugins ship as a normal tree.
            // `fs::copy` keeps the mode (a skill's scripts); a fix rewrites the bytes after.
            fs::copy(&from, &to)
                .with_context(|| format!("copy {} → {}", from.display(), to.display()))?;
            let bytes = fs::read(&to)?;
            let fixed = fix(&rel, bytes.clone());
            if fixed != bytes {
                fs::write(&to, fixed).with_context(|| format!("fix {}", to.display()))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const YES: Apply = Apply {
        dry_run: false,
        backup: false,
        backup_files: 0,
        yes: true,
    };

    /// T79: a JSONC config is refused with the file and the offending line named — the entry
    /// is something the user pastes in by hand — and the file is never rewritten.
    #[test]
    fn jsonc_is_refused_naming_the_file_and_the_line() {
        let dir = tmp("jsonc-refuse");
        let path = dir.join("settings.json");
        let raw = "{\n  // my theme\n  \"theme\": \"x\",\n}\n";
        fs::write(&path, raw).unwrap();
        let err = read_json(&path).unwrap_err().to_string();
        assert!(err.contains("settings.json:"), "{err}");
        assert!(err.contains("The line:"), "{err}");
        assert!(err.contains("// my theme"), "the offending line: {err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), raw);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A `plugins/demo` link with no label, the shape most link tests start from.
    fn demo_link(src: PathBuf, dest: PathBuf) -> PluginLink<'static> {
        PluginLink {
            src_rel: "plugins/demo",
            src,
            dest,
            label: None,
            host: "demo",
        }
    }
    use rstest::rstest;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rtok-agent-sdk-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn apply() -> Apply {
        Apply {
            dry_run: false,
            backup: false,
            backup_files: 0,
            yes: false,
        }
    }

    /// The test harness has no terminal, which is exactly the CI / piped / host-driven case:
    /// the offer must decline itself rather than block waiting for an answer nobody can give.
    #[test]
    fn without_a_terminal_only_yes_accepts() {
        let mut a = apply();
        assert!(!accepted(&a, "install?"), "a headless run must not accept");
        a.yes = true;
        assert!(accepted(&a, "install?"), "--yes must accept without asking");
    }

    /// T246.3: a report that only leaves or asks changed nothing, so it writes nothing; one
    /// real change beside it still writes.
    #[test]
    fn a_leave_only_report_writes_nothing() {
        let a = Apply::default();
        assert!(!a.writes("leave a (changed by you; remove by hand)\n? b (remove asks)"));
        assert!(a.writes("leave a (changed by you; remove by hand)\n3 removed"));
    }

    /// The one line `accepted` reads, judged: Enter keeps the default (yes — the old
    /// dialoguer `.default(true)` contract), and only y/yes spell yes.
    #[test]
    fn the_answer_line_keeps_the_default_yes_contract() {
        assert!(answer("\n", true));
        assert!(answer("", true));
        assert!(answer(" y \n", true));
        assert!(answer("YES\n", true));
        assert!(!answer("n\n", true));
        assert!(!answer("no\n", true));
        assert!(!answer("maybe\n", true));
    }

    #[test]
    fn enter_keeps_the_default_no_of_a_confirm() {
        assert!(!answer("\n", false));
        assert!(!answer("", false));
        assert!(answer("y\n", false));
        assert!(!confirmed(&apply(), "remove?"), "a headless run must keep");
        assert!(confirmed(&YES, "remove?"));
    }

    fn rtok_stem(s: &str) -> bool {
        Path::new(s).file_name().is_some_and(|n| n == "rtok")
    }

    /// T246: remove takes back an entry as rtok wrote it — any rtok path counts as the same —
    /// keeps an edited one without `--yes` (and without writing), removes it with `--yes`, and
    /// never touches an entry named `rtok` that does not run rtok.
    #[test]
    fn unregister_owned_takes_back_only_what_rtok_wrote() {
        let path = tmp("owned").join("mcp.json");
        let ours = mcp_entry("rtok", &["mcp"]);
        let go = |a: &Apply| unregister_owned(a, &path, "mcpServers", "rtok", &ours, rtok_stem);
        let seed = |entry: Value| {
            let body = json!({"mcpServers": {"rtok": entry, "other": {"command": "x"}}});
            fs::write(&path, body.to_string()).unwrap();
        };

        seed(mcp_entry("/opt/bin/rtok", &["mcp"]));
        assert_eq!(go(&apply()).unwrap(), "- mcpServers.rtok");
        assert!(read_json(&path).unwrap()["mcpServers"]["other"].is_object());
        assert_eq!(go(&apply()).unwrap(), NO_CHANGES);

        let edited =
            json!({"type": "stdio", "command": "rtok", "args": ["mcp"], "env": {"A": "1"}});
        seed(edited.clone());
        let before = fs::read_to_string(&path).unwrap();
        let dry = Apply {
            dry_run: true,
            ..apply()
        };
        assert!(
            go(&dry).unwrap().starts_with("? mcpServers.rtok"),
            "dry run never asks"
        );
        let kept = go(&apply()).unwrap();
        assert!(
            kept.starts_with("leave mcpServers.rtok") && kept.contains("changed by you"),
            "{kept}"
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "a kept entry writes nothing"
        );
        assert_eq!(go(&YES).unwrap(), "- mcpServers.rtok");

        seed(json!({"command": "/usr/bin/node", "args": ["server.js"]}));
        let foreign = go(&YES).unwrap();
        assert!(foreign.contains("not rtok's"), "{foreign}");
        assert!(read_json(&path).unwrap()["mcpServers"]["rtok"].is_object());
    }

    /// T283.2: an entry with or without the `--host <id>` pair is rtok's own, whichever of the
    /// two the installer writes now; any other extra argument is still the user's edit.
    #[test]
    fn unregister_owned_ignores_the_host_argument_either_way() {
        let path = tmp("owned-host").join("mcp.json");
        let go = |ours: &Value| {
            unregister_owned(&apply(), &path, "mcpServers", "rtok", ours, rtok_stem).unwrap()
        };
        let seed = |entry: Value| {
            let body = json!({"mcpServers": {"rtok": entry}});
            fs::write(&path, body.to_string()).unwrap();
        };
        let old = mcp_entry("rtok", &["mcp"]);
        let new = mcp_entry("rtok", &["mcp", "--host", "cursor"]);

        seed(old.clone());
        assert_eq!(go(&new), "- mcpServers.rtok", "old entry, new installer");
        seed(new.clone());
        assert_eq!(go(&old), "- mcpServers.rtok", "new entry, old installer");
        seed(mcp_entry("rtok", &["mcp", "--host", "cursor", "--extra"]));
        assert!(
            go(&new).starts_with("leave mcpServers.rtok"),
            "an edit stays"
        );
    }

    /// T265: an entry Claude.app re-saved without `type` is still rtok's; a changed `args`
    /// is still the user's.
    #[test]
    fn unregister_owned_matches_an_entry_missing_the_default_type() {
        let path = tmp("owned-no-type").join("claude_desktop_config.json");
        let ours = mcp_entry("rtok", &["mcp"]);
        let go = |a: &Apply| unregister_owned(a, &path, "mcpServers", "rtok", &ours, rtok_stem);
        let seed = |entry: Value| {
            let body = json!({"mcpServers": {"rtok": entry}});
            fs::write(&path, body.to_string()).unwrap();
        };

        seed(json!({"command": "/Users/x/.ketch/bin/rtok", "args": ["mcp"]}));
        assert_eq!(
            go(&apply()).unwrap(),
            "- mcpServers.rtok",
            "a missing type must not read as changed by the user"
        );
        assert!(
            read_json(&path).unwrap()["mcpServers"]
                .get("rtok")
                .is_none()
        );

        seed(json!({"command": "/Users/x/.ketch/bin/rtok", "args": ["mcp", "--x"]}));
        let kept = go(&apply()).unwrap();
        assert!(
            kept.starts_with("leave mcpServers.rtok") && kept.contains("changed by you"),
            "a real edit must still be kept: {kept}"
        );
        assert!(read_json(&path).unwrap()["mcpServers"]["rtok"].is_object());
    }

    /// The listing twin of `prune_backups`: per base name all but the newest `keep`, foreign
    /// names and other folders never, and `0` keeps every copy.
    #[test]
    fn stale_backups_lists_only_generations_past_the_cap() {
        let dir = tmp("stale-backups").join(BACKUP_DIR);
        fs::create_dir_all(&dir).unwrap();
        for f in [
            "a.json.bak-1",
            "a.json.bak-2-1",
            "a.json.bak-2",
            "b.json.bak-7",
            "a.json.bak-x",
        ] {
            fs::write(dir.join(f), "x").unwrap();
        }
        let names = |keep| {
            let mut n: Vec<String> = stale_backups(&dir, keep)
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            n.sort();
            n
        };
        assert_eq!(names(1), ["a.json.bak-1", "a.json.bak-2"]);
        assert_eq!(names(2), ["a.json.bak-1"]);
        assert!(names(0).is_empty() && names(3).is_empty());
        assert!(
            stale_backups(dir.parent().unwrap(), 1).is_empty(),
            "not a _backup folder"
        );
    }

    #[test]
    fn dry_run_writes_nothing_and_backup_keeps_the_old_file() {
        let dir = tmp("write");
        let path = dir.join("settings.json");
        let dry = Apply {
            dry_run: true,
            backup: true,
            backup_files: 0,
            yes: false,
        };
        write(&dry, &path, "{}", "+ something").unwrap();
        assert!(!path.exists(), "a dry run must not create the file");

        let mut a = apply();
        write(&a, &path, "one\n", "+ something").unwrap();
        write(&a, &path, "ignored\n", NO_CHANGES).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\n");

        a.backup = true;
        write(&a, &path, "two\n", "+ something else").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two\n");
        let backup = dir.join(BACKUP_DIR);
        let baks: Vec<_> = fs::read_dir(&backup)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
            .collect();
        assert_eq!(baks.len(), 1, "one backup per write");
        assert_eq!(fs::read_to_string(baks[0].path()).unwrap(), "one\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[rstest]
    fn backup_skips_a_hundred_preexisting_names_without_clobbering() {
        let dir = tmp("backup-collision");
        let path = dir.join("settings.json");
        // A pinned second: with the real clock, a rollover mid-test lands a fresh `bak-{ts+1}`.
        let ts = 1_700_000_000;
        fs::write(&path, "v0").unwrap();
        let first = backup_at(&path, ts).unwrap().expect("first copy");
        assert_eq!(
            first.file_name().unwrap().to_string_lossy(),
            format!("settings.json.bak-{ts}")
        );

        fs::write(&path, "v1").unwrap();
        let backup = dir.join(BACKUP_DIR);
        fs::create_dir_all(&backup).unwrap();
        for n in 1..100 {
            let slot = backup.join(format!("settings.json.bak-{ts}-{n}"));
            fs::write(&slot, format!("slot-{n}")).unwrap();
        }

        let contents_before: Vec<_> = fs::read_dir(&backup)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
            .map(|e| (e.path(), fs::read_to_string(e.path()).unwrap()))
            .collect();
        assert_eq!(contents_before.len(), 100, "base plus slots 1..=99");

        fs::write(&path, "v2").unwrap();
        let second = backup_at(&path, ts).unwrap().expect("second copy");
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            format!("settings.json.bak-{ts}-100")
        );
        assert_eq!(fs::read_to_string(&second).unwrap(), "v2");

        for (bak_path, content) in contents_before {
            assert_eq!(
                fs::read_to_string(&bak_path).unwrap(),
                content,
                "pre-existing backup must survive: {}",
                bak_path.display()
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// T44.1: a second `setup`/`remove` over an unchanged file must not add a second copy,
    /// whatever suffix the existing copy carries; changed content still gets its own.
    #[test]
    fn backup_skips_when_an_identical_copy_exists_under_any_name() {
        let dir = tmp("backup-identical");
        let path = dir.join("settings.json");
        fs::write(&path, "same").unwrap();
        let first = backup_at(&path, 1).unwrap().expect("first copy");
        assert_eq!(backup_at(&path, 2).unwrap(), None, "same bytes, no copy");
        // Only a differently named identical copy remains: still no new copy.
        let backup = dir.join(BACKUP_DIR);
        fs::rename(&first, backup.join("settings.json.bak-9-7")).unwrap();
        assert_eq!(backup_at(&path, 3).unwrap(), None);
        fs::rename(
            backup.join("settings.json.bak-9-7"),
            backup.join("other.txt"),
        )
        .unwrap();
        assert_eq!(
            backup_at(&path, 3).unwrap(),
            None,
            "name in _backup is ignored"
        );
        // Same size, different bytes: a copy is due.
        fs::write(&path, "diff").unwrap();
        let third = backup_at(&path, 4)
            .unwrap()
            .expect("changed content is copied");
        assert_eq!(fs::read_to_string(&third).unwrap(), "diff");
        // Identical bytes under any name in _backup skip a new copy.
        fs::write(backup.join("other.json"), "back").unwrap();
        fs::write(&path, "back").unwrap();
        assert_eq!(
            backup_at(&path, 5).unwrap(),
            None,
            "bytes anywhere in _backup count"
        );
        let _ = fs::remove_dir_all(dir);
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    }

    /// T249: a cap keeps the newest generations of one name and nothing else is touched.
    #[test]
    fn prune_keeps_the_newest_generations_of_that_name_only() {
        let dir = tmp("backup-prune");
        let path = dir.join("settings.json");
        let backup = dir.join(BACKUP_DIR);
        for (i, ts) in [(1, "10"), (2, "10-1"), (3, "10-2"), (4, "11"), (5, "9")] {
            fs::write(&path, format!("v{i}")).unwrap();
            let b = backup_at(&path, 0).unwrap().unwrap();
            fs::rename(b, backup.join(format!("settings.json.bak-{ts}"))).unwrap();
        }
        let foreign = [
            "settings.json.bak-old",
            "settings.json.bak-1-x",
            "settings.json.bak-+1",
            "settings.json.bak-",
            "settings.json.bak-1.bak-2",
            "mcp.json.bak-1",
            "notes.txt",
        ];
        for f in foreign {
            fs::write(backup.join(f), f).unwrap();
        }
        fs::write(&path, "v6").unwrap();
        let kept = backup_at(&path, 12).unwrap().unwrap();
        prune_backups(&kept, 3);
        let mut want: Vec<String> = foreign.iter().map(|s| s.to_string()).collect();
        want.extend(
            [
                "settings.json.bak-10-2",
                "settings.json.bak-11",
                "settings.json.bak-12",
            ]
            .map(String::from),
        );
        want.sort();
        assert_eq!(names(&backup), want);
        // 0 keeps all; a copy outside a `_backup` folder prunes nothing.
        prune_backups(&kept, 0);
        prune_backups(&dir.join("settings.json.bak-99"), 1);
        assert_eq!(names(&backup), want);
        let _ = fs::remove_dir_all(dir);
    }

    /// T249: the copy just taken is never deleted, even when an older name sorts newer.
    #[test]
    fn prune_never_deletes_the_copy_just_taken() {
        let dir = tmp("backup-prune-kept");
        let path = dir.join("settings.json");
        fs::write(&path, "future").unwrap();
        let future = backup_at(&path, 99).unwrap().unwrap();
        fs::write(&path, "now").unwrap();
        let kept = backup_at(&path, 5).unwrap().unwrap();
        prune_backups(&kept, 1);
        assert!(kept.exists(), "just taken");
        assert!(!future.exists(), "over the cap");
        let _ = fs::remove_dir_all(dir);
    }

    /// T249: within one second a new copy never reuses a slot pruning freed, so it sorts newest.
    #[test]
    fn a_copy_in_the_same_second_sorts_after_every_kept_one() {
        let dir = tmp("backup-slot");
        let path = dir.join("settings.json");
        fs::write(&path, "a").unwrap();
        let first = backup_at(&path, 7).unwrap().unwrap();
        fs::write(&path, "b").unwrap();
        backup_at(&path, 7).unwrap().unwrap();
        fs::remove_file(&first).unwrap();
        fs::write(&path, "c").unwrap();
        let third = backup_at(&path, 7).unwrap().unwrap();
        assert!(
            third.ends_with("settings.json.bak-7-2"),
            "{}",
            third.display()
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T249: `backup` itself prunes after a copy; a skipped (identical) copy prunes nothing.
    #[test]
    fn backup_caps_generations_per_file() {
        let dir = tmp("backup-cap");
        let path = dir.join("settings.json");
        for i in 0..4 {
            fs::write(&path, format!("v{i}")).unwrap();
            backup(&path, 2).unwrap().expect("new bytes are copied");
        }
        let backup_dir = dir.join(BACKUP_DIR);
        assert_eq!(names(&backup_dir).len(), 2);
        let bodies: Vec<String> = names(&backup_dir)
            .iter()
            .map(|n| fs::read_to_string(backup_dir.join(n)).unwrap())
            .collect();
        assert_eq!(bodies, ["v2", "v3"], "the newest two survive");
        assert_eq!(backup(&path, 1).unwrap(), None, "identical: no copy");
        assert_eq!(names(&backup_dir).len(), 2, "and no prune");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn write_leaves_no_temp_file_behind() {
        let dir = tmp("atomic");
        let path = dir.join("settings.json");
        write(&apply(), &path, "one\n", "+ something").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\n");
        write(&apply(), &path, "two\n", "+ something else").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two\n");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".rtok-tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");
        let _ = fs::remove_dir_all(dir);
    }

    /// Windows refuses `rename` over a read-only destination. Agent setup must
    /// still update a host config the user (or another tool) marked read-only.
    #[cfg(windows)]
    #[test]
    fn write_replaces_a_readonly_file() {
        let dir = tmp("readonly");
        let path = dir.join("settings.json");
        fs::write(&path, "old\n").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms).unwrap();
        assert!(fs::metadata(&path).unwrap().permissions().readonly());
        write(&apply(), &path, "new\n", "+ something").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tmp("perms");
        let path = dir.join(".claude.json");
        fs::write(&path, "{}\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        write(&apply(), &path, "{\"a\":1}\n", "+ something").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "write must not loosen an existing file's permissions"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_through_a_symlink_updates_the_target_and_keeps_the_link() {
        let dir = tmp("symlink");
        let real = dir.join("real.json");
        fs::write(&real, "{}\n").unwrap();
        let link = dir.join("linked.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write(&apply(), &link, "{\"a\":1}\n", "+ something").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "write must not replace the symlink with a plain file"
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "{\"a\":1}\n");
        assert_eq!(fs::read_to_string(&link).unwrap(), "{\"a\":1}\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn slots_replace_wrong_shapes_and_keep_right_ones() {
        let mut root = json!([1]);
        object_at(&mut root, "hooks");
        assert_eq!(root, json!({"hooks": {}}));
        let mut root = json!({"hooks": "nope", "keep": {"a": 1}});
        array_at(object_at(&mut root, "hooks"), "Stop").push(json!(1));
        assert_eq!(object_at(&mut root, "keep"), &json!({"a": 1}));
        assert_eq!(root["hooks"], json!({"Stop": [1]}));
    }

    /// A relative link to a file not created yet was replaced by a plain file.
    #[cfg(unix)]
    #[test]
    fn write_through_a_dangling_symlink_creates_the_target_and_keeps_the_link() {
        let dir = tmp("dangling");
        fs::create_dir_all(dir.join("dots")).unwrap();
        let link = dir.join("linked.json");
        std::os::unix::fs::symlink(Path::new("dots/real.json"), &link).unwrap();

        write(&apply(), &link, "{}\n", "+ something").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(dir.join("dots/real.json")).unwrap(),
            "{}\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mcp_registration_is_idempotent_and_leaves_foreign_servers() {
        let dir = tmp("mcp");
        let path = dir.join("mcp.json");
        fs::write(&path, r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();
        let a = apply();
        assert_eq!(
            register_mcp(&a, &path, "rtok", "rtok", &["mcp"]).unwrap(),
            "mcpServers.rtok: rtok mcp"
        );
        assert_eq!(
            register_mcp(&a, &path, "rtok", "rtok", &["mcp"]).unwrap(),
            NO_CHANGES
        );
        assert_eq!(
            unregister_mcp(&a, &path, "rtok").unwrap(),
            "- mcpServers.rtok"
        );
        assert_eq!(unregister_mcp(&a, &path, "rtok").unwrap(), NO_CHANGES);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("other"), "{raw}");
        assert!(!raw.contains("rtok"), "{raw}");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn plugin_link_replaces_a_dangling_link() {
        let dir = tmp("dangling");
        let src = dir.join("plugins/demo");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(dir.join("host")).unwrap();
        let dest = dir.join("host/rtok");
        std::os::unix::fs::symlink(dir.join("gone"), &dest).unwrap();
        let link = demo_link(src, dest.clone());
        assert!(link.run(&YES, false).unwrap().starts_with("+ plugin"));
        assert!(dest.exists(), "the link reaches the source again");
        assert_eq!(link.run(&YES, false).unwrap(), NO_CHANGES);
    }

    /// T164: a symlink whose target still exists but is not this build's source — the
    /// shape a ketch upgrade leaves behind, a prior version's plugin directory not yet
    /// pruned — is not up to date either. `run` must relink it, not report `NO_CHANGES`
    /// forever because *something* still resolves.
    #[cfg(unix)]
    #[test]
    fn plugin_link_relinks_a_stale_version_target() {
        let dir = tmp("stale-version");
        let old_src = dir.join("store/v1/plugins/demo");
        let new_src = dir.join("store/v2/plugins/demo");
        fs::create_dir_all(&old_src).unwrap();
        fs::create_dir_all(&new_src).unwrap();
        fs::create_dir_all(dir.join("host")).unwrap();
        let dest = dir.join("host/rtok");
        std::os::unix::fs::symlink(&old_src, &dest).unwrap();
        let link = demo_link(new_src.clone(), dest.clone());
        assert!(
            !link.up_to_date(keep_bytes),
            "an old version is not up to date"
        );
        assert_eq!(
            link.run(&YES, false).unwrap(),
            format!("+ plugin plugins/demo → {}", dest.display())
        );
        assert_eq!(fs::read_link(&dest).unwrap(), new_src);
        assert_eq!(link.run(&YES, false).unwrap(), NO_CHANGES);
    }

    /// T164: an owned copy (the non-Unix install shape) left over from an older version —
    /// marker present, content stale — is ours but not current, so it is replaced rather
    /// than reported as installed forever.
    #[test]
    fn plugin_link_relinks_a_stale_owned_copy() {
        let dir = tmp("stale-copy");
        let src = dir.join("plugins/demo");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("plugin.json"), "v2").unwrap();
        let dest = dir.join("host/plugins/rtok");
        copy_owned(&src, &dest).unwrap();
        fs::write(dest.join("plugin.json"), "v1").unwrap(); // an older, now-stale copy
        let link = demo_link(src, dest.clone());
        assert!(link.ours(), "an owned copy with our marker is ours");
        assert!(
            !link.up_to_date(keep_bytes),
            "stale content is not up to date"
        );
        assert_eq!(
            link.run(&YES, false).unwrap(),
            format!("+ plugin plugins/demo → {}", dest.display())
        );
        assert_eq!(fs::read_to_string(dest.join("plugin.json")).unwrap(), "v2");
    }

    /// T250.3: a `CopyFix` rewrites only the file it names on the way into an owned copy,
    /// and that fixed copy counts as up to date — not as stale bytes to reinstall each run.
    #[test]
    fn a_fixed_owned_copy_is_up_to_date() {
        fn upper_hooks(rel: &Path, bytes: Vec<u8>) -> Vec<u8> {
            if rel == Path::new("hooks/hooks.json") {
                bytes.to_ascii_uppercase()
            } else {
                bytes
            }
        }
        let dir = tmp("fixed-copy");
        let src = dir.join("plugins/demo");
        fs::create_dir_all(src.join("hooks")).unwrap();
        fs::write(src.join("plugin.json"), "v1").unwrap();
        fs::write(src.join("hooks/hooks.json"), "posix").unwrap();
        let dest = dir.join("host/plugins/rtok");
        copy_owned_with(&src, &dest, upper_hooks).unwrap();
        assert_eq!(
            fs::read_to_string(dest.join("hooks/hooks.json")).unwrap(),
            "POSIX"
        );
        assert_eq!(fs::read_to_string(dest.join("plugin.json")).unwrap(), "v1");
        let link = demo_link(src, dest);
        assert!(link.ours());
        assert!(link.up_to_date(upper_hooks));
        assert!(!link.up_to_date(keep_bytes), "unfixed source bytes differ");
        assert_eq!(link.run_with(&YES, false, upper_hooks).unwrap(), NO_CHANGES);
    }

    /// T164: default-install hosts must never overwrite a foreign directory at the plugin
    /// destination, even though nothing declined the offer — `--yes` alone cannot make a
    /// stranger's files ours. The install must still describe the offer, not silently sit
    /// as `NO_CHANGES`.
    #[test]
    fn plugin_link_never_installs_over_a_foreign_directory() {
        let dir = tmp("foreign-install");
        let src = dir.join("plugins/demo");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("plugin.json"), "ours").unwrap();
        let dest = dir.join("host/plugins/rtok");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("mine.txt"), "keep").unwrap();
        let link = PluginLink {
            src_rel: "plugins/demo",
            src,
            dest: dest.clone(),
            label: Some("~/.demo/plugins"),
            host: "demo",
        };
        let report = link.run(&YES, false).unwrap();
        assert!(
            report.starts_with("offer "),
            "a foreign directory must be offered, not silently skipped: {report}"
        );
        assert!(
            dest.join("mine.txt").exists(),
            "the foreign file must survive"
        );
        assert!(
            !dest.join("plugin.json").exists(),
            "ours must not be copied in"
        );
    }

    #[test]
    fn plugin_link_offers_then_links_then_unlinks() {
        let dir = tmp("link");
        let src = dir.join("plugins/demo");
        fs::create_dir_all(&src).unwrap();
        // A real plugin source is never empty; an empty `src` cannot prove a non-symlink
        // (Windows) install is up to date (T164: `tree_copies` needs at least one file to
        // compare), which would otherwise make the second `run` below reinstall instead of
        // reporting `NO_CHANGES`.
        fs::write(src.join("plugin.json"), "v1").unwrap();
        let link = PluginLink {
            src_rel: "plugins/demo",
            src,
            dest: dir.join("host/plugins/rtok"),
            label: Some("~/.demo/plugins"),
            host: "demo",
        };

        let dry = link
            .run(
                &Apply {
                    dry_run: true,
                    backup: false,
                    backup_files: 0,
                    yes: false,
                },
                false,
            )
            .unwrap();
        assert!(
            dry.starts_with(&format!(
                "offer plugins/demo → ~/.demo/plugins ({}) {KETCH_INSTALL}",
                link.dest.display()
            )),
            "{dry}"
        );
        assert!(!link.linked(), "a dry run must not link");

        // A host with no shorthand for its plugin dir (pi) spells the destination path
        // itself, with the source tree beside it on a dry run.
        let bare = PluginLink {
            src_rel: "plugins/demo",
            src: link.src.clone(),
            dest: dir.join("other/plugins/rtok"),
            label: None,
            host: "demo",
        };
        let bare_dry = bare
            .run(
                &Apply {
                    dry_run: true,
                    backup: false,
                    backup_files: 0,
                    yes: false,
                },
                false,
            )
            .unwrap();
        assert_eq!(
            bare_dry,
            format!(
                "offer plugins/demo → {} ({}) {KETCH_INSTALL}",
                bare.dest.display(),
                link.src.display()
            )
        );

        // No terminal, no --yes: the offer declines itself and still says how to accept.
        let declined = link.run(&apply(), false).unwrap();
        assert_eq!(
            declined,
            format!("offer plugins/demo → ~/.demo/plugins (accept with --yes) {KETCH_INSTALL}")
        );
        assert!(!link.linked());

        let yes = Apply {
            dry_run: false,
            backup: false,
            backup_files: 0,
            yes: true,
        };
        assert_eq!(
            link.run(&yes, false).unwrap(),
            format!(
                "+ plugin plugins/demo → {} ~/.demo/plugins",
                link.dest.display()
            )
        );
        assert!(link.linked());
        assert_eq!(link.run(&yes, false).unwrap(), NO_CHANGES);
        assert!(link.run(&yes, true).unwrap().starts_with("- plugin"));
        assert!(!link.linked());
        assert_eq!(link.run(&yes, true).unwrap(), NO_CHANGES);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_leaves_a_foreign_directory_alone() {
        let dir = tmp("foreign");
        let dest = dir.join("host/plugins/rtok");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("mine.txt"), "keep").unwrap();
        let link = PluginLink {
            src_rel: "plugins/demo",
            src: dir.join("plugins/demo"),
            dest: dest.clone(),
            label: Some("~/.demo/plugins"),
            host: "demo",
        };
        let yes = Apply {
            dry_run: false,
            backup: false,
            backup_files: 0,
            yes: true,
        };
        let report = link.run(&yes, true).unwrap();
        assert!(
            report.starts_with("leave "),
            "foreign dir must survive: {report}"
        );
        assert!(dest.join("mine.txt").exists(), "contents must stay");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn owned_copy_install_remove_is_idempotent() {
        let dir = tmp("owned-copy");
        let src = dir.join("plugins/demo");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("plugin.json"), "{}").unwrap();
        let dest = dir.join("host/plugins/rtok");
        let link = PluginLink {
            src_rel: "plugins/demo",
            src: src.clone(),
            dest: dest.clone(),
            label: Some("~/.demo/plugins"),
            host: "demo",
        };
        let yes = Apply {
            dry_run: false,
            backup: false,
            backup_files: 0,
            yes: true,
        };

        copy_owned(&src, &dest).unwrap();
        assert!(dest.join(OWNED_MARKER).is_file());
        assert!(dest.join("plugin.json").is_file());
        assert_eq!(link.run(&yes, false).unwrap(), NO_CHANGES);

        let report = link.run(&yes, true).unwrap();
        assert_eq!(report, format!("- plugin {}", dest.display()));
        assert!(!dest.exists(), "owned copy must be removed");
        assert_eq!(link.run(&yes, true).unwrap(), NO_CHANGES);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn foreign_dir_without_marker_is_left_on_remove() {
        let dir = tmp("foreign-no-marker");
        let dest = dir.join("host/plugins/rtok");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("mine.txt"), "keep").unwrap();
        // No OWNED_MARKER — even though it is a directory, remove must leave it.
        let link = PluginLink {
            src_rel: "plugins/demo",
            src: dir.join("plugins/demo"),
            dest: dest.clone(),
            label: None,
            host: "demo",
        };
        let yes = Apply {
            dry_run: false,
            backup: false,
            backup_files: 0,
            yes: true,
        };
        let report = link.run(&yes, true).unwrap();
        assert!(report.starts_with("leave "), "{report}");
        assert!(dest.join("mine.txt").exists());
        let _ = fs::remove_dir_all(dir);
    }

    /// T75: a host can materialize our symlink into a plain copy with no marker. The
    /// copy is provably ours (every src byte present) so remove takes it back — that is
    /// the "uninstall did not take effect on disk" half of the stuck green check.
    #[test]
    fn remove_wipes_a_materialized_copy_of_our_tree() {
        let dir = tmp("materialized-copy");
        let src = dir.join("plugins/demo");
        fs::create_dir_all(src.join("hooks")).unwrap();
        fs::write(src.join("plugin.json"), "{}").unwrap();
        fs::write(src.join("hooks").join("pre.js"), "ours").unwrap();
        let dest = dir.join("host/plugins/rtok");
        // A copy of the tree with the marker deliberately absent, plus a host-side extra
        // file beside ours — extra files must not break the ownership proof.
        fs::create_dir_all(dest.join("hooks")).unwrap();
        fs::copy(src.join("plugin.json"), dest.join("plugin.json")).unwrap();
        fs::copy(
            src.join("hooks").join("pre.js"),
            dest.join("hooks").join("pre.js"),
        )
        .unwrap();
        fs::write(dest.join("host-cache.bin"), "host wrote this").unwrap();

        let link = demo_link(src.clone(), dest.clone());
        assert!(link.ours(), "a byte-complete copy of our tree is ours");
        assert_eq!(
            link.run(&YES, true).unwrap(),
            format!("- plugin {}", dest.display())
        );
        assert!(!dest.exists(), "the materialized copy must go");
        assert!(!link.ours(), "and no longer reads as ours");
        assert_eq!(link.run(&YES, true).unwrap(), NO_CHANGES);
        let _ = fs::remove_dir_all(dir);
    }

    /// T75: the read side — a directory that differs from our tree by one byte is not
    /// ours, so an `installed()` built on [`PluginLink::ours`] cannot stay green over a
    /// foreign directory the uninstall was right to leave alone.
    #[test]
    fn a_directory_that_differs_by_a_byte_is_not_ours() {
        let dir = tmp("not-ours");
        let src = dir.join("plugins/demo");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("plugin.json"), "ours").unwrap();
        let dest = dir.join("host/plugins/rtok");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("plugin.json"), "someone else's").unwrap();
        let link = PluginLink {
            src_rel: "plugins/demo",
            src,
            dest,
            label: None,
            host: "demo",
        };
        assert!(link.linked(), "something is there");
        assert!(!link.ours(), "but it is not ours");
        let _ = fs::remove_dir_all(dir);
    }

    fn skill_view(
        present: bool,
        symlink: bool,
        prefixed: bool,
        marked: bool,
        same_bytes: bool,
    ) -> SkillView {
        SkillView {
            present,
            symlink,
            prefixed,
            marked,
            same_bytes,
        }
    }

    #[test]
    fn skill_plan_covers_install_reinstall_remove_and_foreign() {
        use SkillPlan::*;
        let absent = skill_view(false, false, false, false, false);
        assert_eq!(skill_plan(false, absent), Copy);
        assert_eq!(skill_plan(true, absent), NoChanges);
        // Unprefixed + marker, bytes differ: already installed, do not clobber the edit.
        assert_eq!(
            skill_plan(false, skill_view(true, false, false, true, false)),
            NoChanges
        );
        assert_eq!(
            skill_plan(false, skill_view(true, false, false, false, false)),
            LeaveForeign
        );
        assert_eq!(
            skill_plan(true, skill_view(true, false, false, true, false)),
            Remove
        );
        assert_eq!(
            skill_plan(true, skill_view(true, false, false, false, true)),
            Remove
        );
        assert_eq!(
            skill_plan(true, skill_view(true, false, false, false, false)),
            NoChanges
        );
        // Prefix replaces a stale or foreign tree and a symlink; an unprefixed symlink is not ours.
        assert_eq!(
            skill_plan(false, skill_view(true, false, true, false, false)),
            Copy
        );
        assert_eq!(
            skill_plan(false, skill_view(true, true, true, false, false)),
            Copy
        );
        assert_eq!(
            skill_plan(false, skill_view(true, true, false, false, false)),
            LeaveForeign
        );
        assert_eq!(
            skill_plan(true, skill_view(true, true, true, false, false)),
            Remove
        );
        assert_eq!(
            skill_plan(true, skill_view(true, true, false, false, false)),
            NoChanges
        );
    }

    #[test]
    fn skill_copy_install_reinstall_remove_keeps_foreign() {
        let dir = tmp("skill-copy");
        let src = dir.join("src");
        let skills = dir.join("skills");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "hub body\n").unwrap();
        let dest = skills.join("rtok");
        let foreign = skills.join("other");
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("SKILL.md"), "# other\n").unwrap();

        let copy = SkillCopy {
            src,
            dest: dest.clone(),
            label: None,
        };
        let first = copy.run(&apply(), false).unwrap();
        assert!(first.starts_with("+ skill"), "{first}");
        assert!(dest.join(OWNED_MARKER).is_file());
        assert_eq!(copy.run(&apply(), false).unwrap(), NO_CHANGES);
        assert_eq!(
            copy.run(&apply(), true).unwrap(),
            format!("- skill {}", dest.display())
        );
        assert_eq!(copy.run(&apply(), true).unwrap(), NO_CHANGES);
        assert!(foreign.join("SKILL.md").is_file());
        let _ = fs::remove_dir_all(dir);
    }

    /// T246.4: a skill copy the user wrote to after rtok marked it stays unless `--yes`; with
    /// no terminal here nobody answers, so the remove leaves it and says why.
    #[test]
    fn skill_copy_remove_asks_before_taking_an_edited_copy() {
        let dir = tmp("skill-edited");
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "hub body\n").unwrap();
        let dest = dir.join("skills/rtok");
        let copy = SkillCopy {
            src,
            dest: dest.clone(),
            label: None,
        };
        copy.run(&apply(), false).unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        let file = fs::File::options()
            .write(true)
            .open(dest.join("SKILL.md"))
            .unwrap();
        file.set_modified(later).unwrap();

        let out = copy.run(&apply(), true).unwrap();
        assert!(
            out.starts_with("leave ") && out.contains("changed by you"),
            "{out}"
        );
        assert!(dest.join("SKILL.md").is_file());
        let yes = Apply {
            yes: true,
            ..apply()
        };
        assert!(copy.run(&yes, true).unwrap().starts_with("- skill"));
        assert!(!dest.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn skill_copy_leaves_a_foreign_skill_tree() {
        let dir = tmp("skill-foreign");
        let src = dir.join("src");
        let dest = dir.join("skills/notes");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "hub body\n").unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("SKILL.md"), "# foreign\n").unwrap();

        let out = SkillCopy {
            src,
            dest: dest.clone(),
            label: None,
        }
        .run(&apply(), false)
        .unwrap();
        assert!(out.contains("leave"), "{out}");
        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            "# foreign\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T380: a destination named `rtok…` is ours, so a foreign tree there is replaced.
    #[test]
    fn skill_copy_overwrites_a_prefixed_foreign_tree() {
        let dir = tmp("skill-prefix");
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "hub body\n").unwrap();
        let dest = dir.join("skills/rtok-x");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("SKILL.md"), "# foreign\n").unwrap();

        let copy = SkillCopy {
            src,
            dest: dest.clone(),
            label: None,
        };
        let out = copy.run(&apply(), false).unwrap();
        assert!(out.starts_with("+ skill"), "{out}");
        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            "hub body\n"
        );
        assert!(dest.join(OWNED_MARKER).is_file());
        assert!(copy.run(&apply(), true).unwrap().starts_with("- skill"));
        assert!(!dest.exists());
        let _ = fs::remove_dir_all(dir);
    }

    /// T380: an unprefixed directory that matches `src` byte for byte is ours without a marker.
    #[test]
    fn skill_copy_remove_takes_an_unprefixed_byte_copy() {
        let dir = tmp("skill-bytes");
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "hub body\n").unwrap();
        let dest = dir.join("skills/worktrees");
        fs::create_dir_all(&dest).unwrap();
        fs::copy(src.join("SKILL.md"), dest.join("SKILL.md")).unwrap();

        let copy = SkillCopy {
            src,
            dest: dest.clone(),
            label: None,
        };
        assert_eq!(copy.run(&apply(), false).unwrap(), NO_CHANGES);
        assert_eq!(
            copy.run(&apply(), true).unwrap(),
            format!("- skill {}", dest.display())
        );
        assert!(!dest.exists());
        let _ = fs::remove_dir_all(dir);
    }

    /// T380: a prefixed symlink is replaced; the target stays.
    #[test]
    #[cfg(unix)]
    fn skill_copy_unlinks_a_prefixed_symlink_and_leaves_its_target() {
        let dir = tmp("skill-prefix-link");
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "hub body\n").unwrap();
        let target = dir.join("personal");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), "mine\n").unwrap();
        let dest = dir.join("skills/rtok-worktrees");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &dest).unwrap();

        let copy = SkillCopy {
            src,
            dest: dest.clone(),
            label: None,
        };
        let out = copy.run(&apply(), false).unwrap();
        assert!(out.starts_with("+ skill"), "{out}");
        assert!(!dest.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            "hub body\n"
        );
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "mine\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// T380: an unprefixed symlink is not ours. Install and remove both leave the link and target.
    #[test]
    #[cfg(unix)]
    fn skill_copy_leaves_an_unprefixed_symlink() {
        let dir = tmp("skill-foreign-link");
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "hub body\n").unwrap();
        let target = dir.join("personal");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), "mine\n").unwrap();
        let dest = dir.join("skills/worktrees");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &dest).unwrap();

        let copy = SkillCopy {
            src,
            dest: dest.clone(),
            label: None,
        };
        let installed = copy.run(&apply(), false).unwrap();
        assert!(installed.contains("leave"), "{installed}");
        assert!(dest.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(copy.run(&apply(), true).unwrap(), NO_CHANGES);
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "mine\n"
        );
        let _ = fs::remove_dir_all(dir);
    }
}

// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T441.13.3 (F19): a new worktree's Claude Code session shares one task list, named
//! `<project>-<task>`, through `env.CLAUDE_CODE_TASK_LIST_ID` in the worktree's
//! `.claude/settings.local.json`. The variable and the `env` block are documented at
//! https://code.claude.com/docs/en/env-vars and https://code.claude.com/docs/en/settings
//! (checked 2026-10-11). The file is a host config: only our key is written, through the same
//! byte-preserving editor `rtok agents install` uses, and a file we cannot edit safely is
//! warned about and left alone.

use std::path::Path;

use anyhow::{Result, bail};
use rtok_agent_sdk::{Apply, write};
use serde_json::{Value, json};

use super::jsonc;

const FILE: &str = ".claude/settings.local.json";
const KEY: &str = "CLAUDE_CODE_TASK_LIST_ID";

/// Write the task list id for the worktree at `dest`; `cwd` names the project. Fail open: the
/// worktree already exists, so any problem is a warning.
pub fn write_for(cwd: &Path, dest: &Path, task: &str) {
    let Some(project) = crate::project::project_name(cwd) else {
        return;
    };
    // An untracked file would make `worktree remove` refuse the worktree as dirty, and a host
    // config must never reach a commit, so a repository that does not ignore it gets nothing.
    if !crate::worktree::git::is_ignored(dest, FILE) {
        crate::log::stderr_ln(&format!(
            "warning: task list id not set: {FILE} is not git-ignored in {}",
            dest.display()
        ));
        return;
    }
    let path = dest.join(FILE);
    if let Err(e) = set(&path, &format!("{project}-{task}")) {
        crate::log::stderr_ln(&format!("warning: task list id not set: {e:#}"));
    }
}

fn set(path: &Path, id: &str) -> Result<()> {
    let raw = jsonc::read_or_empty(path)?;
    if let Some(why) = refusal(&raw, path, id)? {
        bail!("{} {why}; left untouched", path.display());
    }
    let (text, change) = jsonc::upsert_member(&raw, path, "env", KEY, &json!(id))?;
    if change == jsonc::Upsert::NoChange {
        return Ok(());
    }
    let apply = Apply {
        dry_run: false,
        backup: false,
        backup_files: 0,
        yes: true,
    };
    write(&apply, path, &text, &format!("env.{KEY}: {id}"))
}

/// Why `raw` must not be edited, or `None` when it is blank or an object whose `env` is absent
/// or an object whose key is absent or already ours. `upsert_member` would replace a root or `env` of another
/// shape, and a key the user set is theirs to keep.
fn refusal(raw: &str, path: &Path, id: &str) -> Result<Option<&'static str>> {
    if jsonc::strip_comments(raw).trim().is_empty() {
        return Ok(None);
    }
    let root = jsonc::parse(raw).map_err(|e| anyhow::anyhow!("{}: {e:#}", path.display()))?;
    let Some(root) = root.as_object() else {
        return Ok(Some("is not a JSON object"));
    };
    Ok(match root.get("env") {
        None => None,
        Some(Value::Object(env)) if env.get(KEY).is_some_and(|v| v != id) => {
            Some("already sets another task list id")
        }
        Some(Value::Object(_)) => None,
        Some(_) => Some("has an `env` that is not an object"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-tasklist-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join(FILE)
    }

    #[test]
    fn a_new_file_gets_only_our_key() {
        let path = temp("new");
        set(&path, "rtok-t1").unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v, json!({"env": {KEY: "rtok-t1"}}));
    }

    #[test]
    fn every_other_byte_of_an_existing_file_stays() {
        let path = temp("keep");
        let before = "{\n  // mine\n  \"permissions\": {\"allow\": [\"Bash(ls)\",],},\n  \"env\": {\n    \"A\": \"1\",\n  },\n}\n";
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, before).unwrap();
        set(&path, "rtok-t1").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("// mine"), "{after}");
        assert!(
            after.contains("\"permissions\": {\"allow\": [\"Bash(ls)\",],},"),
            "{after}"
        );
        assert!(after.contains("\"A\": \"1\""), "{after}");
        assert!(
            after.contains("\"CLAUDE_CODE_TASK_LIST_ID\": \"rtok-t1\""),
            "{after}"
        );
        set(&path, "rtok-t1").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), after, "idempotent");
    }

    #[test]
    fn nothing_is_written_where_git_does_not_ignore_the_file() {
        let dir = temp("tracked");
        let repo = dir.parent().unwrap().parent().unwrap();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .status();
            assert!(ok.unwrap().success(), "{args:?}");
        };
        git(&["init", "-q"]);
        // A global excludes file that ignores `.claude/` (common for Claude Code users) would
        // make the first write happen; the repo's own config outranks it.
        let excludes = repo.join("no-global-excludes");
        fs::write(&excludes, "").unwrap();
        git(&["config", "core.excludesFile", excludes.to_str().unwrap()]);
        write_for(repo, repo, "t1");
        assert!(!dir.exists(), "{}", dir.display());
        fs::write(repo.join(".gitignore"), ".claude/\n").unwrap();
        write_for(repo, repo, "t1");
        let v: Value = serde_json::from_str(&fs::read_to_string(&dir).unwrap()).unwrap();
        assert!(v["env"][KEY].as_str().unwrap().ends_with("-t1"), "{v}");
    }

    #[test]
    fn a_malformed_file_is_left_untouched() {
        let path = temp("bad");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ \"env\": ").unwrap();
        assert!(set(&path, "rtok-t1").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ \"env\": ");
    }

    #[test]
    fn a_value_the_user_set_or_a_foreign_shape_is_kept() {
        for before in [
            "{\"env\": {\"CLAUDE_CODE_TASK_LIST_ID\": \"mine\"}}",
            "{\"env\": \"x\"}",
            "[1]",
        ] {
            let path = temp("kept");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, before).unwrap();
            assert!(set(&path, "rtok-t1").is_err(), "{before}");
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
        }
    }
}

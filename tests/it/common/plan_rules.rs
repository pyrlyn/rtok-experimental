// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The rules for one `plan/<id>/` folder (T954, T955). `tests/plan_dir.rs` runs them over the
//! repository's plan; the `disk` adapter's tests run them over the folders it writes.

use std::fs;
use std::path::Path;

const KEYS: [&str; 5] = ["status", "priority", "complexity", "readiness", "agent"];

/// Splits `---\nkey: value\n---\nbody` into its pairs and the body; `None` without front matter.
fn front_matter(text: &str) -> Option<(Vec<(String, String)>, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    let pairs = rest[..end]
        .lines()
        .map(|l| {
            let (k, v) = l.split_once(':').unwrap_or((l, ""));
            (k.trim().to_string(), v.trim().to_string())
        })
        .collect();
    Some((pairs, &rest[end + 5..]))
}

pub fn active_task_errors(id: &str, text: &str) -> Vec<String> {
    let Some((pairs, body)) = front_matter(text) else {
        return vec![format!("{id}: active task without front matter")];
    };
    let mut errs = Vec::new();
    let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
    if keys != KEYS {
        errs.push(format!("{id}: front matter keys {keys:?}, want {KEYS:?}"));
        return errs;
    }
    let v = |k: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
            .unwrap()
    };
    let status = v("status");
    if !matches!(status, "todo" | "in progress") {
        errs.push(format!("{id}: status {status:?}, want todo or in progress"));
    }
    if !matches!(v("priority"), "P0" | "P1" | "P2" | "P3") {
        errs.push(format!("{id}: priority {:?}, want P0..P3", v("priority")));
    }
    if !matches!(v("complexity"), "1" | "2" | "3" | "4" | "5") {
        errs.push(format!("{id}: complexity {:?}, want 1..5", v("complexity")));
    }
    let readiness_ok = v("readiness")
        .strip_suffix('%')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| n <= 100);
    if !readiness_ok {
        errs.push(format!(
            "{id}: readiness {:?}, want 0%..100%",
            v("readiness")
        ));
    }
    let agent = v("agent");
    if (status == "todo") != agent.is_empty() {
        errs.push(format!("{id}: agent {agent:?} with status {status:?}"));
    }
    let checks = body.lines().filter(|l| l.starts_with("Check:")).count();
    if checks != 1 {
        errs.push(format!("{id}: {checks} Check: lines, want 1"));
    }
    if agent.to_lowercase().contains("haiku") && body.contains("src/") {
        errs.push(format!("{id}: low-cost model on a task touching src/"));
    }
    errs
}

/// Every rule a folder breaks, each message starting with the folder's name.
#[allow(dead_code)] // each includer uses the pieces it needs
pub fn folder_errors(path: &Path) -> Vec<String> {
    let id = path.file_name().unwrap().to_string_lossy().into_owned();
    let id_re = regex::Regex::new(r"^[A-Z][0-9]+(?:\.[0-9]+)*[a-z]?$").unwrap();
    if !id_re.is_match(&id) {
        return vec![format!("{id}: folder name is not a task id")];
    }
    let mut errs = Vec::new();
    let Ok(task) = fs::read_to_string(path.join("task.md")) else {
        return vec![format!("{id}: no task.md")];
    };
    let heading = format!("# {id}. ");
    if !task.lines().any(|l| l.starts_with(&heading)) {
        errs.push(format!("{id}: task.md has no `{heading}Title` heading"));
    }
    match fs::read_to_string(path.join("done.md")) {
        Ok(done) if done.trim().is_empty() => errs.push(format!("{id}: empty done.md")),
        Ok(_) => {}
        Err(_) => errs.extend(active_task_errors(&id, &task)),
    }
    errs
}

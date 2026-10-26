// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Structural checks on `plan/` (T954, after T198's `plan.md` checks).
//!
//! Rules enforced here:
//! 1. every folder in `plan/` is named as a task id and holds a `task.md` whose `# <id>. Title`
//!    heading names that same id.
//! 2. a folder with `done.md` is a closed task; its `done.md` is not empty.
//! 3. an active task (no `done.md`) opens with front matter holding exactly `status` (`todo`
//!    or `in progress`), `priority` (`P0`–`P3`), `complexity` (`1`–`5`), `readiness`
//!    (`0%`–`100%`) and `agent` (empty for `todo`, set for `in progress`).
//! 4. an active task has exactly one line starting with `Check:`.
//! 5. no active task whose agent names a low-cost model (`haiku`) mentions `src/` —
//!    AGENTS.md forbids low-cost models on code.

#[path = "common/plan_rules.rs"]
mod plan_rules;

use plan_rules::{active_task_errors, folder_errors};
use std::fs;
use std::path::Path;

#[test]
fn plan_folders_are_sane() {
    let plan = Path::new(env!("CARGO_MANIFEST_DIR")).join("plan");
    let mut errs = Vec::new();
    let mut seen = 0;
    for entry in fs::read_dir(&plan).expect("read plan/") {
        let path = entry.unwrap().path();
        if !path.is_dir() {
            continue;
        }
        seen += 1;
        errs.extend(folder_errors(&path));
    }
    assert!(seen > 0, "no task folders in plan/");
    errs.sort();
    assert!(errs.is_empty(), "plan/ drift:\n{}", errs.join("\n"));
}

#[test]
fn front_matter_rejects_a_bad_task() {
    let text =
        "---\nstatus: done\npriority: P9\ncomplexity: 7\nreadiness: 120%\nagent:\n---\n\n# T1. X\n";
    let errs = active_task_errors("T1", text);
    for want in ["status", "priority", "complexity", "readiness", "Check:"] {
        assert!(
            errs.iter().any(|e| e.contains(want)),
            "{want} not flagged: {errs:?}"
        );
    }
    assert_eq!(active_task_errors("T1", "# T1. X\n").len(), 1);
}

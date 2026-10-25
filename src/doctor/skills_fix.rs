// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok doctor --fix --only stale-skills` (T975): update the installed rtok skills the doctor
//! reports as outdated through `skill::sync`, the copy `rtok agents update` makes, so the two
//! paths cannot disagree on what an rtok skill is or which directory may be replaced.

use rtok_agent_sdk::NO_CHANGES;

use super::checklist::Prompt;
use crate::agents::skill::{self, Outdated};
use crate::config::Config;

/// The report text and the exit code. Without `apply` a `prompt` asks once; no prompt is a dry run.
pub fn run(
    cfg: &Config,
    apply: bool,
    agent: Option<&str>,
    prompt: Option<&mut dyn Prompt>,
) -> (String, i32) {
    let found = for_agent(skill::outdated(cfg), agent);
    if found.is_empty() {
        return ("rtok skills: up to date\n".into(), 0);
    }
    let list: String = found
        .iter()
        .map(|s| format!("outdated skill {} ({})\n", s.path.display(), s.host()))
        .collect();
    let mut out = list.clone();
    if !apply {
        let Some(prompt) = prompt else {
            out.push_str("dry run: nothing is written. Add --yes to update them.\n");
            return (out, 0);
        };
        let answer = prompt.ask(&format!("{list}update {}? [y/N] ", count(found.len())));
        if !answer.is_some_and(|a| rtok_agent_sdk::answer(&a, false)) {
            return ("cancelled: no skill was updated\n".into(), 0);
        }
        out.clear();
    }
    let mut keys: Vec<&str> = found.iter().map(|s| s.key).collect();
    keys.dedup();
    let mut cfg = cfg.clone();
    cfg.setup.dry_run = false;
    let mut code = 0;
    for key in keys {
        match skill::sync(key, &cfg, false) {
            Ok(lines) if lines != NO_CHANGES => out.push_str(&format!("{lines}\n")),
            Ok(_) => {}
            Err(e) => {
                out.push_str(&format!("failed to update the {key} skills: {e:#}\n"));
                code = 1;
            }
        }
    }
    let left = for_agent(skill::outdated(&cfg), agent).len();
    out.push_str(&format!("{} outdated, {left} left\n", count(found.len())));
    (out, code.max(i32::from(left > 0)))
}

fn for_agent(found: Vec<Outdated>, agent: Option<&str>) -> Vec<Outdated> {
    found
        .into_iter()
        .filter(|s| agent.is_none_or(|a| s.host() == a))
        .collect()
}

fn count(n: usize) -> String {
    format!("{n} rtok skill{}", if n == 1 { "" } else { "s" })
}

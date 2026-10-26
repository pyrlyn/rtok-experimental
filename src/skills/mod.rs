// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Skill management: list, enable and disable `SKILL.md` skills of the agent rtok runs in.
//!
//! Every supported agent discovers skills as `<root>/<name>/SKILL.md`. Disabling moves the
//! skill directory to the sibling `<root>-disabled/<name>`, which no agent scans; enabling
//! moves it back. The file contents are never edited, so a disable is fully reversible.
//! Shapes come from the OpenAPI contract in `openapi/skills.yaml` ([`api`]).

#[allow(
    missing_docs,
    unused_imports,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]
pub mod api;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
pub use api::{Agent, Scope, Skill, SkillList, ToggleRequest, ToggleResult};

/// Agents in display order.
pub const AGENTS: [Agent; 3] = [Agent::Claude, Agent::Cursor, Agent::Copilot];

/// Where to look: the user's home and the project directory.
#[derive(Debug, Clone)]
pub struct Roots {
    /// User home (`~`).
    pub home: PathBuf,
    /// Project directory (usually the current directory).
    pub project: PathBuf,
    /// `COPILOT_HOME` override, when set.
    pub copilot_home: Option<PathBuf>,
}

impl Roots {
    /// Skills directory an agent scans for a scope.
    pub fn skills_dir(&self, agent: Agent, scope: Scope) -> PathBuf {
        match (agent, scope) {
            (Agent::Claude, Scope::User) => self.home.join(".claude").join("skills"),
            (Agent::Claude, Scope::Project) => self.project.join(".claude").join("skills"),
            (Agent::Cursor, Scope::User) => self.home.join(".cursor").join("skills"),
            (Agent::Cursor, Scope::Project) => self.project.join(".cursor").join("skills"),
            (Agent::Copilot, Scope::User) => self
                .copilot_home
                .clone()
                .unwrap_or_else(|| self.home.join(".copilot"))
                .join("skills"),
            (Agent::Copilot, Scope::Project) => self.project.join(".github").join("skills"),
        }
    }

    /// Parking directory for disabled skills: `<skills dir>-disabled`.
    pub fn disabled_dir(&self, agent: Agent, scope: Scope) -> PathBuf {
        let dir = self.skills_dir(agent, scope);
        let mut name = dir.file_name().unwrap_or_default().to_os_string();
        name.push("-disabled");
        dir.with_file_name(name)
    }
}

/// Parse an agent name (`claude`, `cursor`, `copilot`).
pub fn parse_agent(s: &str) -> Result<Agent> {
    match s.to_ascii_lowercase().as_str() {
        "claude" | "claude-code" => Ok(Agent::Claude),
        "cursor" => Ok(Agent::Cursor),
        "copilot" | "github-copilot" => Ok(Agent::Copilot),
        other => bail!("unknown agent `{other}` (expected claude, cursor or copilot)"),
    }
}

/// The agent rtok runs inside, from the environment: `RTOK_AGENT`, then the
/// variables each agent sets for the commands it runs.
pub fn detect_agent(env: impl Fn(&str) -> Option<String>) -> Option<Agent> {
    if let Some(name) = env("RTOK_AGENT") {
        return parse_agent(&name).ok();
    }
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    if set("CLAUDECODE") {
        Some(Agent::Claude)
    } else if set("CURSOR_AGENT") || set("CURSOR_TRACE_ID") {
        Some(Agent::Cursor)
    } else if set("COPILOT_CLI") || set("COPILOT_AGENT_SESSION_ID") {
        Some(Agent::Copilot)
    } else {
        None
    }
}

/// Reject names that would escape the skills directory.
fn check_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.starts_with('.')
    {
        bail!("invalid skill name `{name}`");
    }
    Ok(())
}

/// `description:` from the YAML front matter of a `SKILL.md`.
fn description(skill_md: &Path) -> Option<String> {
    let text = std::fs::read_to_string(skill_md).ok()?;
    let body = text.strip_prefix("---")?;
    let end = body.find("\n---")?;
    body[..end].lines().find_map(|line| {
        let v = line.strip_prefix("description:")?.trim();
        let v = v.trim_matches(|c| c == '"' || c == '\'');
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// Total bytes of the regular files under `dir` (symlinks are not followed).
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

/// Display size: `999 B` below 1000 bytes, else KB (1 KB = 1000 bytes) with one decimal,
/// `1.0 KB`, `1.5 KB`, `12.3 KB`. The exact count is `size_bytes` in the contract.
pub fn format_size(bytes: i64) -> String {
    if bytes < 1000 {
        format!("{bytes} B")
    } else {
        #[allow(clippy::cast_precision_loss)]
        let kb = bytes as f64 / 1000.0;
        format!("{kb:.1} KB")
    }
}

fn scan(dir: &Path, agent: Agent, scope: Scope, enabled: bool, out: &mut Vec<Skill>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let md = path.join("SKILL.md");
        if !md.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let size = i64::try_from(dir_size(&path)).unwrap_or(i64::MAX);
        let mut skill = Skill::new(
            name,
            agent,
            scope,
            enabled,
            path.display().to_string(),
            size,
        );
        skill.description = description(&md);
        out.push(skill);
    }
}

/// All skills of an agent, enabled and disabled, optionally for one scope.
pub fn list(roots: &Roots, agent: Agent, scope: Option<Scope>) -> SkillList {
    let mut skills = Vec::new();
    for s in [Scope::User, Scope::Project] {
        if scope.is_some_and(|want| want != s) {
            continue;
        }
        scan(&roots.skills_dir(agent, s), agent, s, true, &mut skills);
        scan(&roots.disabled_dir(agent, s), agent, s, false, &mut skills);
    }
    skills.sort_by(|a, b| (a.scope, &a.name).cmp(&(b.scope, &b.name)));
    SkillList::new(agent, skills)
}

/// Enable or disable one skill by moving its directory.
pub fn toggle(roots: &Roots, req: &ToggleRequest) -> Result<ToggleResult> {
    check_name(&req.name)?;
    let found: Vec<Skill> = list(roots, req.agent, req.scope)
        .skills
        .into_iter()
        .filter(|s| s.name == req.name)
        .collect();
    let skill = match found.as_slice() {
        [] => bail!("no {} skill named `{}`", req.agent, req.name),
        [one] => one.clone(),
        many if many.iter().all(|s| s.scope == many[0].scope) => {
            bail!(
                "`{}` exists both enabled and disabled in {:?} scope; remove one copy",
                req.name,
                many[0].scope
            )
        }
        _ => bail!(
            "`{}` exists in user and project scope; pass --scope",
            req.name
        ),
    };
    if skill.enabled == req.enabled {
        return Ok(ToggleResult::new(skill, false));
    }
    let target_root = if req.enabled {
        roots.skills_dir(req.agent, skill.scope)
    } else {
        roots.disabled_dir(req.agent, skill.scope)
    };
    let to = target_root.join(&req.name);
    if to.exists() {
        bail!("{} already exists", to.display());
    }
    let from = PathBuf::from(&skill.path);
    if !req.dry_run.unwrap_or(false) {
        std::fs::create_dir_all(&target_root)
            .with_context(|| format!("create {}", target_root.display()))?;
        std::fs::rename(&from, &to)
            .with_context(|| format!("move {} -> {}", from.display(), to.display()))?;
    }
    let mut moved = skill;
    moved.enabled = req.enabled;
    moved.path = to.display().to_string();
    let mut result = ToggleResult::new(moved, true);
    result.from = Some(from.display().to_string());
    result.to = Some(to.display().to_string());
    Ok(result)
}

#[cfg(test)]
mod tests;

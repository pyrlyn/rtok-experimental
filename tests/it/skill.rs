// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T71.3 / T155: size limits for every shipped skill (`skills/<name>/SKILL.md`), and the
//! `rtok-worktrees` skill names only commands `rtok worktree` really has, and
//! (T234) no host plugin bundles a copy of a skill.

use crate::it::common;

use clap::CommandFactory;
use common::agents::{rtok, tmp, write_cfg};
use rtok::agents::skill::{SKILL_DESC_MAX, SKILLS};
use rtok::cli::Cli;
use std::fs;
use std::path::PathBuf;

fn hub_skill(name: &str) -> String {
    hub_file(name, "SKILL.md")
}

fn hub_file(name: &str, file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("skills/{name}/{file}"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn split_frontmatter(content: &str) -> (&str, &str) {
    let content = content.strip_prefix("---\n").expect("opening ---");
    let (front, body) = content.split_once("\n---\n").expect("closing ---");
    (front, body)
}

fn description(front: &str) -> String {
    front
        .lines()
        .find_map(|l| l.strip_prefix("description: "))
        .expect("description:")
        .to_string()
}

#[test]
fn every_skill_description_is_at_most_120_chars() {
    for name in SKILLS {
        let content = hub_skill(name);
        let (front, _) = split_frontmatter(&content);
        let desc = description(front);
        assert!(
            desc.chars().count() <= SKILL_DESC_MAX,
            "{name}: description is {} chars: {desc}",
            desc.chars().count()
        );
        assert!(
            front.lines().any(|l| l == format!("name: {name}")),
            "{name}: frontmatter name must match the directory"
        );
    }
}

#[test]
fn every_skill_body_is_at_most_2kb_and_has_no_disable_model_invocation() {
    for name in SKILLS {
        let content = hub_skill(name);
        let (front, body) = split_frontmatter(&content);
        assert!(
            body.len() <= 2048,
            "{name}: body is {} bytes (limit 2048)",
            body.len()
        );
        assert!(
            !front.contains("disable-model-invocation"),
            "{name}: hub skill must stay model-invokable"
        );
    }
}

/// T155: every inline `rtok worktree <sub> … --flag …` span in the skill resolves against the
/// real CLI, so the skill cannot teach a command or flag that `rtok worktree --help` lacks.
#[test]
fn worktrees_skill_names_only_commands_rtok_worktree_has() {
    let cli = Cli::command();
    let worktree = cli.find_subcommand("worktree").expect("rtok worktree");
    let content = hub_skill("rtok-worktrees");
    let (_, body) = split_frontmatter(&content);
    let mut seen = Vec::new();
    for span in body.split('`').skip(1).step_by(2) {
        let Some(rest) = span.strip_prefix("rtok worktree ") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let sub = words.next().expect("subcommand after `rtok worktree`");
        let cmd = worktree
            .find_subcommand(sub)
            .unwrap_or_else(|| panic!("`rtok worktree {sub}` is not a subcommand"));
        for flag in words.filter_map(|w| w.strip_prefix("--")) {
            assert!(
                cmd.get_arguments().any(|a| a.get_long() == Some(flag)),
                "`rtok worktree {sub}` has no `--{flag}`"
            );
        }
        seen.push(sub);
    }
    for sub in ["add", "adopt", "list", "gc", "clean"] {
        assert!(
            seen.contains(&sub),
            "the skill must show `rtok worktree {sub}`"
        );
    }
}

/// T441.10: the rule line is a skill, so every `rtok task` command and flag it shows must
/// exist, and it must teach every subcommand an agent needs to plan and track work.
#[test]
fn tasks_skill_names_only_commands_rtok_task_has() {
    let content = hub_skill("rtok-tasks");
    let (_, body) = split_frontmatter(&content);
    let seen = rtok_task_commands(body);
    for sub in [
        "create", "list", "show", "status", "next", "init", "sync", "export",
    ] {
        assert!(seen.contains(&sub), "the skill must show `rtok task {sub}`");
    }
}

/// T904: the guide to writing a task sits beside the skill, which is at its size cap; the skill
/// must point at it, it may show only real `rtok task` commands and flags, and it must teach
/// splitting and ordering, not just creating.
#[test]
fn tasks_skill_links_a_creating_guide_with_only_real_commands() {
    let content = hub_skill("rtok-tasks");
    let (_, body) = split_frontmatter(&content);
    assert!(
        body.contains("`creating.md`"),
        "SKILL.md must link creating.md"
    );
    let guide = hub_file("rtok-tasks", "creating.md");
    let seen = rtok_task_commands(&guide);
    for sub in ["list", "create", "dep", "conflict", "parallel", "priority"] {
        assert!(
            seen.contains(&sub),
            "creating.md must show `rtok task {sub}`"
        );
    }
    assert!(guide.contains("--parent"), "creating.md must show subtasks");
}

/// T904: the user guide to planning with a model may name only real `rtok task` commands and
/// only the task tools the skill teaches (themselves pinned to the server's list), and the
/// translations carry the section too.
#[test]
fn docs_on_creating_tasks_with_a_model_name_only_real_commands_and_tools() {
    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs");
    let read = |rel: &str| fs::read_to_string(docs.join(rel)).expect(rel);
    let doc = read("tasks.md");
    let start = doc
        .find("## Creating tasks with a model\n")
        .expect("docs/tasks.md has the section");
    let rest = &doc[start..];
    let section = &rest[..rest[3..].find("\n## ").map_or(rest.len(), |i| i + 3)];
    let seen = rtok_task_commands(section);
    for sub in ["init", "list", "show", "status", "create"] {
        assert!(
            seen.contains(&sub),
            "the section must show `rtok task {sub}`"
        );
    }
    let skill = hub_skill("rtok-tasks");
    let tools: Vec<&str> = section
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|s| s.starts_with("task_"))
        .collect();
    assert!(tools.contains(&"task_create"), "{tools:?}");
    for tool in tools {
        assert!(
            skill.contains(&format!("`{tool}`")),
            "`{tool}` is not a task tool"
        );
    }
    assert!(section.contains("`rtok agents install <host>`"));
    for (rel, heading) in [
        ("ru/tasks.md", "## Создание задач с помощью модели\n"),
        ("uk/tasks.md", "## Створення завдань за допомогою моделі\n"),
    ] {
        assert!(read(rel).contains(heading), "docs/{rel} lacks the section");
    }
}

/// Every `rtok task …` code span in `text` names a real subcommand and only its real long
/// flags; returns the subcommands in order.
fn rtok_task_commands(text: &str) -> Vec<&str> {
    let cli = Cli::command();
    let task = cli.find_subcommand("task").expect("rtok task");
    let mut seen = Vec::new();
    for span in text.split('`').skip(1).step_by(2) {
        let Some(rest) = span.strip_prefix("rtok task ") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let sub = words.next().expect("subcommand after `rtok task`");
        let cmd = task
            .find_subcommand(sub)
            .unwrap_or_else(|| panic!("`rtok task {sub}` is not a subcommand"));
        for flag in words.filter_map(|w| w.strip_prefix("--")) {
            assert!(
                cmd.get_arguments().any(|a| a.get_long() == Some(flag)),
                "`rtok task {sub}` has no `--{flag}`"
            );
        }
        seen.push(sub);
    }
    seen
}

/// T441.10: installing the task skill changes nothing but its own directory; a user's own
/// skill beside it stays byte for byte, and a second install reports no change.
#[test]
fn install_adds_the_tasks_skill_and_leaves_a_users_own_skill_alone() {
    let home = tmp("skills-tasks");
    let cfg = write_cfg(&home);
    let root = home.join(".codex/skills");
    let mine = root.join("my-tasks");
    fs::create_dir_all(&mine).unwrap();
    fs::write(mine.join("SKILL.md"), "mine\n").unwrap();

    let out = rtok(&["agents", "install", "codex"], &cfg, &home);
    assert_eq!(
        fs::read_to_string(root.join("rtok-tasks/SKILL.md")).unwrap(),
        hub_skill("rtok-tasks"),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(root.join("rtok-tasks/creating.md")).unwrap(),
        hub_file("rtok-tasks", "creating.md"),
        "{out}"
    );
    let again = rtok(&["agents", "install", "codex"], &cfg, &home);
    assert!(!again.contains("rtok-tasks"), "{again}");
    rtok(&["agents", "remove", "codex"], &cfg, &home);
    assert_eq!(fs::read_to_string(mine.join("SKILL.md")).unwrap(), "mine\n");
    assert!(!root.join("rtok-tasks").exists());
    let _ = fs::remove_dir_all(&home);
}

/// T380: the hub skill keeps the name `rtok`; every other shipped skill is `rtok-<name>`.
#[test]
fn shipped_skills_except_the_hub_start_with_rtok_prefix() {
    assert!(SKILLS.contains(&"rtok"));
    for name in SKILLS {
        if *name == "rtok" {
            continue;
        }
        assert!(
            name.starts_with("rtok-"),
            "{name} must be prefixed so it cannot collide with a user's skill"
        );
    }
}

/// T155: one install copies every shipped skill under the host's skill root and one remove
/// takes them all away (codex: no plugin offer, so no flags).
#[test]
fn install_copies_every_skill_and_remove_takes_them_away() {
    let home = tmp("skills-e2e");
    let cfg = write_cfg(&home);
    let root = home.join(".codex/skills");
    let out = rtok(&["agents", "install", "codex"], &cfg, &home);
    for name in SKILLS {
        assert!(root.join(name).join("SKILL.md").is_file(), "{name}: {out}");
        assert!(out.contains(&format!("skills/{name}")), "{name}: {out}");
    }
    let out = rtok(&["agents", "remove", "codex"], &cfg, &home);
    for name in SKILLS {
        assert!(!root.join(name).exists(), "{name}: {out}");
    }
}

/// T380: install removes a legacy marked `worktrees`, leaves a foreign `worktrees` symlink
/// (and a stray `rtok-x` directory) and still copies the prefixed skills.
#[test]
fn install_retires_our_legacy_worktrees_and_leaves_a_foreign_one() {
    let home = tmp("skills-migrate");
    let cfg = write_cfg(&home);

    let codex = home.join(".codex/skills");
    let legacy = codex.join("worktrees");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("SKILL.md"), "old skill\n").unwrap();
    fs::write(legacy.join(".rtok-owned"), "").unwrap();
    let stray = codex.join("rtok-x");
    fs::create_dir_all(&stray).unwrap();
    fs::write(stray.join("SKILL.md"), "stray\n").unwrap();

    let out = rtok(&["agents", "install", "codex"], &cfg, &home);
    assert!(codex.join("rtok/SKILL.md").is_file(), "{out}");
    assert!(codex.join("rtok-worktrees/SKILL.md").is_file(), "{out}");
    assert!(!legacy.exists(), "marked legacy worktrees must go: {out}");
    assert!(
        out.lines().any(|l| {
            let l = l.replace('\\', "/");
            l.starts_with("- skill ") && l.ends_with("/skills/worktrees")
        }),
        "{out}"
    );
    assert_eq!(
        fs::read_to_string(stray.join("SKILL.md")).unwrap(),
        "stray\n"
    );
    assert!(
        !out.contains("rtok-x"),
        "a stray prefixed directory we do not ship is not a destination: {out}"
    );

    #[cfg(unix)]
    {
        // Cursor's installer does not copy skills; Command Code does, into its own root.
        let cc = home.join(".commandcode/skills");
        fs::create_dir_all(&cc).unwrap();
        let personal = home.join("personal-worktrees");
        fs::create_dir_all(&personal).unwrap();
        fs::write(personal.join("SKILL.md"), "mine\n").unwrap();
        let link = cc.join("worktrees");
        std::os::unix::fs::symlink(&personal, &link).unwrap();
        let out = rtok(&["agents", "install", "commandcode"], &cfg, &home);
        assert!(
            link.symlink_metadata().unwrap().file_type().is_symlink(),
            "{out}"
        );
        assert_eq!(
            fs::read_to_string(personal.join("SKILL.md")).unwrap(),
            "mine\n"
        );
        assert!(out.contains(&format!("leave {}", link.display())), "{out}");
        assert!(cc.join("rtok-worktrees/SKILL.md").is_file(), "{out}");
        assert!(cc.join("rtok/SKILL.md").is_file(), "{out}");
    }

    let _ = fs::remove_dir_all(&home);
}

/// T234: a skill lives only in `skills/<name>/`. Host plugins get it from `rtok agents install`
/// (`skill::sync`), never as a bundled copy that drifts from the hub.
#[test]
fn no_host_plugin_bundles_a_skill() {
    let plugins = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins");
    let copies: Vec<_> = ignore::WalkBuilder::new(&plugins)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() == "SKILL.md")
        .map(|e| e.into_path())
        .collect();
    assert!(
        copies.is_empty(),
        "skills belong in skills/<name>/ only: {copies:?}"
    );
}

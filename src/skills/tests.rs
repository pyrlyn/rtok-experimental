// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::*;

fn roots(tmp: &Path) -> Roots {
    Roots {
        home: tmp.join("home"),
        project: tmp.join("proj"),
        copilot_home: None,
    }
}

fn add(dir: &Path, name: &str, desc: &str) {
    let d = dir.join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {desc}\n---\nbody\n"),
    )
    .unwrap();
}

fn req(agent: Agent, name: &str, enabled: bool) -> ToggleRequest {
    ToggleRequest::new(agent, name.into(), enabled)
}

#[test]
fn lists_user_and_project_skills_with_descriptions() {
    let tmp = crate::testutil::tmp_dir("skills");
    let r = roots(&tmp);
    add(&r.skills_dir(Agent::Claude, Scope::User), "a", "first");
    add(&r.skills_dir(Agent::Claude, Scope::Project), "b", "second");
    std::fs::write(
        r.skills_dir(Agent::Claude, Scope::User).join("loose.md"),
        "x",
    )
    .unwrap();
    let got = list(&r, Agent::Claude, None).skills;
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].name, "a");
    assert_eq!(got[0].description.as_deref(), Some("first"));
    assert_eq!(got[1].scope, Scope::Project);
    assert_eq!(list(&r, Agent::Claude, Some(Scope::User)).skills.len(), 1);
}

#[test]
fn disable_then_enable_round_trips() {
    let tmp = crate::testutil::tmp_dir("skills");
    let r = roots(&tmp);
    add(&r.skills_dir(Agent::Cursor, Scope::User), "s", "d");
    let off = toggle(&r, &req(Agent::Cursor, "s", false)).unwrap();
    assert!(off.changed);
    assert!(!off.skill.enabled);
    assert!(
        r.disabled_dir(Agent::Cursor, Scope::User)
            .join("s/SKILL.md")
            .is_file()
    );
    assert!(!r.skills_dir(Agent::Cursor, Scope::User).join("s").exists());
    let again = toggle(&r, &req(Agent::Cursor, "s", false)).unwrap();
    assert!(!again.changed);
    let on = toggle(&r, &req(Agent::Cursor, "s", true)).unwrap();
    assert!(on.changed && on.skill.enabled);
    assert!(
        r.skills_dir(Agent::Cursor, Scope::User)
            .join("s/SKILL.md")
            .is_file()
    );
}

#[test]
fn dry_run_moves_nothing() {
    let tmp = crate::testutil::tmp_dir("skills");
    let r = roots(&tmp);
    add(&r.skills_dir(Agent::Copilot, Scope::Project), "s", "d");
    let mut q = req(Agent::Copilot, "s", false);
    q.dry_run = Some(true);
    let res = toggle(&r, &q).unwrap();
    assert!(res.changed);
    assert!(
        r.skills_dir(Agent::Copilot, Scope::Project)
            .join("s")
            .is_dir()
    );
    assert!(
        r.skills_dir(Agent::Copilot, Scope::Project)
            .ends_with(".github/skills")
    );
}

#[test]
fn ambiguous_scope_and_bad_names_fail() {
    let tmp = crate::testutil::tmp_dir("skills");
    let r = roots(&tmp);
    add(&r.skills_dir(Agent::Claude, Scope::User), "s", "d");
    add(&r.skills_dir(Agent::Claude, Scope::Project), "s", "d");
    assert!(toggle(&r, &req(Agent::Claude, "s", false)).is_err());
    let mut q = req(Agent::Claude, "s", false);
    q.scope = Some(Scope::Project);
    assert!(toggle(&r, &q).unwrap().changed);
    for bad in ["", "..", "a/b", ".hidden"] {
        assert!(toggle(&r, &req(Agent::Claude, bad, false)).is_err());
    }
    assert!(toggle(&r, &req(Agent::Claude, "missing", false)).is_err());
}

#[test]
fn copilot_home_override() {
    let r = Roots {
        home: "/h".into(),
        project: "/p".into(),
        copilot_home: Some("/c".into()),
    };
    assert_eq!(
        r.skills_dir(Agent::Copilot, Scope::User),
        PathBuf::from("/c/skills")
    );
    assert_eq!(
        r.disabled_dir(Agent::Copilot, Scope::User),
        PathBuf::from("/c/skills-disabled")
    );
}

#[test]
fn detects_agent_from_env() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    };
    assert_eq!(
        detect_agent(env(&[("CLAUDECODE", "1")])),
        Some(Agent::Claude)
    );
    assert_eq!(
        detect_agent(env(&[("CURSOR_AGENT", "1")])),
        Some(Agent::Cursor)
    );
    assert_eq!(
        detect_agent(env(&[("COPILOT_CLI", "1")])),
        Some(Agent::Copilot)
    );
    assert_eq!(
        detect_agent(env(&[("RTOK_AGENT", "cursor"), ("CLAUDECODE", "1")])),
        Some(Agent::Cursor)
    );
    assert_eq!(detect_agent(env(&[])), None);
}

#[test]
fn json_matches_contract_field_names() {
    let mut q = req(Agent::Claude, "s", true);
    q.dry_run = Some(true);
    let v = serde_json::to_value(&q).unwrap();
    assert_eq!(v["agent"], "claude");
    assert_eq!(v["dry_run"], true);
}

#[test]
fn every_agent_and_scope_round_trips() {
    for agent in AGENTS {
        for scope in [Scope::User, Scope::Project] {
            let tmp = crate::testutil::tmp_dir("skills-all");
            let r = roots(&tmp);
            add(&r.skills_dir(agent, scope), "s", "d");
            let mut off = req(agent, "s", false);
            off.scope = Some(scope);
            let res = toggle(&r, &off).unwrap();
            assert!(res.changed, "{agent} {scope:?}");
            assert!(
                r.disabled_dir(agent, scope)
                    .join("s")
                    .join("SKILL.md")
                    .is_file()
            );
            let listed = list(&r, agent, Some(scope)).skills;
            assert_eq!(listed.len(), 1);
            assert!(!listed[0].enabled);
            // Other agents never see this skill.
            for other in AGENTS.into_iter().filter(|a| *a != agent) {
                assert!(
                    list(&r, other, None).skills.is_empty(),
                    "{other} saw {agent}"
                );
            }
            let mut on = off.clone();
            on.enabled = true;
            assert!(toggle(&r, &on).unwrap().skill.enabled);
            assert!(
                r.skills_dir(agent, scope)
                    .join("s")
                    .join("SKILL.md")
                    .is_file()
            );
        }
    }
}

#[test]
fn size_formats_bytes_below_1000_and_kb_from_1000() {
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(999), "999 B");
    assert_eq!(format_size(1000), "1.0 KB");
    assert_eq!(format_size(1001), "1.0 KB");
    assert_eq!(format_size(1500), "1.5 KB");
    assert_eq!(format_size(12_345), "12.3 KB");
}

#[test]
fn size_bytes_sums_every_file_in_the_skill() {
    let tmp = crate::testutil::tmp_dir("skills-size");
    let r = roots(&tmp);
    let dir = r.skills_dir(Agent::Claude, Scope::User).join("big");
    std::fs::create_dir_all(dir.join("ref")).unwrap();
    std::fs::write(dir.join("SKILL.md"), vec![b'a'; 999]).unwrap();
    std::fs::write(dir.join("ref").join("x.txt"), b"ab").unwrap();
    let s = &list(&r, Agent::Claude, None).skills[0];
    assert_eq!(s.size_bytes, 1001);
    let v = serde_json::to_value(s).unwrap();
    assert_eq!(v["size_bytes"], 1001);
}

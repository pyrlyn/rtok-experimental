// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T71.3 / T155: install the skills this repo ships (`skills/<name>/`) into each host's
//! documented skill root (`research.md` §10.1).

use std::path::{Path, PathBuf};

use anyhow::Result;
use rtok_agent_sdk::{NO_CHANGES, SkillCopy};

use crate::config::Config;

use super::{apply, home_dir, skill_src};

/// Skills this repo ships under `skills/`, installed and removed together.
/// Every name but the hub `rtok` starts with `rtok-` so it cannot collide with a user's skill (T380).
pub const SKILLS: &[&str] = &["rtok", "rtok-worktrees", "rtok-tasks"];

/// Longest `description:` a skill listing should carry (`research.md` §10.4): the listing rides
/// every request, so rtok's own skills stay under it and `doctor` flags the rest against it.
pub const SKILL_DESC_MAX: usize = 120;

/// Unprefixed install from before T380. Retired on install only when the marker or a byte copy
/// already proves it ours; a foreign directory or symlink keeps its name and is reported once.
const LEGACY_SKILL: &str = "worktrees";

/// User skill root for a host that documents the Agent Skills format; `None` = untouched.
pub fn root(host: &str, cfg: &Config) -> Option<PathBuf> {
    let home = home_dir();
    match host {
        "claude" => Some(home.join(".claude/skills")),
        "cursor" => Some(home.join(".cursor/skills")),
        "codex" => Some(home.join(".codex/skills")),
        "opencode" => cfg
            .setup
            .opencode
            .config_path
            .parent()
            .map(|p| p.join("skills")),
        "copilot" => Some(cfg.setup.copilot.dir.join("skills")),
        "commandcode" => Some(cfg.setup.commandcode.dir.join("skills")),
        // T234: pi reads user skills beside its extensions tree (`~/.pi/agent/skills`).
        "pi" => cfg
            .setup
            .pi
            .extensions_path
            .parent()
            .map(|p| p.join("skills")),
        // T91.2: Antigravity 2.0 / IDE and the CLI each read their own global root, beside
        // their plugin roots (https://antigravity.google/docs/skills); `antigravity-cli` is the
        // CLI variant's key, not a host id.
        "antigravity" => cfg
            .setup
            .antigravity
            .plugins_path
            .parent()
            .map(|p| p.join("skills")),
        "antigravity-cli" => cfg
            .setup
            .antigravity
            .cli_plugins_path
            .parent()
            .map(|p| p.join("skills")),
        _ => None,
    }
}

/// Where `skills/<name>` lands for `host`.
pub fn dest(host: &str, cfg: &Config, name: &str) -> Option<PathBuf> {
    root(host, cfg).map(|r| r.join(name))
}

fn label(host: &str) -> Option<&'static str> {
    match host {
        "claude" => Some("~/.claude/skills"),
        "cursor" => Some("~/.cursor/skills"),
        "codex" => Some("~/.codex/skills"),
        "opencode" => Some("~/.config/opencode/skills"),
        "copilot" => Some("~/.copilot/skills"),
        "commandcode" => Some("~/.commandcode/skills"),
        "pi" => Some("~/.pi/agent/skills"),
        "antigravity" => Some("~/.gemini/config/skills"),
        "antigravity-cli" => Some("~/.gemini/antigravity-cli/skills"),
        _ => None,
    }
}

/// Copy or remove every shipped skill for `host`: one report line per skill that changes,
/// [`NO_CHANGES`] when none does.
pub fn sync(host: &str, cfg: &Config, remove: bool) -> Result<String> {
    let Some(root) = root(host, cfg) else {
        return Ok(NO_CHANGES.into());
    };
    let mut lines = Vec::new();
    for name in SKILLS {
        let line = SkillCopy {
            src: skill_src(name),
            dest: root.join(name),
            label: label(host).map(|l| format!("{l}/{name}")),
        }
        .run(&apply(cfg), remove)?;
        if line != NO_CHANGES {
            lines.push(line);
        }
    }
    // Install only: a later remove has nothing named `worktrees` in `SKILLS` to take back.
    if !remove && let Some(line) = retire_legacy(host, &root, cfg)? {
        lines.push(line);
    }
    Ok(if lines.is_empty() {
        NO_CHANGES.into()
    } else {
        lines.join("\n")
    })
}

/// One installed rtok skill older than the one this binary ships (T975).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outdated {
    /// The [`sync`] key: a host id, or `antigravity-cli` for that host's CLI root.
    pub key: &'static str,
    pub path: PathBuf,
}

impl Outdated {
    /// The host id `--agent` names: the CLI root belongs to `antigravity`.
    pub fn host(&self) -> &'static str {
        match self.key {
            "antigravity-cli" => "antigravity",
            key => key,
        }
    }
}

/// Every installed rtok skill, over every host skill root, that an install would replace.
pub fn outdated(cfg: &Config) -> Vec<Outdated> {
    let keys = super::HOSTS.iter().copied().chain(["antigravity-cli"]);
    keys.filter_map(|key| root(key, cfg).map(|r| (key, r)))
        .flat_map(|(key, root)| {
            SKILLS.iter().filter_map(move |name| {
                let copy = SkillCopy {
                    src: skill_src(name),
                    dest: root.join(name),
                    label: None,
                };
                copy.outdated().then_some(Outdated {
                    key,
                    path: copy.dest,
                })
            })
        })
        .collect()
}

/// Drop the pre-T380 `worktrees` install when [`SkillCopy`] can already prove it ours.
/// A foreign directory or a symlink (never followed) stays, and is named once.
fn retire_legacy(host: &str, root: &Path, cfg: &Config) -> Result<Option<String>> {
    let dest = root.join(LEGACY_SKILL);
    if std::fs::symlink_metadata(&dest).is_err() {
        return Ok(None);
    }
    let copy = SkillCopy {
        src: skill_src("rtok-worktrees"),
        dest: dest.clone(),
        label: label(host).map(|l| format!("{l}/{LEGACY_SKILL}")),
    };
    let line = copy.run(&apply(cfg), true)?;
    if line != NO_CHANGES {
        return Ok(Some(line));
    }
    Ok(Some(format!(
        "leave {} (not an rtok skill; remove by hand)",
        dest.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::HOSTS;
    use crate::testutil::Vfs;
    use rtok_agent_sdk::{NO_CHANGES, OWNED_MARKER, SkillPlan, SkillView, skill_plan};

    const HUB: &str = "src/SKILL.md";
    const DEST: &str = "skills/rtok";
    const DEST_MD: &str = "skills/rtok/SKILL.md";
    const FOREIGN: &str = "skills/other/SKILL.md";

    fn marker() -> String {
        format!("{DEST}/{OWNED_MARKER}")
    }

    fn dest_exists(vfs: &Vfs) -> bool {
        !vfs.paths_under(DEST).is_empty() || vfs.exists(DEST)
    }

    fn vfs_sync(vfs: &mut Vfs, remove: bool) -> String {
        let same_bytes = vfs.read_str(HUB) == vfs.read_str(DEST_MD);
        match skill_plan(
            remove,
            SkillView {
                present: dest_exists(vfs),
                symlink: false,
                // `skills/rtok` starts with `rtok`, so the slot is ours (T380).
                prefixed: true,
                marked: vfs.exists(&marker()),
                same_bytes,
            },
        ) {
            SkillPlan::NoChanges => NO_CHANGES.into(),
            SkillPlan::LeaveForeign => {
                format!("leave {DEST} (not an rtok skill; remove by hand)")
            }
            SkillPlan::Copy => {
                if let Some(body) = vfs.read(HUB).map(ToOwned::to_owned) {
                    vfs.write(DEST_MD, body);
                }
                vfs.write(marker(), b"");
                format!("+ skill → {DEST}")
            }
            SkillPlan::Remove => {
                let keep: Vec<(String, Vec<u8>)> = vfs
                    .paths()
                    .filter(|p| p != DEST && !p.starts_with("skills/rtok/"))
                    .filter_map(|p| vfs.read(&p).map(|b| (p, b.to_vec())))
                    .collect();
                *vfs = Vfs::new();
                for (p, b) in keep {
                    vfs.write(p, b);
                }
                format!("- skill {DEST}")
            }
        }
    }

    #[test]
    fn install_reinstall_remove_keeps_foreign() {
        let mut vfs = Vfs::new();
        vfs.write(HUB, "hub body\n");
        vfs.write(FOREIGN, "# other\n");

        let first = vfs_sync(&mut vfs, false);
        assert!(first.starts_with("+ skill"), "{first}");
        assert!(vfs.exists(&marker()));
        assert_eq!(vfs.read_str(DEST_MD), Some("hub body\n"));
        let before = vfs.read(DEST_MD).unwrap().to_vec();
        assert_eq!(vfs_sync(&mut vfs, false), NO_CHANGES);
        assert_eq!(vfs.read(DEST_MD), Some(before.as_slice()));
        assert!(vfs_sync(&mut vfs, true).starts_with("- skill"));
        assert_eq!(vfs_sync(&mut vfs, true), NO_CHANGES);
        assert_eq!(vfs.read_str(FOREIGN), Some("# other\n"));
        assert!(!vfs.exists(DEST_MD));
        assert!(!vfs.exists(&marker()));
    }

    #[test]
    fn overwrites_a_foreign_tree_at_a_prefixed_name() {
        let mut vfs = Vfs::new();
        vfs.write(HUB, "hub body\n");
        vfs.write(DEST_MD, "# foreign\n");

        let out = vfs_sync(&mut vfs, false);
        assert!(out.starts_with("+ skill"), "{out}");
        assert_eq!(vfs.read_str(DEST_MD), Some("hub body\n"));
        assert!(vfs.exists(&marker()));
    }

    #[test]
    fn dest_maps_documented_roots_and_skips_the_rest() {
        let cfg = Config::default();
        for id in HOSTS {
            match *id {
                "claude" | "cursor" | "codex" | "opencode" | "copilot" | "commandcode" | "pi"
                | "antigravity" => {
                    assert!(root(id, &cfg).is_some(), "{id} has a §10.1 skill root");
                    assert!(label(id).is_some(), "{id} root has a label");
                }
                _ => assert!(root(id, &cfg).is_none(), "{id} has no skill format"),
            }
        }
        assert!(root("gemini", &cfg).is_none(), "no gemini host installer");
        for (host, tail) in [
            ("claude", ".claude/skills"),
            ("cursor", ".cursor/skills"),
            ("codex", ".codex/skills"),
            ("opencode", "opencode/skills"),
            ("copilot", ".copilot/skills"),
            ("commandcode", ".commandcode/skills"),
            ("pi", ".pi/agent/skills"),
            ("antigravity", ".gemini/config/skills"),
            ("antigravity-cli", ".gemini/antigravity-cli/skills"),
        ] {
            assert!(label(host).unwrap().ends_with(tail), "{host} label");
            assert!(root(host, &cfg).unwrap().ends_with(tail), "{host} root");
            for name in SKILLS {
                let dest = dest(host, &cfg, name).unwrap();
                assert!(
                    dest.ends_with(format!("{tail}/{name}")),
                    "{host} {name} → {}",
                    dest.display()
                );
            }
        }
    }

    /// T155: every shipped skill has a hub directory to copy from, and a `SKILL.md` in it.
    #[test]
    fn every_shipped_skill_exists_in_the_hub() {
        for name in SKILLS {
            let src = skill_src(name);
            assert!(
                src.join("SKILL.md").is_file(),
                "{name}: {} has no SKILL.md",
                src.display()
            );
        }
    }
}

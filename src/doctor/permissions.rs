// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later

//! Explicit repair of old Unix data modes. Never rewrite contents or follow links.

use crate::config::Config;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct Finding {
    pub path: PathBuf,
    pub before: Option<u32>,
    pub target: Option<u32>,
    pub repaired: bool,
    pub error: Option<String>,
}

fn within(home: &Path, path: &Path) -> bool {
    path.strip_prefix(home).is_ok_and(|relative| {
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
    })
}

/// Only rtok's dedicated home and data paths beneath it are eligible. Custom external
/// locations may be shared; report them for manual review rather than chmod their parents.
pub fn check(cfg: &Config, apply: bool) -> Vec<Finding> {
    if !cfg!(unix) {
        return Vec::new();
    }
    let mut targets = vec![(cfg.home.clone(), false)];
    for name in ["config.toml", ".env", "rtok.log", "errors.log"] {
        targets.push((cfg.home.join(name), false));
    }
    targets.push((cfg.core.db_path.clone(), false));
    for suffix in ["-wal", "-shm"] {
        let mut path = cfg.core.db_path.as_os_str().to_os_string();
        path.push(suffix);
        targets.push((path.into(), false));
    }
    targets.push((cfg.core.archive_dir.clone(), true));
    if let Some(dir) = cfg.log.path.parent() {
        targets.push((dir.into(), false));
        for log in [&cfg.log.path, &rtok_log::error_path(&cfg.log.path)] {
            targets.push((log.clone(), false));
            for n in 1..=cfg.log.files.min(100) {
                targets.push((rtok_log::rotated_path(log, n), false));
            }
        }
    }
    let mut findings = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (path, recursive) in targets {
        if !seen.insert(path.clone()) {
            continue;
        }
        let valid_home = cfg.home.is_absolute()
            && cfg.home.parent().is_some()
            && cfg
                .home
                .components()
                .all(|c| matches!(c, Component::RootDir | Component::Normal(_)));
        if !valid_home
            || (path != cfg.home && !within(&cfg.home, &path))
            || (path == cfg.home && recursive)
        {
            findings.push(Finding {
                path,
                before: None,
                target: None,
                repaired: false,
                error: Some(
                    "outside a dedicated rtok data directory; review permissions manually".into(),
                ),
            });
            continue;
        }
        findings.extend(
            rtok_sys::permissions::check(&path, recursive, apply)
                .into_iter()
                .map(|f| Finding {
                    path: f.path,
                    before: f.before,
                    target: f.target,
                    repaired: f.repaired,
                    error: f.error,
                }),
        );
    }
    findings
}

pub fn render(findings: &[Finding]) -> String {
    findings
        .iter()
        .map(|f| {
            if let Some(error) = &f.error {
                format!("permissions {}: not repaired: {error}\n", f.path.display())
            } else {
                format!(
                    "permissions {}: {:04o} -> {:04o} ({})\n",
                    f.path.display(),
                    f.before.unwrap_or_default(),
                    f.target.unwrap_or_default(),
                    if f.repaired {
                        "repaired"
                    } else {
                        "doctor --fix --only permissions --yes"
                    }
                )
            }
        })
        .collect()
}

pub fn run(cfg: &Config, apply: bool) -> (String, i32) {
    let findings = check(cfg, apply);
    let code = i32::from(apply && findings.iter().any(|f| !f.repaired));
    (render(&findings), code)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }
    fn loose(path: &Path, bits: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(bits)).unwrap();
    }

    #[test]
    fn repair_is_explicit_idempotent_and_preserves_data() {
        let tmp = crate::testutil::tmp_dir("permission-repair");
        let home = tmp.canonicalize().unwrap().join("rtok");
        fs::create_dir_all(home.join("archive/sub")).unwrap();
        fs::create_dir_all(home.join("logs")).unwrap();
        let cfg = Config::load_from(&home).unwrap();
        let files = [
            home.join("archive/sub/body"),
            home.join("logs/rtok.log"),
            home.join("config.toml"),
        ];
        for path in &files {
            fs::write(path, "private bytes\n").unwrap();
            loose(path, 0o644);
        }
        for path in [
            &home,
            &home.join("archive"),
            &home.join("archive/sub"),
            &home.join("logs"),
        ] {
            loose(path, 0o755);
        }
        assert!(!check(&cfg, false).is_empty());
        assert_eq!(mode(&home), 0o755);
        assert_eq!(mode(&files[0]), 0o644);
        let fixed = check(&cfg, true);
        assert!(fixed.iter().all(|f| f.repaired), "{fixed:?}");
        assert_eq!(mode(&home), 0o700);
        for path in files {
            assert_eq!(mode(&path), 0o600);
            assert_eq!(fs::read_to_string(path).unwrap(), "private bytes\n");
        }
        assert!(check(&cfg, true).is_empty());
    }

    #[test]
    fn unsafe_links_do_not_prevent_independent_repairs_or_change_targets() {
        let tmp = crate::testutil::tmp_dir("permission-links");
        let root = tmp.canonicalize().unwrap();
        let home = root.join("rtok");
        fs::create_dir_all(home.join("archive")).unwrap();
        let cfg = Config::load_from(&home).unwrap();
        let outside = root.join("outside");
        fs::write(&outside, "keep me").unwrap();
        loose(&outside, 0o644);
        symlink(&outside, home.join("archive/symlink")).unwrap();
        fs::hard_link(&outside, home.join("archive/hardlink")).unwrap();
        let regular = home.join("archive/regular");
        fs::write(&regular, "repair me").unwrap();
        loose(&regular, 0o644);
        let findings = check(&cfg, true);
        assert!(findings.iter().any(|f| f.path == regular && f.repaired));
        assert!(findings.iter().filter(|f| f.error.is_some()).count() >= 2);
        assert_eq!(mode(&outside), 0o644);
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep me");
        fs::remove_file(home.join("archive/symlink")).unwrap();
        fs::remove_file(home.join("archive/hardlink")).unwrap();
        assert!(check(&cfg, true).is_empty());
        let mut external = cfg.clone();
        external.core.archive_dir = root.clone();
        assert!(
            check(&external, true)
                .iter()
                .any(|f| f.path == root && f.error.is_some())
        );
        assert_eq!(mode(&outside), 0o644);
    }

    #[test]
    fn symlinked_parent_is_not_followed() {
        let tmp = crate::testutil::tmp_dir("permission-parent");
        let root = tmp.canonicalize().unwrap();
        let real = root.join("real");
        fs::create_dir_all(&real).unwrap();
        loose(&real, 0o755);
        symlink(&real, root.join("link")).unwrap();
        let findings = rtok_sys::permissions::check(&root.join("link"), true, true);
        assert!(findings.iter().any(|f| f.error.is_some()));
        assert_eq!(mode(&real), 0o755);
    }
}

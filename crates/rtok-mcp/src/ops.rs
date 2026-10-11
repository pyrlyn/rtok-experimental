//! `apply`: the one entry point a host installer calls to install, update or remove rtok's MCP
//! entry (T277) — instead of each host writing its own `register_mcp` / `unregister_mcp`.

use anyhow::Result;
use rtok_agent_sdk::{Apply, NO_CHANGES};

use crate::config::{self, Fs, Written};
use crate::spec::McpSpec;

/// What `apply` should do to the entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Install,
    Update,
    Remove,
}

/// What `apply` did, or refused to do and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    NoChanges,
    /// `<key>.<name>: <command> <args>` for an install or update, `- <key>.<name>` for a remove;
    /// the same lines the hosts' own installers print.
    Wrote(String),
    /// Left alone: `leave …` when the entry is not rtok's or the user edited it and declined to
    /// have it removed, `? …` on a dry run that would ask.
    Left(String),
}

impl Report {
    /// The line a host's `apply` returns.
    pub fn line(self) -> String {
        match self {
            Self::NoChanges => NO_CHANGES.into(),
            Self::Wrote(s) | Self::Left(s) => s,
        }
    }
}

/// Install, update, or remove `spec`'s entry under `ctx` (dry run, `--yes`). Never silently
/// removes an entry the user edited or that never was rtok's (T246's ownership rule, run by
/// `rtok_agent_sdk::judge_owned`, which also asks "remove anyway?"). `is_bin` says whether a
/// string names the rtok binary, as for `rtok_agent_sdk::runs_bin`: the binary's check also
/// knows `rtok.exe`, case folding and its own running path, which this crate cannot see.
pub fn apply(
    spec: &McpSpec,
    mode: Mode,
    ctx: &Apply,
    fs: &mut impl Fs,
    is_bin: fn(&str) -> bool,
) -> Result<Report> {
    let ours = spec.entry();
    let at = format!("{}.{}", spec.key_path.join("."), spec.server.name);
    match mode {
        Mode::Install | Mode::Update => match config::write_entry(fs, spec, &ours)? {
            Written::Unchanged => Ok(Report::NoChanges),
            Written::Changed => Ok(Report::Wrote(format!("{at}: {}", spec.summary()))),
        },
        Mode::Remove => {
            let Some(have) = config::read_entry(fs, spec)? else {
                return Ok(Report::NoChanges);
            };
            let named = format!("{at} in {}", spec.config_path.display());
            if let Some(leave) = rtok_agent_sdk::judge_owned(ctx, &named, &have, &ours, is_bin) {
                return Ok(Report::Left(leave));
            }
            match config::remove_entry(fs, spec)? {
                Written::Unchanged => Ok(Report::NoChanges),
                Written::Changed => Ok(Report::Wrote(format!("- {at}"))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::registry;
    use crate::spec::{Client, DuplicateName, EntryShape, Format, Launch};
    use crate::status::{self, Entry};

    /// An in-memory [`Fs`] for tests: no host disk touched, `backups` counts every call so a
    /// test can assert `apply` backs up before it writes (T277 PR1; the real `Vfs`-backed impl
    /// lands with the first host in PR2).
    #[derive(Default)]
    struct MemFs {
        files: BTreeMap<PathBuf, Vec<u8>>,
        backups: usize,
    }

    impl Fs for MemFs {
        fn read(&self, path: &Path) -> Option<Vec<u8>> {
            self.files.get(path).cloned()
        }
        fn write(&mut self, path: &Path, bytes: Vec<u8>) -> Result<()> {
            self.files.insert(path.to_path_buf(), bytes);
            Ok(())
        }
        fn backup(&mut self, _path: &Path) -> Result<()> {
            self.backups += 1;
            Ok(())
        }
    }

    /// Stands in for the binary check a host passes (`rtok`'s own lives in the binary crate).
    fn is_rtok_bin(s: &str) -> bool {
        Path::new(s).file_stem().is_some_and(|n| n == "rtok")
    }

    fn spec(format: Format, key_path: &str) -> McpSpec {
        McpSpec {
            host: "test",
            client: Client::Cli,
            config_path: PathBuf::from("/config"),
            format,
            key_path: McpSpec::key_path(key_path),
            server: registry::RTOK,
            launch: Launch::default(),
            entry_shape: EntryShape::TypedCommand,
            duplicate_name: DuplicateName::ShowsBoth,
            plugin_serves: None,
        }
    }

    /// One row of the table: a format, its dotted key path (ZCode's `mcp.servers` is the two
    /// -level case), and whatever the file holds before rtok ever touches it.
    struct Case {
        format: Format,
        key_path: &'static str,
        existing: &'static str,
    }

    // The dotted-key case is ZCode's real shape (`mcp.servers`, Toml); the same path is also
    // proven against Json (`mcp.servers` is a made-up shape there) since the dotted-path walk
    // in `config::set_json_at`/`walk_json` is otherwise untested past one level.
    const CASES: &[Case] = &[
        Case {
            format: Format::Json,
            key_path: "mcpServers",
            existing: "",
        },
        Case {
            format: Format::Json,
            key_path: "mcp.servers",
            existing: "",
        },
        Case {
            format: Format::Jsonc,
            key_path: "mcpServers",
            existing: "{\n  // keep me\n  \"other\": 1,\n}\n",
        },
        Case {
            format: Format::Toml,
            key_path: "mcp_servers",
            existing: "",
        },
        Case {
            format: Format::Toml,
            key_path: "mcp.servers",
            existing: "",
        },
    ];

    #[test]
    fn install_then_remove_round_trips_across_every_format() {
        for case in CASES {
            let spec = spec(case.format, case.key_path);
            let mut fs = MemFs::default();
            if !case.existing.is_empty() {
                fs.write(&spec.config_path, case.existing.as_bytes().to_vec())
                    .unwrap();
            }

            let install = apply(
                &spec,
                Mode::Install,
                &Apply::default(),
                &mut fs,
                is_rtok_bin,
            )
            .unwrap();
            assert!(
                matches!(install, Report::Wrote(_)),
                "{:?}: {install:?}",
                case.format
            );
            assert_eq!(
                status::status(&fs, &spec).unwrap().entry,
                Entry::Present,
                "{:?}",
                case.format
            );
            assert_eq!(
                fs.backups, 1,
                "{:?}: install must back up first",
                case.format
            );

            // A second install of the same entry is a no-op — the shared "no changes" gate.
            assert_eq!(
                apply(
                    &spec,
                    Mode::Install,
                    &Apply::default(),
                    &mut fs,
                    is_rtok_bin
                )
                .unwrap(),
                Report::NoChanges
            );
            assert_eq!(
                fs.backups, 1,
                "{:?}: a no-op write must not back up again",
                case.format
            );

            if !case.existing.is_empty() {
                let after = String::from_utf8(fs.read(&spec.config_path).unwrap()).unwrap();
                assert!(
                    after.contains("// keep me"),
                    "{:?}: lost a comment: {after}",
                    case.format
                );
                assert!(
                    after.contains("\"other\": 1"),
                    "{:?}: lost a key: {after}",
                    case.format
                );
            }

            let remove =
                apply(&spec, Mode::Remove, &Apply::default(), &mut fs, is_rtok_bin).unwrap();
            assert!(
                matches!(remove, Report::Wrote(_)),
                "{:?}: {remove:?}",
                case.format
            );
            assert_eq!(
                config::read_entry(&fs, &spec).unwrap(),
                None,
                "{:?}",
                case.format
            );
            assert_eq!(
                apply(&spec, Mode::Remove, &Apply::default(), &mut fs, is_rtok_bin).unwrap(),
                Report::NoChanges
            );
        }
    }

    /// TOML entries are written as `[<key_path>.rtok]` tables, the shape Codex and Grok already
    /// hold, and a created parent (`mcp` above `mcp.servers`) prints no empty header.
    #[test]
    fn toml_entry_is_a_standard_table_under_implicit_parents() {
        for (key_path, header) in [
            ("mcp_servers", "[mcp_servers.rtok]"),
            ("mcp.servers", "[mcp.servers.rtok]"),
        ] {
            let spec = spec(Format::Toml, key_path);
            let mut fs = MemFs::default();
            apply(
                &spec,
                Mode::Install,
                &Apply::default(),
                &mut fs,
                is_rtok_bin,
            )
            .unwrap();
            let body = String::from_utf8(fs.read(&spec.config_path).unwrap()).unwrap();
            assert!(body.starts_with(header), "{body}");
            assert!(!body.contains("rtok = {"), "{body}");
        }
    }

    #[test]
    fn remove_leaves_an_entry_the_user_edited() {
        let spec = spec(Format::Json, "mcpServers");
        let mut fs = MemFs::default();
        apply(
            &spec,
            Mode::Install,
            &Apply::default(),
            &mut fs,
            is_rtok_bin,
        )
        .unwrap();
        let edited = br#"{"mcpServers":{"rtok":{"type":"stdio","command":"rtok","args":["mcp","--extra"]}}}"#;
        fs.write(&spec.config_path, edited.to_vec()).unwrap();

        let report = apply(&spec, Mode::Remove, &Apply::default(), &mut fs, is_rtok_bin).unwrap();
        assert!(matches!(report, Report::Left(_)), "{report:?}");
        assert!(config::read_entry(&fs, &spec).unwrap().is_some());
    }

    #[test]
    fn remove_leaves_an_entry_that_is_not_rtoks() {
        let spec = spec(Format::Json, "mcpServers");
        let mut fs = MemFs::default();
        fs.write(
            &spec.config_path,
            br#"{"mcpServers":{"rtok":{"command":"other-tool"}}}"#.to_vec(),
        )
        .unwrap();

        let report = apply(&spec, Mode::Remove, &Apply::default(), &mut fs, is_rtok_bin).unwrap();
        assert!(matches!(report, Report::Left(_)), "{report:?}");
        assert!(config::read_entry(&fs, &spec).unwrap().is_some());
    }

    /// A host that launches rtok by absolute path with its own `--host` pair (Claude): the
    /// entry is what the spec says, the report line is the host's, and an entry that differs
    /// only by the default `"type": "stdio"` (Claude.app drops it, T265) is already current.
    #[test]
    fn launch_sets_the_entry_and_a_type_only_difference_is_no_change() {
        let mut s = spec(Format::Json, "mcpServers");
        s.host = "claude";
        s.launch = Launch {
            command: Some("/opt/bin/rtok".into()),
            host_arg: true,
        };
        let mut fs = MemFs::default();

        let line = apply(&s, Mode::Install, &Apply::default(), &mut fs, is_rtok_bin).unwrap();
        assert_eq!(
            line,
            Report::Wrote("mcpServers.rtok: /opt/bin/rtok mcp --host claude".into())
        );
        let entry = config::read_entry(&fs, &s).unwrap().unwrap();
        assert_eq!(
            entry,
            serde_json::json!({"type": "stdio", "command": "/opt/bin/rtok", "args": ["mcp", "--host", "claude"]})
        );

        let no_type = br#"{"mcpServers":{"rtok":{"command":"/opt/bin/rtok","args":["mcp","--host","claude"]}}}"#;
        fs.write(&s.config_path, no_type.to_vec()).unwrap();
        for mode in [Mode::Install, Mode::Update] {
            let r = apply(&s, mode, &Apply::default(), &mut fs, is_rtok_bin).unwrap();
            assert_eq!(r.line(), rtok_agent_sdk::NO_CHANGES, "{mode:?}");
        }
        assert_eq!(fs.read(&s.config_path).unwrap(), no_type, "file untouched");
    }

    /// Removal of an entry already in the file, by who owns it and how the run is set:
    /// (entry, `--yes`, dry run) -> (report prefix, entry still there).
    #[test]
    fn remove_follows_ownership_and_the_run_flags() {
        let ours = r#"{"type":"stdio","command":"/old/bin/rtok","args":["mcp","--host","claude"]}"#;
        let edited = r#"{"command":"rtok","args":["mcp","--extra"]}"#;
        let foreign = r#"{"command":"other-tool"}"#;
        let table: &[(&str, bool, bool, &str, bool)] = &[
            // Any path to rtok, with or without the host pair, is rtok's own.
            (ours, false, false, "- mcpServers.rtok", false),
            (
                r#"{"command":"rtok","args":["mcp"]}"#,
                false,
                false,
                "- mcpServers.rtok",
                false,
            ),
            // The user's edit is kept unless `--yes`; a dry run only says it would ask.
            (edited, false, false, "leave mcpServers.rtok in ", true),
            (edited, false, true, "? mcpServers.rtok in ", true),
            (edited, true, false, "- mcpServers.rtok", false),
            // Not rtok's at all: never removed, `--yes` or not.
            (foreign, true, false, "leave mcpServers.rtok in ", true),
        ];
        for (have, yes, dry_run, want, kept) in table {
            let s = spec(Format::Json, "mcpServers");
            let mut fs = MemFs::default();
            let file = format!(r#"{{"keep":1,"mcpServers":{{"rtok":{have}}}}}"#);
            fs.write(&s.config_path, file.into_bytes()).unwrap();
            let ctx = Apply {
                yes: *yes,
                dry_run: *dry_run,
                ..Apply::default()
            };

            let got = apply(&s, Mode::Remove, &ctx, &mut fs, is_rtok_bin)
                .unwrap()
                .line();
            assert!(
                got.starts_with(want),
                "{have} yes={yes} dry={dry_run}: {got}"
            );
            assert_eq!(
                config::read_entry(&fs, &s).unwrap().is_some(),
                *kept,
                "{have} yes={yes} dry={dry_run}"
            );
        }
    }
}

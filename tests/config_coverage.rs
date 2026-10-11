// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Flag ↔ key coverage (plan T12.4, decision D12).
//!
//! Every clap long flag that is not in the allow-list must exist as a dotted key in
//! `config/default.toml`. Every leaf key's last segment must appear in `src/` or `rtok-config`.

use clap::{ArgAction, Command, CommandFactory};
use rtok::cli::Cli;
use rtok::config::layers;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;

const ALLOW: &[&str] = &[
    "config",
    "home",
    "help",
    "version",
    "json",
    "remove",
    "replace",
    "calibrate",
    "save-baseline", // action: writes measurements/<name>.json (T1.3)
    "cache",
    "force",
    "sources",    // action: annotates `config show`; not a stored setting
    "lines",      // expand slice; not a stored setting (T3.5)
    "grep",       // expand filter; not a stored setting (T3.5)
    "context",    // expand grep context window; per call like --grep (T67.2)
    "stdin",      // action: rtok filter reads stdin (T10.2)
    "archive",    // action: rtok filter --archive (T62.3)
    "tool",       // action: rtok guard check --tool (T70.5)
    "call",       // action: rtok mcp --call (T70.3)
    "session",    // action: rtok guard check --session (T70.5)
    "all",        // action: agent sessions also lists ended sessions (T25.2); not a setting
    "cli",        // action: agent setup variant filter (T37.0); not a setting
    "desktop",    // action: agents install variant filter (T37.0); not a setting
    "no-restart", // action: skip this call's close/reopen of a running desktop app (T141); not a setting
    "serve", // action: rtok hook --serve runs the resident hook process (T178, D32); not a setting
    "agent", // action: rtok run --agent <id>, scopes the dedup pointer per call (T127); not a setting
];

/// Flags that are actions on one command rather than settings, so they get no config key.
/// `--dry-run` has a key wherever the command owns a config table (`setup`, `proxy`, `bench`);
/// on `config init` / `config set` it sits beside `--force` and means "print, do not write".
const ALLOW_KEYS: &[&str] = &[
    // `man --dir` (T316): where one call writes the pages, not a stored setting.
    "man.dir",
    // `completions --install/--uninstall` (T318): one-shot actions, not settings.
    "completions.install",
    "completions.uninstall",
    "completions.list",
    "config.init.dry_run",
    "config.set.dry_run",
    // T416.4: print-do-not-write previews on the destructive commands, one-shot like the above.
    "worktree.remove.dry_run",
    "completions.dry_run",
    "graph.projects.remove.dry_run",
    "graph.projects.unlink.dry_run",
    // T416.2: the deletion commands' explicit preview flags, one-shot like the above.
    "worktree.gc.dry_run",
    "worktree.gc.stat",
    "worktree.clean.dry_run",
    "worktree.clean.stat",
    "agents.junk.clear.dry_run",
    "agents.junk.clear.stat",
    "guard.check.host", // overlay `[hook] host` on the plugin CLI path (T70.5)
    "memory.import.dry_run",
    "graph.index.dry_run",
    "graph.impact.depth",
    "graph.impact.to",
    // `graph <cmd> --project` (T329.4.2): which project one call answers for, not a stored setting.
    "graph.index.project",
    "graph.dead.project",
    "graph.status.project",
    "graph.impact.project",
    "graph.affected.project",
    // `graph affected` (T68.x): which diff to read on one call, not a stored setting.
    "graph.affected.since",
    "graph.affected.staged",
    // `graph review` (T454): which diff to read on one call, same as `affected`.
    "graph.review.since",
    "graph.review.staged",
    "graph.review.project",
    // `memory export --project` narrows one dump; the store, not a setting, decides it.
    "memory.export.project",
    // `memory retire/revise` (T69.1): one-shot lifecycle values on a single call, not settings.
    "memory.retire.superseded_by",
    "memory.revise.title",
    "memory.revise.body",
    // `memory sync` (T69.6): per-call file/budget/dry-run; `sync_tokens` is the setting.
    "memory.sync.file",
    "memory.sync.budget",
    "memory.sync.dry_run",
    // `memory status` (T69.4): one-shot filters, not settings.
    "memory.status.project",
    "memory.status.since",
    // `otel flush --coalesce` (T143): hidden, hook-spawned-only mode switch for one call,
    // not a stored setting — a manual `rtok otel flush` never passes it.
    "otel.flush.coalesce",
    // `worktree gc` (T153): deleting is decided per call — a stored `yes` or `owner`
    // would turn a dry run into a removal, or open locks, without anyone typing it.
    "worktree.gc.yes",
    "worktree.gc.owner",
    "worktree.gc.idle",
    "worktree.gc.stale_lock",
    // `worktree add --owner` (T158): who holds this one worktree; `[worktree] root` is the setting.
    "worktree.add.owner",
    // `worktree claim --owner` (T285): the name an old lock carries, per call like `add --owner`.
    "worktree.claim.owner",
    // `worktree adopt` (T289): per call like `claim` — which task and whose lock.
    "worktree.adopt.owner",
    "worktree.adopt.task",
    // `worktree remove` (T286): per call like `claim` — whose old lock, and whether an
    // unmerged branch survives this one removal.
    "worktree.remove.owner",
    "worktree.remove.keep_branch",
    // `worktree clean` (T152): same per-call rule as `gc` — a stored `yes` would delete
    // without anyone typing it; `--idle` is T153's flag with T153's default.
    "worktree.clean.yes",
    "worktree.clean.idle",
    // `agents junk clear --yes` (T182): same per-call rule as `worktree gc`/`clean` — a
    // stored value would delete without anyone typing it, and it must not share `setup.yes`,
    // which confirms a different destructive action (`agents install --replace`).
    "junk.yes",
    // `agents junk list --bytes` (T330.1): how one call prints sizes, not a stored setting.
    "junk.bytes",
    // `agents junk clear --dry-run/--stat` (T416.2): one-shot preview flags, like `junk.yes`.
    "junk.dry_run",
    "junk.stat",
    // `agents junk clear` filters (T330.4): what one run removes. A stored filter would widen
    // a later bare `clear --yes` without anyone typing it; `--trash` is the same per-call choice.
    "junk.agent",
    "junk.kind",
    "junk.include",
    "junk.older_than",
    "junk.trash",
    // `agents junk --session-days` (T330.5.2): a one-run override of `agents.junk.stale_session_days`;
    // the flag name differs from the key, so the doc scan sees it as a keyless flag.
    "junk.session_days",
    // `agents junk list|clear --items --sort --min-size` (T330.6): how one call prints its item
    // breakdown; `--min-size` on `clear` narrows that run's plan, so it is per-call like the filters.
    "junk.items",
    "junk.sort",
    "junk.min_size",
    // `rtok mcp ping --timeout` (T275.1): one call's wait, default 60s, not a stored setting.
    "mcp.ping.timeout_s",
    // `agents send --all-live` / `agents inbox --unread` (T287): who gets one message and
    // which rows one read shows; `agents` flags map onto `setup.*`, which they are not.
    "setup.all_live",
    "setup.unread",
    // `agents send --task` / `agents inbox --task` (T441.13.1): which task one message is about or
    // which thread one read shows, per call like the two flags above.
    "setup.task",
    // `agents update --check` / `agents outdated --exit-code` (T279.1): one-shot
    // listing and a one-shot exit status, not stored settings.
    "setup.check",
    "setup.exit_code",
    // `agents install|uninstall --project` (T289.3): which file one call edits, not a stored setting.
    "setup.project",
    // `agents usage --unpriced` (T358.1): which view one call prints, not a stored setting.
    "agents.usage.unpriced",
    // `doctor --fix --yes --dry-run --only` (T331.5): same per-call rule as `worktree gc` —
    // a stored `yes` would rewrite host configs without anyone typing it.
    "doctor.fix",
    "doctor.yes",
    "doctor.dry_run",
    "doctor.only",
    // T416.3: which preview one dry run prints (stat lines or the diff), a per-call choice.
    "doctor.stat",
    "config.init.stat",
    "config.set.stat",
    "memory.sync.stat",
    "setup.stat",
    // `graph projects link|unlink --from/--both/--reason` (T329.3): which two projects one call
    // links and why, not stored settings.
    "graph.projects.link.from",
    "graph.projects.link.both",
    "graph.projects.link.reason",
    "graph.projects.unlink.from",
    "graph.projects.unlink.both",
    // `graph diff --from/--from-export/--to` (T329.18, T329.29): which two revisions or saved export one call compares, not stored settings.
    "graph.diff.from",
    "graph.diff.from_export",
    "graph.diff.to",
    "graph.diff.project",
    // `graph export` (T329.16): what one export contains and where it goes, not stored settings.
    "graph.export.level",
    "graph.export.focus",
    "graph.export.depth",
    "graph.export.no_redact",
    "graph.export.from",
    "graph.export.output",
    "graph.export.project",
    // `graph export --format/--scale/--transparent` (T329.31): the shape of one picture.
    "graph.export.format",
    "graph.export.scale",
    "graph.export.transparent",
    // `task create|list|status` (T441.5): what one task is and which rows one call shows;
    // `[tasks]` holds the adapter and the prefix, which `task init` writes.
    "task.create.description",
    "task.create.body_file",
    "task.create.parent",
    "task.list.status",
    "task.list.all",
    "task.list.parent",
    "task.status.force",
    // `task conflict|parallel` (T441.13.2): undo flags for one call, nothing to store.
    "task.conflict.remove",
    "task.parallel.off",
    // `task export --dry-run` (T441.13.5): a stored `true` would turn a later export into a
    // silent no-op; the repo is the setting (`tasks.export.github.repo`).
    "task.export.dry_run",
    // `batch submit|status|fetch --provider/--url` (T385.12.1): which provider's Batch API and
    // which proxy one call talks to; the URL defaults to `[proxy] bind`/`port`, and the
    // provider is read from the batch id, so there is nothing to store.
    "batch.submit.provider",
    "batch.submit.url",
    "batch.status.provider",
    "batch.status.url",
    "batch.fetch.provider",
    "batch.fetch.url",
];

#[test]
fn config_coverage() {
    let toml_keys: HashSet<String> = layers::leaf_keys().into_iter().collect();
    let mut flags = Vec::new();
    walk(&Cli::command(), &[], &mut flags);

    for key in &flags {
        assert!(
            toml_keys.contains(key),
            "flag maps to `{key}` which is missing from config/default.toml"
        );
    }

    let src = src_blob();
    for key in &toml_keys {
        let leaf = key.rsplit('.').next().unwrap();
        if leaf.len() < 3 {
            continue;
        }
        assert!(
            src.contains(leaf),
            "config key `{key}` (leaf `{leaf}`) is never read in src/ or rtok-config"
        );
    }
}

fn walk(cmd: &Command, path: &[&str], out: &mut Vec<String>) {
    for arg in cmd.get_arguments() {
        if matches!(
            arg.get_action(),
            ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
        ) {
            continue;
        }
        let Some(long) = arg.get_long() else {
            continue;
        };
        if ALLOW.contains(&long) {
            continue;
        }
        let key = config_key(path, long);
        if ALLOW_KEYS.contains(&key.as_str()) {
            continue;
        }
        out.push(key);
    }
    for sub in cmd.get_subcommands() {
        let mut next = path.to_vec();
        next.push(sub.get_name());
        walk(sub, &next, out);
    }
}

fn config_key(path: &[&str], long: &str) -> String {
    let name = long.replace('-', "_");
    let name = match name.as_str() {
        "timeout" => "timeout_s",
        "compare" => "baseline",
        "mode" if matches!(path, ["proxy"]) => "mode",
        "mode" => "modes",
        _ => name.as_str(),
    };
    match path {
        // `rtok agents junk clear` (T182): its own namespace — never `[setup]`, whose
        // `yes` confirms an unrelated destructive action (`agents install --replace`).
        ["agents", "junk", ..] => format!("junk.{name}"),
        // `rtok agents usage` (T358.1): its own `[agents.usage]` table. `--host` is the `hosts`
        // list; `--daily` / `--monthly` both set `period`.
        ["agents", "usage"] => match name {
            "host" => "agents.usage.hosts".into(),
            "daily" | "monthly" => "agents.usage.period".into(),
            other => format!("agents.usage.{other}"),
        },
        // `rtok agents install|remove` keeps the `[setup]` table it had as `rtok setup`.
        ["agents", ..] => format!("setup.{name}"),
        // `rtok task init --adapter/--prefix` write `[tasks]` itself (T441.5).
        ["task", "init"] => format!("tasks.{name}"),
        // `rtok dashboard` is the hidden deprecated spelling of `rtok web`; one table, `[web]`.
        ["dashboard", ..] => format!("web.{name}"),
        // `rtok mcp --host` (T283.1) overlays `[hook] host`, as `rtok hook --host` does.
        ["mcp"] if name == "host" => "hook.host".into(),
        ["run", ..] | ["filter", ..] => match name {
            "shell" => "plugins.cmd.shell".into(),
            "no_trailer" => "plugins.cmd.trailer_min_lines".into(),
            "cmd" => "filter.cmd".into(),
            other => format!("plugins.cmd.{other}"),
        },
        [] if name == "log_level" => "log.level".into(),
        [] => name.into(),
        _ => format!("{}.{name}", path.join(".")),
    }
}

fn src_blob() -> String {
    let mut out = String::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    fn walk_rs(dir: &Path, out: &mut String) {
        for e in std::fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if p.is_dir() {
                walk_rs(&p, out);
            } else if p.extension().and_then(OsStr::to_str) == Some("rs") {
                out.push_str(&std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    walk_rs(&root.join("src"), &mut out);
    walk_rs(&root.join("crates/rtok-config/src"), &mut out);
    out
}

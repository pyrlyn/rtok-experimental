// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The SQLite store, re-exported from `rtok-store`. Call sites keep `crate::store`.
//!
//! Housekeeping warnings are handed to the stderr surface (`stdio::stderr`). The store
//! crate returns them and writes nothing to the terminal. `log` already uses this
//! module, so the warning cannot go back through `log`.

pub use rtok_store::*;

mod http_embed;

/// Show the migration spinner on a terminal. [`crate::cli::run`] calls this before any open.
pub fn install_migrate_spinner() {
    rtok_store::on_long_migrate(|msg, f| rtok_sys::proc::ProgressRunner::wait(msg, f));
}

/// `[plugins.memory.embed]` as the plain settings the store accepts.
pub fn embed_settings(embed: &crate::config::MemoryEmbed) -> rtok_store::EmbedSettings {
    // T668: only an enabled `provider = "http"` builds a client; every other value keeps the
    // offline hash embedder.
    let remote = (embed.enabled && embed.provider == "http")
        .then(|| http_embed::HttpEmbedder::from_config(embed, |k| std::env::var(k).ok()))
        .flatten()
        .map(|e| rtok_store::Remote(std::sync::Arc::new(e)));
    rtok_store::EmbedSettings {
        enabled: embed.enabled,
        provider: embed.provider.clone(),
        model: embed.model.clone(),
        dimensions: embed.dimensions,
        hybrid: embed.hybrid,
        remote,
    }
}

/// T968: `[core]` limits as the store takes them, in bytes.
pub fn prune_policy(cfg: &crate::config::Config) -> rtok_store::PrunePolicy {
    const MIB: u64 = 1024 * 1024;
    rtok_store::PrunePolicy {
        retain_calls_days: cfg.core.retain_calls_days,
        retain_archive_days: cfg.core.retain_archive_days,
        max_store_bytes: u64::from(cfg.core.max_store_mb) * MIB,
        max_archive_bytes: u64::from(cfg.core.max_archive_mb) * MIB,
    }
}

/// T352: run [`Store::housekeeping`] on a background thread, so the `initialize` handshake
/// (mcp) or the listener (proxy) is never delayed by it. The thread dies with the process;
/// one that is killed mid-run only rolls back its current batch (or its `VACUUM`).
pub fn spawn_retention(cfg: &crate::config::Config, surface: &'static str) {
    let job = rtok_store::RetentionJob {
        db_path: cfg.core.db_path.clone(),
        retain_calls_days: cfg.core.retain_calls_days,
        retain_hook_bodies_days: cfg.core.retain_hook_bodies_days,
        prune: prune_policy(cfg),
    };
    let log_path = cfg.log.path.clone();
    let max_bytes = cfg.log.max_bytes;
    let files = cfg.log.files;
    let level = cfg.log.level.clone();
    let spawned = std::thread::Builder::new()
        .name("rtok-retention".into())
        .spawn(move || {
            for warning in rtok_store::Store::housekeeping(&job) {
                crate::stdio::stderr_ln(&format!("rtok {surface}: {}", warning.message));
                crate::logfile::append(
                    &log_path,
                    max_bytes,
                    files,
                    &level,
                    "warn",
                    surface,
                    "retention",
                    &warning.message,
                );
            }
        });
    if let Err(e) = spawned {
        crate::stdio::stderr_ln(&format!(
            "rtok {surface}: retention thread not started: {e}"
        ));
    }
}

/// `(archive_id, ts)` for a prior read in `session`. A free function so `use crate::store`
/// names this file: the graph ranker resolves `get_read_cache` through that import. The
/// query itself is [`Store::get_read_cache`].
pub fn get_read_cache(
    store: &Store,
    session: &str,
    path: &str,
) -> rtok_store::Result<Option<(Option<String>, i64)>> {
    store.get_read_cache(session, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `%`, `_` and case are literal in the prefix compare — `LIKE` treated them as a pattern
    /// and dropped cached reads of other files.
    #[test]
    fn clear_read_cache_drops_only_that_path_and_its_mode_keys() {
        let store = Store::open_in_memory().unwrap();
        let claude = store.host_id("claude").unwrap();
        for s in ["s", "other"] {
            store
                .upsert_session(s, claude, None, None, Some("hook"))
                .unwrap();
        }
        let gone = ["/a/x_1.rs", "/a/x_1.rs\tmap", "/a/x_1.rs\tlines\t1-9"];
        let kept = [
            "/a/xa1.rs\tmap",
            "/a/X_1.RS\tmap",
            "/a/x_1.rsx",
            "/a/%\tmap",
        ];
        for p in gone.iter().chain(&kept) {
            store.put_read_cache("s", p, "h", None).unwrap();
        }
        store
            .put_read_cache("other", "/a/x_1.rs", "h", None)
            .unwrap();
        store.clear_read_cache("s", "/a/x_1.rs").unwrap();
        store.clear_read_cache("s", "/a/%").unwrap();
        let cached = |s: &str, p: &str| get_read_cache(&store, s, p).unwrap().is_some();
        for p in gone {
            assert!(!cached("s", p), "{p:?} should be cleared");
        }
        for p in &kept[..3] {
            assert!(cached("s", p), "{p:?} is another file");
        }
        assert!(
            !cached("s", "/a/%\tmap"),
            "a literal `%` path clears its own keys"
        );
        assert!(cached("other", "/a/x_1.rs"), "other sessions keep theirs");
    }
}

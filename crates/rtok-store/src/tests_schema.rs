// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::*;

fn call_io_bodies(store: &Store, id: i32) -> (Option<String>, Option<String>, i64) {
    let mut conn = store.lock().unwrap();
    call_io::table
        .filter(call_io::call_id.eq(id))
        .select((
            call_io::request_json,
            call_io::response_json,
            call_io::request_bytes,
        ))
        .first(&mut *conn)
        .unwrap()
}

#[test]
fn retention_clears_old_hook_bodies_and_keeps_the_rest() {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session("s", Some(1), None, None, Some("hook"))
        .unwrap();
    let old = super::tests_archive::hook_call_with_body(&store, "hook", b"{\"old\":1}", 10);
    let fresh = super::tests_archive::hook_call_with_body(&store, "hook", b"{\"fresh\":1}", 1);
    let mcp = super::tests_archive::hook_call_with_body(&store, "mcp_call", b"{\"mcp\":1}", 10);

    store.run_retention(30, 3).unwrap();

    let (req, res, bytes) = call_io_bodies(&store, old);
    assert_eq!((req, res), (None, None));
    assert_eq!(bytes, 9, "byte counts stay");
    assert!(call_io_bodies(&store, fresh).0.is_some());
    assert!(call_io_bodies(&store, mcp).0.is_some());
    assert_eq!(
        store.recent_hook_inputs("s", 10).unwrap(),
        ["{\"fresh\":1}", ""],
        "a cleared body reads back empty"
    );
}

#[test]
fn hook_bodies_clear_across_several_batches() {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session("s", Some(1), None, None, Some("hook"))
        .unwrap();
    let old: Vec<i32> = (0..5)
        .map(|_| super::tests_archive::hook_call_with_body(&store, "hook", b"{}", 10))
        .collect();
    let fresh = super::tests_archive::hook_call_with_body(&store, "hook", b"{}", 1);
    let cutoff = i64::try_from(unix_now()).unwrap() - 3 * 86_400;
    {
        let mut conn = store.lock().unwrap();
        let first = sql_ext::clear_hook_bodies(&mut conn, cutoff, 2).unwrap();
        assert_eq!(first, 2, "one batch clears at most `batch` rows");
    }
    assert_eq!(store.clear_hook_bodies_in_batches(3, 2).unwrap(), 3);
    assert!(old.iter().all(|&id| call_io_bodies(&store, id).0.is_none()));
    assert!(call_io_bodies(&store, fresh).0.is_some());
    assert_eq!(store.clear_hook_bodies_in_batches(3, 2).unwrap(), 0);
}

#[test]
fn zero_hook_body_days_keeps_every_body() {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session("s", Some(1), None, None, Some("hook"))
        .unwrap();
    let old = super::tests_archive::hook_call_with_body(&store, "hook", b"{}", 10);
    store.run_retention(30, 0).unwrap();
    assert!(call_io_bodies(&store, old).0.is_some());
}

#[test]
fn retention_drops_symbol_roots_that_are_no_longer_directories() {
    let store = Store::open_in_memory().unwrap();
    let live = super::support::tmp_dir("t352-live");
    let live = live.to_str().unwrap();
    let gone = "/rtok-t352-no-such-root";
    let row = rtok_plugin_sdk::SymbolRow::new("f", "function", 1, true, 1, "");
    for root in [live, gone, ""] {
        store
            .replace_symbols(root, "a.rs", "s", (0, 0), std::slice::from_ref(&row))
            .unwrap();
        store.set_extractor_fingerprint(root, "fp").unwrap();
    }
    // A stale mark on a file with no rows left, as `mark_symbols_stale_in` leaves it.
    store.mark_symbols_stale_in(gone, "z.rs").unwrap();
    store.run_retention(30, 3).unwrap();
    assert_eq!(store.symbol_count(gone).unwrap(), 0);
    assert!(store.symbol_stale_paths(gone).unwrap().is_empty());
    assert_eq!(store.extractor_fingerprint(gone).unwrap(), None);
    assert_eq!(store.symbol_count(live).unwrap(), 1);
    assert_eq!(store.symbol_count("").unwrap(), 1, "the empty root stays");
}

/// T356: an existing directory that is `home` or `/` is dropped too; a project stays.
#[test]
fn retention_drops_home_and_filesystem_roots() {
    let store = Store::open_in_memory().unwrap();
    let home = super::support::tmp_dir("t356-home");
    let project = super::support::tmp_dir("t356-project");
    let (home, project) = (home.to_str().unwrap(), project.to_str().unwrap());
    let row = rtok_plugin_sdk::SymbolRow::new("f", "function", 1, true, 1, "");
    for root in [home, project, "/"] {
        store
            .replace_symbols(root, "a.rs", "s", (0, 0), std::slice::from_ref(&row))
            .unwrap();
        store.set_extractor_fingerprint(root, "fp").unwrap();
    }
    assert_eq!(
        store.drop_dead_symbol_roots(Some(Path::new(home))).unwrap(),
        2
    );
    for gone in [home, "/"] {
        assert_eq!(store.symbol_count(gone).unwrap(), 0, "{gone}");
        assert_eq!(store.extractor_fingerprint(gone).unwrap(), None, "{gone}");
    }
    assert_eq!(store.symbol_count(project).unwrap(), 1);
}

fn auto_vacuum_mode(store: &Store) -> i32 {
    let mut conn = store.lock().unwrap();
    sql_ext::AutoVacuumMode.get_result(&mut *conn).unwrap()
}

/// T419: a prefix read returns its keys in order, and `_`/`%` in the prefix are
/// literal, so `plugin:a_b:` never matches `plugin:axb:`.
#[test]
fn kv_prefix_is_literal_and_ordered() {
    let dir = super::support::tmp_dir("t419-kv-prefix");
    let store = Store::open(&dir.join("rtok.db")).unwrap();
    for (k, v) in [
        ("plugin:a_b:2", "two"),
        ("plugin:a_b:1", "one"),
        ("plugin:axb:1", "other"),
        ("plugin:a%b:1", "pct"),
        ("plugin:A_B:1", "upper"),
        ("other", "x"),
    ] {
        store.kv_set(k, v).unwrap();
    }
    let got = store.kv_prefix("plugin:a_b:").unwrap();
    assert_eq!(
        got,
        [
            ("plugin:a_b:1".to_string(), "one".to_string()),
            ("plugin:a_b:2".to_string(), "two".to_string()),
        ]
    );
    assert_eq!(store.kv_prefix("plugin:a%b:").unwrap().len(), 1);
    assert!(store.kv_prefix("nothing:").unwrap().is_empty());
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fresh_store_is_incremental_and_its_file_shrinks_after_retention() {
    let dir = super::support::tmp_dir("t352-shrink");
    let db = dir.join("rtok.db");
    let store = Store::open(&db).unwrap();
    assert_eq!(auto_vacuum_mode(&store), 2);
    store
        .upsert_session("s", Some(1), None, None, Some("hook"))
        .unwrap();
    super::tests_archive::hook_call_with_body(&store, "hook", &vec![b'x'; 2 << 20], 10);
    drop(store);
    let big = std::fs::metadata(&db).unwrap().len();
    assert!(big > 2 << 20, "{big}");

    let store = Store::open(&db).unwrap();
    store.run_retention(30, 3).unwrap();
    drop(store);
    let small = std::fs::metadata(&db).unwrap().len();
    assert!(small < big / 2, "{small} vs {big}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_existing_store_converts_once() {
    let dir = super::support::tmp_dir("t352-convert");
    let db = dir.join("old.db");
    // A pre-T352 file: one table exists before any `auto_vacuum` setting.
    let url = db.to_str().unwrap();
    let mut raw = SqliteConnection::establish(url).unwrap();
    diesel::sql_query("CREATE TABLE t (x INTEGER)")
        .execute(&mut raw)
        .unwrap();
    drop(raw);
    let store = Store::open(&db).unwrap();
    assert_eq!(auto_vacuum_mode(&store), 0);
    assert!(store.convert_to_incremental_vacuum().unwrap());
    assert_eq!(auto_vacuum_mode(&store), 2);
    assert!(!store.convert_to_incremental_vacuum().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Session-start housekeeping converts a pre-T352 store on its own, after retention.
#[test]
fn housekeeping_converts_a_pre_t352_store() {
    let dir = super::support::tmp_dir("t352-housekeeping");
    let db = dir.join("rtok.db");
    let mut raw = SqliteConnection::establish(db.to_str().unwrap()).unwrap();
    diesel::sql_query("CREATE TABLE t (x INTEGER)")
        .execute(&mut raw)
        .unwrap();
    drop(raw);
    assert_eq!(auto_vacuum_mode(&Store::open(&db).unwrap()), 0);
    let warnings = Store::housekeeping(&RetentionJob {
        db_path: db.clone(),
        retain_calls_days: 30,
        retain_hook_bodies_days: 3,
        prune: PrunePolicy::default(),
    });
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(auto_vacuum_mode(&Store::open(&db).unwrap()), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

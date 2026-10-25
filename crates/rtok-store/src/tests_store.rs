// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::*;
use schema::notes;

/// FTS5 reads `*`, `(`, `-` and bare operators as syntax, so `mem_search "read("` used to
/// raise a SQL error instead of returning no hits. User text is quoted now.
#[test]
fn note_search_treats_query_text_literally() {
    let store = Store::open_in_memory().unwrap();
    store
        .insert_note(None, "note", "parser", "call read( on the file")
        .unwrap();
    for query in ["read(", "*", "-", "AND", "\"quoted\"", "read()"] {
        let hits = store.search_notes(query, 5).unwrap();
        assert!(hits.len() <= 1, "{query:?} → {hits:?}");
    }
    assert_eq!(
        store.search_notes("read(", 5).unwrap().len(),
        1,
        "literal hit"
    );
    assert!(store.search_notes("   ", 5).unwrap().is_empty());
}

#[test]
fn migration_is_idempotent() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(
        store.migrate().unwrap(),
        0,
        "init already applied everything"
    );
    // Core tables from the embedded migrations — typed counts, not sqlite_master.
    let mut conn = store.lock().unwrap();
    let _: i64 = schema::events::table
        .count()
        .get_result(&mut *conn)
        .unwrap();
    let _: i64 = measurements::table.count().get_result(&mut *conn).unwrap();
    let _: i64 = archive::table.count().get_result(&mut *conn).unwrap();
    let _: i64 = read_cache::table.count().get_result(&mut *conn).unwrap();
    let _: i64 = notes::table.count().get_result(&mut *conn).unwrap();
    let _: i64 = usage::table.count().get_result(&mut *conn).unwrap();
}

/// Diesel keys a migration by the number before the first `_`, so two branches that both
/// took the next number merge into one migration and the other one never runs (0043, T501).
/// The directory is read, not the embedded list, because embedding already lost the second.
#[test]
fn every_migration_has_its_own_number() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut seen = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        let number = name.split('_').next().unwrap().to_owned();
        if let Some(other) = seen.insert(number.clone(), name.clone()) {
            panic!("{other} and {name} share the number {number}; renumber the newer one");
        }
    }
}

/// A database that already recorded `NNNN.sql` in `schema_migrations` must not run those
/// files again when Diesel's version table is empty.
#[test]
fn legacy_schema_migrations_are_not_rerun() {
    let dir = std::env::temp_dir().join(format!("rtok-mig-legacy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("rtok.db");
    let store = Store::open(&db).unwrap();
    let id = store
        .insert_note(None, "note", "kept", "survives the bridge")
        .unwrap();
    drop(store);
    let mut conn = SqliteConnection::establish(db.to_str().unwrap()).unwrap();
    super::migrations::revert_to_legacy_bookkeeping(&mut conn).unwrap();
    drop(conn);
    let store = Store::open(&db).unwrap();
    assert_eq!(store.migrate().unwrap(), 0);
    let row = store.note_row(id).unwrap().unwrap();
    assert_eq!(row.body, "survives the bridge");
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every surface opens the same file, so a fresh store is migrated by whichever of them
/// starts first — and the others start at the same moment. Each `Store::open` is its own
/// connection, so these threads race exactly as separate processes do: before the
/// exclusive transaction, the losers failed on `duplicate column name: mtime` (0007).
#[test]
fn concurrent_opens_of_a_fresh_store_all_migrate() {
    let dir = std::env::temp_dir().join(format!("rtok-mig-race-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("rtok.db");
    let errs: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| s.spawn(|| Store::open(&path).map(|_| ()).map_err(|e| format!("{e:#}"))))
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().unwrap().err())
            .collect()
    });
    let _ = std::fs::remove_dir_all(&dir);
    assert!(errs.is_empty(), "{errs:?}");
}

/// T75: the startup purge (`mcp`/`proxy` session start) must queue behind another
/// process's write transaction instead of dying on "database is locked" — the
/// deferred read-then-write transaction could lose instantly (SQLITE_BUSY_SNAPSHOT
/// skips the busy handler) or after the steady 1 s.
#[test]
fn purge_waits_out_a_concurrent_writer() {
    let dir = std::env::temp_dir().join(format!("rtok-purge-race-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("rtok.db");
    let store = Store::open(&db).unwrap();
    store.upsert_session("s", None, None, None, None).unwrap();
    let call = store
        .insert_call("s", "mcp", "mcp_call", None, None, None, None, None)
        .unwrap();
    store.set_call_ts(call, 1).unwrap(); // older than any cutoff
    drop(store);
    // A second connection holds the WAL writer lock for 1.2 s — past the steady
    // 1 s busy timeout the purge used to die on.
    let (held, held_ack) = std::sync::mpsc::channel();
    let url = db.to_str().unwrap().to_string();
    let holder = std::thread::spawn(move || {
        let mut conn = SqliteConnection::establish(&url).unwrap();
        sql_ext::busy_timeout(&mut conn, 1000).unwrap();
        sql_ext::pragma_journal_wal(&mut conn).unwrap();
        sql_ext::BeginImmediate.execute(&mut conn).unwrap();
        held.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        sql_ext::Commit.execute(&mut conn).unwrap();
    });
    held_ack.recv().unwrap();
    let store = Store::open(&db).unwrap();
    let purged = store
        .run_retention(30, 3)
        .expect("purge queues behind the writer");
    assert_eq!(purged, 1, "the old call is gone once the lock is released");
    assert_eq!(store.count_calls().unwrap(), 0);
    holder.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A fresh on-disk db with every migration before `tag` applied — the fixture a
/// test seeding a previous-schema quirk (0015, 0020, …) builds on before it seeds a
/// row and calls `Store::open` to run the one migration under test.
fn db_before_migration(tag: &str) -> (PathBuf, SqliteConnection) {
    let dir = std::env::temp_dir().join(format!("rtok-mig-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("rtok.db");
    let mut conn = SqliteConnection::establish(db.to_str().unwrap()).unwrap();
    super::migrations::run_before(&mut conn, tag).unwrap();
    (dir, conn)
}

/// T69.1: a `rtok.db` of the previous schema (0001–0014, one note) migrates in place —
/// 0015 adds the lifecycle columns with live defaults and the note survives retiring.
#[test]
fn migration_0015_adds_lifecycle_columns_to_a_previous_schema_db() {
    let (dir, mut conn) = db_before_migration("0015");
    let db = dir.join("rtok.db");
    diesel::insert_into(notes::table)
        .values((
            notes::ts.eq(1i64),
            notes::kind.eq("note"),
            notes::title.eq("old"),
            notes::body.eq("before the lifecycle"),
        ))
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let store = Store::open(&db).unwrap();
    let row = store.note_row(1).unwrap().unwrap();
    assert_eq!(
        (
            row.title.as_str(),
            row.retired,
            row.superseded_by,
            row.pinned
        ),
        ("old", None, None, 0)
    );
    assert!(store.retire_note(1, None).unwrap());
    assert!(
        store.search_notes("before", 5).unwrap().is_empty(),
        "retired note does not search"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// T615: claims written before `seq` existed keep the order the old query gave them
/// (newest `since`, ties by task id) once 0047 numbers them.
#[test]
fn migration_0047_numbers_old_claims_in_their_old_order() {
    let (dir, mut conn) = db_before_migration("0047");
    let db = dir.join("rtok.db");
    for (id, since) in [("R1", 100_i64), ("R2", 100), ("R3", 50), ("R4", 200)] {
        diesel::insert_into(schema::task_claims::table)
            .values((
                schema::task_claims::project.eq("p"),
                schema::task_claims::task_id.eq(id),
                schema::task_claims::agent_id.eq("a"),
                schema::task_claims::title.eq(id),
                schema::task_claims::since.eq(since),
            ))
            .execute(&mut conn)
            .unwrap();
    }
    drop(conn);
    let store = Store::open(&db).unwrap();
    let mut seen = Vec::new();
    while let Some(claim) = store.latest_task_claim("p", "a").unwrap() {
        store.clear_task_claim("p", &claim.task_id).unwrap();
        seen.push(claim.task_id);
    }
    assert_eq!(seen, ["R4", "R1", "R2", "R3"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// T927: `memory`/`observe` rows claimed the whole tool output as saved; 0052 drops them and
/// leaves every other row, `memory`/`brief` included, as it was.
#[test]
fn migration_0052_drops_observe_rows_only() {
    let (dir, mut conn) = db_before_migration("0052");
    let db = dir.join("rtok.db");
    for (plugin, kind) in [
        ("memory", "observe"),
        ("memory", "observe"),
        ("memory", "brief"),
        ("read", "observe"),
    ] {
        diesel::insert_into(measurements::table)
            .values((
                measurements::session.eq("s"),
                measurements::plugin.eq(plugin),
                measurements::kind.eq(kind),
                measurements::before_bytes.eq(99_589_i64),
                measurements::after_bytes.eq(404_i64),
                measurements::est_before.eq(25_000_i32),
                measurements::est_after.eq(100_i32),
            ))
            .execute(&mut conn)
            .unwrap();
    }
    drop(conn);
    let store = Store::open(&db).unwrap();
    let left: Vec<_> = store
        .measurement_totals()
        .unwrap()
        .into_iter()
        .map(|t| (t.plugin, t.kind, t.rows))
        .collect();
    assert_eq!(
        left,
        [
            ("memory".to_string(), "brief".to_string(), 1),
            ("read".to_string(), "observe".to_string(), 1),
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// T592: sessions that ended before `end_seq` existed are numbered in (ended_at, id) order,
/// and the old per-second mark resumes in front of its whole second, replaying rather than
/// guessing which tied sessions were posted.
#[test]
fn migration_0050_numbers_ended_sessions_and_rewinds_the_old_mark() {
    let (dir, mut conn) = db_before_migration("0050");
    let db = dir.join("rtok.db");
    for (id, ended) in [
        ("a", Some(100_i64)),
        ("b", Some(200)),
        ("c", Some(200)),
        ("d", Some(300)),
        ("e", None),
    ] {
        diesel::insert_into(schema::sessions::table)
            .values((
                schema::sessions::id.eq(id),
                schema::sessions::ended_at.eq(ended),
            ))
            .execute(&mut conn)
            .unwrap();
    }
    for (stream, mark) in [("sessions", 200_i64), ("sessions_tail", 1)] {
        diesel::insert_into(schema::otel_export::table)
            .values((
                schema::otel_export::stream.eq(stream),
                schema::otel_export::mark.eq(mark),
            ))
            .execute(&mut conn)
            .unwrap();
    }
    drop(conn);
    let store = Store::open(&db).unwrap();
    assert_eq!(store.otel_mark("sessions_seq").unwrap(), 1);
    assert_eq!(store.otel_mark("sessions").unwrap(), 0);
    assert_eq!(store.otel_mark("sessions_tail").unwrap(), 0);
    let pending = store.sessions_pending_export(1, 10).unwrap();
    let got: Vec<_> = pending.iter().map(|(n, s)| (*n, s.id.as_str())).collect();
    assert_eq!(got, [(2, "b"), (3, "c"), (4, "d")]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn upsert_same_title_with_a_new_body_keeps_the_old_body_in_history() {
    let store = Store::open_in_memory().unwrap();
    let (id, created) = store
        .upsert_note(Some("p"), "decision", "auth", "sessions")
        .unwrap();
    assert!(!created);
    assert!(store.note_versions(id).unwrap().is_empty());
    let (again, updated) = store
        .upsert_note(Some("p"), "decision", "auth", "jwt")
        .unwrap();
    assert!(updated);
    assert_eq!(again, id);
    assert_eq!(
        store.note_versions(id).unwrap(),
        vec![(1, "auth".to_string(), "sessions".to_string())]
    );
    assert_eq!(store.get_note_body(id).unwrap().as_deref(), Some("jwt"));
    let (third, _) = store
        .upsert_note(Some("p"), "decision", "auth", "opaque tokens")
        .unwrap();
    assert_eq!(third, id);
    assert_eq!(
        store.note_versions(id).unwrap(),
        vec![
            (1, "auth".to_string(), "sessions".to_string()),
            (2, "auth".to_string(), "jwt".to_string()),
        ]
    );
}

#[test]
fn checkpoint_upsert_writes_zero_version_rows() {
    let store = Store::open_in_memory().unwrap();
    let (id, _) = store
        .upsert_note(Some("p"), "checkpoint:s", "compact", "first")
        .unwrap();
    store
        .upsert_note(Some("p"), "checkpoint:s", "compact", "second")
        .unwrap();
    assert!(store.note_versions(id).unwrap().is_empty());
    let (sid, _) = store
        .upsert_note(Some("p"), "session:s", "handoff", "one")
        .unwrap();
    store
        .upsert_note(Some("p"), "session:s", "handoff", "two")
        .unwrap();
    assert!(store.note_versions(sid).unwrap().is_empty());
}

#[test]
fn same_body_upsert_writes_zero_version_rows() {
    let store = Store::open_in_memory().unwrap();
    let (id, _) = store.upsert_note(None, "note", "topic", "same").unwrap();
    let (_, updated) = store.upsert_note(None, "note", "topic", "same").unwrap();
    assert!(updated);
    assert!(store.note_versions(id).unwrap().is_empty());
    assert!(store.note_versions(id + 1).unwrap().is_empty());
}

/// T209: a `rtok.db` already holding duplicate `(project, kind, title)` notes — the
/// select-then-insert race migration 0020 closes — migrates by keeping only the
/// newest row per topic key, the same "highest id" tie-break `upsert_note` used
/// before the fix. Covers a NULL-project key and a project-scoped key.
#[test]
fn migration_0020_drops_pre_existing_duplicate_notes() {
    let (dir, mut conn) = db_before_migration("0020");
    let db = dir.join("rtok.db");
    sql_ext::SeedPre0020DuplicateNotes
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let store = Store::open(&db).unwrap();
    let mut conn = store.lock().unwrap();
    let rows: Vec<(i32, String)> = notes::table
        .order(notes::id.asc())
        .select((notes::id, notes::body))
        .load(&mut *conn)
        .unwrap();
    assert_eq!(
        rows,
        vec![(2, "fresh".to_string()), (4, "fresh".to_string())],
        "kept only the newest row per (project, kind, title)"
    );
    let err = diesel::insert_into(notes::table)
        .values((
            notes::project.eq(None::<&str>),
            notes::kind.eq("note"),
            notes::title.eq("dup"),
            notes::body.eq("third"),
        ))
        .execute(&mut *conn)
        .unwrap_err();
    assert!(
        format!("{err}").contains("UNIQUE"),
        "the index rejects a fresh duplicate too: {err}"
    );
    drop(conn);
    let _ = std::fs::remove_dir_all(&dir);
}

/// T245: rows from before migration 0021 (identical ones included) keep a NULL
/// `once_key` and survive; afterwards a second delivery of one keyed call adds nothing.
#[test]
fn migration_0021_keeps_old_rows_and_records_a_keyed_call_once() {
    let (dir, mut conn) = db_before_migration("0021");
    let db = dir.join("rtok.db");
    sql_ext::SeedPre0021Measurements.execute(&mut conn).unwrap();
    drop(conn);
    let store = Store::open(&db).unwrap();
    let m = Measurement {
        plugin: "read",
        kind: "delta",
        before_bytes: 9,
        after_bytes: 1,
        est_before: 3,
        est_after: 1,
        ref_id: None,
        call_id: None,
    };
    for _ in 0..2 {
        store
            .insert_measurement_once("s", &m, Some("PreToolUse:t1"))
            .unwrap();
    }
    store.insert_measurement("s", &m).unwrap();
    assert_eq!(store.count_measurements().unwrap(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

/// T209: two connections (hooks/MCP/proxy/`otel flush` are separate processes) racing
/// the same topic key must not land two rows — the UNIQUE index (migration 0020)
/// makes the atomic `INSERT … ON CONFLICT … DO UPDATE` resolve the race inside
/// SQLite instead of the old select-then-insert gap.
#[test]
fn concurrent_upsert_note_yields_one_row() {
    let dir = std::env::temp_dir().join(format!("rtok-note-race-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = dir.join("rtok.db");
    let a = Store::open(&db).unwrap();
    let b = Store::open(&db).unwrap();
    std::thread::scope(|s| {
        for store in [&a, &b] {
            s.spawn(move || {
                for i in 0..50 {
                    store
                        .upsert_note(Some("rtok"), "note", "topic", &format!("body {i}"))
                        .unwrap();
                }
            });
        }
    });
    let mut conn = a.lock().unwrap();
    let rows: Vec<i32> = notes::table
        .filter(notes::project.eq("rtok"))
        .filter(notes::kind.eq("note"))
        .filter(notes::title.eq("topic"))
        .select(notes::id)
        .load(&mut *conn)
        .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    drop(conn);
    let _ = std::fs::remove_dir_all(&dir);
}

/// T472: a new body keeps the previous one; a checkpoint and a same-body upsert do not.
#[test]
fn upsert_keeps_the_previous_body_and_skips_checkpoints_and_same_body() {
    let store = Store::open_in_memory().unwrap();
    let (id, created) = store
        .upsert_note(Some("rtok"), "decision", "topic", "first body")
        .unwrap();
    assert!(!created);
    assert!(store.note_versions(id).unwrap().is_empty());

    let (same, updated) = store
        .upsert_note(Some("rtok"), "decision", "topic", "second body")
        .unwrap();
    assert!(updated);
    assert_eq!(same, id);
    assert_eq!(
        store.note_versions(id).unwrap(),
        vec![(1, "topic".to_string(), "first body".to_string())]
    );
    assert_eq!(store.note_row(id).unwrap().unwrap().body, "second body");

    store
        .upsert_note(Some("rtok"), "decision", "topic", "second body")
        .unwrap();
    assert_eq!(
        store.note_versions(id).unwrap().len(),
        1,
        "a same-body upsert writes no version row"
    );

    store
        .upsert_note(Some("rtok"), "decision", "topic", "third body")
        .unwrap();
    assert_eq!(
        store.note_versions(id).unwrap(),
        vec![
            (1, "topic".to_string(), "first body".to_string()),
            (2, "topic".to_string(), "second body".to_string()),
        ]
    );

    let (cid, _) = store
        .upsert_note(None, "checkpoint:s", "compact", "checkpoint a")
        .unwrap();
    store
        .upsert_note(None, "checkpoint:s", "compact", "checkpoint b")
        .unwrap();
    assert!(
        store.note_versions(cid).unwrap().is_empty(),
        "checkpoint:* writes no version rows"
    );

    let (sid, _) = store
        .upsert_note(None, "session:s", "handoff", "session a")
        .unwrap();
    store
        .upsert_note(None, "session:s", "handoff", "session b")
        .unwrap();
    assert!(
        store.note_versions(sid).unwrap().is_empty(),
        "session:* writes no version rows"
    );
}

#[test]
fn fts5_match_finds_inserted_note() {
    let store = Store::open_in_memory().unwrap();
    {
        let mut conn = store.lock().unwrap();
        diesel::insert_into(notes::table)
            .values((
                notes::project.eq("rtok"),
                notes::kind.eq("decision"),
                notes::title.eq("WAL mode"),
                notes::body.eq("use sqlite wal journal"),
            ))
            .execute(&mut *conn)
            .unwrap();
    }
    let hits = store.search_notes("journal", 5).unwrap();
    assert_eq!(hits[0].title, "WAL mode");
}

#[test]
fn open_on_disk_uses_wal() {
    let dir = std::env::temp_dir().join(format!("rtok-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir.join("rtok.db")).unwrap();
    let mode = {
        let mut conn = store.lock().unwrap();
        sql_ext::JournalMode
            .get_result::<String>(&mut *conn)
            .unwrap()
    };
    assert_eq!(mode, "wal");
    drop(store);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn schema_0002_seeds_hosts_and_rejects_bad_fk() {
    use schema::calls;
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.migrate().unwrap(), 0);
    let mut conn = store.lock().unwrap();
    let tables: i64 = sql_ext::CountCoreV2Tables.get_result(&mut *conn).unwrap();
    assert_eq!(tables, 8);
    let host_n: i64 = hosts::table.count().get_result(&mut *conn).unwrap();
    // 0002.sql seeds 6; 0010.sql (T25.0) adds `pi`, the slug `rtok agent setup` installs
    // but the original list never had.
    assert_eq!(host_n, 7);
    diesel::insert_into(sessions::table)
        .values(sessions::id.eq("s1"))
        .execute(&mut *conn)
        .unwrap();
    let err = diesel::insert_into(calls::table)
        .values((
            calls::session_id.eq("s1"),
            calls::host_id.eq(999),
            calls::surface.eq("cli"),
            calls::kind.eq("cli"),
        ))
        .execute(&mut *conn);
    assert!(err.is_err(), "bad host_id must fail FK");
}

/// A clean `dir` plus an in-memory store holding one `mcp_call` row in session `s1`: the
/// call `insert_call_io` tests attach request/response bodies to.
fn io_fixture(dir: &Path) -> (Store, i32) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session("s1", Some(1), None, None, None)
        .unwrap();
    let id = store
        .insert_call(
            "s1",
            "mcp",
            "mcp_call",
            Some(1),
            None,
            None,
            Some("read"),
            Some("read"),
        )
        .unwrap();
    (store, id)
}

/// T431: a request body is saved cleaned — the size and the sha describe the saved bytes —
/// and verbatim once `[core] store_raw` is on; the response is never touched.
#[test]
fn request_bodies_are_saved_clean_unless_store_raw() {
    let dir = std::env::temp_dir().join(format!("rtok-io-clean-{}", std::process::id()));
    let (store, id) = io_fixture(&dir);
    let dirty = "{\"prompt\":\"<system-reminder>\\nx\\n</system-reminder>\\n\\u001b[31mfix\\u001b[0m it  \\r\\n\"}";
    let reply = "{\"out\":\"\\u001b[1mok\\u001b[0m\"}";
    let saved = |store: &Store, id: i32| -> (String, String, i64, String) {
        let mut conn = store.lock().unwrap();
        call_io::table
            .filter(call_io::call_id.eq(id))
            .select((
                call_io::request_json.assume_not_null(),
                call_io::response_json.assume_not_null(),
                call_io::request_bytes,
                call_io::request_sha256.assume_not_null(),
            ))
            .first(&mut *conn)
            .unwrap()
    };
    store
        .insert_call_io(
            id,
            Some(dirty.as_bytes()),
            Some(reply.as_bytes()),
            65536,
            None,
        )
        .unwrap();
    let (req, res, bytes, sha) = saved(&store, id);
    assert_eq!(req, r#"{"prompt":"fix it\n"}"#);
    assert_eq!(res, reply);
    assert_eq!(bytes, req.len() as i64);
    assert_eq!(sha, hex_sha256(req.as_bytes()));

    store.set_store_raw(true);
    let raw_id = store
        .insert_call(
            "s1",
            "mcp",
            "mcp_call",
            Some(1),
            None,
            None,
            Some("read"),
            Some("raw"),
        )
        .unwrap();
    store
        .insert_call_io(raw_id, Some(dirty.as_bytes()), None, 65536, None)
        .unwrap();
    let mut conn = store.lock().unwrap();
    let req: Option<String> = call_io::table
        .filter(call_io::call_id.eq(raw_id))
        .select(call_io::request_json)
        .first(&mut *conn)
        .unwrap();
    assert_eq!(req.as_deref(), Some(dirty));
}

/// T599: a wrapper that only appears once noise is gone is removed on the first save, on the
/// plain and the split hook path, so a body cleaned again stays as saved; `store_raw` keeps
/// the bytes, and size and sha describe whatever was saved.
#[test]
fn a_wrapper_hidden_by_noise_is_removed_before_saving() {
    let dir = std::env::temp_dir().join(format!("rtok-io-hidden-{}", std::process::id()));
    let (store, id) = io_fixture(&dir);
    let dirty = "{\"prompt\":\"<system-\\u001b[0mreminder>hidden</system-reminder>\\nvisible\"}";
    let saved = |store: &Store, id: i32| -> (String, i64, String) {
        let mut conn = store.lock().unwrap();
        call_io::table
            .filter(call_io::call_id.eq(id))
            .select((
                call_io::request_json.assume_not_null(),
                call_io::request_bytes,
                call_io::request_sha256.assume_not_null(),
            ))
            .first(&mut *conn)
            .unwrap()
    };
    store
        .insert_call_io(id, Some(dirty.as_bytes()), None, 65536, None)
        .unwrap();
    let (req, bytes, sha) = saved(&store, id);
    assert_eq!(req, r#"{"prompt":"visible"}"#);
    assert_eq!(bytes, req.len() as i64);
    assert_eq!(sha, hex_sha256(req.as_bytes()));
    assert_eq!(&*crate::sanitize::body(req.as_bytes()), req.as_bytes());

    let hook = format!(r#"{{"session_id":"s1",{}"#, &dirty[1..]);
    let hook_id = store
        .insert_call(
            "s1",
            "hook",
            "hook",
            None,
            None,
            None,
            None,
            Some("UserPromptSubmit"),
        )
        .unwrap();
    store
        .insert_hook_call_io(hook_id, hook.as_bytes(), None, 65536)
        .unwrap();
    let (req, bytes, sha) = saved(&store, hook_id);
    assert_eq!(req, r#"{"prompt":"visible"}"#);
    assert_eq!((bytes, sha), (req.len() as i64, hex_sha256(req.as_bytes())));
    let back = store.call_io_request(hook_id).unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&back).unwrap(),
        serde_json::json!({"session_id": "s1", "prompt": "visible"})
    );

    store.set_store_raw(true);
    let raw_id = store
        .insert_call(
            "s1",
            "hook",
            "hook",
            None,
            None,
            None,
            None,
            Some("UserPromptSubmit"),
        )
        .unwrap();
    store
        .insert_hook_call_io(raw_id, hook.as_bytes(), None, 65536)
        .unwrap();
    assert_eq!(
        store.call_io_request(raw_id).unwrap().unwrap(),
        hook.as_bytes()
    );
}

#[test]
fn write_api_round_trip_and_spill() {
    let dir = std::env::temp_dir().join(format!("rtok-io-{}", std::process::id()));
    let (store, id) = io_fixture(&dir);
    store
        .insert_call_io(
            id,
            Some(br#"{"a":1}"#),
            Some(br#"{"ok":true}"#),
            65536,
            Some(&dir),
        )
        .unwrap();
    store
        .insert_tokens(id, Some("read"), "before", "estimate", 10)
        .unwrap();
    store
        .insert_tokens(id, Some("read"), "after", "estimate", 4)
        .unwrap();
    store
        .insert_tokens(id, Some("read"), "mcp", "mcp", 12)
        .unwrap();
    store
        .insert_log(
            "info",
            "plugin",
            "read",
            "ok",
            Some("s1"),
            Some(id),
            Some("read"),
        )
        .unwrap();
    let mut conn = store.lock().unwrap();
    let n: i64 = tokens::table
        .filter(tokens::call_id.eq(id))
        .count()
        .get_result(&mut *conn)
        .unwrap();
    assert_eq!(n, 3);
    let logs_n: i64 = logs::table
        .filter(logs::source.eq("plugin"))
        .filter(logs::call_id.eq(id))
        .count()
        .get_result(&mut *conn)
        .unwrap();
    assert_eq!(logs_n, 1);
    drop(conn);

    let big = vec![b'x'; 70 * 1024];
    let id2 = store
        .insert_call(
            "s1",
            "mcp",
            "mcp_call",
            Some(1),
            None,
            None,
            Some("read"),
            Some("big"),
        )
        .unwrap();
    store
        .insert_call_io(id2, Some(&big), None, 64 * 1024, Some(&dir))
        .unwrap();
    let mut conn = store.lock().unwrap();
    let (request_json, request_archive): (Option<String>, Option<String>) = call_io::table
        .filter(call_io::call_id.eq(id2))
        .select((call_io::request_json, call_io::request_archive))
        .first(&mut *conn)
        .unwrap();
    assert!(request_json.is_none());
    assert!(request_archive.is_some());
    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// T208: `insert_call_io`'s archive-row inserts and its final `call_io` insert commit
/// together. A pre-existing `call_io` row for the same call (its `call_id` is a
/// `PRIMARY KEY`) makes the real call's final insert fail after its archive row already
/// went in inside the same transaction — the rollback must leave no `archive` row and no
/// payload file behind.
#[test]
fn insert_call_io_failure_leaves_no_orphan_archive() {
    let dir = std::env::temp_dir().join(format!("rtok-io-orphan-{}", std::process::id()));
    let (store, id) = io_fixture(&dir);
    {
        let mut conn = store.lock().unwrap();
        diesel::insert_into(call_io::table)
            .values(call_io::call_id.eq(id))
            .execute(&mut *conn)
            .unwrap();
    }
    let big = vec![b'x'; 70 * 1024];
    let err = store
        .insert_call_io(id, Some(&big), None, 64 * 1024, Some(&dir))
        .unwrap_err();
    assert!(err.to_string().contains("UNIQUE"), "{err}");
    let mut conn = store.lock().unwrap();
    let archive_n: i64 = archive::table.count().get_result(&mut *conn).unwrap();
    assert_eq!(
        archive_n, 0,
        "a failed call_io insert must not strand an archive row"
    );
    drop(conn);
    let sha = hex_sha256(&big);
    assert!(
        !dir.join(&sha).exists(),
        "the orphan payload file must be removed on rollback"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A later upsert that omits attribution must not wipe what an earlier one set —
/// hook → proxy (same session id) used to NULL out project/cwd; Runtime → proxy
/// used to NULL out source.
#[test]
fn upsert_session_keeps_non_null_attribution() {
    let store = Store::open_in_memory().unwrap();
    let claude = store.host_id("claude").unwrap().expect("seeded");
    store
        .upsert_session("s1", Some(claude), Some("rtok"), Some("/tmp/rtok"), None)
        .unwrap();
    store
        .upsert_session("s1", Some(claude), None, None, Some("proxy"))
        .unwrap();
    let (slug, project, cwd) = store.session_row("s1").unwrap().unwrap();
    assert_eq!(slug.as_deref(), Some("claude"));
    assert_eq!(
        project.as_deref(),
        Some("rtok"),
        "project survived a None upsert"
    );
    assert_eq!(
        cwd.as_deref(),
        Some("/tmp/rtok"),
        "cwd survived a None upsert"
    );
    let mut conn = store.lock().unwrap();
    let source: Option<String> = sessions::table
        .filter(sessions::id.eq("s1"))
        .select(sessions::source)
        .first(&mut *conn)
        .unwrap();
    assert_eq!(source.as_deref(), Some("proxy"));
    // And a Runtime-shaped upsert (source None) must keep the proxy source.
    drop(conn);
    store
        .upsert_session("s1", Some(claude), None, None, None)
        .unwrap();
    let mut conn = store.lock().unwrap();
    let source: Option<String> = sessions::table
        .filter(sessions::id.eq("s1"))
        .select(sessions::source)
        .first(&mut *conn)
        .unwrap();
    assert_eq!(
        source.as_deref(),
        Some("proxy"),
        "source survived a None upsert"
    );
}

#[test]
fn two_apis_are_two_stats_rows() {
    let store = Store::open_in_memory().unwrap();
    super::support::seed_two_apis(&store);
    assert_eq!(store.usage_by_api().unwrap().len(), 2);
}

/// The old raw SQL grouped by `COALESCE(model, 'unknown')`, so a `NULL`-model row and
/// a row whose model is literally `"unknown"` summed into a single bucket. The typed
/// DSL groups by the raw column, so `usage_by_model` must fold those two groups back
/// together in Rust to keep that behaviour.
#[test]
fn usage_by_model_merges_null_and_literal_unknown() {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session("s1", None, None, None, Some("proxy"))
        .unwrap();
    let call = store
        .insert_call(
            "s1",
            "proxy",
            "api_request",
            None,
            None,
            None,
            None,
            Some("/v1/messages"),
        )
        .unwrap();
    store
        .insert_usage("s1", None, "anthropic", 10, 1, 2, 3, call)
        .unwrap();
    store
        .insert_usage("s1", Some("unknown"), "anthropic", 5, 0, 1, 2, call)
        .unwrap();
    let rows = store.usage_by_model().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model, "unknown");
    assert_eq!(rows[0].input, 15);
    assert_eq!(rows[0].cache_create, 1);
    assert_eq!(rows[0].cache_read, 3);
    assert_eq!(rows[0].output, 5);
}

/// T25.1's Check: three sessions across the two hosts the migrations seed
/// (`claude` from 0002, `pi` from 0010), one ended, one with no usage yet —
/// `session_totals` sums each one's tokens exactly, newest first, and `since`
/// windows on `started_at` without cutting the totals.
#[test]
fn session_totals_sums_each_session_exactly() {
    let store = Store::open_in_memory().unwrap();
    let claude = store.host_id("claude").unwrap().expect("0002 seeds claude");
    let pi = store.host_id("pi").unwrap().expect("0010 seeds pi");
    store
        .upsert_session(
            "a",
            Some(claude),
            Some("rtok"),
            Some("/w/rtok"),
            Some("proxy"),
        )
        .unwrap();
    store
        .upsert_session("b", Some(pi), Some("rtok"), None, None)
        .unwrap();
    store
        .upsert_session("c", Some(pi), None, None, None)
        .unwrap();
    let (pid, mid) = store.upsert_model("anthropic", "claude-x").unwrap();
    let call_a = store
        .insert_call(
            "a",
            "proxy",
            "api_request",
            Some(claude),
            Some(pid),
            Some(mid),
            None,
            Some("/v1/messages"),
        )
        .unwrap();
    let call_b = store
        .insert_call(
            "b",
            "proxy",
            "api_request",
            Some(pi),
            None,
            None,
            None,
            Some("/v1/chat/completions"),
        )
        .unwrap();
    store
        .insert_usage("a", Some("claude-x"), "anthropic", 10, 1, 2, 3, call_a)
        .unwrap();
    store
        .insert_usage("a", Some("claude-x"), "anthropic", 20, 0, 5, 4, call_a)
        .unwrap();
    store
        .insert_usage("b", Some("gpt-x"), "openai_chat", 7, 2, 0, 1, call_b)
        .unwrap();
    store.end_session("b", 2500).unwrap();
    // The write path stamps `unixepoch()`; pin the timeline so last-activity and
    // the `since` window are exact. The in-memory DB is fresh, so rowids are the
    // insert order: usage 1–2 belong to "a", 3 to "b".
    {
        let mut conn = store.lock().unwrap();
        for (id, started) in [("a", 1000i64), ("b", 2000), ("c", 3000)] {
            diesel::update(sessions::table.filter(sessions::id.eq(id)))
                .set(sessions::started_at.eq(started))
                .execute(&mut *conn)
                .unwrap();
        }
        for (id, ts) in [(1i32, 1100i64), (2, 1200), (3, 2100)] {
            diesel::update(usage::table.filter(usage::id.eq(id)))
                .set(usage::ts.eq(ts))
                .execute(&mut *conn)
                .unwrap();
        }
        for (id, ts) in [(call_a, 1500i64), (call_b, 2100)] {
            diesel::update(calls::table.filter(calls::id.eq(id)))
                .set(calls::ts.eq(ts))
                .execute(&mut *conn)
                .unwrap();
        }
    }
    // Newest first. "a": two usage rows summed, last activity from the newer call
    // (ts 1500 > 1200), live. "b": ended, one row. "c": no usage and no calls, so
    // zeroed with last activity = started_at.
    #[allow(clippy::too_many_arguments)]
    fn expect(
        id: &str,
        host: Option<&str>,
        project: Option<&str>,
        provider: Option<&str>,
        api: Option<&str>,
        model: Option<&str>,
        tokens: (i64, i64, i64, i64),
        started_at: i64,
        last_activity: i64,
        ended_at: Option<i64>,
    ) -> SessionTotals {
        SessionTotals {
            id: id.into(),
            host: host.map(String::from),
            project: project.map(String::from),
            provider: provider.map(String::from),
            api: api.map(String::from),
            model: model.map(String::from),
            input: tokens.0,
            cache_create: tokens.1,
            cache_read: tokens.2,
            output: tokens.3,
            started_at,
            last_activity,
            ended_at,
        }
    }
    assert_eq!(
        store.session_totals(0).unwrap(),
        vec![
            expect(
                "c",
                Some("pi"),
                None,
                None,
                None,
                None,
                (0, 0, 0, 0),
                3000,
                3000,
                None
            ),
            expect(
                "b",
                Some("pi"),
                Some("rtok"),
                None,
                Some("openai_chat"),
                Some("gpt-x"),
                (7, 2, 0, 1),
                2000,
                2100,
                Some(2500)
            ),
            expect(
                "a",
                Some("claude"),
                Some("rtok"),
                Some("anthropic"),
                Some("anthropic"),
                Some("claude-x"),
                (30, 1, 7, 7),
                1000,
                1500,
                None
            ),
        ]
    );
    // `since` floors `started_at`; the totals it returns stay whole-session.
    let ids: Vec<String> = store
        .session_totals(2000)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["c", "b"]);
    let only_c = store.session_totals(3000).unwrap();
    assert_eq!(only_c.len(), 1);
    assert_eq!(only_c[0].id, "c");
}

/// T15.5's Check: `recent_calls` is the Calls page's one read — newest first,
/// bounded, the three slugs joined, and the newest `usage` row linked when the call
/// has one (the same row `call_detail` serves the otel span).
#[test]
fn recent_calls_is_newest_first_bounded_and_linked() {
    let store = Store::open_in_memory().unwrap();
    let claude = store.host_id("claude").unwrap().expect("0002 seeds claude");
    store
        .upsert_session("s", Some(claude), None, None, Some("proxy"))
        .unwrap();
    let (pid, mid) = store.upsert_model("anthropic", "claude-x").unwrap();
    let hook = store
        .insert_call("s", "hook", "hook", None, None, None, None, Some("Stop"))
        .unwrap();
    let run = store
        .insert_call(
            "s",
            "hook",
            "plugin_run",
            None,
            None,
            None,
            Some("cmd"),
            None,
        )
        .unwrap();
    store.set_call_parent(run, hook).unwrap();
    let api = store
        .insert_call(
            "s",
            "proxy",
            "api_request",
            Some(claude),
            Some(pid),
            Some(mid),
            None,
            Some("/v1/messages"),
        )
        .unwrap();
    store.set_call_ms(api, 12.5).unwrap();
    store
        .insert_usage("s", Some("claude-x"), "anthropic", 10, 1, 2, 3, api)
        .unwrap();
    // A second usage row on the same call: the linkage is the newest, not the sum.
    store
        .insert_usage("s", Some("claude-x"), "anthropic", 20, 0, 5, 4, api)
        .unwrap();

    let rows = store.recent_calls(10).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![api, run, hook],
        "newest first"
    );
    let top = &rows[0];
    assert_eq!(top.api.as_deref(), Some("anthropic"));
    assert_eq!(
        (top.input, top.cache_create, top.cache_read, top.output),
        (Some(20), Some(0), Some(5), Some(4))
    );
    assert_eq!(top.ms, Some(12.5));
    assert_eq!(top.host.as_deref(), Some("claude"));
    assert_eq!(top.provider.as_deref(), Some("anthropic"));
    assert_eq!(top.model.as_deref(), Some("claude-x"));
    assert_eq!(top.parent_id, None);
    let nested = &rows[1];
    assert_eq!(nested.parent_id, Some(hook), "the plugin run nests");
    assert!(nested.api.is_none(), "a plugin run carries no usage");
    assert!(rows[2].api.is_none(), "a hook call carries no usage");

    // The bound: only the newest survive it.
    let bounded = store.recent_calls(1).unwrap();
    assert_eq!(bounded.len(), 1);
    assert_eq!(bounded[0].id, api);
}

#[test]
fn an_observation_call_key_holds_past_the_narrative_window() {
    let store = Store::open_in_memory().unwrap();
    let obs = rtok_plugin_sdk::NewObservation {
        session_id: "s",
        project: None,
        obs_type: "command_run",
        title: "t",
        narrative: "n",
        dedup: "d",
        files: &[],
    };
    let insert = |once| store.insert_observation_once(&obs, once).unwrap();
    assert!(insert(Some("PostToolUse:a")).is_some());
    assert!(insert(Some("PostToolUse:a")).is_none(), "inside the window");
    store.set_observation_ts("s", 0).unwrap();
    assert!(
        insert(Some("PostToolUse:a")).is_none(),
        "the same call, late"
    );
    assert!(
        insert(Some("PostToolUse:b")).is_some(),
        "another call, late"
    );
    assert!(insert(None).is_none(), "no key: the window still holds");
    store.set_observation_ts("s", 0).unwrap();
    assert!(insert(None).is_some(), "no key, late");
    store.set_observation_ts("s", 0).unwrap();
    assert!(insert(None).is_some(), "keyless rows never collide");
}

/// T617: an observation linked to several queried files counts once, so the limit is in
/// observations and an older distinct hit is not starved by a newer multi-file one.
#[test]
fn file_linked_observations_are_distinct_before_the_limit() {
    let store = Store::open_in_memory().unwrap();
    let add = |project: Option<&str>, dedup: &str, files: &[&str]| {
        let files: Vec<String> = files.iter().map(|f| (*f).to_string()).collect();
        store
            .insert_observation(&rtok_plugin_sdk::NewObservation {
                session_id: "s",
                project,
                obs_type: "file_read",
                title: "t",
                narrative: dedup,
                dedup,
                files: &files,
            })
            .unwrap()
            .unwrap()
    };
    let older = add(Some("p"), "older", &["a.rs"]);
    let newer = add(Some("p"), "newer", &["a.rs", "b.rs"]);
    let other = add(Some("q"), "other", &["a.rs", "b.rs"]);
    let paths = ["a.rs".to_string(), "b.rs".to_string()];
    let ids = |project, paths: &[String], limit| -> Vec<i32> {
        store
            .observations_for_files(project, paths, limit)
            .unwrap()
            .iter()
            .map(|h| h.id)
            .collect()
    };
    assert_eq!(ids(Some("p"), &paths, 2), vec![newer, older]);
    assert_eq!(ids(Some("p"), &paths[..1], 5), vec![newer, older]);
    assert_eq!(ids(Some("p"), &paths[1..], 5), vec![newer]);
    assert_eq!(ids(Some("q"), &paths, 5), vec![other]);
    assert!(ids(Some("p"), &[], 5).is_empty());
}

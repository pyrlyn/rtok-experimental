// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use rstest::rstest;

use super::*;

/// An old call's own rows go; the ledger rows that point at it stay, detached — and the
/// foreign keys never refuse the delete.
#[test]
fn purge_drops_old_calls_and_detaches_their_ledger_rows() {
    let store = Store::open_in_memory().unwrap();
    let claude = store.host_id("claude").unwrap();
    store
        .upsert_session("s", claude, None, None, Some("proxy"))
        .unwrap();
    let call = |kind| {
        store
            .insert_call("s", "proxy", kind, None, None, None, None, None)
            .unwrap()
    };
    let (old, child, fresh) = (call("api_request"), call("plugin_run"), call("hook"));
    store.set_call_parent(child, old).unwrap();
    store
        .insert_usage("s", None, "anthropic", 1, 0, 0, 1, old)
        .unwrap();
    let m = Measurement {
        plugin: "cmd",
        kind: "rule",
        before_bytes: 10,
        after_bytes: 5,
        est_before: 3,
        est_after: 1,
        ref_id: None,
        call_id: Some(old),
    };
    store.insert_measurement("s", &m).unwrap();
    store.insert_provider_tokens(old, 2, 1, 0, 0, 1).unwrap();
    store
        .insert_call_io(old, Some(b"{}"), None, 1024, None)
        .unwrap();
    store
        .insert_log("info", "proxy", "x", "m", Some("s"), Some(old), None)
        .unwrap();
    store.set_call_ts(old, 0).unwrap();

    assert_eq!(store.purge_calls_older_than(1).unwrap(), 1);
    let mut conn = store.lock().unwrap();
    let kept: i64 = calls::table
        .filter(calls::id.eq_any([child, fresh]))
        .count()
        .get_result(&mut *conn)
        .unwrap();
    assert_eq!(kept, 2);
    let attached: i64 = calls::table
        .filter(calls::parent_id.is_not_null())
        .count()
        .get_result(&mut *conn)
        .unwrap();
    assert_eq!(attached, 0, "the child is detached");
    let usage_detached: i64 = usage::table
        .filter(usage::call_id.is_null())
        .count()
        .get_result(&mut *conn)
        .unwrap();
    let meas_detached: i64 = measurements::table
        .filter(measurements::call_id.is_null())
        .count()
        .get_result(&mut *conn)
        .unwrap();
    assert_eq!(usage_detached, 1, "usage row kept, detached");
    assert_eq!(meas_detached, 1, "measurements row kept, detached");
    let token_n: i64 = tokens::table.count().get_result(&mut *conn).unwrap();
    let io_n: i64 = call_io::table.count().get_result(&mut *conn).unwrap();
    let log_n: i64 = logs::table.count().get_result(&mut *conn).unwrap();
    drop(conn);
    assert_eq!(token_n, 0, "tokens");
    assert_eq!(io_n, 0, "call_io");
    assert_eq!(log_n, 0, "logs");
    assert_eq!(
        store.purge_calls_older_than(0).unwrap(),
        0,
        "days <= 0 is a no-op"
    );
}

/// One call whose 70 KiB body spills to archive, backdated past `retain_calls_days = 1` —
/// shared by `run_retention_purges_old_call_and_archive` and
/// `archives_pending_retention_previews_without_deleting` (T182), which exercise the same
/// `doomed_archives` set through the deleting and the previewing entry point.
fn seed_one_spilled_call(dir: &Path) -> (Store, PathBuf) {
    let db = dir.join("rtok.db");
    let archive = dir.join("archive");
    let store = Store::open(&db).unwrap();
    store
        .upsert_session("sess", Some(1), None, None, Some("proxy"))
        .unwrap();
    let call = store
        .insert_call(
            "sess",
            "proxy",
            "api_request",
            Some(1),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let body = vec![b'x'; 70 * 1024];
    store
        .insert_call_io(call, Some(&body), None, 64 * 1024, Some(&archive))
        .unwrap();
    store.set_call_ts(call, 0).unwrap();
    let arch_path = archive.join(hex_sha256(&body));
    (store, arch_path)
}

#[rstest]
fn run_retention_purges_old_call_and_archive() {
    let dir = std::env::temp_dir().join(format!("rtok-retain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (store, arch_path) = seed_one_spilled_call(&dir);
    assert!(arch_path.is_file());
    assert_eq!(store.count_calls().unwrap(), 1);

    assert_eq!(store.run_retention(1, 3).unwrap(), 1);
    assert_eq!(store.count_calls().unwrap(), 0);
    assert!(!arch_path.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// T385.9.1: one summary per archive id and compressor version, first writer wins, and a
/// version bump is a separate row; retention takes the summary with its archive row.
#[test]
fn archive_summaries_are_stored_once_per_version_and_purged_with_the_archive() {
    let dir = std::env::temp_dir().join(format!("rtok-t385-9-1-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (store, arch_path) = seed_one_spilled_call(&dir);
    let id = arch_path.file_name().unwrap().to_str().unwrap().to_string();

    assert_eq!(store.archive_summary(&id, "v1").unwrap(), None);
    assert!(store.put_archive_summary(&id, "v1", "first").unwrap());
    assert!(!store.put_archive_summary(&id, "v1", "second").unwrap());
    assert!(store.put_archive_summary(&id, "v2", "other").unwrap());
    assert_eq!(
        store.archive_summaries().unwrap(),
        [(id.clone(), "first".into()), (id.clone(), "other".into())]
    );
    assert_eq!(
        store.archive_summary(&id, "v1").unwrap().as_deref(),
        Some("first")
    );
    assert_eq!(
        store.archive_summary(&id, "v2").unwrap().as_deref(),
        Some("other")
    );

    assert_eq!(store.run_retention(1, 3).unwrap(), 1);
    assert_eq!(store.archive_summary(&id, "v1").unwrap(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `agents junk clear`'s dry run (T182): the same set `run_retention` would delete,
/// named without touching the database or the file.
#[test]
fn archives_pending_retention_previews_without_deleting() {
    let dir = std::env::temp_dir().join(format!("rtok-t182-preview-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (store, arch_path) = seed_one_spilled_call(&dir);

    let preview = store.archives_pending_retention(1).unwrap();
    assert_eq!(preview, vec![arch_path.clone()]);
    assert!(arch_path.is_file(), "preview must not delete anything");
    assert_eq!(store.count_calls().unwrap(), 1);

    assert_eq!(
        store.archives_pending_retention(0).unwrap(),
        Vec::<PathBuf>::new(),
        "0 = keep forever"
    );

    assert_eq!(store.run_retention(1, 3).unwrap(), 1);
    assert!(!arch_path.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// T45.3: the same `tool_use_id` in two sessions is two decisions (composite key, 0014).
#[test]
fn archive_decision_repeated_id_persists_per_session() {
    let dir = std::env::temp_dir().join(format!("rtok-t453-id-{}", std::process::id()));
    let store = Store::open_in_memory().unwrap();
    let id = store.put_archive("a", b"body", &dir).unwrap();
    store
        .put_archive_decision("tu-1", &id, "a", "ptr-a")
        .unwrap();
    store
        .put_archive_decision("tu-1", &id, "b", "ptr-b")
        .unwrap();
    let a = store
        .archive_decision("a", "tu-1")
        .unwrap()
        .expect("session a");
    let b = store
        .archive_decision("b", "tu-1")
        .unwrap()
        .expect("session b");
    assert_eq!((a.pointer.as_str(), b.pointer.as_str()), ("ptr-a", "ptr-b"));
    assert_eq!(
        store.live_zone_pointer(&id).unwrap().as_deref(),
        Some("ptr-a"),
        "deterministic by (tool_use_id, session)"
    );
    let other = store.put_archive("c", b"none", &dir).unwrap();
    assert_eq!(store.live_zone_pointer(&other).unwrap(), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// T306: `session_live_archives` used to order by `archive::ts` (first-archived time,
/// never re-stamped on a dedup hit), so a body re-archived unchanged by a later decision
/// ranked as stale. Order by `archive_decisions::ts` instead — this session's own
/// pointer time — and return each archive id once even when two decisions name it.
#[test]
fn session_live_archives_orders_by_decision_time_once_each() {
    let dir = std::env::temp_dir().join(format!("rtok-t306-order-{}", std::process::id()));
    let store = Store::open_in_memory().unwrap();
    let x = store.put_archive("s1", b"body-x", &dir).unwrap();
    let y = store.put_archive("s1", b"body-y", &dir).unwrap();
    {
        // X's body was archived first and stays there: dedup never re-stamps it.
        let mut conn = store.lock().unwrap();
        diesel::update(archive::table.filter(archive::id.eq(&x)))
            .set(archive::ts.eq(100))
            .execute(&mut *conn)
            .unwrap();
        diesel::update(archive::table.filter(archive::id.eq(&y)))
            .set(archive::ts.eq(200))
            .execute(&mut *conn)
            .unwrap();
    }
    store
        .put_archive_decision("tu-x1", &x, "s1", "ptr-x1")
        .unwrap();
    store
        .put_archive_decision("tu-y1", &y, "s1", "ptr-y1")
        .unwrap();
    // X re-archived by a new decision (e.g. an unchanged file read again): a new
    // tool_use_id pointing at the same archive id, decided after Y.
    store
        .put_archive_decision("tu-x2", &x, "s1", "ptr-x2")
        .unwrap();
    {
        let mut conn = store.lock().unwrap();
        for (tool_use_id, ts) in [("tu-x1", 100), ("tu-y1", 200), ("tu-x2", 300)] {
            diesel::update(
                archive_decisions::table
                    .filter(archive_decisions::session.eq("s1"))
                    .filter(archive_decisions::tool_use_id.eq(tool_use_id)),
            )
            .set(archive_decisions::ts.eq(ts))
            .execute(&mut *conn)
            .unwrap();
        }
    }
    let rows = store.session_live_archives("s1").unwrap();
    let ids: Vec<&str> = rows.iter().map(|(id, _, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        vec![x.as_str(), y.as_str()],
        "X's newest decision (tu-x2) outranks Y, and X appears once"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn archive_in_session_hits_only_the_writer_session() {
    let dir = std::env::temp_dir().join(format!("rtok-t651-sess-{}", std::process::id()));
    let store = Store::open_in_memory().unwrap();
    let sha = store.put_archive("a", b"same-bytes", &dir).unwrap();
    let hit = store
        .archive_in_session("a", &sha, None)
        .unwrap()
        .expect("writer");
    assert_eq!(hit.0, sha);
    assert_eq!(store.archive_in_session("b", &sha, None).unwrap(), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// T127: a sub-agent's context never dedups on a body only its parent (or a sibling)
/// wrote — the pointer would name bytes that context never saw — but the writer's own
/// context still gets the pointer on its own repeat.
#[test]
fn archive_in_session_scopes_by_context_within_one_session() {
    let dir = std::env::temp_dir().join(format!("rtok-t127-ctx-{}", std::process::id()));
    let store = Store::open_in_memory().unwrap();
    let sha = store
        .put_archive_for("s", b"same-bytes", &dir, Some("agent-a"))
        .unwrap();
    assert_eq!(
        store
            .archive_in_session("s", &sha, Some("agent-b"))
            .unwrap(),
        None,
        "context B never saw what context A archived"
    );
    assert_eq!(store.archive_in_session("s", &sha, None).unwrap(), None);
    let hit = store
        .archive_in_session("s", &sha, Some("agent-a"))
        .unwrap()
        .expect("agent-a still hits its own row");
    assert_eq!(hit.0, sha);
    let _ = std::fs::remove_dir_all(dir);
}

/// T929: a compaction forgets what a context was shown. The forgotten body is no dedup target
/// any more, stays readable for `expand`, and printing it again hands it back to the session.
#[test]
fn forget_seen_ends_dedup_for_that_context_and_a_reprint_reclaims_the_row() {
    let dir = std::env::temp_dir().join(format!("rtok-t929-forget-{}", std::process::id()));
    let store = Store::open_in_memory().unwrap();
    let main = store
        .put_archive_for("s", b"main-body", &dir, None)
        .unwrap();
    let sub = store
        .put_archive_for("s", b"sub-body", &dir, Some("agent-a"))
        .unwrap();
    store.put_read_cache("s", "/a", "h", Some(&main)).unwrap();
    store.put_read_cache("other", "/a", "h", None).unwrap();

    store.forget_seen("s", None).unwrap();

    assert_eq!(store.archive_in_session("s", &main, None).unwrap(), None);
    assert!(
        store.get_archive(&main, Some(&dir)).unwrap().is_some(),
        "still expandable"
    );
    assert!(
        store
            .archive_in_session("s", &sub, Some("agent-a"))
            .unwrap()
            .is_some(),
        "another context's window was not compacted"
    );
    assert!(store.get_read_cache("s", "/a").unwrap().is_none());
    assert!(store.get_read_cache("other", "/a").unwrap().is_some());

    store
        .put_archive_for("s", b"main-body", &dir, None)
        .unwrap();
    assert!(
        store
            .archive_in_session("s", &main, None)
            .unwrap()
            .is_some()
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// T929: another session's identical body is not reclaimed by a reprint.
#[test]
fn a_forgotten_row_is_not_taken_over_by_another_session() {
    let dir = std::env::temp_dir().join(format!("rtok-t929-owner-{}", std::process::id()));
    let store = Store::open_in_memory().unwrap();
    let sha = store.put_archive_for("a", b"same", &dir, None).unwrap();
    store.forget_seen("a", None).unwrap();
    store.put_archive_for("b", b"same", &dir, None).unwrap();
    assert_eq!(store.archive_in_session("b", &sha, None).unwrap(), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// T210: `measurements` is never pruned (`purge_calls_older_than` keeps it forever), and
/// `archive_in_session`'s correlated subquery scans it by `(session, ts)` on every dedup
/// hit. `EXPLAIN QUERY PLAN` on the query's real shape (mirroring the Diesel-generated
/// SQL: `archive` filtered by `id`/`session`/`agent_id`, joined to the correlated
/// `COUNT(*) FROM measurements WHERE session = archive.session AND ts > archive.ts`)
/// must show the subquery using `measurements_session_ts`, never a full table scan.
#[test]
fn archive_in_session_query_plan_uses_the_session_ts_index() {
    let store = Store::open_in_memory().unwrap();
    let mut conn = store.lock().unwrap();
    let rows: Vec<(i32, i32, i32, String)> = sql_ext::ExplainArchiveInSessionPlan
        .load(&mut *conn)
        .unwrap();
    let plan = rows
        .iter()
        .map(|r| r.3.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        plan.contains("USING COVERING INDEX measurements_session_ts")
            || plan.contains("USING INDEX measurements_session_ts"),
        "expected the (session, ts) index on measurements, got: {plan}"
    );
    assert!(
        !plan.contains("SCAN measurements"),
        "measurements scanned: {plan}"
    );
}

/// T210: with 100k unrelated `measurements` rows ahead of it, one `archive_in_session`
/// lookup must stay a `(session, ts)` index search, not a linear scan — the query-plan
/// test above is the hard check; this is a generous, non-flaky wall-clock guard against
/// a regression that keeps the plan right but still degrades in practice.
#[test]
fn archive_in_session_stays_fast_with_100k_measurements() {
    let store = Store::open_in_memory().unwrap();
    let dir = std::env::temp_dir().join(format!("rtok-t210-perf-{}", std::process::id()));
    let sha = store.put_archive("s-target", b"needle", &dir).unwrap();

    {
        let mut conn = store.lock().unwrap();
        conn.transaction::<_, anyhow::Error, _>(|conn| {
            // 1000 rows × 8 binds per statement stays under SQLite's 32766-variable cap.
            for chunk in (0..100_000i64).collect::<Vec<_>>().chunks(1000) {
                let rows: Vec<_> = chunk
                    .iter()
                    .map(|&i| {
                        // Mostly other sessions, so a scan would pay for rows the index skips.
                        let session = if i % 7 == 0 { "s-target" } else { "s-other" };
                        (
                            measurements::ts.eq(i),
                            measurements::session.eq(session),
                            measurements::plugin.eq("cmd"),
                            measurements::kind.eq("rule"),
                            measurements::before_bytes.eq(1i64),
                            measurements::after_bytes.eq(1i64),
                            measurements::est_before.eq(1),
                            measurements::est_after.eq(1),
                        )
                    })
                    .collect();
                diesel::insert_into(measurements::table)
                    .values(&rows)
                    .execute(conn)?;
            }
            Ok(())
        })
        .unwrap();
    }

    let start = std::time::Instant::now();
    let hit = store
        .archive_in_session("s-target", &sha, None)
        .unwrap()
        .expect("writer session still hits its own row");
    let elapsed = start.elapsed();
    assert_eq!(hit.0, sha);
    assert!(
        elapsed.as_millis() < 200,
        "archive_in_session took {elapsed:?} against 100k measurements rows \
             (index-backed lookup expected)"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// T55.11: the expander's session is never the writer's, so one `expand` freezes
/// every session's decision pointing at that archive id; a second expand is a no-op.
#[test]
fn expand_freezes_every_session_pointing_at_the_archive() {
    let dir = std::env::temp_dir().join(format!("rtok-t453-exp-{}", std::process::id()));
    let store = Store::open_in_memory().unwrap();
    let id = store.put_archive("a", b"body", &dir).unwrap();
    store.put_archive_decision("tu-1", &id, "a", "p").unwrap();
    store.put_archive_decision("tu-1", &id, "b", "p").unwrap();
    assert_eq!(store.mark_expanded(&id).unwrap(), 2);
    assert_eq!(store.mark_expanded(&id).unwrap(), 0, "already frozen");
    assert!(
        store
            .archive_decision("a", "tu-1")
            .unwrap()
            .unwrap()
            .expanded
    );
    assert!(
        store
            .archive_decision("b", "tu-1")
            .unwrap()
            .unwrap()
            .expanded
    );
    assert_eq!(store.archive_decision_counts().unwrap(), (2, 2));
    let _ = std::fs::remove_dir_all(dir);
}

/// T208: `mark_expanded_recorded` freezes the decision and inserts its `Measurement` in
/// one transaction. A `before_bytes` that overflows `i64` makes the measurement insert
/// fail before it issues any SQL — the already-applied freeze inside the same
/// transaction must roll back with it, so the decision stays unexpanded and no
/// `measurements` row is stranded.
#[test]
fn mark_and_record_are_atomic() {
    let dir = std::env::temp_dir().join(format!("rtok-t208-mark-{}", std::process::id()));
    let store = Store::open_in_memory().unwrap();
    let id = store.put_archive("a", b"body", &dir).unwrap();
    store.put_archive_decision("tu-1", &id, "a", "p").unwrap();
    let bad = Measurement {
        plugin: "archive",
        kind: "expand",
        before_bytes: u64::MAX,
        after_bytes: 4,
        est_before: 0,
        est_after: 1,
        ref_id: Some(id.clone()),
        call_id: None,
    };
    let err = store.mark_expanded_recorded("a", &id, &bad).unwrap_err();
    assert!(err.to_string().contains("before_bytes"), "{err}");
    assert!(
        !store
            .archive_decision("a", "tu-1")
            .unwrap()
            .unwrap()
            .expanded,
        "a rolled-back measurement insert must roll back the freeze too"
    );
    assert_eq!(store.measurement_count("archive").unwrap(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[rstest]
fn retention_keeps_plugin_archives_without_call_io() {
    let dir = std::env::temp_dir().join(format!("rtok-retain-plugin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("rtok.db");
    let archive = dir.join("archive");
    let store = Store::open(&db).unwrap();
    store
        .upsert_session("sess", Some(1), None, None, Some("mcp"))
        .unwrap();
    let body_decision = b"archive with decision";
    let arch_id = store.put_archive("sess", body_decision, &archive).unwrap();
    store
        .put_archive_decision("tu-1", &arch_id, "sess", "pointer")
        .unwrap();
    store.mark_expanded(&arch_id).unwrap();
    let body_read = b"read/cmd style archive";
    let read_arch_id = store.put_archive("sess", body_read, &archive).unwrap();

    assert_eq!(store.run_retention(1, 3).unwrap(), 0);
    assert_eq!(store.archive_decision_counts().unwrap(), (1, 1));
    assert_eq!(
        store.get_archive(&arch_id, Some(&archive)).unwrap(),
        Some(body_decision.to_vec())
    );
    assert_eq!(
        store.get_archive(&read_arch_id, Some(&archive)).unwrap(),
        Some(body_read.to_vec())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[rstest]
fn spill_archive_carries_session() {
    let dir = std::env::temp_dir().join(format!("rtok-arch-sess-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session("sess-a", Some(1), None, None, None)
        .unwrap();
    let call_id = store
        .insert_call(
            "sess-a",
            "proxy",
            "api_request",
            Some(1),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let body = vec![b'x'; 70 * 1024];
    store
        .insert_call_io(call_id, Some(&body), None, 64 * 1024, Some(&dir))
        .unwrap();
    let mut conn = store.lock().unwrap();
    let session: String = archive::table
        .select(archive::session)
        .first(&mut *conn)
        .unwrap();
    assert_eq!(session, "sess-a");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// T201: the hook path (`archive_dir = None`) never writes or expands a spilled body,
/// so `spill` must not pay a sha256 pass over it either — both sha columns land NULL,
/// same as `request_archive`/`response_archive`.
#[rstest]
fn spill_over_cap_without_archive_dir_skips_hashing() {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session("s", Some(1), None, None, None)
        .unwrap();
    let call_id = store
        .insert_call("s", "hook", "hook", Some(1), None, None, None, None)
        .unwrap();
    let big = vec![b'x'; 70 * 1024];
    store
        .insert_call_io(call_id, Some(&big), Some(&big), 64 * 1024, None)
        .unwrap();
    let mut conn = store.lock().unwrap();
    let row: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = call_io::table
        .filter(call_io::call_id.eq(call_id))
        .select((
            call_io::request_sha256,
            call_io::response_sha256,
            call_io::request_archive,
            call_io::response_archive,
        ))
        .first(&mut *conn)
        .unwrap();
    assert_eq!(
        row,
        (None, None, None, None),
        "over cap + no archive_dir must skip hashing, not just archiving"
    );
}

#[test]
fn hex_sha256_matches_fips_180_2_abc_vector() {
    // sha2 0.11 no longer impls LowerHex on the digest; encode bytes ourselves.
    assert_eq!(
        hex_sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[rstest]
fn inline_sha256_matches_stored_text() {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session("s", Some(1), None, None, None)
        .unwrap();
    let call_id = store
        .insert_call("s", "mcp", "mcp_call", Some(1), None, None, None, None)
        .unwrap();
    store
        .insert_call_io(call_id, Some(b"plain"), None, 1 << 20, None)
        .unwrap();
    {
        let mut conn = store.lock().unwrap();
        let (request_json, request_sha256): (Option<String>, Option<String>) = call_io::table
            .filter(call_io::call_id.eq(call_id))
            .select((call_io::request_json, call_io::request_sha256))
            .first(&mut *conn)
            .unwrap();
        let text = request_json.unwrap();
        assert_eq!(text, "plain");
        assert_eq!(request_sha256.unwrap(), hex_sha256(text.as_bytes()));
    }
    assert_eq!(
        store.call_io_request(call_id).unwrap(),
        Some(b"plain".to_vec())
    );

    // T211: invalid UTF-8 must still hash and round-trip as the exact wire bytes, not
    // the `from_utf8_lossy` text stored for display.
    let bad = [b'b', b'a', b'd', 0xff, 0xfe, b'o', b'k'];
    let call_id2 = store
        .insert_call("s", "mcp", "mcp_call", Some(1), None, None, None, None)
        .unwrap();
    store
        .insert_call_io(call_id2, Some(&bad), None, 1 << 20, None)
        .unwrap();
    {
        let mut conn = store.lock().unwrap();
        let (request_json2, request_sha256_2): (Option<String>, Option<String>) = call_io::table
            .filter(call_io::call_id.eq(call_id2))
            .select((call_io::request_json, call_io::request_sha256))
            .first(&mut *conn)
            .unwrap();
        let text2 = request_json2.unwrap();
        assert_eq!(text2, String::from_utf8_lossy(&bad));
        assert_eq!(request_sha256_2.unwrap(), hex_sha256(&bad));
    }
    assert_eq!(store.call_io_request(call_id2).unwrap(), Some(bad.to_vec()));
}

/// T428: a batch is one event, so a row that cannot be stored takes the others with it.
#[rstest]
fn insert_measurements_once_is_all_or_none() {
    let store = Store::open_in_memory().unwrap();
    let row = |est_before| Measurement {
        plugin: "memory",
        kind: "recall",
        before_bytes: 1,
        after_bytes: 1,
        est_before,
        est_after: 1,
        ref_id: None,
        call_id: None,
    };
    store
        .insert_measurements_once("s", &[row(1), row(2)], None)
        .unwrap();
    assert_eq!(store.measurement_count("memory").unwrap(), 2);
    store
        .insert_measurements_once("s", &[row(3), row(i32::MAX as u32 + 1)], None)
        .unwrap_err();
    assert_eq!(store.measurement_count("memory").unwrap(), 2);
}

/// T977: the ledger split by the project name of the writing session; `expand` stays negative.
#[rstest]
fn measurement_saved_by_project_splits_sessions_by_project() {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session("a1", None, Some("alpha"), None, None)
        .unwrap();
    store
        .upsert_session("a2", None, Some("alpha"), None, None)
        .unwrap();
    store
        .upsert_session("b1", None, Some("beta"), None, None)
        .unwrap();
    let row = |plugin, kind, est_before, est_after| Measurement {
        plugin,
        kind,
        before_bytes: 1,
        after_bytes: 1,
        est_before,
        est_after,
        ref_id: None,
        call_id: None,
    };
    store
        .insert_measurement("a1", &row("read", "map", 100, 40))
        .unwrap();
    store
        .insert_measurement("a2", &row("read", "map", 50, 10))
        .unwrap();
    store
        .insert_measurement("a2", &row("read", "expand", 0, 30))
        .unwrap();
    store
        .insert_measurement("b1", &row("read", "map", 70, 20))
        .unwrap();
    store
        .insert_measurement("loose", &row("read", "map", 9, 1))
        .unwrap();

    let got = store.measurement_saved_by_project(0).unwrap();
    let find = |project: Option<&str>, kind: &str| {
        got.iter()
            .find(|(p, t)| p.as_deref() == project && t.kind == kind)
            .map(|(_, t)| (t.rows, t.est_before - t.est_after))
    };
    assert_eq!(find(Some("alpha"), "map"), Some((2, 100)));
    assert_eq!(find(Some("alpha"), "expand"), Some((1, -30)));
    assert_eq!(find(Some("beta"), "map"), Some((1, 50)));
    assert_eq!(find(None, "map"), Some((1, 8)));
    assert!(
        store
            .measurement_saved_by_project(i64::MAX)
            .unwrap()
            .is_empty()
    );
}

#[rstest]
fn insert_measurement_rejects_out_of_range_estimates() {
    let store = Store::open_in_memory().unwrap();
    let m = Measurement {
        plugin: "cmd",
        kind: "rule",
        before_bytes: 1,
        after_bytes: 1,
        est_before: i32::MAX as u32 + 1,
        est_after: 1,
        ref_id: None,
        call_id: None,
    };
    let err = store.insert_measurement("s", &m).unwrap_err();
    assert!(
        err.to_string().contains("est_before"),
        "expected est_before error, got {err}"
    );
}

// T103: `memory_recall_totals` and `call_io_archives` had no direct test.

fn recall(store: &Store, session: &str, plugin: &'static str, kind: &'static str, b: u64, a: u64) {
    let m = Measurement {
        plugin,
        kind,
        before_bytes: b,
        after_bytes: a,
        est_before: 0,
        est_after: 0,
        ref_id: None,
        call_id: None,
    };
    store.insert_measurement(session, &m).unwrap();
}

#[test]
fn memory_recall_totals_sums_recalls_in_the_window_only() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(
        store.memory_recall_totals(0).unwrap(),
        (0, 0, 0),
        "empty store"
    );

    recall(&store, "s1", "memory", "recall", 1000, 100);
    assert_eq!(
        store.memory_recall_totals(0).unwrap(),
        (1, 1000, 100),
        "one row"
    );

    recall(&store, "s2", "memory", "recall", 500, 50);
    recall(&store, "s3", "memory", "recall", 250, 25);
    // Same plugin, other kind; other plugin, same kind — neither is a recall.
    recall(&store, "s3", "memory", "save", 9999, 9999);
    recall(&store, "s3", "read", "recall", 9999, 9999);
    assert_eq!(
        store.memory_recall_totals(0).unwrap(),
        (3, 1750, 175),
        "many sessions, recalls only"
    );

    // Age s1's row out of the window.
    let now = i64::try_from(unix_now()).unwrap_or(i64::MAX);
    {
        let mut conn = store.lock().unwrap();
        diesel::update(measurements::table.filter(measurements::session.eq("s1")))
            .set(measurements::ts.eq(now - 86_400))
            .execute(&mut *conn)
            .unwrap();
    }
    assert_eq!(
        store.memory_recall_totals(now - 3600).unwrap(),
        (2, 750, 75)
    );
    assert_eq!(store.memory_recall_totals(now + 3600).unwrap(), (0, 0, 0));
}

#[test]
fn call_io_archives_names_only_spilled_bodies() {
    let dir = std::env::temp_dir().join(format!("rtok-call-io-arch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session("sess", Some(1), None, None, None)
        .unwrap();
    let call = || {
        store
            .insert_call(
                "sess",
                "proxy",
                "api_request",
                Some(1),
                None,
                None,
                None,
                None,
            )
            .unwrap()
    };
    let cap = 16;
    let big = vec![b'x'; 64];

    assert_eq!(
        store.call_io_archives(9999).unwrap(),
        (None, None),
        "no call_io row"
    );

    let inline = call();
    store
        .insert_call_io(inline, Some(b"small"), Some(b"tiny"), cap, Some(&dir))
        .unwrap();
    assert_eq!(
        store.call_io_archives(inline).unwrap(),
        (None, None),
        "inline"
    );

    let req_only = call();
    store
        .insert_call_io(req_only, Some(&big), Some(b"ok"), cap, Some(&dir))
        .unwrap();
    let (req, res) = store.call_io_archives(req_only).unwrap();
    assert_eq!(req.as_deref(), Some(hex_sha256(&big).as_str()));
    assert!(
        dir.join(req.unwrap()).is_file(),
        "the id names the archived body"
    );
    assert_eq!(res, None);

    let both = call();
    let big_res = vec![b'y'; 64];
    store
        .insert_call_io(both, Some(&big), Some(&big_res), cap, Some(&dir))
        .unwrap();
    let (req, res) = store.call_io_archives(both).unwrap();
    assert!(req.is_some());
    assert_eq!(res.as_deref(), Some(hex_sha256(&big_res).as_str()));

    // Over cap without an archive dir (the hook path): metadata only, nothing to expand.
    let no_dir = call();
    store
        .insert_call_io(no_dir, Some(&big), Some(&big), cap, None)
        .unwrap();
    assert_eq!(store.call_io_archives(no_dir).unwrap(), (None, None));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every `migrations/<version>/up.sql` directory is embedded, in filename order.
#[test]
fn migrations_list_matches_the_directory() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut dirs: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().join("up.sql").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    dirs.sort();
    let dir_versions: Vec<String> = dirs
        .iter()
        .map(|d| d.split('_').next().unwrap().to_string())
        .collect();
    let listed = super::migrations::versions().unwrap();
    assert_eq!(
        listed, dir_versions,
        "embedded migrations drifted from migrations/"
    );
}

// T220: schema-drift guard — per-column type/NOT NULL/PK, the full table set, and (since
// `table!` models neither) defaults/indexes/triggers via a golden `sqlite_master` dump.
/// A migrated table with no `table!` macro, and why.
const RAW_SQL_TABLES: &[&str] = &[
    "notes_fts",         // 0001: FTS5 virtual table, MATCH/bm25 in sql_ext (T163.3)
    "notes_fts_data",    // FTS5 shadow table for notes_fts
    "notes_fts_idx",     // FTS5 shadow table for notes_fts
    "notes_fts_docsize", // FTS5 shadow table for notes_fts
    "notes_fts_config",  // FTS5 shadow table for notes_fts
    "observations_fts",  // 0036: FTS5 virtual table
    "observations_fts_data",
    "observations_fts_idx",
    "observations_fts_docsize",
    "observations_fts_config",
    "symbols_fts",           // 0038: FTS5 virtual table, MATCH/bm25 in sql_ext (T474)
    "symbols_fts_data",      // FTS5 shadow table for symbols_fts
    "symbols_fts_idx",       // FTS5 shadow table for symbols_fts
    "symbols_fts_docsize",   // FTS5 shadow table for symbols_fts
    "symbols_fts_config",    // FTS5 shadow table for symbols_fts
    "doc_items_fts",         // 0039: FTS5 virtual table for cached rustdoc (T455)
    "doc_items_fts_data",    // FTS5 shadow table for doc_items_fts
    "doc_items_fts_idx",     // FTS5 shadow table for doc_items_fts
    "doc_items_fts_docsize", // FTS5 shadow table for doc_items_fts
    "doc_items_fts_config",  // FTS5 shadow table for doc_items_fts
    "__diesel_schema_migrations", // diesel_migrations version table, not a migrations/*.sql file (T163.4)
];

/// One `diesel::table!`: its name, declared PK columns, and (SQL name, Diesel type) pairs.
struct SchemaTable {
    name: String,
    pk: Vec<String>,
    cols: Vec<(String, String)>,
}

/// Every `diesel::table!` in `schema.rs`, parsed from source.
fn schema_tables() -> Vec<SchemaTable> {
    let src = include_str!("schema.rs");
    let mut out = Vec::new();
    let mut lines = src.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line != "diesel::table! {" {
            continue;
        }
        let head = lines.next().unwrap();
        let name = head.split_whitespace().next().unwrap().to_string();
        // No PK column carries `#[sql_name]` today, so the head's names are SQL names too.
        let pk = head[head.find('(').unwrap() + 1..head.find(')').unwrap()]
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();
        let mut cols = Vec::new();
        let mut rename = None;
        for l in lines.by_ref() {
            if l == "}" {
                break;
            }
            if let Some(n) = l
                .strip_prefix("#[sql_name = \"")
                .and_then(|r| r.strip_suffix("\"]"))
            {
                rename = Some(n.to_string());
            } else if let Some((col, ty)) = l.split_once(" -> ") {
                let name = rename.take().unwrap_or_else(|| col.to_string());
                cols.push((name, ty.trim_end_matches(',').to_string()));
            }
        }
        out.push(SchemaTable { name, pk, cols });
    }
    out
}

/// SQLite's column-type-affinity rule, collapsed to the 3 affinities `schema.rs` uses:
/// `table_xinfo` returns the declared type verbatim (`BIGINT`, not `INTEGER`).
fn sqlite_affinity(declared: &str) -> &'static str {
    let d = declared.to_uppercase();
    if d.contains("INT") {
        "INTEGER"
    } else if d.contains("CHAR") || d.contains("CLOB") || d.contains("TEXT") {
        "TEXT"
    } else if d.contains("REAL") || d.contains("FLOA") || d.contains("DOUB") {
        "REAL"
    } else {
        "OTHER"
    }
}

/// The affinity a `table!` Diesel type expects — only the types `schema.rs` uses today.
fn diesel_affinity(ty: &str) -> Option<&'static str> {
    match ty {
        "Integer" | "BigInt" => Some("INTEGER"),
        "Text" => Some("TEXT"),
        "Double" => Some("REAL"),
        _ => None,
    }
}

/// `sqlite_master` normalized for a golden diff: tables/indexes/triggers, sorted, `sql`
/// collapsed to single-spaced so reindenting a migration is not itself drift.
fn live_schema_snapshot(conn: &mut SqliteConnection) -> String {
    let mut rows: Vec<(String, String, String, Option<String>)> =
        sql_ext::SqliteMasterSnapshot.load(conn).unwrap();
    rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    rows.into_iter()
        .map(|(kind, name, tbl_name, sql)| {
            let sql = sql.unwrap_or_default();
            let sql = sql.split_whitespace().collect::<Vec<_>>().join(" ");
            format!("{kind}|{name}|{tbl_name}|{sql}\n")
        })
        .collect()
}

/// Every mismatch between `schema.rs`/`schema_snapshot.txt` and a live, migrated
/// connection, one string each. Takes the connection so a test can run it on a broken DB.
fn schema_drift(conn: &mut SqliteConnection) -> Vec<String> {
    let mut out = Vec::new();
    let live_snapshot = live_schema_snapshot(conn);
    let live_tables: std::collections::BTreeSet<&str> = live_snapshot
        .lines()
        .filter_map(|l| l.strip_prefix("table|"))
        .map(|l| l.split('|').next().unwrap())
        .collect();
    let tables = schema_tables();
    assert!(tables.len() >= 16, "parsed {} table! macros", tables.len());
    let mut expected: std::collections::BTreeSet<&str> =
        tables.iter().map(|t| t.name.as_str()).collect();
    expected.extend(RAW_SQL_TABLES);
    if expected != live_tables {
        out.push(format!(
            "migrated tables {live_tables:?} vs table! \u{222a} RAW_SQL_TABLES {expected:?}"
        ));
    }

    for t in &tables {
        // `notnull` is a SQLite keyword; the pragma's own column of that name needs quoting.
        let live: Vec<(String, String, i32, i32)> = sql_ext::PragmaTableXinfo {
            table: t.name.clone(),
        }
        .load(conn)
        .unwrap();
        let live_names: std::collections::BTreeSet<&str> =
            live.iter().map(|c| c.0.as_str()).collect();
        let want_names: std::collections::BTreeSet<&str> =
            t.cols.iter().map(|c| c.0.as_str()).collect();
        if live_names != want_names {
            out.push(format!(
                "{}: schema.rs columns {want_names:?} vs live {live_names:?}",
                t.name
            ));
            continue;
        }
        for (name, ty) in &t.cols {
            let live = live.iter().find(|c| &c.0 == name).unwrap();
            let (base, nullable) = ty
                .strip_prefix("Nullable<")
                .map_or((ty.as_str(), false), |i| (i.trim_end_matches('>'), true));
            let want_pk = t.pk.iter().any(|p| p == name);
            let ty_ok = diesel_affinity(base).is_none_or(|w| sqlite_affinity(&live.1) == w);
            let pk_ok = want_pk == (live.3 > 0);
            // A bare SQLite `PRIMARY KEY` does not itself imply `NOT NULL` (unlike standard
            // SQL, and several migrations rely on it), so a PK column's live `notnull` is
            // never compared against `table!`'s always-non-`Nullable` Rust type.
            let notnull_ok = want_pk || nullable != (live.2 != 0);
            if !(ty_ok && pk_ok && notnull_ok) {
                out.push(format!(
                    "{}.{name}: schema.rs `{ty}` pk={want_pk} vs live `{}` notnull={} pk={}",
                    t.name, live.1, live.2, live.3
                ));
            }
        }
    }

    let want_snapshot = include_str!("schema_snapshot.txt");
    if live_snapshot != want_snapshot {
        out.push(format!("sqlite_master drifted from schema_snapshot.txt (defaults, indexes or triggers) — regenerate with `RTOK_BLESS=1 mise exec -- cargo test --lib schema_matches_the_migrated_tables_and_snapshot`\n--- want\n{want_snapshot}--- live\n{live_snapshot}"));
    }
    out
}

/// After every migration, `schema.rs` and `schema_snapshot.txt` match a fresh DB exactly.
#[test]
fn schema_matches_the_migrated_tables_and_snapshot() {
    let store = Store::open_in_memory().unwrap();
    let mut conn = store.lock().unwrap();
    if std::env::var_os("RTOK_BLESS").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/schema_snapshot.txt");
        std::fs::write(path, live_schema_snapshot(&mut conn)).unwrap();
    }
    let mismatches = schema_drift(&mut conn);
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// Runs each statement on a fresh migrated in-memory DB, then the guard.
fn drift_after(sql: &[&'static str]) -> Vec<String> {
    let store = Store::open_in_memory().unwrap();
    let mut conn = store.lock().unwrap();
    for s in sql {
        sql_ext::FixtureSql { sql: s }.execute(&mut *conn).unwrap();
    }
    schema_drift(&mut conn)
}

// A changed default and a dropped index are invisible to `table!`; only the snapshot
// catches them. A column dropped from a live table still fails, as it always has.
#[test]
fn schema_drift_catches_a_changed_default() {
    let m = drift_after(&[
        "DROP TABLE otel_export",
        "CREATE TABLE otel_export (stream TEXT PRIMARY KEY, mark BIGINT NOT NULL DEFAULT 1)",
    ]);
    assert!(m.iter().any(|s| s.contains("schema_snapshot.txt")), "{m:?}");
}

#[test]
fn schema_drift_catches_a_dropped_index() {
    let m = drift_after(&["DROP INDEX usage_call"]); // 0013
    assert!(m.iter().any(|s| s.contains("schema_snapshot.txt")), "{m:?}");
}

#[test]
fn schema_drift_catches_a_column_removed_from_the_live_table() {
    let m = drift_after(&[
        "DROP TABLE otel_export",
        "CREATE TABLE otel_export (stream TEXT PRIMARY KEY)",
    ]);
    assert!(m.iter().any(|s| s.starts_with("otel_export:")), "{m:?}");
}

/// T324: the archive file is content-addressed and read lock-free by `expand`, so a write
/// must never expose a truncated file: it goes to a temp file and is renamed over the
/// target (a new inode), which a hard link to the old file proves.
#[test]
fn archive_file_write_replaces_the_target_atomically() {
    let dir = super::support::tmp_dir("t324-atomic");
    let sha = hex_sha256(b"complete body");
    std::fs::write(dir.join(&sha), b"trunc").unwrap();
    let reader = dir.join("reader-view");
    std::fs::hard_link(dir.join(&sha), &reader).unwrap();
    let (path, created) = write_archive_file(&dir, &sha, b"complete body").unwrap();
    assert!(!created);
    assert_eq!(std::fs::read(&path).unwrap(), b"complete body");
    assert_eq!(
        std::fs::read(&reader).unwrap(),
        b"trunc",
        "written in place, not renamed"
    );
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [sha, "reader-view".to_string()],
        "temp file left behind"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// T352: a hook call with an inline stdin body, `age_days` old.
pub(crate) fn hook_call_with_body(store: &Store, kind: &str, body: &[u8], age_days: i64) -> i32 {
    let id = store
        .insert_call("s", "hook", kind, None, None, None, None, None)
        .unwrap();
    store
        .insert_call_io(id, Some(body), Some(body), body.len() + 1, None)
        .unwrap();
    let ts = i64::try_from(unix_now()).unwrap() - age_days * 86_400;
    store.set_call_ts(id, ts).unwrap();
    id
}

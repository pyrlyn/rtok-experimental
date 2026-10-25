// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

use super::*;

const DAY: i64 = 86_400;

fn now() -> i64 {
    i64::try_from(unix_now()).unwrap()
}

/// A session with one call `age_days` old.
fn with_call(store: &Store, id: &str, age_days: i64) -> i32 {
    store
        .upsert_session(id, Some(1), None, None, Some("proxy"))
        .unwrap();
    let call = store
        .insert_call(id, "proxy", "api_request", Some(1), None, None, None, None)
        .unwrap();
    store.set_call_ts(call, now() - age_days * DAY).unwrap();
    call
}

/// A body of `n` bytes archived by `session`, stamped `age_days` ago.
fn archived(store: &Store, dir: &Path, session: &str, tag: u8, n: usize, age_days: i64) -> String {
    let id = store
        .put_archive(session, &vec![tag; n], &dir.join("archive"))
        .unwrap();
    let mut conn = store.lock().unwrap();
    diesel::update(archive::table.filter(archive::id.eq(&id)))
        .set(archive::ts.eq(now() - age_days * DAY))
        .execute(&mut *conn)
        .unwrap();
    id
}

fn has_row(store: &Store, id: &str) -> bool {
    let mut conn = store.lock().unwrap();
    archive::table
        .filter(archive::id.eq(id))
        .count()
        .get_result::<i64>(&mut *conn)
        .unwrap()
        > 0
}

fn policy() -> PrunePolicy {
    PrunePolicy {
        retain_calls_days: 30,
        retain_archive_days: 90,
        ..PrunePolicy::default()
    }
}

/// The card's Check: the age limit takes an idle session's old body and its file, and never a
/// body that a session with a recent call can still `expand` — its own, a resumed one's, or one
/// that its decisions name although another session wrote it.
#[test]
fn the_age_limit_spares_what_an_open_session_can_expand() {
    let dir = super::support::tmp_dir("t968-age");
    let db = dir.join("rtok.db");
    let store = Store::open(&db).unwrap();
    with_call(&store, "idle", 120);
    with_call(&store, "live", 0);
    with_call(&store, "resumed", 0);
    store.end_session("resumed", now() - 5 * DAY).unwrap();
    let gone = archived(&store, &dir, "idle", 1, 100, 120);
    let own = archived(&store, &dir, "live", 2, 100, 120);
    let resumed = archived(&store, &dir, "resumed", 3, 100, 120);
    let shared = archived(&store, &dir, "idle", 4, 100, 120);
    let recent = archived(&store, &dir, "idle", 5, 100, 10);
    store
        .put_archive_decision("toolu_1", &shared, "live", "[pointer]")
        .unwrap();

    let r = store.prune(&db, &policy(), true).unwrap();
    assert_eq!((r.archive_entries, r.archive_bytes), (1, 100));
    assert!(has_row(&store, &gone), "a dry run deletes nothing");

    let r = store.prune(&db, &policy(), false).unwrap();
    assert_eq!((r.archive_entries, r.archive_bytes), (1, 100));
    assert!(!has_row(&store, &gone));
    assert!(!dir.join("archive").join(&gone).exists());
    for id in [&own, &resumed, &shared, &recent] {
        assert!(has_row(&store, id), "{id}");
        assert!(dir.join("archive").join(id).is_file());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The size limit drops the oldest idle bodies until the archive fits, and a body that a
/// surviving `call_io` row names goes without breaking the foreign key.
#[test]
fn the_archive_size_limit_drops_the_oldest_idle_bodies_first() {
    let dir = super::support::tmp_dir("t968-size");
    let db = dir.join("rtok.db");
    let store = Store::open(&db).unwrap();
    let call = with_call(&store, "idle", 20);
    with_call(&store, "live", 0);
    let oldest = archived(&store, &dir, "idle", 1, 1000, 25);
    let middle = archived(&store, &dir, "idle", 2, 1000, 24);
    let newest = archived(&store, &dir, "idle", 3, 1000, 23);
    let live = archived(&store, &dir, "live", 4, 1000, 26);
    let spill = vec![b'x'; 2000];
    store
        .insert_call_io(call, Some(&spill), None, 1000, Some(&dir.join("archive")))
        .unwrap();
    let spill = hex_sha256(&spill);
    {
        let mut conn = store.lock().unwrap();
        diesel::update(archive::table.filter(archive::id.eq(&spill)))
            .set(archive::ts.eq(now() - 30 * DAY))
            .execute(&mut *conn)
            .unwrap();
    }
    // 6000 bytes in all; `live` is untouchable, so the cap of 3000 takes the two oldest
    // idle bodies: the call's spill (still named by its `call_io` row) and `oldest`.
    let cap = PrunePolicy {
        retain_calls_days: 30,
        max_archive_bytes: 3000,
        ..PrunePolicy::default()
    };
    let r = store.prune(&db, &cap, false).unwrap();
    assert_eq!(r.archive_entries, 2, "{r:?}");
    assert!(!has_row(&store, &spill), "named by call_io, deleted anyway");
    assert!(!has_row(&store, &oldest));
    assert!(has_row(&store, &middle));
    assert!(has_row(&store, &newest));
    assert!(has_row(&store, &live));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Over `max_store_mb` the call window shrinks toward its floor, the WAL ends up empty, and
/// calls newer than the floor stay.
#[test]
fn the_store_size_limit_shortens_call_retention_and_empties_the_wal() {
    let dir = super::support::tmp_dir("t968-store");
    let db = dir.join("rtok.db");
    let store = Store::open(&db).unwrap();
    let body = vec![b'y'; 150_000];
    let fill = |id: &str, age: i64, n: usize| {
        for _ in 0..n {
            let call = with_call(&store, id, age);
            store
                .insert_call_io(call, Some(&body), None, 1 << 20, None)
                .unwrap();
        }
    };
    fill("old", 20, 4);
    fill("recent", 2, 2);
    let before = store_file_bytes(&db);
    assert!(before > 900_000, "{before}");

    let fits = PrunePolicy {
        retain_calls_days: 30,
        max_store_bytes: before * 2,
        ..PrunePolicy::default()
    };
    assert_eq!(store.prune(&db, &fits, false).unwrap().calls, 0);

    let tight = PrunePolicy {
        max_store_bytes: 1,
        ..fits
    };
    let r = store.prune(&db, &tight, false).unwrap();
    assert_eq!(
        r.calls, 4,
        "the 20-day-old calls go once the window is 15 days"
    );
    assert_eq!(store.count_calls().unwrap(), 2, "the floor is 7 days");
    assert!(r.store_after < r.store_before, "{r:?}");
    let wal = std::fs::metadata(dir.join("rtok.db-wal")).map_or(0, |m| m.len());
    assert_eq!(wal, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

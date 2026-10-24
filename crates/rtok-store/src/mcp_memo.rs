// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Store methods: the persistent MCP tool-result memo (`mcp_memo`).

use super::*;
use crate::schema::mcp_memo;
use sha2::{Digest, Sha256};

/// One memo row to write.
pub struct MemoRow<'a> {
    pub key: &'a str,
    pub tool: &'a str,
    pub value: &'a str,
    pub schema_hash: &'a str,
    pub rtok_version: &'a str,
}

/// Hex sha256 of a canonical memo key (the indexed lookup column).
pub fn memo_key_hash(key: &str) -> String {
    Sha256::digest(key.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn now_secs() -> i64 {
    i64::try_from(unix_now()).unwrap_or(i64::MAX)
}

impl Store {
    /// The value memoized for `key`, if it is fresh, was written by `rtok_version` for a tool
    /// whose schema hashes to `schema_hash`, and its stored key equals `key` byte for byte. A
    /// stale or mismatched row is deleted.
    pub fn memo_get(
        &self,
        key: &str,
        schema_hash: &str,
        rtok_version: &str,
        ttl_secs: i64,
    ) -> Result<Option<String>> {
        let hash = memo_key_hash(key);
        let mut conn = self.lock()?;
        let row: Option<(String, String, i64, String, String)> = mcp_memo::table
            .find(&hash)
            .select((
                mcp_memo::key,
                mcp_memo::value,
                mcp_memo::created,
                mcp_memo::schema_hash,
                mcp_memo::rtok_version,
            ))
            .first(&mut *conn)
            .optional()?;
        let Some((stored, value, created, schema, version)) = row else {
            return Ok(None);
        };
        let now = now_secs();
        if stored != key {
            // A hash collision: never serve it, and leave the row to its own key.
            return Ok(None);
        }
        if schema != schema_hash || version != rtok_version || now - created >= ttl_secs {
            diesel::delete(mcp_memo::table.find(&hash)).execute(&mut *conn)?;
            return Ok(None);
        }
        diesel::update(mcp_memo::table.find(&hash))
            .set(mcp_memo::last_used.eq(now))
            .execute(&mut *conn)?;
        Ok(Some(value))
    }

    /// Write-through of one memo row, then evict the least recently used past `max_entries`.
    pub fn memo_put(&self, row: &MemoRow<'_>, max_entries: i64) -> Result<()> {
        let hash = memo_key_hash(row.key);
        let now = now_secs();
        let mut conn = self.lock()?;
        diesel::replace_into(mcp_memo::table)
            .values((
                mcp_memo::key_hash.eq(&hash),
                mcp_memo::key.eq(row.key),
                mcp_memo::tool.eq(row.tool),
                mcp_memo::value.eq(row.value),
                mcp_memo::created.eq(now),
                mcp_memo::last_used.eq(now),
                mcp_memo::schema_hash.eq(row.schema_hash),
                mcp_memo::rtok_version.eq(row.rtok_version),
            ))
            .execute(&mut *conn)?;
        let count: i64 = mcp_memo::table.count().get_result(&mut *conn)?;
        if count > max_entries {
            let old: Vec<String> = mcp_memo::table
                .select(mcp_memo::key_hash)
                .filter(mcp_memo::key_hash.ne(&hash))
                .order((mcp_memo::last_used.asc(), mcp_memo::created.asc()))
                .limit(count - max_entries)
                .load(&mut *conn)?;
            diesel::delete(mcp_memo::table.filter(mcp_memo::key_hash.eq_any(old)))
                .execute(&mut *conn)?;
        }
        Ok(())
    }

    /// Drop every memo row (a possibly mutating tool call ran).
    pub fn memo_clear(&self) -> Result<usize> {
        let mut conn = self.lock()?;
        Ok(diesel::delete(mcp_memo::table).execute(&mut *conn)?)
    }

    /// Startup invalidation: drop rows of another rtok version, or of a tool whose current
    /// schema hash differs (`schemas` is `(tool, hash)`; tools not listed are dropped too).
    pub fn memo_invalidate(&self, rtok_version: &str, schemas: &[(&str, &str)]) -> Result<usize> {
        let mut conn = self.lock()?;
        let rows: Vec<(String, String, String, String)> = mcp_memo::table
            .select((
                mcp_memo::key_hash,
                mcp_memo::tool,
                mcp_memo::schema_hash,
                mcp_memo::rtok_version,
            ))
            .load(&mut *conn)?;
        let stale: Vec<String> = rows
            .into_iter()
            .filter(|(_, tool, schema, version)| {
                version != rtok_version || !schemas.iter().any(|(t, h)| t == tool && h == schema)
            })
            .map(|(h, ..)| h)
            .collect();
        Ok(
            diesel::delete(mcp_memo::table.filter(mcp_memo::key_hash.eq_any(stale)))
                .execute(&mut *conn)?,
        )
    }

    /// `(entries, bytes of key + value)` of the memo, for metrics.
    pub fn memo_stats(&self) -> Result<(i64, i64)> {
        let mut conn = self.lock()?;
        let rows: Vec<(String, String)> = mcp_memo::table
            .select((mcp_memo::key, mcp_memo::value))
            .load(&mut *conn)?;
        let bytes = rows.iter().map(|(k, v)| (k.len() + v.len()) as i64).sum();
        Ok((rows.len() as i64, bytes))
    }

    /// Test hook: write a row under an explicit hash, to simulate a collision.
    #[doc(hidden)]
    pub fn memo_put_raw_for_test(&self, hash: &str, row: &MemoRow<'_>) -> Result<()> {
        let now = now_secs();
        let mut conn = self.lock()?;
        diesel::replace_into(mcp_memo::table)
            .values((
                mcp_memo::key_hash.eq(hash),
                mcp_memo::key.eq(row.key),
                mcp_memo::tool.eq(row.tool),
                mcp_memo::value.eq(row.value),
                mcp_memo::created.eq(now),
                mcp_memo::last_used.eq(now),
                mcp_memo::schema_hash.eq(row.schema_hash),
                mcp_memo::rtok_version.eq(row.rtok_version),
            ))
            .execute(&mut *conn)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(key: &'a str, value: &'a str) -> MemoRow<'a> {
        MemoRow {
            key,
            tool: "docs_get",
            value,
            schema_hash: "s1",
            rtok_version: "1.0.0",
        }
    }

    #[test]
    fn memo_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("rtok-memo-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rtok.db");
        {
            let store = Store::open(&path).unwrap();
            store.memo_put(&row("k", "v"), 10).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.memo_get("k", "s1", "1.0.0", 60).unwrap().as_deref(),
            Some("v")
        );
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn schema_version_or_ttl_mismatch_is_never_served() {
        let store = Store::open_in_memory().unwrap();
        store.memo_put(&row("k", "v"), 10).unwrap();
        assert_eq!(store.memo_get("k", "s2", "1.0.0", 60).unwrap(), None);
        store.memo_put(&row("k", "v"), 10).unwrap();
        assert_eq!(store.memo_get("k", "s1", "1.0.1", 60).unwrap(), None);
        store.memo_put(&row("k", "v"), 10).unwrap();
        assert_eq!(store.memo_get("k", "s1", "1.0.0", 0).unwrap(), None);
        // Each mismatch deleted the row.
        assert_eq!(store.memo_stats().unwrap().0, 0);
    }

    #[test]
    fn startup_invalidation_drops_changed_tools_and_versions() {
        let store = Store::open_in_memory().unwrap();
        store.memo_put(&row("a", "1"), 10).unwrap();
        let other = MemoRow {
            tool: "mem_get",
            ..row("b", "2")
        };
        store.memo_put(&other, 10).unwrap();
        let dropped = store.memo_invalidate("1.0.0", &[("docs_get", "s1"), ("mem_get", "s9")]);
        assert_eq!(dropped.unwrap(), 1);
        assert_eq!(
            store.memo_get("a", "s1", "1.0.0", 60).unwrap().as_deref(),
            Some("1")
        );
        assert_eq!(
            store
                .memo_invalidate("2.0.0", &[("docs_get", "s1")])
                .unwrap(),
            1
        );
        assert_eq!(store.memo_stats().unwrap().0, 0);
    }

    #[test]
    fn overflow_evicts_the_least_recently_used() {
        let store = Store::open_in_memory().unwrap();
        for k in ["a", "b", "c"] {
            store.memo_put(&row(k, k), 3).unwrap();
        }
        // All rows share one second, so the oldest by insertion order goes first.
        store.memo_put(&row("d", "d"), 3).unwrap();
        let (entries, bytes) = store.memo_stats().unwrap();
        assert_eq!(entries, 3);
        assert_eq!(bytes, 6);
        // The row just written is never the one evicted.
        assert_eq!(
            store.memo_get("d", "s1", "1.0.0", 60).unwrap().as_deref(),
            Some("d")
        );
    }

    #[test]
    fn a_hash_collision_never_serves_the_other_key() {
        let store = Store::open_in_memory().unwrap();
        let forged = memo_key_hash("mine");
        store
            .memo_put_raw_for_test(&forged, &row("theirs", "secret"))
            .unwrap();
        assert_eq!(store.memo_get("mine", "s1", "1.0.0", 60).unwrap(), None);
        // The colliding row itself is untouched.
        assert_eq!(store.memo_stats().unwrap().0, 1);
    }

    #[test]
    fn clear_drops_everything() {
        let store = Store::open_in_memory().unwrap();
        store.memo_put(&row("a", "1"), 10).unwrap();
        assert_eq!(store.memo_clear().unwrap(), 1);
        assert_eq!(store.memo_get("a", "s1", "1.0.0", 60).unwrap(), None);
    }
}

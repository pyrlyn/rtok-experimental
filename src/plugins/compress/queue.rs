// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.9.2: the background queue of the P28 compressor. The proxy hands over the archive ids
//! a request just rewrote and returns at once; each job runs on the tokio runtime and writes
//! its summary beside the archive row, so a request only ever *reads* what an earlier job left.
//! Absent when `[plugins.compress.llm] enabled` is off, so a disabled build spawns nothing.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::llm::{Auth, Compressor};
use crate::config::Config;
use crate::store::Store;

/// Ids remembered as already tried; cleared when full so a long-lived proxy stays bounded.
const SEEN_CAP: usize = 10_000;

pub struct Queue {
    compressor: Compressor,
    // Its own handle: a job outlives the request whose `Runtime` offered it.
    store: Store,
    seen: Mutex<HashSet<String>>,
    in_flight: AtomicUsize,
    max_in_flight: usize,
}

/// Frees the slot even when a job panics.
struct Slot(Arc<Queue>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Queue {
    /// `None` while the flag is off.
    pub fn new(cfg: &Config) -> anyhow::Result<Option<Arc<Self>>> {
        let Some(compressor) = Compressor::new(cfg)? else {
            return Ok(None);
        };
        let store = Store::open(&cfg.core.db_path)?;
        store.set_store_raw(cfg.core.store_raw);
        Ok(Some(Arc::new(Self {
            compressor,
            store,
            seen: Mutex::new(HashSet::new()),
            in_flight: AtomicUsize::new(0),
            max_in_flight: cfg.plugins.compress.llm.max_in_flight.max(1) as usize,
        })))
    }

    /// Queue `ids` and return without waiting. An id is tried once per process; ids past the
    /// in-flight cap are not taken, so the next request that offers them starts them.
    pub fn submit(self: &Arc<Self>, auth: Option<Auth>, ids: Vec<String>) {
        // No credential, or no runtime to run on (a sync caller): the extractive view stays.
        let (Some(auth), Ok(runtime)) = (auth, tokio::runtime::Handle::try_current()) else {
            return;
        };
        for id in ids {
            let slot = match self.admit(&id) {
                Admit::Full => break,
                Admit::Seen => continue,
                Admit::Poisoned => return,
                Admit::Run(slot) => slot,
            };
            let auth = auth.clone();
            runtime.spawn(async move {
                // `run` warns about every failure itself.
                slot.0.compressor.run(&slot.0.store, &auth, &id).await;
            });
        }
    }

    /// T655: the slot is taken before the id is marked seen, in one compare-and-swap, so
    /// concurrent submits cannot both pass the cap, and an id refused at the cap stays
    /// retryable. A refused id drops its slot, which hands the reservation back.
    fn admit(self: &Arc<Self>, id: &str) -> Admit {
        let reserved = self
            .in_flight
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.max_in_flight).then_some(n + 1)
            });
        if reserved.is_err() {
            return Admit::Full;
        }
        let slot = Slot(self.clone());
        let Ok(mut seen) = self.seen.lock() else {
            return Admit::Poisoned;
        };
        if seen.len() >= SEEN_CAP {
            seen.clear();
        }
        if !seen.insert(id.to_string()) {
            return Admit::Seen;
        }
        Admit::Run(slot)
    }
}

enum Admit {
    Full,
    Seen,
    Poisoned,
    Run(Slot),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    fn queue(tag: &str, cap: u32) -> (Arc<Queue>, std::path::PathBuf) {
        let (mut cfg, dir) = crate::testutil::config(tag);
        cfg.plugins.compress.llm.enabled = true;
        cfg.plugins.compress.llm.max_in_flight = cap;
        (Queue::new(&cfg).unwrap().expect("flag on"), dir)
    }

    fn in_flight(q: &Queue) -> usize {
        q.in_flight.load(Ordering::Acquire)
    }

    /// The race the card measured: every thread passes the cap check before any increments.
    /// With the reservation in one CAS, exactly one distinct id is admitted at cap 1.
    #[test]
    fn concurrent_distinct_ids_never_exceed_the_cap() {
        const THREADS: usize = 32;
        for round in 0..20 {
            let (q, dir) = queue(&format!("t655-race-{round}"), 1);
            let start = Barrier::new(THREADS);
            let hold = Barrier::new(THREADS);
            let admitted = std::thread::scope(|s| {
                let handles: Vec<_> = (0..THREADS)
                    .map(|i| {
                        let (q, start, hold) = (&q, &start, &hold);
                        s.spawn(move || {
                            start.wait();
                            let got = q.admit(&format!("id-{i}"));
                            // Slots stay held until every thread has tried.
                            hold.wait();
                            matches!(got, Admit::Run(_))
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|h| h.join().unwrap().then_some(()))
                    .count()
            });
            assert!(admitted <= 1, "round {round}: cap 1 admitted {admitted}");
            assert_eq!(in_flight(&q), 0, "every slot is released");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn an_id_refused_at_the_cap_is_retried_once_a_slot_frees() {
        let (q, dir) = queue("t655-retry", 1);
        let Admit::Run(first) = q.admit("a") else {
            panic!("empty queue admits");
        };
        assert!(matches!(q.admit("b"), Admit::Full));
        assert_eq!(in_flight(&q), 1);
        drop(first);
        assert!(
            matches!(q.admit("b"), Admit::Run(_)),
            "b was not marked seen"
        );
        assert_eq!(in_flight(&q), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_duplicate_hands_its_reservation_back() {
        let (q, dir) = queue("t655-dup", 2);
        let Admit::Run(_held) = q.admit("a") else {
            panic!("empty queue admits");
        };
        assert!(matches!(q.admit("a"), Admit::Seen));
        assert_eq!(in_flight(&q), 1, "only the running job holds a slot");
        let _ = std::fs::remove_dir_all(dir);
    }
}

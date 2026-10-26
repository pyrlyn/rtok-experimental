// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T385.12.2: one store with an agent turn, Flex and standard bulk turns and a Batch turn, for
//! the `stats` and `report` lane/tier tables.

use rtok::store::Store;

/// `(kind, reported tier, [input, cache_create, cache_read, output])` of each seeded call.
pub const LANE_TIER_CALLS: [(&str, Option<&str>, [i64; 4]); 4] = [
    ("api_request", Some("default"), [5, 5, 90, 7]),
    ("api_request:bulk", Some("flex"), [100, 0, 0, 9]),
    ("api_request:bulk", Some("default"), [10, 10, 80, 2]),
    ("api_request:batch", None, [50, 0, 0, 5]),
];

pub fn seed_lane_tiers(store: &Store) {
    store
        .upsert_session("s1", None, None, None, Some("proxy"))
        .unwrap();
    for (kind, tier, [input, cache_create, cache_read, output]) in LANE_TIER_CALLS {
        let call = store
            .insert_call("s1", "proxy", kind, None, None, None, None, Some("/v1/x"))
            .unwrap();
        if let Some(tier) = tier {
            store.set_call_service_tier(call, tier).unwrap();
        }
        store
            .insert_usage(
                "s1",
                Some("m"),
                "openai_chat",
                input,
                cache_create,
                cache_read,
                output,
                call,
            )
            .unwrap();
    }
}

// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T15.11: `rtok stats` renders the D23 model. This file pins the command's output against a
//! fixture store (two transcript sessions, `usage` rows on both APIs, one `Measurement`), so
//! the move of the query into `src/web/model.rs` cannot change a printed number — and neither
//! can the next one. The goldens were recorded from the pre-move binary on 2026-09-09 and are
//! byte-for-byte what it printed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    crate::it::common::bin_path("rtok")
}

use crate::it::common;

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtok-t1511-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn rtok(args: &[&str], home: &Path) -> String {
    let out = Command::new(bin())
        .args(args)
        .env("RTOK_HOME", home)
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "rtok {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const S1: &str = "\
{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"Bash\",\"input\":{\"command\":\"cd /tmp && git status\"}}],\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}
{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t1\",\"content\":\"hello world!!\"}]}}
{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"done\"}],\"usage\":{\"input_tokens\":20,\"cache_read_input_tokens\":80,\"output_tokens\":2}}}
";

const S2: &str = "\
{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"id\":\"r1\",\"name\":\"Read\",\"input\":{\"path\":\"x.rs\"}}],\"usage\":{\"input_tokens\":5,\"output_tokens\":1}}}
{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"r1\",\"content\":\"fn main() {}\"}]}}
{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}],\"usage\":{\"input_tokens\":7,\"cache_creation_input_tokens\":3,\"output_tokens\":1}}}
";

/// Two transcript sessions, a store with two `usage` rows (both APIs) and one `Measurement`.
fn seed(home: &Path) {
    let projects = home.join(".claude/projects/acme");
    fs::create_dir_all(&projects).unwrap();
    fs::write(projects.join("s1.jsonl"), S1).unwrap();
    fs::write(projects.join("s2.jsonl"), S2).unwrap();

    let cfg = rtok::config::Config::load_from(home).expect("config");
    let store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
    store
        .upsert_session("s1", None, None, None, Some("proxy"))
        .unwrap();
    let id1 = store
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
    let id2 = store
        .insert_call(
            "s1",
            "proxy",
            "api_request",
            None,
            None,
            None,
            None,
            Some("/v1/chat/completions"),
        )
        .unwrap();
    store
        .insert_usage("s1", Some("m"), "anthropic", 10, 1, 2, 3, id1)
        .unwrap();
    store
        .insert_usage("s1", Some("m"), "openai_chat", 20, 0, 5, 4, id2)
        .unwrap();
    store
        .insert_measurement(
            "s1",
            &rtok::Measurement {
                plugin: "cmd",
                kind: "filter",
                before_bytes: 100,
                after_bytes: 40,
                est_before: 25,
                est_after: 10,
                ref_id: None,
                call_id: None,
            },
        )
        .unwrap();
}

/// The table, byte for byte: `sessions` counts transcript files (two), the `api` rows come
/// from the store's `usage`, the tool rows from the transcripts.
#[test]
fn stats_table_is_unchanged_on_a_fixture_store() {
    let h = home("table");
    seed(&h);
    assert_eq!(
        rtok(&["stats"], &h),
        "\
sessions 2  compact 0  checkpoint 0  no_checkpoint 2  lines 6  malformed 0
usage input=42 cache_create=3 cache_read=80 output=5  hit=64.0%  median_context=100
api                         input cache_create cache_read output    hit
anthropic                      10            1          2      3  15.4%
openai_chat                    20            0          5      4  20.0%
archive replay (estimate) ctt 14 → 14  -0.0%  over 0 results
tool                       count        bytes     mean      p95      max   est_tokens          ctt
Bash                           1           13       13       13       13            4            8
Read                           1           12       12       12       12            3            6
bash                     filter      count        bytes     mean      p95      max   est_tokens          ctt
git                      formatter       1           13       13       13       13            4            8
mcp                        count        bytes     mean      p95      max   est_tokens          ctt
"
    );
    let _ = fs::remove_dir_all(&h);
}

#[test]
fn stats_json_is_unchanged_on_a_fixture_store() {
    let h = home("json");
    seed(&h);
    assert_eq!(
        rtok(&["stats", "--json"], &h),
        r#"{
  "sessions": 2,
  "no_checkpoint": 2,
  "lines": 6,
  "malformed": 0,
  "tools": {
    "Bash": {
      "count": 1,
      "total_bytes": 13,
      "mean": 13,
      "p95": 13,
      "max": 13,
      "est_tokens": 4,
      "ctt": 8
    },
    "Read": {
      "count": 1,
      "total_bytes": 12,
      "mean": 12,
      "p95": 12,
      "max": 12,
      "est_tokens": 3,
      "ctt": 6
    }
  },
  "bash_families": {
    "git": {
      "count": 1,
      "total_bytes": 13,
      "mean": 13,
      "p95": 13,
      "max": 13,
      "est_tokens": 4,
      "ctt": 8
    }
  },
  "bash_filter": {
    "git": "formatter"
  },
  "mcp_groups": {},
  "usage_input": 42,
  "usage_cache_create": 3,
  "usage_cache_read": 80,
  "usage_output": 5,
  "cache_hit_rate": 0.64,
  "median_final_context": 100,
  "ctt_total": 14,
  "ctt_archive": 14,
  "archive_candidates": 0,
  "api": {
    "anthropic": {
      "input": 10,
      "cache_create": 1,
      "cache_read": 2,
      "output": 3,
      "hit": 0.15384615384615385
    },
    "openai_chat": {
      "input": 20,
      "cache_create": 0,
      "cache_read": 5,
      "output": 4,
      "hit": 0.2
    }
  }
}"#
    );
    let _ = fs::remove_dir_all(&h);
}

#[test]
fn stats_plugin_json_is_unchanged_on_a_fixture_store() {
    let h = home("plugin");
    seed(&h);
    assert_eq!(
        rtok(&["stats", "--plugin", "cmd"], &h),
        r#"{
  "archive_hits": 0,
  "plugin": "cmd",
  "rows": [
    {
      "after": 40,
      "before": 100,
      "est_after": 10,
      "est_before": 25,
      "kind": "filter",
      "ref_id": null
    }
  ]
}"#
    );
    let _ = fs::remove_dir_all(&h);
}

/// T61.1: an `isMeta` record keyed by `sourceToolUseID` folds into a `skills`
/// section — one row per skill, `resident` = body bytes × the API requests at or
/// after the injection. The default goldens above stay byte-identical because the
/// section is absent when no session injected a skill body.
#[test]
fn stats_renders_injected_skill_bodies() {
    let h = home("skills");
    let projects = h.join(".claude/projects/acme");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("sk.jsonl"),
        concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"id\":\"k1\",\"name\":\"Skill\",\"input\":{\"skill\":\"pixel\",\"args\":\"\"}}],\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n",
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"k1\",\"content\":\"ideas.md\"}]}}\n",
            "{\"type\":\"user\",\"isMeta\":true,\"sourceToolUseID\":\"k1\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"line one\\nline two\\nline three\\n\"}]}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"working\"}],\"usage\":{\"input_tokens\":200,\"cache_read_input_tokens\":40,\"output_tokens\":2}}}\n",
        ),
    )
    .unwrap();
    let table = rtok(&["stats"], &h);
    assert!(table.contains("resident"), "the skills table: {table}");
    assert!(table.contains("pixel"), "one row per skill: {table}");
    assert!(
        table.contains("lines 4"),
        "the isMeta record is one of the counted lines: {table}"
    );
    let js = rtok(&["stats", "--json"], &h);
    for needle in [
        "\"skills\"",
        "\"pixel\"",
        "\"count\": 1",
        "\"bytes\": 29",
        "\"est_tokens\": 8",
        "\"resident\": 29",
    ] {
        assert!(js.contains(needle), "`{needle}` missing from: {js}");
    }
    let _ = fs::remove_dir_all(&h);
}

#[test]
fn stats_cache_table_is_unchanged_on_a_fixture_store() {
    let h = home("cache");
    seed(&h);
    assert_eq!(
        rtok(&["stats", "--cache"], &h),
        "\
session                                   turns   cache_read cache_create  busts
s1                                            2            7            1      0
"
    );
    let _ = fs::remove_dir_all(&h);
}

/// T227: the Stats page carries the same numbers `rtok stats --price` and `rtok stats
/// --cache` print — one transcript scan feeds the CLI and the page (D27), so the page
/// cannot drift from the fixture goldens above without this failing too.
#[test]
fn stats_page_matches_price_and_cache_on_the_fixture_store() {
    let h = home("page");
    seed(&h);
    let price = rtok(&["stats", "--price"], &h);
    let cache = rtok(&["stats", "--cache"], &h);

    // `load_from` pins the config *file* to `h`, but a default like `~/.claude/projects`
    // still expands against the real `$HOME` (T74's leak) — `config_file_in` moves every
    // snapshot probe under `h`, then the transcripts point at the fixture (T252).
    // T985: this module runs inside the lib's test build; use the production stats scan.
    crate::model::BACKGROUND_STATS_IN_TESTS.with(|on| on.set(true));
    let mut cfg = rtok::testutil::config_file_in(&h);
    cfg.stats.transcripts_dir = h.join(".claude/projects");
    cfg.stats.codex_dir = h.join(".codex/sessions");
    // T947: the scan runs off the tick, so the first snapshots say "reading transcripts…".
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let page = loop {
        let page = rtok::web::model::snapshot(&cfg)
            .stats
            .expect("the stats page has text");
        if page != "reading transcripts…\n" {
            break page;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the stats scan never landed"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };

    for line in price.lines() {
        assert!(
            page.contains(line),
            "page is missing a --price line: {line}\n---\n{page}"
        );
    }
    for line in cache.lines().filter(|l| !l.is_empty()) {
        assert!(
            page.contains(line),
            "page is missing a --cache line: {line}\n---\n{page}"
        );
    }
    let _ = fs::remove_dir_all(&h);
}

/// T385.6: one store, three lanes. The agent lane keeps its cache, the bulk lane never hits,
/// and the `internal` lane sits in between; `lane` rows carry each lane's own hit rate.
fn seed_lanes(home: &Path) {
    let cfg = rtok::config::Config::load_from(home).expect("config");
    let store = rtok::store::Store::open(&cfg.core.db_path).expect("store");
    store
        .upsert_session("s1", None, None, None, Some("proxy"))
        .unwrap();
    for (kind, legs) in [
        ("api_request", (5, 5, 90, 7)),
        ("api_request", (0, 0, 100, 1)),
        ("api_request:bulk", (100, 0, 0, 9)),
        ("api_request:internal", (10, 10, 80, 2)),
    ] {
        let call = store
            .insert_call(
                "s1",
                "proxy",
                kind,
                None,
                None,
                None,
                None,
                Some("/v1/messages"),
            )
            .unwrap();
        let (input, cache_create, cache_read, output) = legs;
        store
            .insert_usage(
                "s1",
                Some("m"),
                "anthropic",
                input,
                cache_create,
                cache_read,
                output,
                call,
            )
            .unwrap();
    }
}

#[test]
fn stats_shows_the_cache_hit_rate_per_lane() {
    let h = home("lanes");
    seed_lanes(&h);
    let out = rtok(&["stats"], &h);
    assert!(
        out.contains(
            "\
lane                        input cache_create cache_read output    hit
agent                           5            5        190      8  95.0%
bulk                          100            0          0      9   0.0%
internal                       10           10         80      2  80.0%
"
        ),
        "{out}"
    );
    let json: serde_json::Value = serde_json::from_str(&rtok(&["stats", "--json"], &h)).unwrap();
    assert_eq!(json["lanes"]["agent"]["hit"], 0.95);
    assert_eq!(json["lanes"]["bulk"]["hit"], 0.0);
    assert_eq!(json["lanes"]["internal"]["cache_read"], 80);
    // The `api` table is still the whole ledger, lanes folded together.
    assert_eq!(json["api"]["anthropic"]["cache_read"], 270);
    let _ = fs::remove_dir_all(&h);
}

/// A store that only ever saw agent turns prints no `lane` table (the goldens above).
#[test]
fn stats_has_no_lane_table_for_agent_only_traffic() {
    let h = home("lanes-agent-only");
    seed(&h);
    let json: serde_json::Value = serde_json::from_str(&rtok(&["stats", "--json"], &h)).unwrap();
    assert!(json.get("lanes").is_none());
    assert!(!rtok(&["stats"], &h).contains("lane "));
    let _ = fs::remove_dir_all(&h);
}

/// T385.12.2: the lanes split again by the tier the provider reported, a Batch call (no tier in
/// its response) printing `-`, and the table is a JSON field of its own.
#[test]
fn stats_breaks_the_lanes_down_by_service_tier() {
    let h = home("lane-tiers");
    let cfg = rtok::config::Config::load_from(&h).expect("config");
    common::lanes::seed_lane_tiers(&rtok::store::Store::open(&cfg.core.db_path).expect("store"));
    let out = rtok(&["stats"], &h);
    assert!(
        out.contains(
            "\
lane/tier                   input cache_create cache_read output    hit
agent/default                   5            5         90      7  90.0%
batch/-                        50            0          0      5   0.0%
bulk/default                   10           10         80      2  80.0%
bulk/flex                     100            0          0      9   0.0%
"
        ),
        "{out}"
    );
    let json: serde_json::Value = serde_json::from_str(&rtok(&["stats", "--json"], &h)).unwrap();
    assert_eq!(json["lane_tiers"]["bulk/flex"]["input"], 100);
    // The per-lane table folds the tiers back together.
    assert_eq!(json["lanes"]["bulk"]["input"], 110);
    let _ = fs::remove_dir_all(&h);
}

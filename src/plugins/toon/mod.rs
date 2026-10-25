// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `toon` — tabular JSON → TOON encoding (vendor bench: −42.6 % tokens). On by default;
//! a block is only rewritten when the encoding estimates fewer tokens than the original.
//!
//! Spec: the catalogue in `plan.md` §1 names the tools this replaces; none is a
//! dependency (D6) — the behaviour is re-implemented here. Proxy rewrite respects the
//! same live-zone / `archive.keep_turns` rule as `archive` (recent turns stay intact).

use serde_json::Value;

use rtok_plugin_sdk::{
    Class, Ctx, DashboardPage, Manifest, Measurement, Plugin, Surface, ToolResultRef, WireRequest,
};

pub struct ToonPlugin;

/// How a toon pointer starts; `archive` shares the `archive_decisions` table and uses it to
/// leave toon's blocks alone.
pub(crate) const PREFIX: &str = "[toon ";

impl Plugin for ToonPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "toon",
            surfaces: &[Surface::Proxy, Surface::Mcp],
            default_on: true,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "TOON",
            "Compact tabular JSON in old tool results; expand returns the original.",
            true,
        )
    }

    fn proxy_filter(&self, req: &mut WireRequest<'_>, cx: &Ctx) -> Vec<Measurement> {
        if !cx.plugin_config::<crate::config::Toon>("toon").enabled {
            return Vec::new();
        }
        rewrite(req.tool_results(), cx)
    }
}

fn rewrite(results: Vec<ToolResultRef<'_>>, cx: &Ctx) -> Vec<Measurement> {
    let min_rows = cx.plugin_config::<crate::config::Toon>("toon").min_rows as usize;
    // Same live-zone rule as `archive`: never touch the last `keep_turns` turns.
    crate::plugins::archive::outside_live_zone(results, cx)
        .filter_map(|r| rewrite_block(&r.id, r.content, cx, min_rows))
        .collect()
}

/// A decision this plugin did not write. `json_tree` is named on its own so a build
/// without that feature still compiles, and so its pointer is never replayed as TOON.
fn skips_foreign_pointer(pointer: &str) -> bool {
    #[cfg(feature = "json_tree")]
    if pointer.starts_with(crate::plugins::json_tree::PREFIX) {
        return true;
    }
    !pointer.starts_with(PREFIX)
}

fn rewrite_block(
    tool_use_id: &str,
    content: &mut Value,
    cx: &Ctx,
    min_rows: usize,
) -> Option<Measurement> {
    // The tool-result text (a string, or text blocks as MCP tools return it) — what
    // `expand` must recover; a bare array of rows is kept as its JSON. The table is parsed
    // from this text.
    let orig = crate::plugins::archive::block_text(content)
        .or_else(|| content.is_array().then(|| content.to_string()))?;

    match cx.archive_decision(tool_use_id) {
        Ok(Some(d)) if d.expanded => return None,
        // An `archive` or `json_tree` pointer under the same `tool_use_id` is that plugin's
        // block, and its saving was measured there; replaying it here added a second `toon` row.
        Ok(Some(d)) if skips_foreign_pointer(&d.pointer) => return None,
        Ok(Some(d)) => {
            let m = Measurement {
                plugin: "toon",
                kind: "encode",
                before_bytes: orig.len() as u64,
                after_bytes: d.pointer.len() as u64,
                est_before: cx.estimate(&orig, Class::Code),
                est_after: cx.estimate(&d.pointer, Class::Code),
                ref_id: Some(d.archive_id.clone()),
                call_id: None,
            };
            *content = Value::String(d.pointer);
            return Some(m);
        }
        Ok(None) => {}
        Err(e) => {
            cx.log("error", "plugin", "toon", &format!("decision: {e}"));
            return None;
        }
    }

    let table: Value = serde_json::from_str(&orig).ok()?;
    let keys = tabular_keys(&table, min_rows)?;
    let rows = table.as_array()?;
    let encoded = encode(rows, &keys);
    // The pointer prints the 8-character id `rtok expand` resolves (T914). A table that is
    // already compact can still come out larger than the JSON it replaces. Decide before
    // anything is written, so a skipped block leaves no archive decision behind.
    let est_before = cx.estimate(&orig, Class::Code);
    let est_pointer = cx.estimate(
        &format!("{PREFIX}{}]\n", "0".repeat(crate::names::MIN_PREFIX)),
        Class::Code,
    );
    if est_pointer + cx.estimate(&encoded, Class::Code) >= est_before {
        return None;
    }
    let archive_id = cx
        .put_archive(orig.as_bytes())
        .map_err(|e| cx.log("error", "plugin", "toon", &format!("put: {e}")))
        .ok()?;
    let replacement = format!(
        "{PREFIX}{}]\n{encoded}",
        crate::names::short_id(&archive_id)
    );
    cx.put_archive_decision(tool_use_id, &archive_id, &replacement)
        .map_err(|e| cx.log("error", "plugin", "toon", &format!("decision: {e}")))
        .ok()?;
    let m = Measurement {
        plugin: "toon",
        kind: "encode",
        before_bytes: orig.len() as u64,
        after_bytes: replacement.len() as u64,
        est_before,
        est_after: cx.estimate(&replacement, Class::Code),
        ref_id: Some(archive_id),
        call_id: None,
    };
    *content = Value::String(replacement);
    Some(m)
}

/// Only arrays of uniform scalar objects encode; shared with the `--ai` report
/// rendering (T22.4), which shapes the same tables for a model. `pub(crate)` so the
/// encoder has one home (D6: one code path per method).
pub(crate) fn tabular_keys(value: &Value, min_rows: usize) -> Option<Vec<String>> {
    let arr = value.as_array()?;
    if arr.len() < min_rows.max(1) {
        return None;
    }
    let first = arr[0].as_object()?;
    if first.len() < 3 {
        return None;
    }
    let keys: Vec<String> = first.keys().cloned().collect();
    // The header `{a,b,c}` has no quoting: a key with a delimiter would shift every column.
    if keys.iter().any(|k| needs_quotes(k) || k.contains(':')) {
        return None;
    }
    for item in arr {
        let obj = item.as_object()?;
        if obj.len() != keys.len() {
            return None;
        }
        for k in &keys {
            match obj.get(k)? {
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
                _ => return None,
            }
        }
    }
    Some(keys)
}

/// TOON block for uniform rows under `keys`, in the caller's column order.
/// `pub(crate)` with [`tabular_keys`]: the `--ai` rendering dogfoods this encoder
/// instead of growing a second table syntax.
pub(crate) fn encode(rows: &[Value], keys: &[String]) -> String {
    let mut out = format!("[{}]{{{}}}:", rows.len(), keys.join(","));
    for row in rows {
        out.push_str("\n  ");
        for (i, k) in keys.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&encode_cell(row.get(k).unwrap_or(&Value::Null)));
        }
    }
    out
}

fn encode_cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) if needs_quotes(s) => {
            format!(
                "\"{}\"",
                s.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t")
            )
        }
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// A bare cell must read back as the same string. Unquoted, `""` was the `null` cell, `"x"`
/// lost its own quotes, and `123` / `true` / `null` read as a number, bool or null — the
/// model saw a different type than the tool returned.
fn needs_quotes(s: &str) -> bool {
    s.is_empty()
        || s.contains([',', '{', '}', '"', '\n', '\r', '\t'])
        || s.starts_with(' ')
        || s.ends_with(' ')
        || matches!(
            serde_json::from_str::<Value>(s),
            Ok(Value::Number(_) | Value::Bool(_) | Value::Null)
        )
}

/// The round-trip oracle for `encode` (the plugin's lossless rule); no runtime path reads TOON
/// back, so it, `unescape_quoted_cell` and `decode_cell` are compiled for tests only.
#[cfg(test)]
fn decode(toon: &str) -> Option<Value> {
    let (header, body) = toon.split_once('\n')?;
    let s = header.trim().strip_suffix(':')?;
    let brace = s.find('{')?;
    if !s.starts_with('[') || !s[..brace].ends_with(']') || !s.ends_with('}') {
        return None;
    }
    let keys: Vec<String> = s[brace + 1..s.len() - 1]
        .split(',')
        .map(str::to_string)
        .collect();
    let declared: usize = s[1..s.find(']')?].parse().ok()?;
    let mut rows = Vec::new();
    for line in body.lines() {
        let cells: Vec<Value> = split_cells(line.trim_start())?
            .into_iter()
            .map(decode_cell)
            .collect();
        if cells.len() != keys.len() {
            return None;
        }
        let mut obj = serde_json::Map::new();
        for (k, v) in keys.iter().zip(cells) {
            obj.insert(k.clone(), v);
        }
        rows.push(Value::Object(obj));
    }
    (rows.len() == declared).then_some(Value::Array(rows))
}

/// Split a row on commas outside quoted cells, following `encode_cell`'s grammar: a quote
/// opens only at a cell start, `\` escapes the next char inside it, and the closing quote
/// must end the cell. Anything else is malformed.
#[cfg(test)]
fn split_cells(line: &str) -> Option<Vec<&str>> {
    let mut cells = Vec::new();
    let mut start = 0;
    let mut chars = line.char_indices().peekable();
    loop {
        if line[start..].starts_with('"') {
            chars.next_if(|&(i, _)| i == start);
            let mut closed = false;
            while let Some((_, c)) = chars.next() {
                match c {
                    '\\' => {
                        chars.next()?;
                    }
                    '"' => {
                        closed = true;
                        break;
                    }
                    _ => {}
                }
            }
            if !closed {
                return None;
            }
            if let Some(&(_, c)) = chars.peek()
                && c != ','
            {
                return None;
            }
        }
        match chars.find(|&(_, c)| c == ',') {
            Some((i, _)) => {
                cells.push(&line[start..i]);
                start = i + 1;
            }
            None => {
                cells.push(&line[start..]);
                return Some(cells);
            }
        }
    }
}

#[cfg(test)]
fn unescape_quoted_cell(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
fn decode_cell(s: &str) -> Value {
    if s.is_empty() {
        return Value::Null;
    }
    if let Some(inner) = s.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        return Value::String(unescape_quoted_cell(inner));
    }
    match serde_json::from_str(s) {
        Ok(v @ (Value::Number(_) | Value::Bool(_))) => v,
        _ => Value::String(s.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expand;
    use crate::proxy::anthropic::ANTHROPIC;
    use rtok_plugin_sdk::WireRequest;
    use serde_json::json;

    fn cx(name: &str, enabled: bool, min_rows: u32) -> crate::plugin::Runtime {
        let dir = std::env::temp_dir().join(format!("rtok-toon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cx = crate::plugin::Runtime::in_memory("s").unwrap();
        cx.config.core.archive_dir = dir;
        cx.config.plugins.toon.enabled = enabled;
        cx.config.plugins.toon.min_rows = min_rows;
        // Encoding unit tests use a single turn; live-zone coverage sets keep_turns itself.
        cx.config.plugins.archive.keep_turns = 0;
        cx
    }

    fn tool_req(table: Value) -> Value {
        let content = serde_json::to_string_pretty(&table).unwrap();
        json!({"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content": content}]}]})
    }

    /// Six user turns, each with the same tabular tool result. Turns are counted from
    /// the end (`turn` = users after this one), matching the Anthropic wire.
    fn six_turn_req(table: Value) -> Value {
        let content = serde_json::to_string_pretty(&table).unwrap();
        let mut messages = Vec::new();
        for i in 1..=6 {
            messages.push(json!({
                "role": "user",
                "content": [{"type":"tool_result","tool_use_id": format!("t{i}"), "content": content}]
            }));
            if i < 6 {
                messages.push(json!({"role":"assistant","content":"ok"}));
            }
        }
        json!({"messages": messages})
    }

    fn rows_3x4() -> Value {
        json!([
            {"a": 11, "b": "alpha-value-one", "c": 33, "d": "delta-value-one"},
            {"a": 44, "b": "beta-value-twoo", "c": 66, "d": "delta-value-two"},
            {"a": 77, "b": "gamma-value-thr", "c": 99, "d": "delta-value-tre"},
        ])
    }

    fn filter(body: &mut Value, cx: &Ctx) -> Vec<Measurement> {
        ToonPlugin.proxy_filter(&mut WireRequest::new(&ANTHROPIC, body), cx)
    }

    #[test]
    fn default_off_leaves_bytes_identical() {
        let cx = cx("off", false, 3);
        let mut body = tool_req(json!([
            {"a": 1, "b": 2, "c": 3},
            {"a": 4, "b": 5, "c": 6},
            {"a": 7, "b": 8, "c": 9},
            {"a": 10, "b": 11, "c": 12},
        ]));
        let original = body.clone();
        assert!(filter(&mut body, &Ctx::new(&cx)).is_empty());
        assert_eq!(body, original);
    }

    #[test]
    fn encodes_3x4_table_and_decode_recovers_keys() {
        let cx = cx("enc", true, 3);
        let mut body = tool_req(rows_3x4());
        let ms = filter(&mut body, &Ctx::new(&cx));
        assert_eq!(ms.len(), 1);
        assert!(ms[0].after_bytes < ms[0].before_bytes);
        let text = body["messages"][0]["content"][0]["content"]
            .as_str()
            .unwrap();
        let decoded = decode(text.split_once('\n').unwrap().1).unwrap();
        let keys: Vec<_> = decoded[0].as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["a", "b", "c", "d"]);
    }

    use rstest::rstest;

    #[rstest]
    #[case("\n", "beta\nline-two")]
    #[case("\r", "gamma\rvalue")]
    #[case("\t", "delta\tvalue")]
    #[case("empty", "")]
    #[case("quotes", "\"quoted\"")]
    #[case("number", "123")]
    #[case("bool", "true")]
    #[case("null", "null")]
    #[case("comma", "x,y")]
    #[case("only-comma", ",")]
    #[case("quote-comma", "\",\"")]
    #[case("backslash", "back\\slash")]
    #[case("trailing-backslash", "end\\")]
    #[case("escaped-quote", "a\\\"b,c")]
    #[case("brace", "{k,v}")]
    #[case("padded", " lead, and trail ")]
    #[case("control", "bell\u{7},tab\t,nl\n")]
    fn round_trip_values(#[case] _control: &str, #[case] cell: &str) {
        let table = json!([
            {"a": 11, "b": "alpha-value-one", "c": 33, "d": "delta-value-one"},
            {"a": 44, "b": cell, "c": 66, "d": "delta-value-two"},
            {"a": 77, "b": "gamma-value-thr", "c": 99, "d": "delta-value-tre"},
        ]);
        let rows = table.as_array().unwrap();
        let keys = tabular_keys(&table, 3).unwrap();
        let encoded = encode(rows, &keys);
        let body_lines = encoded.split_once('\n').unwrap().1.lines().count();
        assert_eq!(body_lines, rows.len(), "each row must be one physical line");
        assert_eq!(decode(&encoded).unwrap(), table);
    }

    /// T560: the accepted table from the audit probe, plus every awkward cell shape in one
    /// row, must read back verbatim with the declared row and column counts.
    #[test]
    fn generated_cells_round_trip_with_declared_counts() {
        let cells = [
            "", ",", "x,y", "\"", "\\", "\\\"", "a\\nb", "\n", "\r\n", "\t", "123", "-1.5e3",
            "true", "null", " x", "x ", "{}", "é,ü", "plain",
        ];
        let table = Value::Array(
            cells
                .iter()
                .flat_map(|l| {
                    cells
                        .iter()
                        .map(move |r| json!({"a": l, "b": r, "c": 1, "d": null}))
                })
                .collect(),
        );
        let rows = table.as_array().unwrap();
        let keys = tabular_keys(&table, 1).unwrap();
        let encoded = encode(rows, &keys);
        assert_eq!(decode(&encoded).unwrap(), table);
        let one = json!([{"a": "x,y", "b": 1, "c": true}]);
        let keys = tabular_keys(&one, 1).unwrap();
        assert_eq!(
            decode(&encode(one.as_array().unwrap(), &keys)).unwrap(),
            one
        );
    }

    #[rstest]
    #[case::unterminated_quote("[1]{a,b}:\n  \"x,1")]
    #[case::text_after_closing_quote("[1]{a,b}:\n  \"x\"y,1")]
    #[case::too_few_cells("[1]{a,b}:\n  \"x,y\"")]
    #[case::too_many_cells("[1]{a,b}:\n  x,y,z")]
    #[case::row_count_mismatch("[2]{a,b}:\n  x,y")]
    fn malformed_rows_are_rejected(#[case] toon: &str) {
        assert_eq!(decode(toon), None);
    }

    #[test]
    fn live_zone_turns_are_untouched() {
        let mut cx = cx("live", true, 3);
        cx.config.plugins.archive.keep_turns = 4;
        let table = rows_3x4();
        let mut body = six_turn_req(table.clone());
        let original = body.clone();
        let ms = filter(&mut body, &Ctx::new(&cx));
        // keep_turns=4 → only turns with turn>=4 rewrite (first two of six).
        assert_eq!(ms.len(), 2, "only older-than-live-zone results encode");
        let texts: Vec<_> = (0..6)
            .map(|i| {
                // message index: user, assistant, user, ... → user at 2*i
                body["messages"][2 * i]["content"][0]["content"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert!(
            texts[0].starts_with("[toon "),
            "turn1 should encode: {}",
            texts[0]
        );
        assert!(
            texts[1].starts_with("[toon "),
            "turn2 should encode: {}",
            texts[1]
        );
        let pretty = serde_json::to_string_pretty(&table).unwrap();
        for t in &texts[2..] {
            assert_eq!(t, &pretty, "live-zone turn must stay original");
        }
        // Prefix before first rewrite stays byte-stable vs a fresh filter of the original.
        let mut again = original.clone();
        filter(&mut again, &Ctx::new(&cx));
        assert_eq!(body, again);
    }

    /// The same `tool_use_id` archived by `archive` first must not come back as a `toon`
    /// row too (both plugins key `archive_decisions` by that id).
    #[test]
    fn an_archive_pointer_is_not_replayed_as_a_toon_saving() {
        use rtok_plugin_sdk::Archive;
        let cx = cx("archive-owned", true, 3);
        let table = rows_3x4();
        let id = cx
            .put_archive(serde_json::to_string_pretty(&table).unwrap().as_bytes())
            .unwrap();
        cx.store
            .put_archive_decision("t", &id, "s", "[archived x: 3 lines]")
            .unwrap();
        let mut body = tool_req(table);
        let original = body.clone();
        assert!(filter(&mut body, &Ctx::new(&cx)).is_empty());
        assert_eq!(body, original);
    }

    /// MCP tools return `content: [{type: text, text}]`; the table inside must encode, and
    /// `expand` returns the text, not the wire's block array.
    #[test]
    fn text_block_content_encodes() {
        let cx = cx("blocks", true, 3);
        let text = serde_json::to_string_pretty(&rows_3x4()).unwrap();
        let mut body = json!({"messages":[{"role":"user","content":[{"type":"tool_result",
            "tool_use_id":"t","content":[{"type":"text","text": text}]}]}]});
        let ms = filter(&mut body, &Ctx::new(&cx));
        assert_eq!(ms.len(), 1);
        let archived = cx
            .store
            .get_archive(
                ms[0].ref_id.as_ref().unwrap(),
                Some(&cx.config.core.archive_dir),
            )
            .unwrap()
            .unwrap();
        assert_eq!(archived, text.as_bytes());
    }

    /// Two compact rows are still larger as an 8-character pointer plus TOON than as JSON.
    /// Three rows of the same shape now save, so the size gate is this smaller table.
    #[test]
    fn a_table_that_would_grow_is_left_alone() {
        use rtok_plugin_sdk::Archive;
        let cx = cx("grow", true, 2);
        let table = json!([{"a":1,"b":2,"c":3},{"a":4,"b":5,"c":6}]);
        let mut body = json!({"messages":[{"role":"user","content":[{"type":"tool_result",
            "tool_use_id":"t","content": table.to_string()}]}]});
        let original = body.clone();
        assert!(filter(&mut body, &Ctx::new(&cx)).is_empty());
        assert_eq!(body, original);
        assert!(cx.archive_decision("t").unwrap().is_none());
    }

    #[test]
    fn keys_with_delimiters_do_not_encode() {
        let table = json!([
            {"a,b": 1, "c": 2, "d": 3},
            {"a,b": 4, "c": 5, "d": 6},
            {"a,b": 7, "c": 8, "d": 9},
        ]);
        assert!(tabular_keys(&table, 3).is_none());
    }

    #[test]
    fn archived_bytes_match_original_text() {
        let cx = cx("archive-bytes", true, 3);
        let table = rows_3x4();
        let original = serde_json::to_string_pretty(&table).unwrap();
        let mut body = tool_req(table);
        let ms = filter(&mut body, &Ctx::new(&cx));
        let archive_id = ms[0].ref_id.clone().unwrap();
        let archived = cx
            .store
            .get_archive(&archive_id, Some(&cx.config.core.archive_dir))
            .unwrap()
            .unwrap();
        assert_eq!(archived, original.as_bytes());
    }

    #[test]
    fn expand_freezes_id_and_records_toon_expand_row() {
        let cx = cx("expand", true, 3);
        let table = rows_3x4();
        let original = serde_json::to_string_pretty(&table).unwrap();
        let mut first = tool_req(table.clone());
        let ms = filter(&mut first, &Ctx::new(&cx));
        let archive_id = ms[0].ref_id.clone().unwrap();
        let encoded = first["messages"][0]["content"][0]["content"]
            .as_str()
            .unwrap()
            .to_string();
        let shown = crate::names::short_id(&archive_id);
        assert_eq!(archive_id.len(), 64);
        assert!(encoded.starts_with(&format!("[toon {shown}]")), "{encoded}");
        assert!(
            !encoded.contains(&archive_id),
            "the pointer prints the prefix, not the sha"
        );

        let mut second = tool_req(table.clone());
        filter(&mut second, &Ctx::new(&cx));
        assert_eq!(
            second["messages"][0]["content"][0]["content"]
                .as_str()
                .unwrap(),
            encoded,
            "decision must be deterministic"
        );

        expand::fetch(&cx, shown).unwrap();
        let expand_rows: Vec<_> = cx
            .store
            .list_measurements("toon")
            .unwrap()
            .into_iter()
            .filter(|m| m.kind == "expand")
            .collect();
        assert_eq!(expand_rows.len(), 1);
        assert_eq!(expand_rows[0].ref_id.as_deref(), Some(archive_id.as_str()));

        let mut third = tool_req(table);
        assert_eq!(filter(&mut third, &Ctx::new(&cx)).len(), 0);
        assert_eq!(
            third["messages"][0]["content"][0]["content"]
                .as_str()
                .unwrap(),
            original,
            "expanded id must not re-encode"
        );
    }
}

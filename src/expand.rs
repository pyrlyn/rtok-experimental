// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok expand <id>` (plan T3.5) and the shared fetch used by the MCP `expand` tool (T5.4).

use crate::config::Config;
use crate::names::MIN_PREFIX;
use crate::plugin::{Measurement, Runtime};
use crate::tokens::Class;
use anyhow::{Result, bail};
use regex::Regex;

/// How a json_tree pointer starts. The plugin re-exports this so expand does not
/// depend on it, and a build that left the feature off still recognises the prefix.
pub(crate) const JSON_TREE_PREFIX: &str = "[json-tree ";

/// A `[json-tree ` pointer belongs to that plugin even in a build that left the
/// feature off, so expand cost is not counted under `archive`.
fn json_tree_pointer(pointer: &str) -> bool {
    pointer.starts_with(JSON_TREE_PREFIX)
}

/// Read an archived payload. When the id is a live-zone pointer (T5.3) this freezes it:
/// the owning plugin sends the original from the next request on, and one `expand`
/// measurement records the cost — `rtok stats --plugin <id>` derives the expand rate.
pub fn fetch(cx: &Runtime, id: &str) -> Result<Option<Vec<u8>>> {
    fetch_range(cx, id, None)
}

/// [`fetch`] for a call that asked for `lines`. T534: the range is checked against the
/// payload's real line count before anything is frozen or charged, so a range past the end
/// fails without turning a decision permanent for an expansion that returned no bytes.
pub fn fetch_range(cx: &Runtime, id: &str, lines: Option<&str>) -> Result<Option<Vec<u8>>> {
    let full = resolve_id(cx, id)?;
    let id = full.as_str();
    let Some(bytes) = cx
        .store
        .get_archive(id, Some(&cx.config.core.archive_dir))?
    else {
        return Ok(None);
    };
    if let Some(spec) = lines {
        parse_range(spec, String::from_utf8_lossy(&bytes).lines().count())?;
    }
    // T208: freeze + measurement commit in one transaction (`Store::mark_expanded_recorded`)
    // — a crash or a failed insert used to leave the decision frozen with no ledger row,
    // permanently under-counting `report_expand.cost`.
    let n = bytes.len() as u64;
    let plugin = match cx.store.live_zone_pointer(id)? {
        Some(p) if p.starts_with("[toon ") => "toon",
        Some(p) if json_tree_pointer(&p) => "json_tree",
        Some(_) => "archive",
        None => "archive",
    };
    cx.store.mark_expanded_recorded(
        &cx.session,
        id,
        &Measurement {
            plugin,
            kind: "expand",
            before_bytes: 0,
            after_bytes: n,
            est_before: 0,
            est_after: cx.estimate(&String::from_utf8_lossy(&bytes), Class::Code),
            ref_id: Some(id.to_string()),
            call_id: cx.call_id,
        },
    )?;
    Ok(Some(bytes))
}

/// T661: a hex prefix of at least 8 chars (the trailer prints 8) names the one archive
/// that starts with it. A full id, or a prefix with no match, passes through unchanged so
/// the caller reports it as unknown; two matches is an error naming the ambiguity.
/// A restricted runtime instead resolves only its own grants, including full IDs, and
/// never reveals candidates from the global archive or falls back to a global lookup.
pub fn resolve_id(cx: &Runtime, id: &str) -> Result<String> {
    let scoped = cx.scoped_archive_ids(id);
    let restricted = scoped.is_some();
    if id.len() > 64 || id.len() < MIN_PREFIX || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        if restricted {
            bail!(unknown_id_message(id));
        }
        return Ok(id.to_string());
    }
    if id.len() == 64 && !restricted {
        return Ok(id.to_string());
    }
    let hits = match scoped {
        Some(ids) => ids,
        None => cx.store.archive_ids_with_prefix(id, 2)?,
    };
    match hits.as_slice() {
        [one] => Ok(one.clone()),
        [] if restricted => bail!(unknown_id_message(id)),
        [] => Ok(id.to_string()),
        _ if restricted => bail!("archive id prefix {id} is ambiguous; use a longer prefix"),
        _ => bail!("{}", ambiguous_message(id, &hits)),
    }
}

/// The ambiguity error names each candidate by a prefix four chars longer than the one
/// given, so the next call can pick one without a lookup.
fn ambiguous_message(id: &str, hits: &[String]) -> String {
    let longer: Vec<&str> = hits
        .iter()
        .map(|h| &h[..h.len().min(id.len() + 4)])
        .collect();
    format!(
        "archive id prefix {id} is ambiguous; use one of: {}",
        longer.join(", ")
    )
}

/// The error text for an id with no archive row. An id that cannot be one — `-`,
/// `/dev/stdin`, anything but the 64-char hex sha256 — gets a hint (T354): agents guess a
/// stdin form (`cmd | rtok expand -`) to reach raw output, and nothing reads stdin.
pub fn unknown_id_message(id: &str) -> String {
    if id.len() >= 8 && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        format!("unknown archive id: {id}")
    } else {
        format!(
            "unknown archive id: {id} (expand takes the hex id from an \"expand <id>\" trailer; it does not read stdin)"
        )
    }
}

/// 1-based inclusive line range over already-split lines.
pub fn slice_lines<T>(lines: Vec<T>, a: usize, b: usize) -> Vec<T> {
    lines
        .into_iter()
        .take(b)
        .skip(a.saturating_sub(1))
        .collect()
}

/// Optional `--lines` / `--grep` filtering shared with the MCP `expand` tool.
///
/// `grep` is a regex (a pattern that does not compile is matched literally) and every hit
/// prints as `N:line`, numbered by its position in the archived payload, so a following
/// `--lines a-b` can pull the context around a hit instead of the whole body — the
/// search-then-slice loop of recursive-llm (`research.md` §12, T66.1).
///
/// `context` (T67.2, `--context N`, per call like `--grep`): with `grep`, print each hit
/// with N lines on either side, windows that overlap or touch merged into one block and
/// blocks separated by `--`. Numbers stay absolute; without `grep` it is ignored, and 0
/// keeps the hits-only output.
pub fn filter_lines(
    text: &str,
    lines: Option<&str>,
    grep: Option<&str>,
    context: usize,
) -> Result<Vec<String>> {
    let mut out: Vec<(usize, &str)> = text.lines().enumerate().map(|(i, l)| (i + 1, l)).collect();
    if let Some(spec) = lines {
        let (a, b) = parse_range(spec, out.len())?;
        out = slice_lines(out, a, b);
    }
    let Some(g) = grep else {
        return Ok(out.into_iter().map(|(_, l)| l.to_string()).collect());
    };
    let re = Regex::new(g).or_else(|_| Regex::new(&regex::escape(g)))?;
    if context == 0 {
        return Ok(out
            .into_iter()
            .filter(|(_, l)| re.is_match(l))
            .map(|(n, l)| format!("{n}:{l}"))
            .collect());
    }
    // Inclusive `out`-index windows `[hit-N, hit+N]`, union of overlapping or adjacent.
    let mut windows: Vec<(usize, usize)> = Vec::new();
    for (i, (_, l)) in out.iter().enumerate() {
        if !re.is_match(l) {
            continue;
        }
        let lo = i.saturating_sub(context);
        let hi = i.saturating_add(context).min(out.len() - 1);
        match windows.last_mut() {
            Some(last) if lo <= last.1 + 1 => last.1 = last.1.max(hi),
            _ => windows.push((lo, hi)),
        }
    }
    let mut rendered: Vec<String> = Vec::new();
    for (w, (lo, hi)) in windows.iter().enumerate() {
        if w > 0 {
            rendered.push("--".to_string());
        }
        rendered.extend(out[*lo..=*hi].iter().map(|(n, l)| format!("{n}:{l}")));
    }
    Ok(rendered)
}

/// Head and tail of `text` around `marker`, at most `max` chars in total. Shared by the
/// `read` cap and the MCP `expand` cap; the caller decides what the marker names.
///
/// The marker is the floor: below its length only the marker comes back, because the id
/// in it is what makes the cut lossless (`config validate` rejects `max_chars` < 100).
pub(crate) fn cut(text: &str, marker: &str, max: usize) -> String {
    let body_budget = max.saturating_sub(marker.chars().count());
    let keep = body_budget / 2;
    // Byte offsets of the first and last `keep` chars; no `Vec<char>` copy of the whole text.
    let head_end = text.char_indices().nth(keep).map_or(text.len(), |(i, _)| i);
    let tail_start = match keep {
        0 => text.len(),
        k => text.char_indices().rev().nth(k - 1).map_or(0, |(i, _)| i),
    };
    format!("{}{marker}{}", &text[..head_end], &text[tail_start..])
}

fn cap_lines<T>(out: &mut Vec<T>, max_lines: u32) -> usize {
    if max_lines == 0 {
        return 0;
    }
    let max = max_lines as usize;
    if out.len() <= max {
        return 0;
    }
    let omitted = out.len() - max;
    out.truncate(max);
    omitted
}

/// Render filtered lines plus an optional `[expand] max_lines` trailer.
pub(crate) fn render_lines(
    text: &str,
    id: &str,
    lines: Option<&str>,
    grep: Option<&str>,
    context: usize,
    max_lines: u32,
) -> Result<String> {
    let mut out = filter_lines(text, lines, grep, context)?;
    let omitted = cap_lines(&mut out, max_lines);
    let mut rendered = out.join("\n");
    if omitted > 0 {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        // A full sha in the caller's id prints as 8 hex chars. A longer prefix they
        // already sent (an ambiguous 8) stays, so the marker does not re-ambiguate it.
        let shown = if id.len() >= 64 {
            crate::names::short_id(id)
        } else {
            id
        };
        rendered.push_str(&format!("… {omitted} lines omitted (expand {shown})"));
    }
    Ok(rendered)
}

/// The bytes `rtok expand` writes. `--lines a-b` is 1-based inclusive; `--grep` is a regex whose
/// hits come back `N:`-numbered (see [`filter_lines`]); `--context N` widens those hits.
/// Raw archive bytes when no filter applies; otherwise the rendered text plus a trailing
/// newline. Empty when a filter matches nothing.
pub fn run(
    cfg: &Config,
    id: &str,
    lines: Option<&str>,
    grep: Option<&str>,
    context: usize,
) -> Result<Vec<u8>> {
    // Validate before fetch: fetching a live-zone pointer freezes it. A malformed
    // range must not mutate archive state even though no payload can be printed.
    if let Some(spec) = lines {
        parse_range(spec, usize::MAX)?;
    }
    let cx = Runtime::open(cfg.clone(), "expand")?;
    let Some(bytes) = fetch_range(&cx, id, lines)? else {
        bail!(unknown_id_message(id));
    };
    let max_lines = cfg.expand.max_lines;
    if lines.is_none() && grep.is_none() && max_lines == 0 {
        return Ok(bytes);
    }
    let text = String::from_utf8_lossy(&bytes);
    let rendered = render_lines(&text, id, lines, grep, context, max_lines)?;
    if rendered.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = rendered.into_bytes();
    out.push(b'\n');
    Ok(out)
}

/// Strips one matching pair of surrounding quotes (`"a-b"` or `'a-b'`). A model that
/// quotes a range it copied from prior output (T172) should not fail parsing over it.
fn strip_quotes(spec: &str) -> &str {
    let bytes = spec.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &spec[1..spec.len() - 1];
        }
    }
    spec
}

/// End index (exclusive) of an HTML break tag at `s[start..]`: `<br>`, `<br/>`, `<br />`.
fn br_tag_end(s: &str, start: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = start + 1;
    if i + 1 >= b.len()
        || !b[i].eq_ignore_ascii_case(&b'b')
        || !b[i + 1].eq_ignore_ascii_case(&b'r')
    {
        return None;
    }
    i += 2;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < b.len() && b[i] == b'/' {
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
    }
    if i < b.len() && b[i] == b'>' {
        Some(i + 1)
    } else {
        None
    }
}

fn strip_trailing_br(s: &str) -> Option<&str> {
    let t = s.trim_end();
    let start = t.rfind('<')?;
    let end = br_tag_end(t, start)?;
    (end == t.len()).then(|| &t[..start])
}

fn is_range_sep(c: char) -> bool {
    matches!(c, '-' | ',' | ':' | '_') || c.is_whitespace()
}

/// Fold the range spellings agents actually send (T522) into the `a[-b]` form:
/// surrounding `[…]`, spaces around `-`, `:`/`_`/`,` as the pair separator, a quoted
/// separator (`235"-"270`), and HTML `<br>` breaks. `lone` marks a trailing `<br>`
/// with no second number — that is one line (`52<br>`), not `52..end`.
fn normalize_range(raw: &str) -> (String, bool) {
    let mut s = raw;
    if s.len() >= 2 && s.starts_with('[') && s.ends_with(']') {
        s = s[1..s.len() - 1].trim();
    }
    let mut lone = false;
    while let Some(rest) = strip_trailing_br(s) {
        lone = true;
        s = rest.trim_end();
    }
    let mut out = String::with_capacity(s.len());
    let mut pending_sep = false;
    let mut idx = 0;
    while idx < s.len() {
        if s.as_bytes()[idx] == b'<'
            && let Some(end) = br_tag_end(s, idx)
        {
            idx = end;
            pending_sep = true;
            continue;
        }
        let c = s[idx..].chars().next().expect("idx < len");
        let next = s[idx + c.len_utf8()..].chars().next();
        // Interior quotes around a separator (`235"-"270`) are noise; a quote that
        // opens or closes the value stays, so a half-quoted spec still fails.
        if matches!(c, '"' | '\'') && idx > 0 && next.is_some() {
            let prev_sep = pending_sep || out.chars().next_back().is_some_and(is_range_sep);
            if prev_sep || next.is_some_and(is_range_sep) {
                idx += c.len_utf8();
                continue;
            }
        }
        if is_range_sep(c) {
            pending_sep = true;
            idx += c.len_utf8();
            continue;
        }
        if pending_sep {
            out.push('-');
            pending_sep = false;
        }
        out.push(c);
        idx += c.len_utf8();
    }
    if pending_sep {
        out.push('-');
    }
    (out, lone)
}

pub(crate) fn parse_range(spec: &str, n: usize) -> Result<(usize, usize)> {
    let spec = strip_quotes(spec.trim()).trim();
    if spec.is_empty() {
        bail!("invalid line range `{spec}`: expected a positive line or a-b");
    }
    // The error text keeps the caller's spelling (after the T172 quote strip).
    let (norm, lone) = normalize_range(spec);
    if !norm.bytes().any(|b| b.is_ascii_digit()) {
        bail!("invalid line range `{spec}`: expected a positive line or a-b");
    }
    let mut parts = norm.splitn(2, '-');
    let start = parts.next().unwrap_or_default();
    let end = parts.next();
    let a = if start.is_empty() {
        1
    } else {
        start.parse::<usize>().map_err(|_| {
            anyhow::anyhow!("invalid line range `{spec}`: expected a positive line or a-b")
        })?
    };
    let requested_b = match end {
        Some("") | None => None,
        Some(s) => Some(s.parse::<usize>().map_err(|_| {
            anyhow::anyhow!("invalid line range `{spec}`: expected a positive line or a-b")
        })?),
    };
    if a == 0 || requested_b == Some(0) {
        bail!("invalid line range `{spec}`: lines are 1-based");
    }
    if requested_b.is_some_and(|b| a > b) {
        bail!("invalid line range `{spec}`: start exceeds end");
    }
    // A start past the last line would slice to nothing and print empty output
    // with exit 0; fail loudly instead so the caller knows the range is wrong.
    if a > n {
        bail!("invalid line range `{spec}`: start exceeds line count {n}");
    }
    if lone && requested_b.is_none() {
        return Ok((a, a));
    }
    Ok((a, requested_b.unwrap_or(n).min(n)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::short_id;
    use rstest::rstest;

    fn cfg(name: &str) -> Config {
        crate::testutil::config(name).0
    }

    #[test]
    fn short_id_is_the_resolvable_minimum() {
        let id = "ab".repeat(32);
        assert_eq!(short_id(&id), "abababab");
        assert_eq!(short_id("abc"), "abc");
        assert!(short_id(&id).len() >= MIN_PREFIX);
    }

    #[test]
    fn ambiguous_prefix_lists_the_longer_candidates() {
        let pad = "0".repeat(52);
        let msg = ambiguous_message(
            "abababab",
            &[format!("abababab1111{pad}"), format!("abababab2222{pad}")],
        );
        assert!(msg.contains("abababab1111"), "{msg}");
        assert!(msg.contains("abababab2222"), "{msg}");
    }

    /// The MCP `expand` tool takes `context` straight from the model's JSON (`u64`), so a
    /// huge value must clamp to the whole payload instead of overflowing `hit + context`
    /// (a panic in debug builds, a wrapped window that dropped the hit itself in release).
    #[test]
    fn huge_grep_context_clamps_to_the_whole_payload() {
        let text = "a\nb\nHIT\nc";
        let out = filter_lines(text, None, Some("HIT"), usize::MAX).unwrap();
        assert_eq!(out, ["1:a", "2:b", "3:HIT", "4:c"]);
    }

    #[test]
    fn malformed_range_is_rejected_before_archive_lookup() {
        let c = cfg("bad-range");
        let err = run(&c, "no-such", Some("3-2"), None, 0).unwrap_err();
        assert!(err.to_string().contains("invalid line range"), "{err}");
    }

    #[test]
    fn unknown_id_is_err() {
        let c = cfg("unknown");
        let err = run(&c, "no-such", None, None, 0).unwrap_err();
        assert!(err.to_string().contains("unknown archive id"), "{err}");
    }

    #[test]
    fn non_id_gets_the_trailer_hint_and_a_hex_id_does_not() {
        let hint = unknown_id_message("-");
        assert!(hint.starts_with("unknown archive id: - ("), "{hint}");
        assert!(hint.contains("\"expand <id>\" trailer"), "{hint}");
        assert!(hint.contains("does not read stdin"), "{hint}");
        assert!(!hint.contains('\n'), "{hint}");
        assert!(unknown_id_message("/dev/stdin").contains("trailer"));
        let hex = "ab".repeat(32);
        assert_eq!(
            unknown_id_message(&hex),
            format!("unknown archive id: {hex}")
        );
        let c = cfg("hint");
        let err = run(&c, "-", None, None, 0).unwrap_err();
        assert!(err.to_string().contains("does not read stdin"), "{err}");
        let err = run(&c, &hex, None, None, 0).unwrap_err();
        assert_eq!(err.to_string(), format!("unknown archive id: {hex}"));
    }

    #[test]
    fn round_trip_from_put_archive() {
        let c = cfg("round");
        let cx = crate::plugin::Runtime::open(c.clone(), "expand").unwrap();
        let id = cx
            .store
            .put_archive("expand", b"hello\nworld\n", &c.core.archive_dir)
            .unwrap();
        let got = cx
            .store
            .get_archive(&id, Some(&c.core.archive_dir))
            .unwrap()
            .unwrap();
        assert_eq!(got, b"hello\nworld\n");
        drop(cx);
        run(&c, &id, None, None, 0).unwrap();
    }

    #[test]
    fn moved_archive_dir_still_reads() {
        let c = cfg("moved");
        let cx = crate::plugin::Runtime::open(c.clone(), "expand").unwrap();
        let id = cx
            .store
            .put_archive("expand", b"relocated\n", &c.core.archive_dir)
            .unwrap();
        drop(cx);
        let dest = c.core.archive_dir.parent().unwrap().join("archive-moved");
        std::fs::rename(&c.core.archive_dir, &dest).unwrap();
        let mut c2 = c.clone();
        c2.core.archive_dir = dest;
        let cx = crate::plugin::Runtime::open(c2.clone(), "expand").unwrap();
        assert_eq!(fetch(&cx, &id).unwrap().unwrap(), b"relocated\n");
    }

    #[test]
    fn missing_file_is_unknown_id() {
        let c = cfg("gone");
        let cx = crate::plugin::Runtime::open(c.clone(), "expand").unwrap();
        let id = cx
            .store
            .put_archive("expand", b"bye\n", &c.core.archive_dir)
            .unwrap();
        std::fs::remove_file(c.core.archive_dir.join(&id)).unwrap();
        drop(cx);
        let err = run(&c, &id, None, None, 0).unwrap_err();
        assert!(err.to_string().contains("unknown archive id"), "{err}");
    }

    /// T55.11: `rtok expand` runs under session "expand", not the proxy session that
    /// owns the decision — the freeze must still reach the owning session's pointer.
    #[test]
    fn cli_expand_freezes_the_owning_sessions_pointer() {
        let c = cfg("freeze-owner");
        let cx_proxy = crate::plugin::Runtime::open(c.clone(), "proxy-sess").unwrap();
        let id = cx_proxy
            .store
            .put_archive("proxy-sess", b"payload\n", &c.core.archive_dir)
            .unwrap();
        cx_proxy
            .store
            .put_archive_decision("tu-1", &id, "proxy-sess", &format!("[archived {id}]"))
            .unwrap();
        drop(cx_proxy);
        let cx_cli = crate::plugin::Runtime::open(c, "expand").unwrap();
        assert_eq!(fetch(&cx_cli, &id).unwrap().unwrap(), b"payload\n");
        assert!(
            cx_cli
                .store
                .archive_decision("proxy-sess", "tu-1")
                .unwrap()
                .unwrap()
                .expanded,
            "the writer's session must see the freeze"
        );
    }

    /// T534: seeds one live-zone decision on a one-line archive; returns the id and the config.
    fn seeded(name: &str, body: &[u8]) -> (Config, String) {
        let c = cfg(name);
        let cx = crate::plugin::Runtime::open(c.clone(), "proxy-sess").unwrap();
        let id = cx
            .store
            .put_archive("proxy-sess", body, &c.core.archive_dir)
            .unwrap();
        cx.store
            .put_archive_decision("tu-1", &id, "proxy-sess", &format!("[archived {id}]"))
            .unwrap();
        (c, id)
    }

    fn frozen_and_charged(c: &Config, id: &str) -> (bool, usize) {
        let cx = crate::plugin::Runtime::open(c.clone(), "probe").unwrap();
        let frozen = cx
            .store
            .archive_decision("proxy-sess", "tu-1")
            .unwrap()
            .unwrap()
            .expanded;
        let rows = cx.store.list_measurements("archive").unwrap();
        (
            frozen,
            rows.iter()
                .filter(|r| r.ref_id.as_deref() == Some(id))
                .count(),
        )
    }

    /// T534: a range past the end of the payload is an error that leaves the decision unfrozen
    /// and the ledger untouched, on the CLI and on the shared fetch the MCP tool uses; a valid
    /// range then freezes once and records once.
    #[rstest]
    #[case(b"only line\n".as_slice(), "2-3")]
    #[case(b"".as_slice(), "1")]
    fn a_range_past_the_end_neither_freezes_nor_charges(#[case] body: &[u8], #[case] spec: &str) {
        let (c, id) = seeded("range-past-end", body);
        let err = run(&c, &id, Some(spec), None, 0).unwrap_err();
        assert!(err.to_string().contains("exceeds line count"), "{err}");
        let cx = crate::plugin::Runtime::open(c.clone(), "mcp-1").unwrap();
        assert!(fetch_range(&cx, &id, Some(spec)).is_err());
        assert_eq!(frozen_and_charged(&c, &id), (false, 0));
        if !body.is_empty() {
            let out = run(&c, &id, Some("1"), None, 0).unwrap();
            assert_eq!(out, b"only line\n");
            assert_eq!(frozen_and_charged(&c, &id), (true, 1));
        }
    }

    /// T55.11: an expand of a toon pointer attributes its Measurement to `toon`;
    /// `live_zone_pointer` no longer needs the writer's session.
    #[test]
    fn expand_measurement_attributes_toon_pointers() {
        let c = cfg("toon-attr");
        let cx_proxy = crate::plugin::Runtime::open(c.clone(), "proxy-sess").unwrap();
        let id = cx_proxy
            .store
            .put_archive("proxy-sess", b"table\n", &c.core.archive_dir)
            .unwrap();
        cx_proxy
            .store
            .put_archive_decision("tu-1", &id, "proxy-sess", &format!("[toon {id}]\na,b"))
            .unwrap();
        drop(cx_proxy);
        let cx_cli = crate::plugin::Runtime::open(c, "expand").unwrap();
        assert!(fetch(&cx_cli, &id).unwrap().is_some());
        let rows = cx_cli.store.list_measurements("toon").unwrap();
        assert!(rows.iter().any(|r| r.kind == "expand"), "{rows:?}");
    }

    /// T66.1: a hit carries its line number so `--lines` can follow; the pattern is a
    /// regex, or the literal text when it does not compile.
    #[test]
    fn grep_is_regex_numbered_by_archive_line_and_falls_back_to_literal() {
        let text = "alpha\nerror[E0308]: mismatched\nbeta\nerror[E0599]: no method\n";
        assert_eq!(
            filter_lines(text, None, Some(r"error\[E0\d+\]"), 0).unwrap(),
            ["2:error[E0308]: mismatched", "4:error[E0599]: no method"]
        );
        // Numbers stay absolute inside a range.
        assert_eq!(
            filter_lines(text, Some("3-4"), Some("error"), 0).unwrap(),
            ["4:error[E0599]: no method"]
        );
        // An unclosed bracket is not a regex: match it literally, never error.
        assert_eq!(
            filter_lines(text, None, Some("[E0308"), 0).unwrap(),
            ["2:error[E0308]: mismatched"]
        );
        // Without grep the output is the bare lines, as before (context is ignored too).
        assert_eq!(
            filter_lines(text, Some("1-2"), None, 7).unwrap(),
            ["alpha", "error[E0308]: mismatched"]
        );
    }

    /// T67.2: `context N` widens each grep hit to `[hit-N, hit+N]` with absolute numbers;
    /// windows that overlap or touch merge into one block and blocks separate on `--`.
    #[test]
    fn context_windows_merge_and_separate_on_dash_dash() {
        let text = "a1\na2\nHIT\na4\na5\nb1\nHIT\nb3\nb4\nb5\n";
        // Hits at lines 3 and 7, context 2: windows [1,5] and [5,9] overlap → one block.
        assert_eq!(
            filter_lines(text, None, Some("HIT"), 2).unwrap(),
            [
                "1:a1", "2:a2", "3:HIT", "4:a4", "5:a5", "6:b1", "7:HIT", "8:b3", "9:b4",
            ]
        );
        // Hits at lines 2 and 9, context 1: a gap between the windows keeps both and the `--`.
        let spread = "h1\nHIT\nx3\nx4\nx5\nx6\nx7\nx8\nHIT\n";
        assert_eq!(
            filter_lines(spread, None, Some("HIT"), 1).unwrap(),
            ["1:h1", "2:HIT", "3:x3", "--", "8:x8", "9:HIT"]
        );
    }

    /// A window at either edge clamps to the file instead of under- or overflowing.
    #[test]
    fn context_windows_clamp_at_the_file_edges() {
        assert_eq!(
            filter_lines("HIT\nb\n", None, Some("HIT"), 3).unwrap(),
            ["1:HIT", "2:b"]
        );
        assert_eq!(
            filter_lines("a\nHIT", None, Some("HIT"), 3).unwrap(),
            ["1:a", "2:HIT"]
        );
    }

    /// With `--lines` the windows are computed inside the slice but the numbers stay
    /// absolute, matching how bare hits number today.
    #[test]
    fn context_inside_a_lines_range_keeps_absolute_numbers() {
        let text = "l1\nl2\nhit\nl4\nl5\nl6\nhit\nl8\n";
        assert_eq!(
            filter_lines(text, Some("2-8"), Some("hit"), 1).unwrap(),
            ["2:l2", "3:hit", "4:l4", "--", "6:l6", "7:hit", "8:l8"]
        );
    }

    /// `context 0` (the default) is byte-identical to the old hits-only output, and the
    /// joined output still answers to `[expand] max_lines` with its trailer.
    #[test]
    fn context_zero_is_the_old_output_and_max_lines_still_caps() {
        let text: String = (1..=20)
            .map(|n| {
                if n == 10 || n == 16 {
                    "hit".into()
                } else {
                    format!("l{n}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            filter_lines(&text, None, Some("hit"), 0).unwrap(),
            ["10:hit", "16:hit"]
        );
        // Hits at 10 and 16, context 3: windows [7,13] and [13,19] merge to 13 lines.
        let wide = filter_lines(&text, None, Some("hit"), 3).unwrap();
        assert_eq!(wide.len(), 13);
        assert!(wide.iter().all(|l| l != "--"));
        let out = render_lines(&text, "arc123", None, Some("hit"), 3, 5).unwrap();
        assert_eq!(
            out.lines().filter(|l| !l.contains("lines omitted")).count(),
            5
        );
        assert!(out.contains("8 lines omitted (expand arc123)"), "{out}");
    }

    #[test]
    fn a_full_archive_id_in_the_omitted_marker_prints_eight_hex_chars() {
        let id = "ab".repeat(32);
        let text = (1..=20)
            .map(|n| format!("l{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = render_lines(&text, &id, None, None, 0, 5).unwrap();
        let short = crate::names::short_id(&id);
        assert!(out.contains(&format!("expand {short}")), "{out}");
        assert!(!out.contains(&id), "{out}");
    }

    #[test]
    fn parse_range_bare_start_runs_to_end() {
        assert_eq!(parse_range("10", 20).unwrap(), (10, 20));
        assert_eq!(parse_range("5-5", 20).unwrap(), (5, 5));
        assert_eq!(parse_range("-5", 20).unwrap(), (1, 5));
        assert_eq!(parse_range("5-", 20).unwrap(), (5, 20));
    }

    /// T172: a range the model quoted (double or single) parses the same as the
    /// bare form instead of failing with `invalid line range`.
    #[test]
    fn parse_range_strips_surrounding_quotes() {
        assert_eq!(parse_range("\"5-10\"", 20).unwrap(), (5, 10));
        assert_eq!(parse_range("'5-10'", 20).unwrap(), (5, 10));
        assert_eq!(parse_range(" \"5-10\" ", 20).unwrap(), (5, 10));
        // A lone or mismatched quote is not a pair — still rejected, not silently stripped.
        assert!(parse_range("\"5-10", 20).is_err());
        assert!(parse_range("\"5-10'", 20).is_err());
    }

    /// T353: `a,b` and `a, b` are the same range as `a-b`; the same checks still apply.
    #[test]
    fn parse_range_accepts_comma_pairs() {
        assert_eq!(parse_range("5,10", 20).unwrap(), (5, 10));
        assert_eq!(parse_range("5, 10", 20).unwrap(), (5, 10));
        assert_eq!(parse_range("\"5, 10\"", 20).unwrap(), (5, 10));
        assert_eq!(parse_range("5,", 20).unwrap(), (5, 20));
        assert_eq!(parse_range("7,100", 10).unwrap(), (7, 10));
        for spec in ["1,2,3", "3,2", "0,2", "a,b"] {
            let err = parse_range(spec, 20).unwrap_err().to_string();
            assert!(err.contains(&format!("`{spec}`")), "{err}");
        }
    }

    #[test]
    fn parse_range_rejects_malformed_zero_and_reverse_ranges() {
        for spec in ["", "abc", "1-two", "1-2-3", "0", "0-2", "3-2"] {
            let err = parse_range(spec, 20).unwrap_err();
            assert!(
                err.to_string().contains("invalid line range"),
                "{spec}: {err}"
            );
        }
    }

    #[test]
    fn parse_range_start_past_end_is_err_not_empty() {
        for spec in ["100-200", "11", "11-"] {
            let err = parse_range(spec, 10).unwrap_err();
            assert!(
                err.to_string().contains("start exceeds line count 10"),
                "{spec}: {err}"
            );
        }
        assert_eq!(parse_range("8-100", 10).unwrap(), (8, 10));
    }

    /// T522: the 25 `invalid line range` strings agents sent 2026-10-03..10
    /// (`~/.rtok/errors.log`), each to the two line numbers in it.
    #[test]
    fn parse_range_accepts_the_logged_spellings() {
        let cases = [
            ("36<br/>-60", (36, 60)),
            ("320 - 360", (320, 360)),
            ("10:45", (10, 45)),
            ("[1, 50]", (1, 50)),
            ("[200, 260]", (200, 260)),
            ("[242, 300]", (242, 300)),
            ("[317, 390]", (317, 390)),
            ("560 650", (560, 650)),
            ("12 - 100", (12, 100)),
            ("580:650", (580, 650)),
            ("200 - 380", (200, 380)),
            ("1860_1960", (1860, 1960)),
            ("283 350", (283, 350)),
            ("1 40", (1, 40)),
            ("136\"-\"215", (136, 215)),
            ("610 614", (610, 614)),
            ("490<br>-800", (490, 800)),
            ("12 - 70", (12, 70)),
            ("215\"-\"460", (215, 460)),
            ("235\"-\"270", (235, 270)),
            ("52<br>", (52, 52)),
            ("60<br>", (60, 60)),
            ("410<br>", (410, 410)),
            ("95<br>", (95, 95)),
            ("420 - 640", (420, 640)),
        ];
        assert_eq!(cases.len(), 25);
        for (spec, want) in cases {
            assert_eq!(parse_range(spec, 5000).unwrap(), want, "{spec}");
        }
    }

    #[test]
    fn parse_range_trailing_br_is_one_line_not_to_end() {
        assert_eq!(parse_range("52", 100).unwrap(), (52, 100));
        assert_eq!(parse_range("52<br>", 100).unwrap(), (52, 52));
        assert_eq!(parse_range("52<br/>", 100).unwrap(), (52, 52));
        assert_eq!(parse_range("52<br><br>", 100).unwrap(), (52, 52));
        // A second number is still a range; the trailing break is noise.
        assert_eq!(parse_range("52-70<br>", 100).unwrap(), (52, 70));
        assert_eq!(parse_range("490<br>-800", 1000).unwrap(), (490, 800));
    }

    #[test]
    fn parse_range_still_rejects_non_lines() {
        for spec in [
            "abc", "1-two", "1-2-3", "", "[]", "<br>", ",,,", "a b", "1\"2",
        ] {
            let err = parse_range(spec, 20).unwrap_err().to_string();
            assert!(err.contains("invalid line range"), "{spec}: {err}");
        }
    }

    #[rstest]
    fn max_lines_truncates_and_reports() {
        let text: String = (1..=150)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = render_lines(&text, "arc123", None, None, 0, 100).unwrap();
        assert_eq!(
            out.lines().filter(|l| !l.contains("lines omitted")).count(),
            100
        );
        assert!(out.contains("50 lines omitted (expand arc123)"), "{out}");
    }
}

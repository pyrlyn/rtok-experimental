// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok config validate` and `rtok config set` (plan T12.3, decision D14).
//!
//! Validate walks a TOML file against [`Config::default()`] and reports unknown keys,
//! wrong types, and out-of-range values with `file:line`. `set` writes the user file
//! through `toml_edit` so comments survive. Figment does not write files.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use figment::value::{Dict, Value as FigValue};

use crate::Result;
use toml_edit::{DocumentMut, Item, TableLike, Value as TomlValue};

use super::Config;

/// Parse `path` and return human-readable errors (`file:line: …`). Empty = valid.
pub fn issues(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    Ok(issues_in(path, &text))
}

/// [`issues`] over text already in hand (`set` checks before it writes).
///
/// Parsed as a [`toml_edit::Document`], not a `DocumentMut`: only the immutable document
/// keeps item spans, and the spans are what make `file:line` name the offending line rather
/// than the first line that happens to start with the same key.
pub fn issues_in(path: &Path, text: &str) -> Vec<String> {
    let doc: toml_edit::Document<String> = match text.to_owned().parse() {
        Ok(d) => d,
        Err(e) => return vec![format!("{}:{e}", path.display())],
    };
    let schema = FigValue::serialize(Config::default())
        .expect("Config serializes")
        .into_dict()
        .expect("Config is a table");
    let mut errors = Vec::new();
    check_table(path, text, "", doc.as_table(), &schema, &mut errors);
    check_graph_maps(path, text, doc.as_table(), &mut errors);
    errors
}

fn check_graph_maps(path: &Path, src: &str, doc: &dyn TableLike, errors: &mut Vec<String>) {
    let Some(plugins) = doc.get("plugins").and_then(|i| i.as_table_like()) else {
        return;
    };
    let Some(graph) = plugins.get("graph").and_then(|i| i.as_table_like()) else {
        return;
    };
    // Both maps are keyed by free names (a suffix, a language), so only their values can be checked.
    for (map, allowed) in [
        ("extensions", GRAPH_GRAMMARS),
        ("backend_by_language", GRAPH_BACKENDS),
    ] {
        let Some(table) = graph.get(map).and_then(|i| i.as_table_like()) else {
            continue;
        };
        for (k, item) in TableLike::iter(table) {
            let dotted = format!("plugins.graph.{map}.{k}");
            let at = loc(path, src, item);
            match item.as_str() {
                Some(s) if allowed.contains(&s) => {}
                Some(s) => errors.push(format!(
                    "{at}: {dotted} must be one of {} (got {s})",
                    allowed.join(", ")
                )),
                _ => errors.push(format!("{at}: {dotted}: expected string")),
            }
        }
    }
}

/// Edit `<home>/config.toml` at `key` (dotted), preserving comments. Creates the
/// reference file when it is missing. Refuses a write that would fail [`issues`]
/// (unknown plugin id, `enabled = "yes"`, …) so the file never stops loading.
/// Returns the file and a `git diff` of the edit — empty when the value was already there.
/// `dry_run` renders that diff and writes nothing; the value is validated either way, so a
/// preview refuses exactly what the real run would refuse.
pub fn set(home: &Path, key: &str, raw: &str, dry_run: bool) -> Result<(PathBuf, String)> {
    set_with(home, None, key, raw, dry_run)
}

/// [`set`] with an explicit `--config` / `RTOK_CONFIG` override.
pub fn set_with(
    home: &Path,
    config_file: Option<&Path>,
    key: &str,
    raw: &str,
    dry_run: bool,
) -> Result<(PathBuf, String)> {
    set_all_with(home, config_file, &[(key, raw)], dry_run)
}

/// [`set_with`] over several `(key, raw)` pairs in one read, check and write (T254).
pub fn set_all_with(
    home: &Path,
    config_file: Option<&Path>,
    pairs: &[(&str, &str)],
    dry_run: bool,
) -> Result<(PathBuf, String)> {
    for (key, _) in pairs {
        if key.is_empty() || key.split('.').any(str::is_empty) {
            bail!("empty key");
        }
    }
    let path = Config::user_path(home, config_file);
    if !path.exists() {
        if dry_run {
            bail!("no config file yet; run `rtok config init` first");
        }
        Config::init_maybe(home, config_file, false, false)?;
    }
    let original = std::fs::read_to_string(&path)?;
    // A fresh init leaves defaults commented. Uncomment the target line first so the
    // in-place swap below keeps its padding and trailing comment instead of appending
    // a second key.
    let mut source = original.clone();
    for (key, _) in pairs {
        source = reveal_commented_key(&source, key);
    }
    let mut doc: DocumentMut = source.parse().with_context(|| path.display().to_string())?;
    for (key, raw) in pairs {
        assign(&mut doc, key, parse_value(raw))?;
    }
    let after = doc.to_string();
    let errs = issues_in(&path, &after);
    if !errs.is_empty() {
        bail!("{}", errs.join("\n"));
    }
    let diff = file_diff(&path, &original, &after);
    if !dry_run {
        super::write_file(&path, &after)?;
    }
    Ok((path, diff))
}

fn parse_value(raw: &str) -> TomlValue {
    raw.parse()
        .unwrap_or_else(|_| TomlValue::from(raw.to_string()))
}

/// `rtok config set <key> <value>`: walk the dotted key, creating intermediate tables.
/// `toml_edit`'s index operators panic on a path that runs through a scalar
/// (`set proxy.port.foo 1`), so the walk is explicit and reports the clash instead.
fn assign(doc: &mut DocumentMut, key: &str, value: TomlValue) -> Result<()> {
    let mut parts = key.split('.').peekable();
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            // `insert` replaces the whole (Key, Item) pair, dropping the key's padding and its
            // trailing `# passthrough | compress` comment; swap the value in place instead.
            match table.get_mut(part).and_then(Item::as_value_mut) {
                Some(old) => {
                    let decor = old.decor().clone();
                    *old = value;
                    *old.decor_mut() = decor;
                }
                None => {
                    table.insert(part, toml_edit::value(value));
                }
            }
            return Ok(());
        }
        if !table.contains_key(part) {
            table.insert(part, toml_edit::table());
        }
        table = table
            .get_mut(part)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| anyhow::anyhow!("{key}: {part} is not a table"))?;
    }
    bail!("empty key");
}

/// A fresh init comments defaults out. Reveal each documented assignment on the path so
/// [`assign`] edits the leaf in place. An intermediate scalar has to be visible too:
/// `set proxy.port.foo` used to report "not a table" because `port` was a live number;
/// leaving it commented made assign invent a table. A key that is already set is left
/// alone (uncommenting it too would duplicate).
fn reveal_commented_key(text: &str, dotted: &str) -> String {
    let parts: Vec<&str> = dotted.split('.').collect();
    let mut out = text.to_string();
    for (i, leaf) in parts.iter().enumerate() {
        let table = (i > 0).then(|| parts[..i].join("."));
        if live_key_present(&out, table.as_deref(), leaf) {
            continue;
        }
        out = reveal_assignment(&out, table.as_deref(), leaf);
    }
    out
}

fn reveal_assignment(text: &str, table: Option<&str>, leaf: &str) -> String {
    let mut current: Option<String> = None;
    let mut revealed = false;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let raw = line.trim_end_matches(['\r', '\n']);
        let trimmed = raw.trim();
        if let Some(header) = table_header(trimmed) {
            current = Some(header);
            out.push_str(line);
            continue;
        }
        let here = match table {
            None => current.is_none(),
            Some(want) => current.as_deref() == Some(want),
        };
        if !revealed
            && here
            && let Some(bare) = reveal_line(raw, leaf)
        {
            revealed = true;
            out.push_str(&bare);
            if line.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        out.push_str(line);
    }
    out
}

fn live_key_present(text: &str, table: Option<&str>, leaf: &str) -> bool {
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(header) = table_header(trimmed) {
            current = Some(header);
            continue;
        }
        let here = match table {
            None => current.is_none(),
            Some(want) => current.as_deref() == Some(want),
        };
        if !here || trimmed.starts_with('#') {
            continue;
        }
        let code = trimmed.split('#').next().unwrap_or("").trim();
        if code
            .split_once('=')
            .is_some_and(|(key, _)| key.trim() == leaf)
        {
            return true;
        }
    }
    false
}

fn table_header(trimmed: &str) -> Option<String> {
    let code = trimmed.split('#').next()?.trim();
    let inner = code.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() || inner.contains('[') {
        return None;
    }
    Some(inner.to_string())
}

fn reveal_line(raw: &str, leaf: &str) -> Option<String> {
    let trimmed = raw.trim_start();
    let indent = raw.len() - trimmed.len();
    let rest = trimmed.strip_prefix("# ")?;
    let code = rest.split('#').next()?.trim();
    let key = code.split_once('=')?.0.trim();
    if key != leaf {
        return None;
    }
    Some(format!("{}{rest}", &raw[..indent]))
}

/// Explicit keys whose value is not the current default. A note, not an error: rtok cannot
/// tell a stale init from a deliberate pin, so nothing here is rewritten. Unreadable or
/// unparsable files yield nothing — [`issues`] is the error path, and doctor stays up.
pub fn pinned_notes(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    pinned_notes_in(path, &text)
}

pub(crate) fn pinned_notes_in(path: &Path, text: &str) -> Vec<String> {
    let Ok(doc) = text.parse::<toml_edit::Document<String>>() else {
        return Vec::new();
    };
    let schema = FigValue::serialize(Config::default())
        .expect("Config serializes")
        .into_dict()
        .expect("Config is a table");
    let mut notes = Vec::new();
    note_table(path, text, "", doc.as_table(), &schema, &mut notes);
    notes
}

fn note_table(
    path: &Path,
    src: &str,
    prefix: &str,
    table: &dyn TableLike,
    schema: &Dict,
    notes: &mut Vec<String>,
) {
    for (k, item) in TableLike::iter(table) {
        let dotted = if prefix.is_empty() {
            k.to_string()
        } else {
            format!("{prefix}.{k}")
        };
        match schema.get(k) {
            Some(FigValue::Dict(_, nested)) => {
                if let Some(t) = item.as_table_like() {
                    note_table(path, src, &dotted, t, nested, notes);
                }
            }
            Some(expected) if !same_leaf(item, expected) => {
                notes.push(format!(
                    "note {}: {dotted} = {} (default {})",
                    loc(path, src, item),
                    shown(item),
                    super::layers::display(expected)
                ));
            }
            _ => {}
        }
    }
}

fn same_leaf(item: &Item, expected: &FigValue) -> bool {
    let Some(value) = item.as_value() else {
        return false;
    };
    // An f64 literal stored as f32 does not share bits with the TOML float, but both
    // read as `4.2`. The note is about that reading.
    value_matches(value, expected) || shown_value(value) == super::layers::display(expected)
}

fn value_matches(value: &TomlValue, expected: &FigValue) -> bool {
    match expected {
        FigValue::String(_, s) => value.as_str() == Some(s.as_str()),
        FigValue::Bool(_, b) => value.as_bool() == Some(*b),
        FigValue::Num(_, n) => num_matches(value, n),
        FigValue::Array(_, items) => value.as_array().is_some_and(|arr| {
            arr.len() == items.len() && arr.iter().zip(items).all(|(v, e)| value_matches(v, e))
        }),
        _ => false,
    }
}

fn num_matches(value: &TomlValue, num: &figment::value::Num) -> bool {
    use figment::value::Num;
    if let Some(i) = value.as_integer() {
        return match *num {
            Num::I64(n) => i == n,
            Num::I32(n) => i == i64::from(n),
            Num::I16(n) => i == i64::from(n),
            Num::I8(n) => i == i64::from(n),
            Num::I128(n) => i128::from(i) == n,
            Num::ISize(n) => i64::try_from(n).ok() == Some(i),
            Num::U64(n) => u64::try_from(i).ok() == Some(n),
            Num::U32(n) => u32::try_from(i).ok() == Some(n),
            Num::U16(n) => u16::try_from(i).ok() == Some(n),
            Num::U8(n) => u8::try_from(i).ok() == Some(n),
            Num::U128(n) => u128::try_from(i).ok() == Some(n),
            Num::USize(n) => u64::try_from(i).ok() == u64::try_from(n).ok(),
            Num::F64(n) => f_eq(i as f64, n),
            Num::F32(n) => f_eq(i as f64, f64::from(n)),
        };
    }
    if let Some(f) = value.as_float() {
        return match *num {
            Num::F64(n) => f_eq(f, n),
            Num::F32(n) => f_eq(f, f64::from(n)),
            _ => false,
        };
    }
    false
}

/// `4.2` from TOML and `4.2` from a Rust literal are not the same bits, but they are the
/// same default a person would read. The note is about that reading.
fn f_eq(a: f64, b: f64) -> bool {
    a == b || a.to_string() == b.to_string()
}

fn shown(item: &Item) -> String {
    item.as_value()
        .map(shown_value)
        .unwrap_or_else(|| item.to_string())
}

fn shown_value(value: &TomlValue) -> String {
    match value {
        TomlValue::String(s) => format!("\"{}\"", s.value()),
        TomlValue::Integer(i) => i.value().to_string(),
        TomlValue::Float(f) => f.value().to_string(),
        TomlValue::Boolean(b) => b.value().to_string(),
        TomlValue::Array(a) => {
            let parts: Vec<_> = a.iter().map(shown_value).collect();
            format!("[{}]", parts.join(", "))
        }
        other => other.to_string(),
    }
}

/// [`issues`] over the values the merged config takes from a layer other than the defaults and
/// the user file (project file, `.env`, environment), as [`super::layers::sourced`] yields them.
/// `issues` reads only the file, so without this `RTOK_LOG_LEVEL=verbose` loaded and silently
/// dropped every log line below error while `validate` said ok. Each message names the layer.
pub fn layered_issues(values: Vec<(String, FigValue, String)>) -> Vec<String> {
    let mut docs: std::collections::BTreeMap<String, DocumentMut> = Default::default();
    for (key, value, source) in values {
        if matches!(source.as_str(), "default" | "user") {
            continue;
        }
        if let Some(value) = toml_value(&value) {
            // A leaf key never runs through a scalar, so `assign` has no clash to report.
            let _ = assign(docs.entry(source).or_default(), &key, value);
        }
    }
    let mut out = Vec::new();
    for (source, doc) in docs {
        for e in issues_in(Path::new(&source), &doc.to_string()) {
            // `source:LINE: msg` → `source: msg`; the line is of a synthetic document.
            let msg = e
                .strip_prefix(&format!("{source}:"))
                .and_then(|rest| rest.split_once(": "))
                .map_or(e.as_str(), |(_, msg)| msg);
            out.push(format!("{source}: {msg}"));
        }
    }
    out
}

fn toml_value(v: &FigValue) -> Option<TomlValue> {
    use figment::value::Num;
    Some(match v {
        FigValue::String(_, s) => TomlValue::from(s.as_str()),
        FigValue::Bool(_, b) => TomlValue::from(*b),
        FigValue::Num(_, Num::F32(_) | Num::F64(_)) => TomlValue::from(v.to_num()?.to_f64()?),
        FigValue::Num(_, n) => {
            // `to_i128` is `None` for the unsigned variants, which most integer keys use.
            let wide = n.to_i128().or_else(|| i128::try_from(n.to_u128()?).ok())?;
            TomlValue::from(i64::try_from(wide).ok()?)
        }
        FigValue::Array(_, items) => items.iter().filter_map(toml_value).collect(),
        _ => return None,
    })
}

type RulesFn = fn(&Path, &Path) -> Vec<String>;
type DiffFn = fn(&Path, &str, &str) -> String;

static RULES: Mutex<Option<RulesFn>> = Mutex::new(None);
static DIFF: Mutex<Option<DiffFn>> = Mutex::new(None);

/// The CLI registers the coloured diff so a tty paints it. With nothing registered (library
/// tests) the diff is the same text, uncoloured.
pub fn register_file_diff(paint: DiffFn) {
    *DIFF
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(paint);
}

pub(crate) fn file_diff(path: &Path, before: &str, after: &str) -> String {
    let paint = *DIFF
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match paint {
        Some(paint) => paint(path, before, after),
        None => change_preview::unified_diff(path, before, after),
    }
}

/// `cli::run` registers the `cmd` rules checker. Config stays free of `plugins`.
/// With nothing registered (the `cmd` feature off, or a library load) there is
/// nothing to check — the same empty result the feature-off build returned.
pub fn register_rules(check: RulesFn) {
    *RULES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(check);
}

/// Malformed `cmd` filter rules for `rtok config validate` (T50.2): the single
/// `rules` file when present, plus every `rules.d/*.toml`.
pub fn rules_issues(rules: &Path, rules_dir: &Path) -> Vec<String> {
    match *RULES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        Some(check) => check(rules, rules_dir),
        None => Vec::new(),
    }
}

fn is_open(dotted: &str) -> bool {
    // `bench.configs` is a free-form name → path map; `stats.prices` is keyed by
    // provider model id, which no schema can enumerate — both skip value checks.
    dotted == "bench.configs"
        || dotted == "stats.prices"
        || dotted == "plugins.graph.extensions"
        || dotted == "plugins.graph.backend_by_language"
        || dotted.starts_with("plugins.graph.backend_by_language.")
        || dotted.starts_with("plugins.graph.extensions.")
}

const GRAPH_BACKENDS: &[&str] = &["tags", "lsp", "auto", "text"];

const GRAPH_GRAMMARS: &[&str] = &[
    "rust", "ts", "tsx", "js", "mjs", "cjs", "py", "dart", "c", "h", "go",
];

fn line_of(src: &str, span: Option<std::ops::Range<usize>>) -> usize {
    let off = span.map(|s| s.start).unwrap_or(0).min(src.len());
    src[..off].bytes().filter(|&b| b == b'\n').count() + 1
}

/// `path:line` for an item. `toml_edit` records the span of every parsed item, so this is
/// the offending line — a hand-rolled "first line whose text starts with the leaf name"
/// scan reported the wrong one whenever two tables share a key
/// (`[plugins.measure] enabled` / `[plugins.cmd] enabled`).
fn loc(path: &Path, src: &str, item: &Item) -> String {
    format!("{}:{}", path.display(), line_of(src, item.span()))
}

fn check_table(
    path: &Path,
    src: &str,
    prefix: &str,
    table: &dyn TableLike,
    schema: &Dict,
    errors: &mut Vec<String>,
) {
    for (k, item) in TableLike::iter(table) {
        let dotted = if prefix.is_empty() {
            k.to_string()
        } else {
            format!("{prefix}.{k}")
        };
        if is_open(prefix) {
            continue;
        }
        if is_open(&dotted) {
            continue;
        }
        match schema.get(k) {
            None => errors.push(format!("{}: unknown key: {dotted}", loc(path, src, item))),
            // `as_table_like`: the loader accepts `proxy = { port = 2 }`, so validate must too.
            Some(FigValue::Dict(_, nested)) => match item.as_table_like() {
                Some(t) => check_table(path, src, &dotted, t, nested, errors),
                None => errors.push(format!(
                    "{}: {dotted}: expected table",
                    loc(path, src, item)
                )),
            },
            Some(expected) => check_leaf(path, src, dotted.as_str(), item, expected, errors),
        }
    }
}

/// String keys that take one of a fixed set of values (`rtok config validate` names the set).
const CHOICES: &[(&str, &[&str])] = &[
    ("log.tspin", &["auto", "always", "off"]),
    ("plugins.graph.map_rank", &["refs", "pagerank"]),
    ("plugins.graph.backend", GRAPH_BACKENDS),
    ("proxy.flex.on_429", &["none", "backoff", "default"]),
    // Any other value turns the semantic tier on with the placeholder hash embedding
    // (`proxy::semantic_cache`); `"hash"` is the only backend until P29 ships real ones.
    ("plugins.proxy.semantic_cache.embed_backend", &["hash"]),
];

/// Float keys limited to `(0, 1]`: `threshold <= 0` makes every cached entry a semantic hit,
/// and a `delta_max_ratio` outside the range disables deltas or sends diffs above the file.
const UNIT_RATIO_KEYS: &[&str] = &[
    "plugins.proxy.semantic_cache.threshold",
    "plugins.read.delta_max_ratio",
];

fn check_leaf(
    path: &Path,
    src: &str,
    dotted: &str,
    item: &Item,
    expected: &FigValue,
    errors: &mut Vec<String>,
) {
    let at = loc(path, src, item);
    // `[[agents.junk.extra]]` is an array of tables, which the generic arm below would refuse.
    if dotted == "agents.junk.extra" {
        return check_junk_extra(&at, item, errors);
    }
    match expected {
        FigValue::String(..) => {
            let Some(value) = item.as_str() else {
                errors.push(format!("{at}: {dotted}: expected string"));
                return;
            };
            // A closed set is rejected here, not at the one call site that would read it.
            if let Some((_, choices)) = CHOICES.iter().find(|(key, _)| *key == dotted)
                && !choices.contains(&value)
            {
                errors.push(format!(
                    "{at}: {dotted}: expected one of {}",
                    choices.join(" | ")
                ));
            }
        }
        FigValue::Bool(..) => {
            if item.as_bool().is_none() {
                errors.push(format!("{at}: {dotted}: expected bool"));
                return;
            }
        }
        FigValue::Num(_, num) => {
            // A float default accepts a float; an integer default does not. Accepting both
            // let `port = 8790.5` pass `validate` and then fail `Config::load`.
            let ok = if matches!(
                num,
                figment::value::Num::F32(_) | figment::value::Num::F64(_)
            ) {
                item.as_integer().is_some() || item.as_float().is_some()
            } else {
                item.as_integer().is_some()
            };
            if !ok {
                errors.push(format!("{at}: {dotted}: expected number"));
                return;
            }
            // An unsigned default refuses a negative value here, not later in `Config::load`.
            let unsigned = matches!(
                num,
                figment::value::Num::U8(_)
                    | figment::value::Num::U16(_)
                    | figment::value::Num::U32(_)
                    | figment::value::Num::U64(_)
                    | figment::value::Num::U128(_)
                    | figment::value::Num::USize(_)
            );
            if unsigned && item.as_integer().is_some_and(|n| n < 0) {
                errors.push(format!("{at}: {dotted} must be ≥ 0"));
                return;
            }
        }
        FigValue::Array(..) if item.as_array().is_none() => {
            errors.push(format!("{at}: {dotted}: expected array"));
            return;
        }
        FigValue::Array(..) => {}
        _ => {}
    }
    // Integers are valid for a float key (`threshold = 1`), so range-check both.
    if UNIT_RATIO_KEYS.contains(&dotted)
        && let Some(x) = item
            .as_float()
            .or_else(|| item.as_integer().map(|n| n as f64))
        && !(x > 0.0 && x <= 1.0)
    {
        errors.push(format!("{at}: {dotted} must be in (0, 1]"));
    }
    if let Some(n) = item.as_integer() {
        match dotted {
            "proxy.port" | "web.port" if !(1..=65535).contains(&n) => {
                errors.push(format!("{at}: {dotted} out of range (1–65535)"));
            }
            "plugins.archive.keep_turns" if n < 1 => {
                errors.push(format!("{at}: {dotted} must be ≥ 1"));
            }
            // Each retry holds the client's connection open; past a handful it is a hang.
            "proxy.flex.retries" if n > 5 => {
                errors.push(format!("{at}: {dotted} must be ≤ 5"));
            }
            // A zero window or rate would blank the live panel; 1000 rows is what a frame carries.
            "plugins.graph.live_heat_window_s"
            | "plugins.graph.live_max_events_per_s"
            | "plugins.graph.live_feed_rows"
                if n < 1 =>
            {
                errors.push(format!("{at}: {dotted} must be ≥ 1"));
            }
            "plugins.graph.live_feed_rows" if n > 1000 => {
                errors.push(format!("{at}: {dotted} must be ≤ 1000"));
            }
            "tui.tick_secs" if n < 1 => {
                errors.push(format!("{at}: {dotted} must be ≥ 1"));
            }
            // `read` never trims the `… archived <id> …` marker (79 chars): the id is what keeps
            // a cut lossless. A smaller cap passed validation and was then silently exceeded.
            "plugins.read.max_chars" if n < 100 => {
                errors.push(format!("{at}: {dotted} must be ≥ 100"));
            }
            // Every line rotates when the file has no room: O(files) renames on the
            // hook path (D1 ≤ 10 ms). Zero disables rotation only by deleting history.
            "log.max_bytes" if n < 1024 => {
                errors.push(format!("{at}: {dotted} must be ≥ 1024"));
            }
            // Rotation is one rename per kept file; past a handful it is all cost, no
            // history an operator scrolls through (`rtok logs` shows `[log] lines`).
            "log.files" if n > 20 => {
                errors.push(format!("{at}: {dotted} must be ≤ 20"));
            }
            // Ten years: a larger floor is a typo, and it would keep junk forever.
            "agents.junk.keep_logs_days"
            | "agents.junk.stale_session_days"
            | "agents.junk.stale_worktree_days"
            | "agents.junk.crash_dump_min_age_days"
                if n > 3650 =>
            {
                errors.push(format!("{at}: {dotted} must be ≤ 3650"));
            }
            "agents.junk.temp_min_age_hours" if n > 87_600 => {
                errors.push(format!("{at}: {dotted} must be ≤ 87600"));
            }
            _ => {}
        }
    }
    // A glob that does not compile would keep nothing it was meant to protect.
    if dotted == "agents.junk.exclude"
        && let Some(globs) = item.as_array()
    {
        for (i, g) in globs.iter().enumerate() {
            match g.as_str().map(globset::Glob::new) {
                Some(Ok(_)) => {}
                Some(Err(e)) => errors.push(format!("{at}: {dotted}[{i}]: {e}")),
                None => errors.push(format!("{at}: {dotted}[{i}]: expected string")),
            }
        }
    }
    if let Some(s) = item.as_str() {
        match dotted {
            "proxy.mode" if !matches!(s, "passthrough" | "compress") => {
                errors.push(format!("{at}: {dotted} must be passthrough or compress"));
            }
            "plugins.read.default_mode"
                if !matches!(s, "full" | "lines" | "map" | "signatures") =>
            {
                errors.push(format!(
                    "{at}: {dotted} must be full, lines, map, or signatures"
                ));
            }
            "plugins.graph.watch" if !matches!(s, "off" | "notify") => {
                errors.push(format!("{at}: {dotted} must be off or notify"));
            }
            "tasks.adapter" if !super::TASK_ADAPTERS.contains(&s) => {
                let all = super::TASK_ADAPTERS.join(", ");
                errors.push(format!("{at}: {dotted} must be one of {all}"));
            }
            // Empty means "the project name's first letter"; anything else must parse as ids.
            "tasks.prefix" if !s.is_empty() => {
                if let Err(e) = rtok_plugin_sdk::check_task_prefix(s) {
                    errors.push(format!("{at}: {dotted}: {e}"));
                }
            }
            // The one parser every reader of these windows uses, so `set` cannot store a value
            // that `rtok stats`, `rtok report`, `doctor` and the web model then refuse.
            "stats.since" | "report.since" => {
                if let Err(e) = crate::parse_since_from(s, dotted) {
                    errors.push(format!("{at}: {e}"));
                }
            }
            // An unknown level ranks most severe (`log::rank`), so a typo silently
            // drops everything below error while `validate` says ok.
            "log.level"
                if !matches!(
                    s.to_ascii_lowercase().as_str(),
                    "error" | "warn" | "info" | "debug"
                ) =>
            {
                errors.push(format!(
                    "{at}: {dotted} must be error, warn, info, or debug"
                ));
            }
            // Caught here, not as a 400 on every request of that lane once the proxy runs.
            lane if lane.starts_with("proxy.lanes.")
                && lane.ends_with(".upstream")
                && !s.trim().is_empty()
                && !url::Url::parse(s.trim())
                    .is_ok_and(|u| matches!(u.scheme(), "http" | "https")) =>
            {
                errors.push(format!("{at}: {dotted} must be empty or an http(s) URL"));
            }
            _ => {}
        }
    }
}

/// `[agents.junk] extra`: every entry a table of a known host (or `rtok`), an extra kind and a
/// path. Each problem names the entry, so `[[agents.junk.extra]]` and the inline form read alike.
fn check_junk_extra(at: &str, item: &Item, errors: &mut Vec<String>) {
    let dotted = "agents.junk.extra";
    let entries: Vec<Option<&dyn TableLike>> = match (item.as_array_of_tables(), item.as_array()) {
        (Some(tables), _) => tables.iter().map(|t| Some(t as &dyn TableLike)).collect(),
        (None, Some(values)) => values
            .iter()
            .map(|v| v.as_inline_table().map(|t| t as &dyn TableLike))
            .collect(),
        (None, None) => return errors.push(format!("{at}: {dotted}: expected array")),
    };
    let kinds = super::JUNK_EXTRA_KINDS;
    for (i, entry) in entries.into_iter().enumerate() {
        let key = format!("{dotted}[{i}]");
        let Some(t) = entry else {
            errors.push(format!("{at}: {key}: expected table"));
            continue;
        };
        for (k, _) in t.iter() {
            if !matches!(k, "host" | "kind" | "path") {
                errors.push(format!("{at}: unknown key: {key}.{k}"));
            }
        }
        let field = |k: &str| t.get(k).and_then(|v| v.as_str());
        match field("host") {
            Some(h) if h == "rtok" || super::HOSTS.contains(&h) => {}
            _ => errors.push(format!(
                "{at}: {key}.host must be rtok or a host id (see `rtok agents list`)"
            )),
        }
        if !field("kind").is_some_and(|k| kinds.contains(&k)) {
            let all = kinds.join(", ");
            errors.push(format!("{at}: {key}.kind must be one of {all}"));
        }
        if field("path").is_none_or(str::is_empty) {
            errors.push(format!("{at}: {key}.path must be a non-empty path"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-val-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `set` swaps in a new file (new inode) instead of truncating the old one in place, so a
    /// crash mid-write cannot leave a half-written `config.toml`.
    #[cfg(unix)]
    #[test]
    fn set_replaces_the_file_atomically() {
        use std::os::unix::fs::MetadataExt;
        let home = tmp("atomic");
        let (path, _) = Config::init_maybe(&home, None, false, false).unwrap();
        let ino = std::fs::metadata(&path).unwrap().ino();
        set(&home, "proxy.port", "9999", false).unwrap();
        assert_ne!(std::fs::metadata(&path).unwrap().ino(), ino);
        assert!(std::fs::read_to_string(&path).unwrap().contains("9999"));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// T225.1: `log.tspin` is a closed set; the message names it.
    #[test]
    fn a_tspin_value_outside_the_set_is_rejected_by_name() {
        let dir = tmp("tspin");
        let path = dir.join("c.toml");
        std::fs::write(&path, "[log]\ntspin = \"sometimes\"\n").unwrap();
        let errs = issues(&path).unwrap();
        assert!(
            errs.iter()
                .any(|e| e.contains("log.tspin") && e.contains("auto | always | off")),
            "{errs:?}"
        );
        for ok in ["auto", "always", "off"] {
            std::fs::write(&path, format!("[log]\ntspin = \"{ok}\"\n")).unwrap();
            assert!(issues(&path).unwrap().is_empty(), "{ok}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T330.5.1: every `[agents.junk]` key refuses what it cannot mean, naming the key.
    #[test]
    fn agents_junk_values_are_checked_by_name() {
        let dir = tmp("agents-junk");
        let path = dir.join("c.toml");
        let errs = |body: &str| {
            std::fs::write(&path, format!("[agents.junk]\n{body}\n")).unwrap();
            issues(&path).unwrap()
        };
        for (body, want) in [
            (
                "keep_logs_days = -1",
                "agents.junk.keep_logs_days must be ≥ 0",
            ),
            (
                "keep_logs_days = 2.5",
                "agents.junk.keep_logs_days: expected number",
            ),
            (
                "keep_logs_days = \"abc\"",
                "agents.junk.keep_logs_days: expected number",
            ),
            (
                "keep_logs_days = 3651",
                "agents.junk.keep_logs_days must be ≤ 3650",
            ),
            (
                "stale_session_days = 3651",
                "agents.junk.stale_session_days must be ≤ 3650",
            ),
            (
                "stale_session_days = -1",
                "agents.junk.stale_session_days must be ≥ 0",
            ),
            (
                "stale_session_days = 2.5",
                "agents.junk.stale_session_days: expected number",
            ),
            (
                "stale_worktree_days = 3651",
                "agents.junk.stale_worktree_days must be ≤ 3650",
            ),
            (
                "crash_dump_min_age_days = 3651",
                "agents.junk.crash_dump_min_age_days must be ≤ 3650",
            ),
            (
                "temp_min_age_hours = 87601",
                "agents.junk.temp_min_age_hours must be ≤ 87600",
            ),
            ("exclude = [\"[\"]", "agents.junk.exclude[0]"),
            ("exclude = [1]", "agents.junk.exclude[0]: expected string"),
            (
                "extra = [{ host = \"nope\", kind = \"cache\", path = \"/x\" }]",
                "agents.junk.extra[0].host",
            ),
            (
                "extra = [{ host = \"cursor\", kind = \"sessions\", path = \"/x\" }]",
                "agents.junk.extra[0].kind must be one of cache, temp, logs, crash-dumps",
            ),
            (
                "extra = [{ host = \"rtok\", kind = \"temp\" }]",
                "agents.junk.extra[0].path",
            ),
            (
                "extra = [{ host = \"rtok\", kind = \"temp\", path = \"/x\", why = 1 }]",
                "unknown key: agents.junk.extra[0].why",
            ),
            ("extra = [\"/x\"]", "agents.junk.extra[0]: expected table"),
        ] {
            let got = errs(body);
            assert!(got.iter().any(|e| e.contains(want)), "{body}: {got:?}");
        }
        let ok = "keep_logs_days = 0\nexclude = [\"~/keep/**\"]\n\n[[agents.junk.extra]]\nhost = \"cursor\"\nkind = \"cache\"\npath = \"~/c\"";
        assert_eq!(errs(ok), Vec::<String>::new());
        // Loading refuses a value of the wrong type the same way, naming the key.
        std::fs::write(
            dir.join("config.toml"),
            "[agents.junk]\nkeep_logs_days = \"abc\"\n",
        )
        .unwrap();
        let err = format!("{:#}", Config::load_from(&dir).unwrap_err());
        assert!(err.contains("keep_logs_days"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T441.2: the adapter is a closed set and the prefix must be one ids can carry.
    #[test]
    fn tasks_adapter_and_prefix_are_checked() {
        let dir = tmp("tasks");
        let path = dir.join("c.toml");
        for (body, key) in [
            ("adapter = \"jira\"", "tasks.adapter"),
            ("prefix = \"R2\"", "tasks.prefix"),
            ("prefix = \"TOOLONGPX\"", "tasks.prefix"),
        ] {
            std::fs::write(&path, format!("[tasks]\n{body}\n")).unwrap();
            let errs = issues(&path).unwrap();
            assert!(errs.iter().any(|e| e.contains(key)), "{body}: {errs:?}");
        }
        let ok =
            "[tasks]\nadapter = \"gitlab\"\nprefix = \"at\"\n[tasks.gitlab]\nproject = \"g/n\"\n";
        std::fs::write(&path, ok).unwrap();
        assert!(issues(&path).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_read_cap_below_the_archive_marker_is_rejected() {
        let dir = tmp("readcap");
        let path = dir.join("c.toml");
        std::fs::write(&path, "[plugins.read]\nmax_chars = 50\n").unwrap();
        let errs = issues(&path).unwrap();
        assert!(
            errs.iter().any(|e| e.contains("plugins.read.max_chars")),
            "{errs:?}"
        );
        std::fs::write(&path, "[plugins.read]\nmax_chars = 100\n").unwrap();
        assert!(issues(&path).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_level_typo_and_hook_path_killing_bounds_are_rejected() {
        let dir = tmp("logbounds");
        let path = dir.join("c.toml");
        for (body, key) in [
            ("[log]\nlevel = \"verbose\"\n", "log.level"),
            ("[log]\nmax_bytes = 0\n", "log.max_bytes"),
            ("[log]\nfiles = 100\n", "log.files"),
        ] {
            std::fs::write(&path, body).unwrap();
            let errs = issues(&path).unwrap();
            assert!(errs.iter().any(|e| e.contains(key)), "{body}: {errs:?}");
        }
        std::fs::write(
            &path,
            "[log]\nlevel = \"Warn\"\nmax_bytes = 1024\nfiles = 20\n",
        )
        .unwrap();
        assert!(issues(&path).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lane_upstream_must_be_an_http_url() {
        let dir = tmp("laneup");
        let path = dir.join("c.toml");
        for bad in ["localhost:8080", "ftp://mirror", "not a url"] {
            std::fs::write(&path, format!("[proxy.lanes.bulk]\nupstream = \"{bad}\"\n")).unwrap();
            let errs = issues(&path).unwrap();
            assert!(
                errs.iter().any(|e| e.contains("proxy.lanes.bulk.upstream")),
                "{bad}: {errs:?}"
            );
        }
        for good in ["", "http://127.0.0.1:4000", "https://gateway.example/v1"] {
            std::fs::write(
                &path,
                format!("[proxy.lanes.bulk]\nupstream = \"{good}\"\n"),
            )
            .unwrap();
            assert!(issues(&path).unwrap().is_empty(), "{good}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn port_70000_names_the_line() {
        let dir = tmp("port");
        let path = dir.join("bad.toml");
        std::fs::write(&path, "[proxy]\nport = 70000\n").unwrap();
        let errs = issues(&path).unwrap();
        assert!(
            errs.iter()
                .any(|e| e.contains(":2:") && e.contains("proxy.port")),
            "{errs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plugin_enabled_is_validated() {
        let dir = tmp("plug");
        let path = dir.join("bad.toml");
        std::fs::write(
            &path,
            "[plugins.cmd]\nenabled = \"yes\"\n[plugins.nope]\nenabled = true\n",
        )
        .unwrap();
        let errs = issues(&path).unwrap();
        assert!(
            errs.iter()
                .any(|e| e.contains("plugins.cmd.enabled") && e.contains("bool")),
            "{errs:?}"
        );
        assert!(
            errs.iter().any(|e| e.contains("unknown key: plugins.nope")),
            "{errs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reported line is the offending one, not the first line whose leaf name matches:
    /// two tables both holding `enabled` used to point at the wrong one.
    #[test]
    fn the_reported_line_is_the_offending_one() {
        let dir = tmp("line");
        let path = dir.join("bad.toml");
        std::fs::write(
            &path,
            "[plugins.measure]\nenabled = true\n[plugins.cmd]\nenabled = \"yes\"\n",
        )
        .unwrap();
        let errs = issues(&path).unwrap();
        assert!(
            errs.iter()
                .any(|e| e.contains(":4:") && e.contains("plugins.cmd.enabled")),
            "{errs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `validate` has to refuse what the loader refuses: a float in an integer key passed
    /// validation and then failed `Config::load`.
    #[test]
    fn a_float_is_not_a_number_for_an_integer_key() {
        let dir = tmp("float");
        let path = dir.join("bad.toml");
        std::fs::write(&path, "[proxy]\nport = 8790.5\n").unwrap();
        let errs = issues(&path).unwrap();
        assert!(
            errs.iter().any(|e| e.contains("proxy.port")),
            "float accepted: {errs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `config set` used to panic through `toml_edit`'s indexing when the key path walked
    /// into a scalar (`set proxy.port.foo 1`).
    #[test]
    fn set_through_a_scalar_reports_instead_of_panicking() {
        let home = tmp("scalar");
        Config::init(&home, false).unwrap();
        let before = std::fs::read_to_string(Config::path_for(&home)).unwrap();
        let err = set(&home, "proxy.port.foo", "1", false).unwrap_err();
        assert!(err.to_string().contains("not a table"), "{err}");
        assert_eq!(
            before,
            std::fs::read_to_string(Config::path_for(&home)).unwrap(),
            "nothing written"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn set_refuses_a_value_the_loader_would_reject() {
        let home = tmp("setbad");
        Config::init(&home, false).unwrap();
        let before = std::fs::read_to_string(Config::path_for(&home)).unwrap();
        assert!(set(&home, "plugins.nope.enabled", "true", false).is_err());
        assert!(set(&home, "plugins.cmd.enabled", "yes", false).is_err());
        assert_eq!(
            std::fs::read_to_string(Config::path_for(&home)).unwrap(),
            before
        );
        set(&home, "plugins.cmd.enabled", "false", false).unwrap();
        let cfg = Config::load_from(&home).unwrap();
        assert!(!cfg.plugin_enabled("cmd", true));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// T364, T379: `stats.since` and `report.since` go through `parse_since`, for `validate`
    /// and `set` alike.
    #[test]
    fn a_malformed_since_window_is_rejected() {
        for table in ["stats", "report"] {
            let key = format!("{table}.since");
            let dir = tmp(&format!("{table}-since"));
            let path = dir.join("c.toml");
            for bad in ["7x", "d", "-1d", ""] {
                std::fs::write(&path, format!("[{table}]\nsince = \"{bad}\"\n")).unwrap();
                let errs = issues(&path).unwrap();
                assert!(
                    errs.iter()
                        .any(|e| e.contains(&key) && e.contains("c.toml:2")),
                    "{key} = {bad:?}: {errs:?}"
                );
            }
            for ok in ["30d", "12h", "7"] {
                std::fs::write(&path, format!("[{table}]\nsince = \"{ok}\"\n")).unwrap();
                assert!(issues(&path).unwrap().is_empty(), "{key} = {ok}");
            }

            let home = tmp(&format!("{table}-since-set"));
            Config::init(&home, false).unwrap();
            let before = std::fs::read_to_string(Config::path_for(&home)).unwrap();
            assert!(set(&home, &key, "7x", false).is_err(), "{key}");
            assert_eq!(
                std::fs::read_to_string(Config::path_for(&home)).unwrap(),
                before,
                "a refused set leaves the file unchanged"
            );
            set(&home, &key, "12h", false).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }
    }

    /// T365: values from the project file, `.env` and the environment go through the same rules,
    /// each message names its layer, and the file layers `issues` already read are not repeated.
    #[test]
    fn layered_values_are_checked_and_name_their_layer() {
        let v = |key: &str, value: FigValue, source: &str| (key.to_string(), value, source.into());
        let errs = layered_issues(vec![
            v("proxy.port", FigValue::from(70000_u32), "env"),
            v(
                "plugins.read.delta_max_ratio",
                FigValue::from(5.0_f32),
                "project",
            ),
            v("log.level", FigValue::from("debug"), "env"),
            v("proxy.port", FigValue::from(0_u32), "user"),
            v("log.path", FigValue::from("123"), "dotenv"),
        ]);
        assert_eq!(errs.len(), 2, "{errs:?}");
        assert!(
            errs.iter()
                .any(|e| e.starts_with("env: proxy.port out of range"))
        );
        assert!(
            errs.iter()
                .any(|e| e.starts_with("project: plugins.read.delta_max_ratio"))
        );
    }

    /// T363: the `(0, 1]` float keys and the `embed_backend` set go through the one rule table,
    /// for `validate` and for `set` alike.
    #[test]
    fn unit_ratio_keys_and_embed_backend_are_range_checked() {
        let dir = tmp("ratio");
        let path = dir.join("c.toml");
        let cases = [
            ("plugins.proxy.semantic_cache", "threshold", "-1"),
            ("plugins.proxy.semantic_cache", "threshold", "0"),
            ("plugins.proxy.semantic_cache", "threshold", "5"),
            ("plugins.proxy.semantic_cache", "threshold", "1.5"),
            ("plugins.read", "delta_max_ratio", "-3"),
            ("plugins.read", "delta_max_ratio", "0.0"),
            (
                "plugins.proxy.semantic_cache",
                "embed_backend",
                "\"openai\"",
            ),
        ];
        for (table, key, bad) in cases {
            std::fs::write(&path, format!("[{table}]\n{key} = {bad}\n")).unwrap();
            let errs = issues(&path).unwrap();
            assert!(
                errs.iter().any(|e| e.contains(key)),
                "{key} = {bad}: {errs:?}"
            );
        }
        for (table, key, ok) in [
            ("plugins.proxy.semantic_cache", "threshold", "0.99"),
            ("plugins.proxy.semantic_cache", "threshold", "1"),
            ("plugins.read", "delta_max_ratio", "0.6"),
            ("plugins.proxy.semantic_cache", "embed_backend", "\"hash\""),
        ] {
            std::fs::write(&path, format!("[{table}]\n{key} = {ok}\n")).unwrap();
            assert!(issues(&path).unwrap().is_empty(), "{key} = {ok}");
        }

        let home = tmp("ratio-set");
        Config::init(&home, false).unwrap();
        let before = std::fs::read_to_string(Config::path_for(&home)).unwrap();
        for (key, bad) in [
            ("plugins.proxy.semantic_cache.threshold", "-1"),
            ("plugins.proxy.semantic_cache.threshold", "0"),
            ("plugins.proxy.semantic_cache.threshold", "5"),
            ("plugins.read.delta_max_ratio", "-3"),
            ("plugins.proxy.semantic_cache.embed_backend", "openai"),
        ] {
            assert!(set(&home, key, bad, false).is_err(), "{key} = {bad}");
        }
        assert_eq!(
            std::fs::read_to_string(Config::path_for(&home)).unwrap(),
            before,
            "a refused set leaves the file unchanged"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn flex_policy_values_are_checked() {
        let dir = tmp("flex");
        let path = dir.join("c.toml");
        for (key, bad) in [("on_429", "\"retry\""), ("retries", "6")] {
            std::fs::write(&path, format!("[proxy.flex]\n{key} = {bad}\n")).unwrap();
            let errs = issues(&path).unwrap();
            assert!(
                errs.iter().any(|e| e.contains(key)),
                "{key} = {bad}: {errs:?}"
            );
        }
        for (key, ok) in [
            ("on_429", "\"backoff\""),
            ("on_429", "\"default\""),
            ("retries", "5"),
        ] {
            std::fs::write(&path, format!("[proxy.flex]\n{key} = {ok}\n")).unwrap();
            assert!(issues(&path).unwrap().is_empty(), "{key} = {ok}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_keeps_proxy_comment() {
        let home = tmp("set");
        Config::init(&home, false).unwrap();
        set(&home, "proxy.port", "8791", false).unwrap();
        let text = std::fs::read_to_string(Config::path_for(&home)).unwrap();
        assert!(
            text.contains("port            = 8791") || text.contains("port = 8791"),
            "{text}"
        );
        assert!(text.contains("# rtok proxy"), "{text}");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// `set` swaps the value in place: the key's padding and its trailing doc comment stay.
    #[test]
    fn set_keeps_the_trailing_comment_and_padding() {
        let home = tmp("decor");
        Config::init(&home, false).unwrap();
        set(&home, "proxy.mode", "compress", false).unwrap();
        let text = std::fs::read_to_string(Config::path_for(&home)).unwrap();
        assert!(
            text.contains("mode            = \"compress\"       # passthrough | compress"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The loader accepts an inline table; validate used to report `expected table` for it.
    #[test]
    fn an_inline_table_is_a_table() {
        let dir = tmp("inline");
        let path = dir.join("c.toml");
        std::fs::write(&path, "proxy = { port = 2 }\n").unwrap();
        assert!(issues(&path).unwrap().is_empty());
        std::fs::write(&path, "proxy = { port = 70000 }\n").unwrap();
        let errs = issues(&path).unwrap();
        assert!(errs.iter().any(|e| e.contains("proxy.port")), "{errs:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A negative value for an unsigned key passed validation and then failed `Config::load`.
    #[test]
    fn a_negative_number_is_rejected_for_an_unsigned_key() {
        let dir = tmp("negative");
        let path = dir.join("c.toml");
        std::fs::write(&path, "[core]\nretain_calls_days = -1\n").unwrap();
        let errs = issues(&path).unwrap();
        assert!(
            errs.iter()
                .any(|e| e.contains("core.retain_calls_days") && e.contains("≥ 0")),
            "{errs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::super::layers;
    use rstest::rstest;

    /// T36.7: `config path` / `set` / `get` / `validate` all resolve `--config`.
    #[rstest]
    fn config_subcommands_honour_config_flag() {
        let home = tmp("cfg-flag");
        let ci = home.join("ci.toml");
        std::fs::write(&ci, "[proxy]\nport = 1111\n").unwrap();

        assert_eq!(Config::user_path(&home, Some(&ci)), ci);

        set_with(&home, Some(&ci), "proxy.port", "2222", false).unwrap();
        assert!(
            std::fs::read_to_string(&ci).unwrap().contains("2222"),
            "set must write --config file"
        );
        assert!(
            !Config::path_for(&home).exists(),
            "set must not write <home>/config.toml"
        );

        let cfg = layers::load(&home, Some(&ci), None).unwrap();
        assert_eq!(cfg.proxy.port, 2222);

        let errs = issues(&ci).unwrap();
        assert!(errs.is_empty(), "{errs:?}");
        let ok = format!("ok {}", ci.display());
        assert!(
            ok.contains(&ci.display().to_string()),
            "validate must name the config path: {ok}"
        );

        let _ = std::fs::remove_dir_all(&home);
    }
}

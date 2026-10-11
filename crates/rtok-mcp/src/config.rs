//! Read, write and remove one MCP entry in a host's config file, whatever shape that host uses
//! (T277): strict JSON, JSONC (comments and formatting kept), or TOML (formatting kept via
//! `toml_edit`). Every host walks the same dotted `key_path`; the [`spec::Format`] is the only
//! thing that changes what runs underneath.
//!
//! Per `rust.md`'s "Config files" rule: this is the one module that owns this I/O, and it is
//! deliberately schema-less — a foreign config gets no schema from rtok. It reads and writes
//! only `key_path.<server.name>` and leaves every other key exactly as it found it.
//!
//! All I/O goes through [`Fs`], so a test plugs in an in-memory map instead of the real disk
//! (this PR's own test does exactly that). The disk-backed `Fs` a host installer uses lands
//! with the first host (T277 PR 2) and must write atomically and back up before every change,
//! the same contract `rtok_agent_sdk::write_atomic` / `backup` already give real files.

use std::path::Path;

use anyhow::{Context, Result};
use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use rtok_agent_sdk::{parse_strict_json, without_default_type};
use serde_json::Value;

use crate::spec::{Format, McpSpec};

/// The disk (or in-memory stand-in) [`read_entry`]/[`write_entry`]/[`remove_entry`] go through.
/// A real implementation must write atomically and name the file in any parse error (`rust.md`);
/// `backup`'s default is a no-op, which is only valid for a throwaway or in-memory `Fs` — a
/// disk-backed one reuses `rtok_agent_sdk::backup` instead of a second copy of that logic.
pub trait Fs {
    /// Bytes at `path`, or `None` if it does not exist.
    fn read(&self, path: &Path) -> Option<Vec<u8>>;
    /// Overwrite (or create) `path` with `bytes`.
    fn write(&mut self, path: &Path, bytes: Vec<u8>) -> Result<()>;
    /// Best-effort copy of `path`'s current contents before it changes.
    fn backup(&mut self, _path: &Path) -> Result<()> {
        Ok(())
    }
}

/// Whether a write actually changed the file — the "no changes" gate every host installer keeps
/// today (`rtok_agent_sdk::NO_CHANGES`), now computed in one place instead of per host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Written {
    Changed,
    Unchanged,
}

/// The entry at `spec.key_path.<spec.server.name>`, or `None` if the file, the key, or the name
/// is absent — never a substring match on the raw text (the bug `src/agents/mcp.rs::has_entry`
/// already fixed for the read side; this is the same rule for every format).
pub fn read_entry(fs: &impl Fs, spec: &McpSpec) -> Result<Option<Value>> {
    Ok(read_servers(fs, spec)?.and_then(|m| m.get(spec.server.name).cloned()))
}

/// Every server in the table at `spec.key_path`, whatever its name, or `None` if the file or the
/// table is absent. The doctor's duplicate check reads whole tables through this, so it parses
/// a host's file exactly as [`read_entry`] does.
pub fn read_servers(
    fs: &impl Fs,
    spec: &McpSpec,
) -> Result<Option<serde_json::Map<String, Value>>> {
    match spec.format {
        Format::Json => servers_json(fs, spec),
        Format::Jsonc => servers_jsonc(fs, spec),
        Format::Toml => servers_toml(fs, spec),
    }
}

/// Write `entry` at that path, keeping every other key as it was — JSONC comments and TOML's
/// own formatting survive because [`Format::Jsonc`]/[`Format::Toml`] edit the parsed document in
/// place instead of rebuilding it from a plain value. `Unchanged` when the file already holds
/// exactly `entry` there.
pub fn write_entry(fs: &mut impl Fs, spec: &McpSpec, entry: &Value) -> Result<Written> {
    // Claude.app drops the default `"type": "stdio"` when it re-saves its config (T265), so an
    // entry that differs only by that field is already ours.
    let same = |have: &Value| without_default_type(have) == without_default_type(entry);
    if read_entry(fs, spec)?.as_ref().is_some_and(same) {
        return Ok(Written::Unchanged);
    }
    match spec.format {
        Format::Json => write_json(fs, spec, Some(entry)),
        Format::Jsonc => write_jsonc(fs, spec, Some(entry)),
        Format::Toml => write_toml(fs, spec, Some(entry)),
    }
}

/// Remove the entry. `Unchanged` when there was nothing to remove.
pub fn remove_entry(fs: &mut impl Fs, spec: &McpSpec) -> Result<Written> {
    if read_entry(fs, spec)?.is_none() {
        return Ok(Written::Unchanged);
    }
    match spec.format {
        Format::Json => write_json(fs, spec, None),
        Format::Jsonc => write_jsonc(fs, spec, None),
        Format::Toml => write_toml(fs, spec, None),
    }
}

// ---- JSON (strict; T79 refuses to rewrite a file that fails to parse as plain JSON) ----

fn parse_json(bytes: &[u8], path: &Path) -> Result<Value> {
    let raw =
        std::str::from_utf8(bytes).with_context(|| format!("{}: not UTF-8", path.display()))?;
    parse_strict_json(path, raw)
}

fn servers_json(fs: &impl Fs, spec: &McpSpec) -> Result<Option<serde_json::Map<String, Value>>> {
    let Some(bytes) = fs.read(&spec.config_path) else {
        return Ok(None);
    };
    let root = parse_json(&bytes, &spec.config_path)?;
    Ok(walk_json(&root, &spec.key_path).cloned())
}

fn walk_json<'a>(
    root: &'a Value,
    key_path: &[String],
) -> Option<&'a serde_json::Map<String, Value>> {
    key_path.iter().try_fold(root, |v, k| v.get(k))?.as_object()
}

fn write_json(fs: &mut impl Fs, spec: &McpSpec, entry: Option<&Value>) -> Result<Written> {
    let mut root = match fs.read(&spec.config_path) {
        Some(bytes) => parse_json(&bytes, &spec.config_path)?,
        None => Value::Object(Default::default()),
    };
    set_json_at(&mut root, &spec.key_path, spec.server.name, entry).with_context(|| {
        format!(
            "{}: the root or {} is not an object",
            spec.config_path.display(),
            spec.key_path.join(".")
        )
    })?;
    let mut body = serde_json::to_string_pretty(&root)?;
    body.push('\n');
    fs.backup(&spec.config_path)?;
    fs.write(&spec.config_path, body.into_bytes())?;
    Ok(Written::Changed)
}

/// Insert or remove `name` under the dotted `key_path` inside `node`: creates intermediate
/// objects for a write, and drops them again once they empty out for a remove, so a removed
/// host entry leaves the file exactly as it read before rtok ever touched it. A node that is not
/// an object (root or table) is an error, never replaced: the user's value stays as it was.
fn set_json_at(
    node: &mut Value,
    key_path: &[String],
    name: &str,
    entry: Option<&Value>,
) -> Result<()> {
    let Some(obj) = node.as_object_mut() else {
        anyhow::bail!("not an object");
    };
    let Some((head, rest)) = key_path.split_first() else {
        match entry {
            Some(v) => {
                obj.insert(name.to_string(), v.clone());
            }
            None => {
                obj.remove(name);
            }
        }
        return Ok(());
    };
    if entry.is_none() && !obj.contains_key(head) {
        return Ok(()); // nothing to remove
    }
    let child = obj
        .entry(head.clone())
        .or_insert_with(|| Value::Object(Default::default()));
    set_json_at(child, rest, name, entry)?;
    if entry.is_none() && child.as_object().is_some_and(serde_json::Map::is_empty) {
        obj.remove(head);
    }
    Ok(())
}

// ---- JSONC (comments and formatting kept via jsonc-parser's lossless CST) ----

fn jsonc_options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        // 0.34 adds JSON5. Stay on comments and trailing commas only.
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

fn parse_jsonc(bytes: Option<Vec<u8>>, path: &Path) -> Result<CstRootNode> {
    let text = bytes
        .map(String::from_utf8)
        .transpose()
        .with_context(|| format!("{}: not UTF-8", path.display()))?
        .unwrap_or_default();
    CstRootNode::parse(&text, &jsonc_options())
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

fn servers_jsonc(fs: &impl Fs, spec: &McpSpec) -> Result<Option<serde_json::Map<String, Value>>> {
    let Some(bytes) = fs.read(&spec.config_path) else {
        return Ok(None);
    };
    let root = parse_jsonc(Some(bytes), &spec.config_path)?;
    let Some(mut obj) = root.object_value() else {
        return Ok(None);
    };
    for k in &spec.key_path {
        let Some(next) = obj.object_value(k) else {
            return Ok(None);
        };
        obj = next;
    }
    Ok(match obj.to_serde_value() {
        Some(Value::Object(m)) => Some(m),
        _ => None,
    })
}

fn write_jsonc(fs: &mut impl Fs, spec: &McpSpec, entry: Option<&Value>) -> Result<Written> {
    let root = parse_jsonc(fs.read(&spec.config_path), &spec.config_path)?;
    let mut obj = root
        .object_value_or_create()
        .with_context(|| format!("{}: root is not an object", spec.config_path.display()))?;
    for k in &spec.key_path {
        obj = obj
            .object_value_or_create(k)
            .with_context(|| format!("{}: {k} is not an object", spec.config_path.display()))?;
    }
    match entry {
        Some(v) => match obj.get(spec.server.name) {
            Some(prop) => prop.set_value(json_to_cst(v)),
            None => {
                obj.append(spec.server.name, json_to_cst(v));
            }
        },
        // Left as a (possibly now-empty) table: JSONC's surgical editor only ever removes the
        // one property it owns, never a container it did not create (T277 PR1 scope).
        None => {
            if let Some(prop) = obj.get(spec.server.name) {
                prop.remove();
            }
        }
    }
    fs.backup(&spec.config_path)?;
    fs.write(&spec.config_path, root.to_string().into_bytes())?;
    Ok(Written::Changed)
}

fn json_to_cst(v: &Value) -> CstInputValue {
    match v {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(a) => CstInputValue::Array(a.iter().map(json_to_cst).collect()),
        Value::Object(m) => {
            CstInputValue::Object(m.iter().map(|(k, v)| (k.clone(), json_to_cst(v))).collect())
        }
    }
}

// ---- TOML (formatting kept via toml_edit's document model) ----

fn parse_toml(bytes: &[u8], path: &Path) -> Result<toml_edit::DocumentMut> {
    if bytes.is_empty() {
        return Ok(toml_edit::DocumentMut::new());
    }
    let text =
        std::str::from_utf8(bytes).with_context(|| format!("{}: not UTF-8", path.display()))?;
    text.parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("{}: not valid TOML", path.display()))
}

fn servers_toml(fs: &impl Fs, spec: &McpSpec) -> Result<Option<serde_json::Map<String, Value>>> {
    let Some(bytes) = fs.read(&spec.config_path) else {
        return Ok(None);
    };
    let doc = parse_toml(&bytes, &spec.config_path)?;
    let mut table: &dyn toml_edit::TableLike = doc.as_table();
    for k in &spec.key_path {
        let Some(next) = table.get(k).and_then(toml_edit::Item::as_table_like) else {
            return Ok(None);
        };
        table = next;
    }
    Ok(Some(
        table
            .iter()
            .map(|(k, v)| (k.to_string(), toml_item_to_json(v)))
            .collect(),
    ))
}

fn write_toml(fs: &mut impl Fs, spec: &McpSpec, entry: Option<&Value>) -> Result<Written> {
    let mut doc = match fs.read(&spec.config_path) {
        Some(bytes) => parse_toml(&bytes, &spec.config_path)?,
        None => toml_edit::DocumentMut::new(),
    };
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for k in &spec.key_path {
        // Implicit: a created `[mcp]` above `[mcp.servers.rtok]` prints no empty header of its own.
        let mut created = toml_edit::Table::new();
        created.set_implicit(true);
        table = table
            .entry(k)
            .or_insert(toml_edit::Item::Table(created))
            .as_table_like_mut()
            .with_context(|| format!("{}: {k} is not a table", spec.config_path.display()))?;
    }
    match entry {
        Some(v) => {
            table.insert(spec.server.name, json_to_toml_item(v));
        }
        None => {
            table.remove(spec.server.name);
        }
    }
    fs.backup(&spec.config_path)?;
    fs.write(&spec.config_path, doc.to_string().into_bytes())?;
    Ok(Written::Changed)
}

fn toml_item_to_json(item: &toml_edit::Item) -> Value {
    match item {
        toml_edit::Item::None => Value::Null,
        toml_edit::Item::Value(v) => toml_value_to_json(v),
        toml_edit::Item::Table(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.to_string(), toml_item_to_json(v)))
                .collect(),
        ),
        toml_edit::Item::ArrayOfTables(a) => Value::Array(
            a.iter()
                .map(|t| {
                    Value::Object(
                        t.iter()
                            .map(|(k, v)| (k.to_string(), toml_item_to_json(v)))
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}

fn toml_value_to_json(v: &toml_edit::Value) -> Value {
    match v {
        toml_edit::Value::String(s) => Value::String(s.value().clone()),
        toml_edit::Value::Integer(i) => Value::Number((*i.value()).into()),
        toml_edit::Value::Float(f) => serde_json::Number::from_f64(*f.value())
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml_edit::Value::Boolean(b) => Value::Bool(*b.value()),
        toml_edit::Value::Datetime(d) => Value::String(d.value().to_string()),
        toml_edit::Value::Array(a) => Value::Array(a.iter().map(toml_value_to_json).collect()),
        toml_edit::Value::InlineTable(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.to_string(), toml_value_to_json(v)))
                .collect(),
        ),
    }
}

/// The entry itself as a standard table (`[mcp_servers.rtok]`, the shape Codex and Grok write
/// today), not an inline `rtok = { … }`; values inside it stay inline.
fn json_to_toml_item(v: &Value) -> toml_edit::Item {
    match v {
        Value::Object(m) => {
            let mut t = toml_edit::Table::new();
            for (k, x) in m {
                t.insert(k, toml_edit::Item::Value(json_to_toml_value(x)));
            }
            toml_edit::Item::Table(t)
        }
        _ => toml_edit::Item::Value(json_to_toml_value(v)),
    }
}

fn json_to_toml_value(v: &Value) -> toml_edit::Value {
    match v {
        // TOML has no null; an entry rtok writes never contains one (command/args/env are all
        // strings), so this is an unreachable fallback, not a real conversion choice.
        Value::Null => toml_edit::Value::from(String::new()),
        Value::Bool(b) => toml_edit::Value::from(*b),
        Value::Number(n) => n
            .as_i64()
            .map(toml_edit::Value::from)
            .or_else(|| n.as_f64().map(toml_edit::Value::from))
            .unwrap_or_else(|| toml_edit::Value::from(0i64)),
        Value::String(s) => toml_edit::Value::from(s.clone()),
        Value::Array(a) => {
            let mut arr = toml_edit::Array::new();
            for x in a {
                arr.push(json_to_toml_value(x));
            }
            toml_edit::Value::Array(arr)
        }
        Value::Object(m) => {
            let mut t = toml_edit::InlineTable::new();
            for (k, x) in m {
                t.insert(k.as_str(), json_to_toml_value(x));
            }
            toml_edit::Value::InlineTable(t)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::registry::RTOK;
    use crate::spec::{Client, DuplicateName, EntryShape, Launch};

    #[derive(Default)]
    struct MemFs(HashMap<PathBuf, Vec<u8>>);

    impl Fs for MemFs {
        fn read(&self, path: &Path) -> Option<Vec<u8>> {
            self.0.get(path).cloned()
        }
        fn write(&mut self, path: &Path, bytes: Vec<u8>) -> Result<()> {
            self.0.insert(path.to_path_buf(), bytes);
            Ok(())
        }
    }

    fn spec(format: Format) -> McpSpec {
        McpSpec {
            host: "test",
            client: Client::Cli,
            config_path: PathBuf::from("/cfg"),
            format,
            key_path: McpSpec::key_path("mcpServers"),
            server: RTOK,
            launch: Launch::default(),
            entry_shape: EntryShape::Command,
            duplicate_name: DuplicateName::ShowsBoth,
            plugin_serves: None,
        }
    }

    /// T328: a root or servers table of the wrong type is an error that leaves the file
    /// untouched (as JSONC and TOML do), not silently replaced by `{}`.
    #[test]
    fn strict_json_refuses_a_non_object_root_or_table_and_keeps_the_file() {
        let entry = json!({"command": "rtok", "args": ["mcp"]});
        for raw in [
            "[1, 2]",
            "null",
            "\"text\"",
            r#"{"mcpServers": []}"#,
            r#"{"mcpServers": null}"#,
            r#"{"mcpServers": "x"}"#,
        ] {
            let mut fs = MemFs::default();
            fs.0.insert(PathBuf::from("/cfg"), raw.as_bytes().to_vec());
            let spec = spec(Format::Json);
            assert!(write_entry(&mut fs, &spec, &entry).is_err(), "write {raw}");
            // Nothing of ours to remove: a no-op, still the user's bytes.
            let _ = remove_entry(&mut fs, &spec);
            assert_eq!(fs.0[&PathBuf::from("/cfg")], raw.as_bytes(), "{raw}");
        }
    }
}

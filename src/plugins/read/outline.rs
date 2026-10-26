// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `mode=map` / `mode=signatures` via tree-sitter-tags (plan T4.3) or a line scan for
//! Markdown (T68.8); `mode=stripped` via tree-sitter comment nodes (plan T50.3).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::Result;
use tree_sitter_tags::{TagsConfiguration, TagsContext};

/// One tags-query hit (definition or reference).
#[derive(Debug, Clone)]
pub struct TagHit {
    pub kind: String,
    pub name: String,
    pub line: usize,
    /// Last line of the tagged node, from the tag's byte range (T8.5). A reference spans
    /// one line, so `end_line == line`; a definition covers its whole body.
    pub end_line: usize,
    /// First byte of the tagged node (`tag.range.start`).
    pub start_byte: usize,
    /// One past the last byte of the tagged node (`tag.range.end`).
    pub end_byte: usize,
    pub is_def: bool,
    pub line_text: String,
    /// Full import specifier when `kind == "import"` (T368). `name` stays the last segment.
    pub import_path: String,
}

/// True when `path` has a tags-supported extension (cheap; does not parse).
pub fn supported(path: &Path) -> bool {
    grammar_for_ext(path.extension().and_then(OsStr::to_str)).is_some()
}

/// Whether a grammar name from `[plugins.graph.extensions]` is available in this build.
pub fn grammar_available(grammar: &str) -> bool {
    grammar_for_ext(Some(grammar)).is_some()
}

fn grammar_for_ext(ext: Option<&str>) -> Option<&'static str> {
    match ext? {
        #[cfg(feature = "lang-rust")]
        "rs" | "rust" => Some("rust"),
        #[cfg(feature = "lang-ts")]
        "ts" => Some("ts"),
        #[cfg(feature = "lang-ts")]
        "tsx" => Some("tsx"),
        #[cfg(feature = "lang-js")]
        "js" | "mjs" | "cjs" => Some("js"),
        #[cfg(feature = "lang-python")]
        "py" => Some("py"),
        #[cfg(feature = "lang-dart")]
        "dart" => Some("dart"),
        #[cfg(feature = "lang-c")]
        "c" => Some("c"),
        #[cfg(feature = "lang-c")]
        "h" => Some("h"),
        #[cfg(feature = "lang-go")]
        "go" => Some("go"),
        _ => None,
    }
}

/// True when `path` is indexable with built-in extensions or `extensions` remap (T68.10).
pub fn supported_with(path: &Path, extensions: &HashMap<String, String>) -> bool {
    if supported(path) {
        return true;
    }
    let Some(ext) = path.extension().and_then(OsStr::to_str) else {
        return false;
    };
    extensions
        .get(ext)
        .is_some_and(|grammar| grammar_available(grammar))
}

/// Definitions and references from the grammar's tags query. Unknown language → empty.
pub fn tags(path: &Path, src: &str) -> Result<Vec<TagHit>> {
    tags_with_extensions(path, src, &HashMap::new())
}

/// Like [`tags`], but `[plugins.graph.extensions]` can remap the file suffix (T68.10).
pub fn tags_with_extensions(
    path: &Path,
    src: &str,
    extensions: &HashMap<String, String>,
) -> Result<Vec<TagHit>> {
    let Some(cfg) = config_with_extensions(path, extensions) else {
        return Ok(Vec::new());
    };
    let cfg = cfg?;
    let mut ctx = TagsContext::new();
    let (tags, _) = ctx
        .generate_tags(cfg, src.as_bytes(), None)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    // Byte offset of each line start; `partition_point` turns a byte into a 1-based line.
    let starts: Vec<usize> = std::iter::once(0)
        .chain(src.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |b: usize| starts.partition_point(|&s| s <= b).max(1);
    let mut out = Vec::new();
    for tag in tags.flatten() {
        let Some(name) = src.get(tag.name_range.clone()) else {
            continue;
        };
        let line_text = src.get(tag.line_range.clone()).unwrap_or("").to_string();
        let kind = cfg.syntax_type_name(tag.syntax_type_id).to_string();
        let line = tag.span.start.row + 1;
        let end_line = line_of(tag.range.end.saturating_sub(1));
        let start_byte = tag.range.start;
        let end_byte = tag.range.end;
        let is_def = tag.is_definition;
        let items = if kind == "import" {
            expand_import(name)
        } else {
            vec![(String::new(), name.to_string())]
        };
        for (import_path, item) in items {
            out.push(TagHit {
                kind: kind.clone(),
                name: item,
                line,
                end_line,
                start_byte,
                end_byte,
                is_def,
                line_text: line_text.clone(),
                import_path,
            });
        }
    }
    // The bare last-segment capture keeps that node an import; the full clause is the row we store.
    let full: Vec<(usize, String)> = out
        .iter()
        .filter(|h| h.kind == "import" && h.import_path.contains([':', '/', '.', '\\']))
        .map(|h| (h.line, h.name.clone()))
        .collect();
    out.retain(|h| {
        h.kind != "import"
            || h.import_path.contains([':', '/', '.', '\\'])
            || !full
                .iter()
                .any(|(line, name)| *line == h.line && name == &h.name)
    });
    Ok(out)
}

/// One ATX heading: 1-based line, `#` level, full trimmed line, first body line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdHeading {
    pub line: usize,
    pub level: usize,
    pub text: String,
    pub body: Option<String>,
}

/// `#` headings with the first non-empty body line; fenced blocks skipped (T68.8).
pub fn markdown_headings(src: &str) -> Vec<MdHeading> {
    let mut out = Vec::new();
    let (mut fenced, mut slot) = (false, None);
    for (i, line) in src.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("```") {
            fenced = !fenced;
        } else if fenced {
        } else if t.starts_with('#') {
            let level = t.chars().take_while(|c| *c == '#').count().clamp(1, 6);
            out.push(MdHeading {
                line: i + 1,
                level,
                text: t.to_string(),
                body: None,
            });
            slot = Some(out.len() - 1);
        } else if let Some(j) = slot
            && !t.is_empty()
        {
            out[j].body = Some(t.to_string());
            slot = None;
        }
    }
    out
}

/// Heading map for skill digest: full heading line plus indented first body line.
pub fn markdown_digest(src: &str) -> String {
    let mut s = String::new();
    for h in markdown_headings(src) {
        s.push_str(&h.text);
        s.push('\n');
        if let Some(b) = &h.body {
            s.push_str("  ");
            s.push_str(b);
            s.push('\n');
        }
    }
    s
}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md" | "mdx")
    )
}

/// One `llms.txt` list link. The URL is recorded, never fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmsLink {
    pub title: String,
    pub url: String,
    pub description: String,
}

/// `llms.txt` and `llms-full.txt` are markdown with a `.txt` name, so
/// `is_markdown` does not see them.
pub fn is_llms_txt(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("llms.txt" | "llms-full.txt")
    )
}

/// `- [title](url)` and `* [title](url): note`. Fenced blocks are skipped.
pub fn llms_links(src: &str) -> Vec<LlmsLink> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in src.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(link) = parse_llms_link(t) {
            out.push(link);
        }
    }
    out
}

fn parse_llms_link(line: &str) -> Option<LlmsLink> {
    let rest = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))?
        .trim();
    let rest = rest.strip_prefix('[')?;
    let (title, rest) = rest.split_once("](")?;
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    let (url, tail) = rest.split_once(')')?;
    let url = url.trim();
    if url.is_empty() || url.contains(char::is_whitespace) {
        return None;
    }
    let description = tail
        .trim()
        .strip_prefix(':')
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    Some(LlmsLink {
        title: title.to_string(),
        url: url.to_string(),
        description,
    })
}

fn render_llms_links(links: &[LlmsLink]) -> String {
    links
        .iter()
        .map(|link| {
            let mut row = format!("link {} {}", link.title, link.url);
            if !link.description.is_empty() {
                row.push_str(" — ");
                row.push_str(&link.description);
            }
            row
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn markdown_render(headings: &[MdHeading], mode: &str) -> String {
    if mode == "signatures" {
        return headings
            .iter()
            .map(|h| h.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
    }
    headings
        .iter()
        .map(|h| {
            let title = h.text.trim_start_matches('#').trim();
            let mut row = format!("h{} {} {}", h.level, title, h.line);
            if let Some(b) = &h.body {
                row.push_str(&format!("\n  {b}"));
            }
            row
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One row per named item: `(full specifier, last segment)`. A brace group is one node (T368).
fn expand_import(raw: &str) -> Vec<(String, String)> {
    let t = raw.trim().trim_matches(|c| matches!(c, '"' | '\'' | '`'));
    let t = t.trim().trim_end_matches(';').trim();
    expand_clause("", t)
}

fn expand_clause(prefix: &str, clause: &str) -> Vec<(String, String)> {
    let clause = clause
        .split(" as ")
        .next()
        .unwrap_or(clause)
        .trim()
        .trim_end_matches("::*")
        .trim();
    if let Some(start) = clause.find('{') {
        let end = clause.rfind('}').unwrap_or(clause.len());
        let head = clause[..start].trim().trim_end_matches("::").trim();
        let next = if prefix.is_empty() {
            head.to_string()
        } else if head.is_empty() {
            prefix.to_string()
        } else {
            format!("{prefix}::{head}")
        };
        let mut out = Vec::new();
        for item in split_commas(&clause[start + 1..end]) {
            out.extend(expand_clause(&next, item));
        }
        return out;
    }
    if clause.is_empty() || clause == "*" || clause == "self" {
        return if prefix.is_empty() {
            Vec::new()
        } else {
            one_import(prefix)
        };
    }
    let full = if prefix.is_empty() {
        clause.to_string()
    } else {
        format!("{prefix}::{clause}")
    };
    one_import(&full)
}

fn one_import(path: &str) -> Vec<(String, String)> {
    let path = path.trim();
    if path.is_empty() {
        return Vec::new();
    }
    vec![(path.to_string(), import_last_segment(path))]
}

/// Commas at brace depth 0, so `a::{b, c}` stays one item inside an outer list.
fn split_commas(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in body.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&body[start..]);
    out
}

/// Last path segment of an import / use / require specifier (T68.6).
fn import_last_segment(raw: &str) -> String {
    let t = raw.trim().trim_matches(|c| matches!(c, '"' | '\'' | '`'));
    let t = t
        .strip_suffix(".dart")
        .or_else(|| t.strip_suffix(".tsx"))
        .or_else(|| t.strip_suffix(".ts"))
        .or_else(|| t.strip_suffix(".mjs"))
        .or_else(|| t.strip_suffix(".cjs"))
        .or_else(|| t.strip_suffix(".jsx"))
        .or_else(|| t.strip_suffix(".js"))
        .unwrap_or(t);
    t.rsplit([':', '.', '/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(t)
        .to_string()
}

/// Definitions as `kind name line`, or verbatim definition lines.
pub fn render(path: &Path, src: &str, mode: &str) -> Result<String> {
    if is_markdown(path) || is_llms_txt(path) {
        let hs = markdown_headings(src);
        let links = if is_llms_txt(path) {
            llms_links(src)
        } else {
            Vec::new()
        };
        if hs.is_empty() && links.is_empty() {
            return Ok(fallback(src));
        }
        let mut out = if hs.is_empty() {
            String::new()
        } else {
            markdown_render(&hs, mode)
        };
        if !links.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&render_llms_links(&links));
        }
        return Ok(out);
    }
    let hits = tags(path, src)?;
    let mut seen = std::collections::HashSet::new();
    let imports: Vec<&str> = hits
        .iter()
        .filter(|h| h.kind == "import" && !h.is_def && seen.insert(h.name.as_str()))
        .map(|h| h.name.as_str())
        .collect();
    let defs: Vec<&TagHit> = hits.iter().filter(|h| h.is_def).collect();
    if defs.is_empty() && imports.is_empty() {
        return Ok(fallback(src));
    }
    let mut out = Vec::new();
    if !imports.is_empty() && mode != "signatures" {
        out.push(format!("imports: {}", imports.join(", ")));
    }
    for hit in defs {
        if mode == "signatures" {
            out.push(hit.line_text.trim_end().to_string());
        } else {
            let kw = hit
                .line_text
                .split_whitespace()
                .next()
                .unwrap_or(hit.kind.as_str());
            out.push(format!("{} {} {}", kw, hit.name, hit.line));
        }
    }
    Ok(out.join("\n"))
}

fn fallback(src: &str) -> String {
    let body = src
        .lines()
        .take(60)
        .enumerate()
        .map(|(i, l)| format!("{}:{l}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{body}\n(note: unknown language, showing lines 1-60)")
}

/// A language's compiled tags query, kept for the process. A failed compile keeps its message,
/// so every later call gets the same `Err` rather than a retry.
type Cached = OnceLock<Result<TagsConfiguration, String>>;

fn cached(
    cell: &'static Cached,
    build: impl FnOnce() -> Result<TagsConfiguration, tree_sitter_tags::Error>,
) -> Option<Result<&'static TagsConfiguration>> {
    let cfg = cell.get_or_init(|| build().map_err(|e| format!("tags query: {e}")));
    Some(cfg.as_ref().map_err(|e| anyhow::anyhow!("{e}")))
}

/// One `static` cell per match arm, so each language compiles at most once.
macro_rules! compiled {
    ($lang:expr, $tags:expr, $locals:expr) => {{
        static CELL: Cached = OnceLock::new();
        cached(&CELL, || {
            TagsConfiguration::new($lang.into(), $tags, $locals)
        })
    }};
}

/// The upstream Rust tags query only sees bare and method calls; path-qualified calls
/// (`tokens::estimate(..)`) are the common form, so `callers` (T8.2) needs this pattern too.
#[cfg(feature = "lang-rust")]
pub(crate) const RUST_SCOPED_CALL: &str = "
(call_expression
    function: (scoped_identifier
        name: (identifier) @name)) @reference.call
";

/// `use` as `kind = import` (T68.6). Whole-clause captures feed `expand_import`; the inner ones claim the last segment (T368).
#[cfg(feature = "lang-rust")]
pub(crate) const RUST_IMPORT: &str = "
(use_declaration argument: (identifier) @name) @reference.import
(use_declaration argument: (scoped_identifier) @name) @reference.import
(use_declaration argument: (scoped_identifier name: (identifier) @name)) @reference.import
(use_declaration argument: (use_as_clause) @name) @reference.import
(use_declaration argument: (use_as_clause path: (identifier) @name)) @reference.import
(use_declaration argument: (use_as_clause path: (scoped_identifier name: (identifier) @name))) @reference.import
(use_declaration argument: (use_list) @name) @reference.import
(use_declaration argument: (scoped_use_list) @name) @reference.import
(use_list (identifier) @name) @reference.import
(use_list (scoped_identifier name: (identifier) @name)) @reference.import
(use_list (use_as_clause path: (identifier) @name)) @reference.import
(use_list (use_as_clause path: (scoped_identifier name: (identifier) @name))) @reference.import
(use_declaration argument: (use_wildcard) @name) @reference.import
(use_wildcard (identifier) @name) @reference.import
(use_wildcard (scoped_identifier name: (identifier) @name)) @reference.import
";

/// `import` / `require` last path segment (T68.6). Shared by JS and TS/TSX.
#[cfg(any(feature = "lang-js", feature = "lang-ts"))]
pub(crate) const JS_IMPORT: &str = "
(import_statement source: (string) @name) @reference.import
(import_specifier name: (identifier) @name) @reference.import
(call_expression
    function: (identifier) @doc
    arguments: (arguments (string) @name)
    (#eq? @doc \"require\")) @reference.import
";

/// `import` / `from … import` last path segment (T68.6).
#[cfg(feature = "lang-python")]
pub(crate) const PYTHON_IMPORT: &str = "
(import_statement name: (dotted_name) @name) @reference.import
(import_statement name: (aliased_import name: (dotted_name) @name)) @reference.import
(import_from_statement name: (dotted_name) @name) @reference.import
(import_from_statement name: (aliased_import name: (dotted_name) @name)) @reference.import
(import_from_statement module_name: (dotted_name) @name) @reference.import
";

/// `import \"path\"` last path segment (T68.6).
#[cfg(feature = "lang-go")]
pub(crate) const GO_IMPORT: &str = "
(import_spec path: (interpreted_string_literal) @name) @reference.import
(import_spec path: (raw_string_literal) @name) @reference.import
";

/// `import 'uri'` last path segment (T68.6).
#[cfg(feature = "lang-dart")]
pub(crate) const DART_IMPORT: &str = "
(import_specification uri: [(configurable_uri) (uri)] @name) @reference.import
";

/// Kotlin tags, hand-written: tree-sitter-kotlin-ng 1.1.0 ships no tags query (T52.2), and the
/// grammar crate itself is not linked.
#[cfg(feature = "lang-kotlin")]
pub(crate) const KOTLIN_TAGS: &str = "
(class_declaration name: (identifier) @name) @definition.class
(object_declaration name: (identifier) @name) @definition.class
(function_declaration name: (identifier) @name) @definition.function
";

/// C# tags, hand-written: tree-sitter-c-sharp 0.23.5 gates `TAGS_QUERY` behind
/// `cfg(with_tags_query)`, and the grammar crate itself is not linked.
#[cfg(feature = "lang-csharp")]
pub(crate) const CSHARP_TAGS: &str = "
(class_declaration name: (identifier) @name) @definition.class
(interface_declaration name: (identifier) @name) @definition.interface
(method_declaration name: (identifier) @name) @definition.method
(namespace_declaration name: (identifier) @name) @definition.module
";

/// T52.5: the upstream Rust tags query sees no type positions (`Vec<Foo>`,
/// `Surface::Mcp`, `fn f(m: Manifest)` — 64 of the 74 T8.8 misses). A bare
/// `type_identifier` is every one of those sites; the `path` arm catches the
/// module half of a two-segment type path (`Surface` in `Surface::Mcp`; the
/// `Mcp` half is a `type_identifier` the bare arm already sees). Same-node
/// double capture is what the Go grammar's own tags query does
/// (`(type_identifier) @name @reference.type`).
///
/// The two `scoped_identifier` arms catch every `a::b` segment, so intermediate
/// path pieces count as references: `plugin` in `crate::plugin::Surface::Mcp`,
/// `Registry` in `Registry::new(..)`, `store`/`Store` in
/// `use crate::store::Store`. `self`/`crate`/`super` never match — the grammar
/// parses them as their own node types, not `identifier` (`Self::foo` does
/// record a `Self` row; one junk name, accepted). The final call name is already
/// claimed by an earlier pattern and one tag per node wins, so nothing counts
/// twice (verified on the `truth-constructs` fixture).
///
/// T663: macro arguments parse as an opaque `token_tree`, so `assert!(f())` and
/// `assert_eq!(r.m(), 1)` showed no call. Inside a `token_tree`, an identifier directly
/// followed by a parenthesised group is a call or tuple-struct use at any depth; the
/// identifier alone is captured (not the group), so two calls in one macro both count.
#[cfg(feature = "lang-rust")]
pub(crate) const RUST_EXTRA_REF: &str = "
(type_identifier) @name @reference.type
(scoped_type_identifier
    path: (identifier) @name) @reference.type
(scoped_identifier
    name: (identifier) @name) @reference.call
(scoped_identifier
    path: (identifier) @name) @reference.call
(token_tree
    (identifier) @name @reference.call
    .
    (token_tree))
";

/// T52.5: the upstream TypeScript tags query sees no calls at all — only
/// `type_annotation` and `new` with a bare constructor. Plain calls, member
/// calls (`r.method(..)`, the `scoped_identifier` analog, one nesting level),
/// and member constructions (`new ns.Foo()`) need these patterns; the bare
/// `type_identifier` arm catches generic arguments (`Array<Foo>`) the
/// `type_annotation` arm's direct-child match misses, and the `module` arm the
/// namespace half of `ns.Foo`. Receiver roots (`holder` in `holder.x()`) are
/// deliberately not captured — they are usually locals. Like the Rust extras,
/// these come last so an earlier pattern wins any same-node tie.
#[cfg(feature = "lang-ts")]
pub(crate) const TS_CALL_TYPE_REF: &str = "
(call_expression
    function: (identifier) @name) @reference.call
(call_expression
    function: (member_expression
        property: (property_identifier) @name)) @reference.call
(call_expression
    function: (member_expression
        object: (member_expression
            property: (property_identifier) @name))) @reference.call
(new_expression
    constructor: (member_expression
        property: (property_identifier) @name)) @reference.class
(type_identifier) @name @reference.type
(nested_type_identifier
    module: (identifier) @name) @reference.type
";

/// The query for `path`'s language, compiled on first use (T35.1): the compile was 19 ms of a
/// 26.5 ms `tags` call on `graph/index.rs` (debug, 2026-09-10), paid again on every file.
fn config_with_extensions(
    path: &Path,
    extensions: &HashMap<String, String>,
) -> Option<Result<&'static TagsConfiguration>> {
    let ext = path.extension()?.to_str()?;
    let grammar = extensions
        .get(ext)
        .map(String::as_str)
        .or_else(|| grammar_for_ext(Some(ext)));
    config_for_grammar(grammar)
}

fn config_for_grammar(grammar: Option<&str>) -> Option<Result<&'static TagsConfiguration>> {
    match grammar? {
        #[cfg(feature = "lang-rust")]
        "rs" | "rust" => compiled!(
            tree_sitter_rust::LANGUAGE,
            &format!(
                "{}{RUST_SCOPED_CALL}{RUST_IMPORT}{RUST_EXTRA_REF}",
                tree_sitter_rust::TAGS_QUERY
            ),
            ""
        ),
        #[cfg(feature = "lang-ts")]
        "ts" => compiled!(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            &format!(
                "{}{JS_IMPORT}{TS_CALL_TYPE_REF}",
                tree_sitter_typescript::TAGS_QUERY
            ),
            tree_sitter_typescript::LOCALS_QUERY
        ),
        #[cfg(feature = "lang-ts")]
        "tsx" => compiled!(
            tree_sitter_typescript::LANGUAGE_TSX,
            &format!(
                "{}{JS_IMPORT}{TS_CALL_TYPE_REF}",
                tree_sitter_typescript::TAGS_QUERY
            ),
            tree_sitter_typescript::LOCALS_QUERY
        ),
        #[cfg(feature = "lang-js")]
        "js" | "mjs" | "cjs" => compiled!(
            tree_sitter_javascript::LANGUAGE,
            &format!("{}{JS_IMPORT}", tree_sitter_javascript::TAGS_QUERY),
            tree_sitter_javascript::LOCALS_QUERY
        ),
        #[cfg(feature = "lang-python")]
        "py" => compiled!(
            tree_sitter_python::LANGUAGE,
            &format!("{}{PYTHON_IMPORT}", tree_sitter_python::TAGS_QUERY),
            ""
        ),
        #[cfg(feature = "lang-dart")]
        "dart" => compiled!(
            tree_sitter_dart::LANGUAGE,
            &format!("{}{DART_IMPORT}", tree_sitter_dart::TAGS_QUERY),
            tree_sitter_dart::LOCALS_QUERY
        ),
        #[cfg(feature = "lang-c")]
        "c" | "h" => compiled!(tree_sitter_c::LANGUAGE, tree_sitter_c::TAGS_QUERY, ""),
        #[cfg(feature = "lang-go")]
        "go" => compiled!(
            tree_sitter_go::LANGUAGE,
            &format!("{}{GO_IMPORT}", tree_sitter_go::TAGS_QUERY),
            ""
        ),
        _ => None,
    }
}

fn ts_lang(path: &Path) -> Option<tree_sitter::Language> {
    match path.extension()?.to_str()? {
        #[cfg(feature = "lang-rust")]
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        #[cfg(feature = "lang-ts")]
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        #[cfg(feature = "lang-ts")]
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        #[cfg(feature = "lang-js")]
        "js" | "mjs" | "cjs" => Some(tree_sitter_javascript::LANGUAGE.into()),
        #[cfg(feature = "lang-python")]
        "py" => Some(tree_sitter_python::LANGUAGE.into()),
        #[cfg(feature = "lang-dart")]
        "dart" => Some(tree_sitter_dart::LANGUAGE.into()),
        #[cfg(feature = "lang-c")]
        "c" | "h" => Some(tree_sitter_c::LANGUAGE.into()),
        #[cfg(feature = "lang-go")]
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        _ => None,
    }
}

/// Comments removed, newlines kept so original line numbers still match. `None` → no grammar
/// or parse fail; the caller serves `full` (T50.3).
pub fn stripped(path: &Path, src: &str) -> Option<String> {
    let lang = ts_lang(path)?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).ok()?;
    let tree = parser.parse(src, None)?;
    Some(drop_comments(src, tree.root_node()))
}

fn drop_comments(src: &str, root: tree_sitter::Node<'_>) -> String {
    let bytes = src.as_bytes();
    let mut drop = vec![false; bytes.len()];
    mark_comments(root, &mut drop);
    let mut out = Vec::with_capacity(bytes.len());
    for (i, &b) in bytes.iter().enumerate() {
        if !drop[i] || b == b'\n' {
            out.push(b);
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

fn mark_comments(node: tree_sitter::Node<'_>, drop: &mut [bool]) {
    if node.kind().contains("comment") {
        let start = node.start_byte().min(drop.len());
        let end = node.end_byte().min(drop.len());
        drop[start..end].fill(true);
        return;
    }
    let mut c = node.walk();
    for child in node.children(&mut c) {
        mark_comments(child, drop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(src: &str) -> String {
        let mut s = src.to_string();
        if !s.ends_with('\n') {
            s.push('\n');
        }
        let n = s.lines().count();
        for i in n..20 {
            s.push_str(&format!("// pad {i}\n"));
        }
        s
    }

    #[test]
    fn golden_per_language() {
        let cases = [
            ("a.rs", "fn main() {}\nfn helper() {}\n", "fn main"),
            (
                "a.ts",
                "function greet(n: string) { return n; }\n",
                "function greet",
            ),
            ("a.js", "function add(x) { return x; }\n", "function add"),
            ("a.py", "def run():\n    return 1\n", "def run"),
            ("a.dart", "void start() {}\n", "void start"),
            ("a.c", "int add(int x) { return x; }\n", "int add"),
            ("a.go", "func Sum() int { return 0 }\n", "func Sum"),
            ("a.java", "class App { void start() {} }\n", "class App"),
            ("a.kt", "class App { fun start() {} }\n", "class App"),
            ("a.swift", "class App { func start() {} }\n", "class App"),
            ("a.cs", "class App { void Start() {} }\n", "class App"),
            ("a.rb", "def run; end\n", "def run"),
            ("a.php", "<?php\nfunction run() {}\n", "function run"),
        ];
        for (path, src, needle) in cases {
            let src = pad(src);
            let map = render(Path::new(path), &src, "map").unwrap();
            assert!(map.contains(needle), "{path} map: {map}");
            let sig = render(Path::new(path), &src, "signatures").unwrap();
            let name = needle.split_whitespace().nth(1).unwrap_or(needle);
            assert!(
                sig.lines().any(|l| l.contains(name)),
                "{path} signatures: {sig}"
            );
        }
    }

    /// Same pointer for two files of one language: the query compiled once (T35.1). A
    /// recompile per call would hand back a fresh configuration each time.
    #[test]
    fn each_language_compiles_once() {
        let cfg = |p: &str| config_with_extensions(Path::new(p), &HashMap::new());
        let a = cfg("a.rs").unwrap().unwrap();
        let b = cfg("src/b.rs").unwrap().unwrap();
        assert!(std::ptr::eq(a, b), "a second .rs file recompiled the query");
        let ts = cfg("a.ts").unwrap().unwrap();
        assert!(
            !std::ptr::eq(a, ts),
            "two languages share one configuration"
        );
    }

    #[test]
    fn unknown_language_falls_back() {
        let src = "hello\nworld\n";
        let out = render(Path::new("a.txt"), src, "map").unwrap();
        assert!(out.contains("1:hello"), "{out}");
        assert!(out.contains("unknown language"), "{out}");
    }
    #[test]
    fn markdown_map_skips_fenced_headings() {
        let src =
            "# One\nFirst.\n```sh\n# not a heading\n```\n## Two\nSecond.\n### Three\nThird.\n";
        let hs = markdown_headings(src);
        assert_eq!(hs.len(), 3);
        assert_eq!(hs[0].body.as_deref(), Some("First."));
        assert_eq!(hs[1].text, "## Two");
        assert_eq!(hs[2].text, "### Three");
        let map = render(Path::new("doc.md"), src, "map").unwrap();
        assert!(map.contains("h1 One 1"));
        assert!(map.contains("h2 Two 6"));
        assert!(map.contains("h3 Three 8"));
        assert!(map.contains("  Second."));
        assert!(!map.contains("not a heading"), "{map}");
        let sig = render(Path::new("doc.mdx"), src, "signatures").unwrap();
        assert_eq!(sig.lines().count(), 3);
        let digest = markdown_digest(src);
        assert!(digest.contains("## Two\n  Second."));
        assert!(!digest.contains("not a heading"), "{digest}");
    }

    #[test]
    fn llms_txt_map_lists_links_and_skips_fences() {
        let src = "# Docs\nIntro.\n## Pages\n- [Real](https://example.com/a): a note\n```sh\n- [Fake](https://example.com/no)\n```\n- [Other](docs/b.md)\n";
        let links = llms_links(src);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].title, "Real");
        assert_eq!(links[0].url, "https://example.com/a");
        assert_eq!(links[0].description, "a note");
        assert_eq!(links[1].title, "Other");
        assert_eq!(links[1].url, "docs/b.md");
        assert!(links[1].description.is_empty());
        let map = render(Path::new("docs/llms.txt"), src, "map").unwrap();
        assert!(map.contains("h1 Docs 1"), "{map}");
        assert!(
            map.contains("link Real https://example.com/a — a note"),
            "{map}"
        );
        assert!(map.contains("link Other docs/b.md"), "{map}");
        assert!(!map.contains("Fake"), "{map}");
        let full = render(Path::new("llms-full.txt"), src, "map").unwrap();
        assert!(full.contains("link Other docs/b.md"), "{full}");
        let md = render(Path::new("doc.md"), src, "map").unwrap();
        assert!(!md.contains("link "), "{md}");
        assert!(md.contains("h1 Docs 1"), "{md}");
    }

    #[test]
    fn stripped_per_language() {
        let cases = [
            ("a.rs", "/// gone\nfn keep() {}\n", "gone", "fn keep"),
            (
                "a.ts",
                "// gone\nfunction keep() { return 1; }\n",
                "gone",
                "function keep",
            ),
            (
                "a.js",
                "// gone\nfunction keep() { return 1; }\n",
                "gone",
                "function keep",
            ),
            (
                "a.py",
                "# gone\ndef keep():\n    return 1\n",
                "gone",
                "def keep",
            ),
            ("a.dart", "// gone\nvoid keep() {}\n", "gone", "void keep"),
            (
                "a.c",
                "/* gone */\nint keep(void) { return 1; }\n",
                "gone",
                "int keep",
            ),
            (
                "a.go",
                "// gone\nfunc Keep() int { return 1 }\n",
                "gone",
                "func Keep",
            ),
        ];
        for (path, src, gone, keep) in cases {
            let src = pad(src);
            let out = stripped(Path::new(path), &src).expect(path);
            assert!(!out.contains(gone), "{path} still has comment: {out}");
            assert!(out.contains(keep), "{path} lost body: {out}");
        }
    }

    #[test]
    fn stripped_unknown_language_is_none() {
        assert!(stripped(Path::new("a.txt"), "// gone\nkeep\n").is_none());
    }
    /// T68.6: import rows are last path segment, kind import, not definitions.
    #[test]
    fn import_queries_capture_last_segment() {
        let cases = [
            ("a.rs", "use crate::foo::Bar;\nfn main() {}\n", "Bar"),
            (
                "a.ts",
                "import { foo } from './mod/helper';\nexport function greet() {}\n",
                "helper",
            ),
            (
                "a.js",
                "const x = require('pkg/util');\nfunction add(x) { return x; }\n",
                "util",
            ),
            (
                "a.py",
                "from pkg.mod import helper\ndef run():\n    return 1\n",
                "helper",
            ),
            (
                "a.go",
                "package p\nimport \"github.com/x/y\"\nfunc Sum() int { return 0 }\n",
                "y",
            ),
            (
                "a.dart",
                "import 'package:foo/bar.dart';\nvoid start() {}\n",
                "bar",
            ),
        ];
        for (path, src, name) in cases {
            let src = pad(src);
            let hits = tags(Path::new(path), &src).unwrap();
            let imports: Vec<_> = hits
                .iter()
                .filter(|h| h.kind == "import" && !h.is_def)
                .map(|h| h.name.as_str())
                .collect();
            assert!(
                imports.contains(&name),
                "{path} missing import {name}: {imports:?}"
            );
            if path == "a.rs" {
                let full = hits
                    .iter()
                    .find(|h| h.kind == "import" && h.name == "Bar")
                    .expect("Bar import");
                assert_eq!(full.import_path, "crate::foo::Bar");
            }
            let map = render(Path::new(path), &src, "map").unwrap();
            assert!(map.starts_with("imports: "), "{path} outline: {map}");
            assert!(map.contains(name), "{path} outline: {map}");
        }
    }

    /// T368: a brace `use` is one row per item, prefix included, last segment as the name.
    #[test]
    fn rust_use_group_keeps_each_full_path() {
        let src = pad("use crate::foo::{Bar, baz::Qux as Alias};\nfn main() {}\n");
        let imports: Vec<_> = tags(Path::new("a.rs"), &src)
            .unwrap()
            .into_iter()
            .filter(|h| h.kind == "import" && !h.is_def)
            .map(|h| (h.name, h.import_path))
            .collect();
        assert!(
            imports.contains(&("Bar".into(), "crate::foo::Bar".into())),
            "{imports:?}"
        );
        assert!(
            imports.contains(&("Qux".into(), "crate::foo::baz::Qux".into())),
            "{imports:?}"
        );
    }

    /// T474: the tag keeps the node's byte range, so a later read can seek to it.
    #[test]
    fn tag_hit_keeps_the_byte_range() {
        let src = "fn keep() {\n    1\n}\n";
        let hits = tags(Path::new("a.rs"), src).unwrap();
        let def = hits
            .iter()
            .find(|h| h.is_def && h.name == "keep")
            .expect("keep");
        assert!(def.end_byte > def.start_byte);
        let slice = &src[def.start_byte..def.end_byte];
        assert!(slice.starts_with("fn keep"), "{slice}");
        assert!(slice.contains('1'), "{slice}");
    }
}

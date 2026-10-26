// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T329.31: the export as a picture. SVG is drawn from an `Export` and nothing else, so a saved
//! file draws the same picture as the live scope, and PNG is that SVG rasterised. The layout is a
//! pure function of the export (no random seed, no clock): two runs over the same file give the
//! same bytes.

use std::cmp::Reverse;
use std::collections::HashMap;

#[cfg(feature = "graph-png")]
use anyhow::Context as _;
use anyhow::{Result, bail};
#[cfg(feature = "graph-png")]
use resvg::{tiny_skia, usvg};

use super::drill::{DrillNodeKind, node_kind};
use super::export::{Export, Node};

/// More nodes than this are an unreadable hairball, and the SVG would grow without bound.
/// The JSON keeps every node.
pub const MAX_DRAWN: usize = 200;
/// A PNG past this many pixels would take a quarter of a gigabyte or more.
#[cfg(feature = "graph-png")]
const MAX_PIXELS: u64 = 64_000_000;
const MAX_LABELS: usize = 40;
const SPACING: f64 = 30.0;
const NODE_R: f64 = 7.0;
const GAP: f64 = 56.0;
const HEAD: f64 = 44.0;
const MARGIN: f64 = 24.0;
const TITLE: f64 = 40.0;
const MIN_WIDTH: f64 = 900.0;
const LINE: f64 = 15.0;
const FONT: &str = "Helvetica, Arial, 'DejaVu Sans', sans-serif";
/// Okabe-Ito: distinguishable with the common colour-vision deficiencies.
const COLORS: [&str; 8] = [
    "#0072B2", "#E69F00", "#009E73", "#CC79A7", "#56B4E9", "#D55E00", "#8E6BBF", "#7A7A7A",
];

const BG: &str = "#ffffff";
const PANEL: &str = "#f6f7f9";
const INK: &str = "#1b1f24";
const MUTE: &str = "#6b7280";
const WARN: &str = "#b45309";
const DASH: &str = r#" stroke-dasharray="4 3""#;

/// Control characters are not legal XML, and one in a file name must not break the file. The
/// escape of `report::html` is out of reach: `report` sits above `plugins` in the module layers.
fn esc(s: &str) -> String {
    s.replace(char::is_control, "?")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars()
        .take(max.saturating_sub(1))
        .chain(['\u{2026}'])
        .collect()
}

fn when(secs: i64) -> String {
    format!("{} UTC", crate::log::stamp(secs.max(0) as u64))
}

fn text(x: f64, y: f64, size: u32, fill: &str, extra: &str, s: &str) -> String {
    format!(
        r#"<text x="{x:.1}" y="{y:.1}" font-size="{size}" fill="{fill}"{extra}>{}</text>
"#,
        esc(s)
    )
}

/// Text that stays readable over lines: a halo in the background colour.
fn halo(bg: &str) -> String {
    format!(r#" stroke="{bg}" stroke-width="3" paint-order="stroke""#)
}

fn line(a: (f64, f64), b: (f64, f64), width: f64, extra: &str) -> String {
    format!(
        r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{MUTE}" stroke-width="{width:.1}"{extra}/>
"#,
        a.0, a.1, b.0, b.1
    )
}

const ARROW: &str = r#" marker-end="url(#arrow)""#;

struct Spot<'a> {
    node: &'a Node,
    at: (f64, f64),
}

/// Where a line from `a` to `b` leaves a box of half size `(hw, hh)` around `a`.
fn edge_of(a: (f64, f64), b: (f64, f64), hw: f64, hh: f64) -> (f64, f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (hw / dx.abs()).min(hh / dy.abs()).min(1.0);
    if t.is_finite() {
        (a.0 + dx * t, a.1 + dy * t)
    } else {
        a
    }
}

/// Circle: function, square: type, diamond: module (the kinds `drill` tells apart).
fn shape(kind: &str, (x, y): (f64, f64), fill: &str, stroke: &str, sw: f64) -> String {
    let style = format!(r#"fill="{fill}" stroke="{stroke}" stroke-width="{sw}""#);
    let r = NODE_R;
    let out = match node_kind(kind) {
        DrillNodeKind::Type => format!(
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="2" {style}/>"#,
            x - r,
            y - r,
            2.0 * r,
            2.0 * r
        ),
        DrillNodeKind::Module => {
            let d = r + 2.0;
            format!(
                r#"<polygon points="{x:.1},{:.1} {:.1},{y:.1} {x:.1},{:.1} {:.1},{y:.1}" {style}/>"#,
                y - d,
                x + d,
                y + d,
                x - d
            )
        }
        _ => format!(r#"<circle cx="{x:.1}" cy="{y:.1}" r="{r}" {style}/>"#),
    };
    out + "\n"
}

/// `transparent` leaves the background out, for a picture that goes onto someone else's page.
pub fn svg(e: &Export, transparent: bool) -> String {
    let mut at: HashMap<i32, usize> = HashMap::new();
    for (i, pr) in e.projects.iter().enumerate() {
        at.entry(pr.id).or_insert(i);
    }
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for edge in &e.edges {
        for end in [&edge.from, &edge.to] {
            *degree.entry(end.as_str()).or_default() += 1;
        }
    }
    // The best connected nodes survive the cap: they are the ones a reader is looking for.
    let deg = |i: usize| degree.get(e.nodes[i].id.as_str()).copied().unwrap_or(0);
    let mut order: Vec<usize> = (0..e.nodes.len()).collect();
    order.sort_by_key(|&i| (Reverse(deg(i)), i));
    let hidden = order.len().saturating_sub(MAX_DRAWN);
    order.truncate(MAX_DRAWN);

    let mut per: Vec<Vec<usize>> = vec![Vec::new(); e.projects.len()];
    for &i in &order {
        if let Some(group) = at.get(&e.nodes[i].project).and_then(|&pi| per.get_mut(pi)) {
            group.push(i);
        }
    }
    let widest = per.iter().map(Vec::len).max().unwrap_or(0);
    let (pw, ph) = if widest == 0 {
        (220.0, 80.0)
    } else {
        let side = (2.0 * (SPACING * (widest as f64).sqrt() + SPACING + NODE_R)).max(220.0);
        (side, side + HEAD)
    };
    let n = e.projects.len().max(1);
    let cols = (n as f64).sqrt().ceil() as usize;
    let rows = n.div_ceil(cols);
    let gw = cols as f64 * pw + (cols - 1) as f64 * GAP;
    let gh = rows as f64 * ph + (rows - 1) as f64 * GAP;
    let width = (gw + 2.0 * MARGIN).max(MIN_WIDTH);
    let (x0, y0) = ((width - gw) / 2.0, TITLE + MARGIN);
    let corner = |pi: usize| {
        (
            x0 + (pi % cols) as f64 * (pw + GAP),
            y0 + (pi / cols) as f64 * (ph + GAP),
        )
    };
    let centre = |pi: usize| (corner(pi).0 + pw / 2.0, corner(pi).1 + ph / 2.0);
    let color = |pi: usize| COLORS[pi % COLORS.len()];

    let mut spots: Vec<Spot> = Vec::new();
    let mut spot_of: HashMap<&str, usize> = HashMap::new();
    for (pi, group) in per.iter().enumerate() {
        let (cx, cy) = (
            corner(pi).0 + pw / 2.0,
            corner(pi).1 + HEAD + (ph - HEAD) / 2.0,
        );
        for (rank, &i) in group.iter().enumerate() {
            // Sunflower spiral: the hubs sit in the middle and no two nodes land on one point.
            let (r, a) = (
                SPACING * (rank as f64 + 0.5).sqrt(),
                rank as f64 * 2.399_963,
            );
            spot_of.insert(e.nodes[i].id.as_str(), spots.len());
            spots.push(Spot {
                node: &e.nodes[i],
                at: (cx + r * a.cos(), cy + r * a.sin()),
            });
        }
    }

    let (mut body, mut link_labels) = (String::new(), String::new());
    for l in &e.links {
        let (Some(&a), Some(&b)) = (at.get(&l.from), at.get(&l.to)) else {
            continue;
        };
        if a == b {
            continue;
        }
        let (s, t) = (
            edge_of(centre(a), centre(b), pw / 2.0, ph / 2.0),
            edge_of(centre(b), centre(a), pw / 2.0, ph / 2.0),
        );
        body += &line(
            s,
            t,
            1.0 + (1.0 + l.references as f64).log2().min(4.0) * 0.6,
            ARROW,
        );
        let label = format!("{} \u{b7} {} calls", l.kind, l.references);
        let mid = ((s.0 + t.0) / 2.0, (s.1 + t.1) / 2.0 - 4.0);
        link_labels += &text(
            mid.0,
            mid.1,
            10,
            MUTE,
            &format!(r#" text-anchor="middle"{}"#, halo(BG)),
            &label,
        );
    }
    for (pi, pr) in e.projects.iter().enumerate() {
        let (x, y) = corner(pi);
        body += &format!(
            r#"<rect x="{x:.1}" y="{y:.1}" width="{pw:.1}" height="{ph:.1}" rx="10" fill="{PANEL}" stroke="{}" stroke-width="2"/>
"#,
            color(pi)
        );
        body += &text(
            x + 12.0,
            y + 20.0,
            14,
            INK,
            r#" font-weight="bold""#,
            &short(&pr.name, 28),
        );
        body += &text(
            x + 12.0,
            y + 35.0,
            10,
            MUTE,
            "",
            &format!("{} \u{b7} {}", pr.backend, pr.health),
        );
    }
    for edge in &e.edges {
        let (Some(&a), Some(&b)) = (
            spot_of.get(edge.from.as_str()),
            spot_of.get(edge.to.as_str()),
        ) else {
            continue;
        };
        let (s, t) = (&spots[a], &spots[b]);
        let len = (t.at.0 - s.at.0).hypot(t.at.1 - s.at.1);
        if len < 1.0 {
            continue;
        }
        // Stop short of the callee so the arrow head is not under its shape.
        let k = (len - NODE_R - 3.0) / len;
        let end = (
            s.at.0 + (t.at.0 - s.at.0) * k,
            s.at.1 + (t.at.1 - s.at.1) * k,
        );
        let dash = if s.node.project == t.node.project {
            ""
        } else {
            DASH
        };
        body += &line(
            s.at,
            end,
            1.0,
            &format!(r#" stroke-opacity="0.7"{dash}{ARROW}"#),
        );
    }
    let mut labels = String::new();
    for (i, sp) in spots.iter().enumerate() {
        let focus = e.meta.focus.as_deref() == Some(sp.node.name.as_str());
        let pi = at.get(&sp.node.project).copied().unwrap_or(0);
        let (stroke, sw) = if focus { (INK, 3.0) } else { (PANEL, 1.5) };
        body += &shape(&sp.node.kind, sp.at, color(pi), stroke, sw);
        if focus || i < MAX_LABELS {
            let pos = (sp.at.0 + NODE_R + 3.0, sp.at.1 + 3.0);
            labels += &text(
                pos.0,
                pos.1,
                9,
                INK,
                &halo(PANEL),
                &short(&sp.node.name, 22),
            );
        }
    }
    body += &labels;
    body += &link_labels;

    let ly = y0 + gh + MARGIN;
    body += &legend(e, ly);
    let foot = footer(e, hidden);
    let fy = ly + 4.0 * 20.0 + 10.0;
    for (i, (s, warn)) in foot.iter().enumerate() {
        body += &text(
            MARGIN,
            fy + i as f64 * LINE,
            11,
            if *warn { WARN } else { MUTE },
            "",
            s,
        );
    }
    let height = fy + foot.len() as f64 * LINE + MARGIN;
    let bg = if transparent {
        String::new()
    } else {
        format!(r#"<rect width="100%" height="100%" fill="{BG}"/>"#)
    };
    let title = text(
        MARGIN,
        30.0,
        18,
        INK,
        r#" font-weight="bold""#,
        &format!("Code graph: {}", short(&e.meta.scope.join(", "), 90)),
    );
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.0} {height:.0}" font-family="{FONT}">
<defs><marker id="arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto"><path d="M0 0L8 4L0 8z" fill="{MUTE}"/></marker></defs>
{bg}
{title}{body}</svg>
"#
    )
}

/// Four rows: project colours, node shapes, edge styles; each entry is a mark and a label.
fn legend(e: &Export, y: f64) -> String {
    let mut out = String::new();
    let mut put = |row: f64, x: &mut f64, mark: String, label: &str| {
        out += &mark;
        out += &text(*x + 18.0, y + row * 20.0 + 4.0, 11, INK, "", label);
        *x += 30.0 + label.chars().count() as f64 * 6.2;
    };
    let mut x = MARGIN;
    let shown = e.projects.len().min(COLORS.len());
    for (i, pr) in e.projects.iter().take(shown).enumerate() {
        let swatch = format!(
            r#"<rect x="{x:.1}" y="{:.1}" width="12" height="12" rx="2" fill="{}"/>
"#,
            y - 6.0,
            COLORS[i]
        );
        put(0.0, &mut x, swatch, &short(&pr.name, 24));
    }
    if e.projects.len() > shown {
        put(
            0.0,
            &mut x,
            String::new(),
            &format!("+{} more", e.projects.len() - shown),
        );
    }
    x = MARGIN;
    for (kind, label) in [
        ("function", "function or method"),
        ("struct", "type"),
        ("module", "module"),
    ] {
        let mark = shape(kind, (x + 6.0, y + 20.0), MUTE, MUTE, 1.0);
        put(1.0, &mut x, mark, label);
    }
    x = MARGIN;
    let (a, b) = ((0.0, y + 40.0), (14.0, y + 40.0));
    for (extra, width, label) in [
        ("", 1.0, "call within a project (caller to callee)"),
        (DASH, 1.0, "call across projects"),
        ("", 3.0, "project link (width: calls across it)"),
    ] {
        let mark = line((x + a.0, a.1), (x + b.0, b.1), width, extra);
        put(2.0, &mut x, mark, label);
    }
    out
}

/// The lines under the picture; the flag marks the ones to draw as a warning.
fn footer(e: &Export, hidden: usize) -> Vec<(String, bool)> {
    let m = &e.meta;
    let focus = m.focus.as_ref().map_or(String::new(), |f| {
        format!(" {f}, depth {}", m.depth.unwrap_or(0))
    });
    let mut lines = vec![(
        format!(
            "rtok {} \u{b7} exported {} \u{b7} scope: {} \u{b7} level: {}{focus}{}",
            m.rtok_version,
            when(m.exported_at as i64),
            m.scope.join(", "),
            m.level,
            if m.redacted {
                " \u{b7} paths redacted"
            } else {
                ""
            }
        ),
        false,
    )];
    if m.partial {
        lines.push((
            "PARTIAL: a project was still indexing, not indexed or could not answer".into(),
            true,
        ));
    }
    if hidden > 0 {
        let all = e.nodes.len();
        lines.push((format!("{hidden} nodes hidden: the {MAX_DRAWN} best connected are drawn, the JSON export has all {all}"), true));
    }
    lines.extend(e.projects.iter().map(|pr| {
        let indexed = pr.indexed_at.map_or("never".into(), when);
        (
            format!(
                "{}: backend {}, {}, indexed {indexed}",
                short(&pr.name, 40),
                pr.backend,
                pr.health
            ),
            false,
        )
    }));
    lines.extend(m.notes.iter().take(3).map(|n| (short(n, 150), false)));
    lines
}

/// The SVG rasterised at `scale` times its own size. Text needs a font, so a machine without
/// any gets an error instead of a picture with the names missing.
#[cfg(feature = "graph-png")]
pub fn png(svg: &str, scale: u32) -> Result<Vec<u8>> {
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    if opt.fontdb.is_empty() {
        bail!("no fonts on this machine, so a PNG would have no text; use --format svg");
    }
    let tree = usvg::Tree::from_str(svg, &opt).context("the drawn SVG did not parse")?;
    let size = tree
        .size()
        .to_int_size()
        .scale_by(scale as f32)
        .context("the picture is too large")?;
    if u64::from(size.width()) * u64::from(size.height()) > MAX_PIXELS {
        bail!(
            "a {}x{} px picture is too large; use a smaller --scale",
            size.width(),
            size.height()
        );
    }
    let mut map =
        tiny_skia::Pixmap::new(size.width(), size.height()).context("the picture is empty")?;
    let by = scale as f32;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(by, by),
        &mut map.as_mut(),
    );
    map.encode_png().context("PNG encoding failed")
}

/// Without the `graph-png` feature the rasteriser (resvg, its font stack and tiny-skia) is not
/// linked, so PNG export is an error that names the feature; JSON and SVG are unaffected.
#[cfg(not(feature = "graph-png"))]
pub fn png(_svg: &str, _scale: u32) -> Result<Vec<u8>> {
    bail!("PNG export needs rtok built with the `graph-png` feature; use --format svg")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::graph::export::{Edge, Link, Meta, Proj, SCHEMA};

    fn node(project: i32, name: &str, kind: &str, line: i32) -> Node {
        Node {
            id: format!("{project}:lib.rs:{line}:{name}"),
            project,
            kind: kind.into(),
            name: name.into(),
            path: "lib.rs".into(),
            line,
        }
    }

    fn export(nodes: Vec<Node>, edges: Vec<Edge>) -> Export {
        let proj = |id: i32, name: &str| Proj {
            id,
            name: name.into(),
            root: "~/x".into(),
            origin: "manual".into(),
            backend: "tags".into(),
            health: "ok".into(),
            indexed_at: Some(1_700_000_000),
        };
        Export {
            schema: SCHEMA.into(),
            projects: vec![proj(1, "alpha"), proj(2, "beta")],
            links: vec![Link {
                from: 1,
                to: 2,
                kind: "manual".into(),
                reason: None,
                references: 3,
            }],
            nodes,
            edges,
            meta: Meta {
                scope: vec!["alpha".into(), "beta".into()],
                level: "symbols".into(),
                focus: None,
                depth: None,
                exported_at: 1_700_000_100,
                rtok_version: "9.9.9".into(),
                redacted: true,
                partial: true,
                notes: Vec::new(),
            },
        }
    }

    #[cfg(feature = "graph-png")]
    fn parse(s: &str) -> usvg::Tree {
        usvg::Tree::from_str(s, &usvg::Options::default()).expect("well-formed SVG")
    }

    /// Without `graph-png` there is no SVG parser linked; check the document frame only.
    #[cfg(not(feature = "graph-png"))]
    fn parse(s: &str) {
        let t = s.trim();
        assert!(
            t.starts_with("<svg") || t.starts_with("<?xml"),
            "not an SVG: {t:.80}"
        );
        assert!(t.ends_with("</svg>"), "unterminated SVG");
    }

    fn edge(a: &Node, b: &Node) -> Edge {
        Edge {
            from: a.id.clone(),
            to: b.id.clone(),
            kind: "call".into(),
        }
    }

    fn small() -> Export {
        let (a, b, c) = (
            node(1, "a_fn", "function", 1),
            node(2, "B", "struct", 2),
            node(2, "m", "module", 3),
        );
        let edges = vec![edge(&a, &b), edge(&b, &c)];
        export(vec![a, b, c], edges)
    }

    #[test]
    fn svg_is_valid_xml_with_legend_and_footer() {
        let s = svg(&small(), false);
        parse(&s);
        for want in [
            "alpha",
            "beta",
            "function or method",
            "call across projects",
            "rtok 9.9.9",
            "2023-11-14 22:15:00 UTC",
            "scope: alpha, beta",
            "backend tags, ok, indexed 2023-11-14 22:13:20 UTC",
            "PARTIAL",
            "paths redacted",
            "manual \u{b7} 3 calls",
        ] {
            assert!(s.contains(want), "missing {want:?} in\n{s}");
        }
        assert!(!s.contains("nodes hidden"));
        assert_eq!(s, svg(&small(), false), "the picture is deterministic");
    }

    #[test]
    fn names_are_escaped() {
        let mut e = small();
        e.nodes[0].name = "a<b>&\"c\u{1}".into();
        e.meta.focus = Some(e.nodes[0].name.clone());
        let s = svg(&e, false);
        parse(&s);
        assert!(s.contains("a&lt;b&gt;&amp;&quot;c?"));
    }

    #[test]
    fn more_than_the_cap_draws_the_cap_and_says_how_many_are_hidden() {
        let nodes: Vec<Node> = (0..MAX_DRAWN as i32 + 25)
            .map(|i| node(1, &format!("n{i}"), "function", i))
            .collect();
        let edges = vec![edge(&nodes[MAX_DRAWN + 24], &nodes[0])];
        let s = svg(&export(nodes, edges), false);
        assert_eq!(
            s.matches("<circle").count() - 1,
            MAX_DRAWN,
            "the legend sample is one more"
        );
        assert!(s.contains("25 nodes hidden"));
        // The best connected node survives the cap although it is the last one in file order.
        assert!(s.contains(">n224<"));
    }

    #[test]
    fn transparent_has_no_background() {
        let e = small();
        assert!(svg(&e, false).contains(r##"width="100%" height="100%" fill="#ffffff""##));
        assert!(!svg(&e, true).contains(r#"width="100%""#));
    }

    #[test]
    fn overview_without_nodes_draws_project_panels_and_links() {
        let mut e = small();
        e.nodes.clear();
        e.edges.clear();
        let s = svg(&e, false);
        assert_eq!(s.matches(r#"rx="10""#).count(), 2);
        assert!(s.contains(r#"marker-end="url(#arrow)""#));
    }

    #[cfg(feature = "graph-png")]
    fn dims(png: &[u8]) -> (u32, u32, u8) {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let be = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
        (be(16), be(20), png[25])
    }

    #[cfg(feature = "graph-png")]
    #[test]
    fn png_is_the_svg_at_1x_2x_and_4x() {
        let s = svg(&small(), false);
        let (w, h) = {
            let t = usvg::Tree::from_str(&s, &usvg::Options::default()).unwrap();
            (t.size().width() as u32, t.size().height() as u32)
        };
        for k in [1, 2, 4] {
            let (pw, ph, _) = dims(&png(&s, k).unwrap());
            assert_eq!((pw, ph), (w * k, h * k), "{k}x");
        }
    }

    #[cfg(feature = "graph-png")]
    #[test]
    fn transparent_png_has_an_alpha_corner() {
        let transparent = svg(&small(), true);
        let decode = |svg: &str| {
            let map = tiny_skia::Pixmap::decode_png(&png(svg, 1).unwrap()).unwrap();
            map.pixel(0, 0).unwrap().alpha()
        };
        assert_eq!(decode(&transparent), 0);
        assert_eq!(decode(&svg(&small(), false)), 255);
    }

    #[cfg(not(feature = "graph-png"))]
    #[test]
    fn png_without_the_feature_names_it() {
        let err = png(&svg(&small(), false), 1).unwrap_err().to_string();
        assert!(err.contains("graph-png"), "{err}");
    }
}

// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The shell (T15.2): a header line, the tab bar, the page body, a footer. The tabs
//! are the model's page list, never a second one (D23). The Overview tab renders CTT,
//! per-plugin savings bars and a per-turn sparkline off the snapshot (T15.3); the
//! Plugins tab lists the catalogue with a row cursor and a toggle that writes
//! `plugins.<id>.enabled` through `config set`'s writer (T15.4); Calls (T15.5) lists
//! ledger rows with a detail pane; Doctor (T15.6) and Logs (T15.7) render their model
//! pages; a page the model adds ahead of its tab falls through to a placeholder that
//! says so. Every colour and frame comes from [`super::theme`] (T226): the body is one
//! rounded pane with the tabs in its top border, tables have an accent header and a
//! full-width cursor row, and the key hints are `key description` pairs.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Clear, Paragraph, Row, Sparkline, Table, TableState, Tabs, Wrap};

use super::app::{App, keys_for};
use super::health;
use super::theme::{self, ACCENT, ERR, OK, WARN};
use crate::model::{self, PluginPage};
use crate::store::CallRow;
use crate::web::calls_store::{ScopeRow, scope_names};

/// One screen: header · [alert] · framed body with the tabs in its top border · footer,
/// with the `?` overlay on top when it is open (T60.8). The alert row stays up for the
/// whole disabled period (proxy/core enabled=false).
pub(super) fn draw(frame: &mut Frame, app: &App) {
    let alert = app.snapshot().usage.alerts.first().cloned();
    let store_error = app.snapshot().error.clone();
    let [header, alert_area, error_area, body, footer] = Layout::vertical([
        Constraint::Length(1),
        // Alert row: height 0 when absent, same slots either way.
        Constraint::Length(u16::from(alert.is_some())),
        Constraint::Length(u16::from(store_error.is_some())),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(frame, app, header);
    if let Some(msg) = alert {
        frame.render_widget(Paragraph::new(theme::banner('⚠', &msg, WARN)), alert_area);
    }
    if let Some(msg) = store_error {
        frame.render_widget(Paragraph::new(theme::banner('✕', &msg, ERR)), error_area);
    }
    let pane = theme::frame();
    let inner = pane.inner(body);
    frame.render_widget(pane, body);
    // The tabs ride the pane's top border: `╭─ overview ─ plugins ─ … ─╮`.
    let tabs = Rect::new(body.x + 2, body.y, body.width.saturating_sub(4), 1);
    frame.render_widget(tab_bar(app), tabs);
    render_page(frame, app, inner);
    frame.render_widget(Paragraph::new(footer_line()), footer);
    if app.help_open() {
        render_help(frame, app);
    }
}

/// The `?` overlay (T60.8): the global keys plus the current page's, generated from
/// [`super::app::keys_for`] — never a hand-written list. Centred, over a cleared box.
fn render_help(frame: &mut Frame, app: &App) {
    let page = if app.page() == "graph" && app.compare().is_open() {
        "compare"
    } else if app.page() == "graph" && app.exporter().is_open() {
        "export"
    } else {
        app.page()
    };
    let rows = keys_for(page);
    let lines: Vec<Line<'static>> = rows
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!("{k:<13}"), theme::title()),
                Span::raw(*d),
            ])
        })
        .collect();
    let title = format!("keys — {page}");
    let width = lines
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        .max(title.chars().count() + 12) as u16
        + 4; // borders and padding
    let height = lines.len() as u16 + 2;
    let area = centered(frame.area(), width, height);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            theme::pane(title)
                .title_bottom(Line::styled(" ? closes ", theme::muted()).right_aligned()),
        ),
        area,
    );
}

/// A `width`×`height` box in the middle of `area`, clipped to it.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [_, mid, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, rect, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width),
        Constraint::Fill(1),
    ])
    .areas(mid);
    rect
}

/// The header: the badge, the project and the window on the left; the tick on the right.
fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let status = header_status(app);
    let [left, right] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(status.width() as u16),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(header_line(frame.area())), left);
    frame.render_widget(Paragraph::new(status), right);
}

/// ` rtok  <project>  <cols>×<rows>` — what, where, and the window it is in.
fn header_line(area: Rect) -> Line<'static> {
    Line::from(vec![
        Span::styled(" rtok ", theme::badge()),
        Span::styled(format!(" {}", project()), Style::new().bold()),
        Span::styled(format!("  {}×{}", area.width, area.height), theme::muted()),
    ])
}

/// When the data last came off the model, and whether a read is in flight.
fn header_status(app: &App) -> Line<'static> {
    let stamp = crate::log::stamp(app.updated());
    let time = stamp.rsplit_once(' ').map_or("-", |(_, t)| t).to_owned();
    let mut spans = Vec::new();
    if app.loading() {
        spans.push(Span::styled("◌ loading  ", Style::new().fg(WARN)));
    }
    spans.push(Span::styled(format!("updated {time} UTC "), theme::muted()));
    Line::from(spans)
}

/// The project is the current directory's name: the store the model reads is scoped
/// to where the operator sits, so that is what the header names.
fn project() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|dir| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "-".into())
}

/// The model's pages in order, the selected one as a badge, `─` between them so the
/// row reads as the pane's border.
fn tab_bar(app: &App) -> Tabs<'static> {
    let titles = app
        .tab_names()
        .into_iter()
        .map(|name| format!(" {name} "))
        .collect::<Vec<_>>();
    Tabs::new(titles)
        .select(app.selected())
        .padding("", "")
        .divider(Span::styled("─", theme::muted()))
        .highlight_style(theme::selected())
}

/// The selected page's body. No catch-all: a page the model adds ahead of its TUI
/// body breaks this match at compile time and fails `tests/surface_parity.rs`
/// (T60.10 deleted the placeholder that used to hide the gap).
fn render_page(frame: &mut Frame, app: &App, area: Rect) {
    match app.page() {
        "overview" => render_overview(frame, app, area),
        "plugins" => render_plugins(frame, app, area),
        "calls" => render_calls(frame, app, area),
        "sessions" => render_sessions(frame, app, area),
        "doctor" => frame.render_widget(
            app.doctor_fix().paragraph().unwrap_or_else(|| doctor(app)),
            area,
        ),
        "logs" => frame.render_widget(logs_text(app), area),
        "skills" => render_skills(frame, app, area),
        "stats" => frame.render_widget(stats(app), area),
        "graph" if app.compare().render(frame, area) || app.exporter().render(frame, area) => {}
        "graph" => render_graph(frame, app, area),
        "hosts" => frame.render_widget(
            app.junk()
                .paragraph(app.cfg())
                .unwrap_or_else(|| hosts_page(app)),
            area,
        ),
        "config" => frame.render_widget(config_page(app), area),
        "services" => frame.render_widget(services_page(app), area),
        "worktrees" => frame.render_widget(worktrees_page(app), area),
        "usage" => frame.render_widget(usage_page(app), area),
        "tasks" => frame.render_widget(tasks_page(app), area),
        page => unreachable!("page `{page}` has no TUI body — surface_parity holds the list"),
    }
}

/// An empty-state line, muted.
fn empty(msg: &str) -> Paragraph<'static> {
    Paragraph::new(Line::styled(msg.to_owned(), theme::muted()))
}

/// The model's Overview page (T15.3): usage totals as cards, CTT, one savings bar per
/// measured plugin, and the per-turn sparkline — every number off the snapshot the
/// model served (D23), never a second query.
fn render_overview(frame: &mut Frame, app: &App, area: Rect) {
    let usage = &app.snapshot().usage;
    let [alerts, totals, cards, savings, spark] = Layout::vertical([
        Constraint::Length(usage.alerts.len() as u16),
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Min(0),
        Constraint::Length(3),
    ])
    .areas(area);
    let alert_lines: Vec<Line<'static>> = usage
        .alerts
        .iter()
        .map(|a| theme::banner('⚠', a, WARN))
        .collect();
    frame.render_widget(Paragraph::new(alert_lines), alerts);
    frame.render_widget(
        Paragraph::new(Line::styled(
            "usage totals (usage rows, all apis) · ctt = input-side tok × turns after",
            theme::muted(),
        )),
        totals,
    );
    render_cards(frame, app, cards);
    frame.render_widget(
        Paragraph::new(savings_lines(&app.snapshot().plugins))
            .block(theme::section("saved by plugin (Measurement rows)")),
        savings,
    );
    let data: Vec<u64> = usage.turns.iter().map(|t| (*t).max(0) as u64).collect();
    let title = if data.is_empty() {
        "ctx tokens per turn (no usage rows yet)".to_string()
    } else {
        format!("ctx tokens per turn (last {} turns)", data.len())
    };
    frame.render_widget(
        Sparkline::default()
            .block(theme::section(title))
            .style(Style::new().fg(ACCENT))
            .data(&data),
        spark,
    );
}

/// Five cards in one row: the four usage counters and CTT.
fn render_cards(frame: &mut Frame, app: &App, area: Rect) {
    let usage = &app.snapshot().usage;
    let t = &usage.totals;
    let cards = [
        ("input", t.input),
        ("output", t.output),
        ("cache create", t.cache_create),
        ("cache read", t.cache_read),
        ("ctt", usage.ctt),
    ];
    let areas = Layout::horizontal([Constraint::Fill(1); 5])
        .spacing(1)
        .split(area);
    for ((label, value), rect) in cards.into_iter().zip(areas.iter()) {
        let card = Paragraph::new(vec![
            Line::styled(value.to_string(), theme::title()),
            Line::styled(label, theme::muted()),
        ])
        .block(theme::frame());
        frame.render_widget(card, *rect);
    }
}

/// One bar per plugin that measured a saving, busiest saver first, each scaled to the
/// largest — the longest bar always fills [`BAR_WIDTH`]. A plugin that saved nothing
/// is T22.5's finding, not a bar; with no measured savings the tab says so instead of
/// drawing zeros.
fn savings_lines(plugins: &[PluginPage]) -> Vec<Line<'static>> {
    let mut saved: Vec<(&str, i64)> = plugins
        .iter()
        .filter_map(|p| p.stats.as_ref().map(|s| (p.id, s.est_before - s.est_after)))
        .filter(|(_, saved)| *saved > 0)
        .collect();
    if saved.is_empty() {
        return vec![Line::styled("no measured savings yet", theme::muted())];
    }
    saved.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let max = saved.iter().map(|(_, s)| *s).max().unwrap_or(0);
    saved
        .into_iter()
        .map(|(id, s)| {
            Line::from(vec![
                Span::styled(format!("{id:9} "), Style::new().bold()),
                Span::styled(
                    format!("{:<BAR_WIDTH$}", bar(s, max)),
                    Style::new().fg(ACCENT),
                ),
                Span::styled(format!(" {s} tok"), theme::muted()),
            ])
        })
        .collect()
}

/// `█` scaled to the largest saving, so any positive saving draws at least one block.
/// Zero, negative and empty-maximum inputs draw nothing.
fn bar(saved: i64, max: i64) -> String {
    if saved <= 0 || max <= 0 {
        return String::new();
    }
    let full = (saved as u64)
        .saturating_mul(BAR_WIDTH as u64)
        .div_ceil(max as u64) as usize;
    "█".repeat(full)
}

/// The longest savings bar, in blocks. Fits beside the id and the count on an
/// 80-column terminal with room to spare.
const BAR_WIDTH: usize = 16;

/// The model's Plugins page (T15.4): the catalogue table with a cursor row marking
/// the row a toggle would hit, and a status line below — the keys, and what the last
/// toggle did. The toggle itself is [`App`]'s; this only renders what it left behind.
fn render_plugins(frame: &mut Frame, app: &App, area: Rect) {
    let [table, fields, status] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(plugins_table(app), table);
    frame.render_widget(Paragraph::new(plugin_fields_line(app)), fields);
    frame.render_widget(Paragraph::new(plugins_status_line(app)), status);
}

/// The cursor row's page fields, `key value` pairs on one line (T419): the same pairs the
/// web Plugins page lists, so a non-saving signal such as checkpoint prompt quality is
/// readable here without a detail pane.
fn plugin_fields_line(app: &App) -> Line<'static> {
    let Some(plugin) = app.snapshot().plugins.get(app.plugin_cursor()) else {
        return Line::default();
    };
    let text = plugin
        .fields
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect::<Vec<_>>()
        .join(" · ");
    Line::styled(text, theme::muted())
}

/// One row per catalogue plugin, `▸` on the cursor row, the `on` column saying what the
/// model last read — which, after a toggle, is what the file now says.
fn plugins_table(app: &App) -> Table<'static> {
    let cursor = app.plugin_cursor();
    let rows = app
        .snapshot()
        .plugins
        .iter()
        .enumerate()
        .map(|(i, plugin)| {
            let on = if plugin.enabled {
                Cell::from("on").style(Style::new().fg(OK))
            } else {
                Cell::from("off").style(theme::muted())
            };
            let row = Row::new(vec![
                Cell::from(if i == cursor { "▸" } else { " " }),
                Cell::from(plugin.id.to_string()),
                Cell::from(plugin.title.clone()),
                on,
                Cell::from(plugin.stats.as_ref().map_or_else(
                    || "-".into(),
                    |stats| {
                        format!(
                            "{before}->{after} tok ({rows} rows)",
                            before = stats.est_before,
                            after = stats.est_after,
                            rows = stats.rows
                        )
                    },
                )),
            ]);
            if i == cursor {
                row.style(theme::selected())
            } else {
                row
            }
        });
    Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(9),
            Constraint::Min(24),
            Constraint::Length(4),
            Constraint::Min(24),
        ],
    )
    .header(theme::header(["", "id", "title", "on", "saved"]))
}

/// The Plugins tab's status line: the row keys, and the last toggle's outcome until
/// the cursor moves.
fn plugins_status_line(app: &App) -> Line<'static> {
    // T60.8: the hints are the KEYS table's plugin rows.
    status_line("plugins", app.plugin_status())
}

/// A page's status row: its key hints, then a note (a filter, a toggle's outcome).
pub(super) fn status_line(page: &str, note: &str) -> Line<'static> {
    let page_keys: Vec<_> = keys_for(page)
        .into_iter()
        .filter(|(k, _)| !keys_for("").iter().any(|(g, _)| g == k))
        .collect();
    let mut line = theme::hints(&page_keys);
    if !note.is_empty() {
        line.push_span(Span::styled(format!("  {note}"), Style::new().fg(WARN)));
    }
    line
}

/// The model's Doctor page (T15.6), verbatim: the same text `rtok doctor` prints, from
/// the same model query the command renders (D27) — the snapshot already carries it,
/// so this is a rendering, not a second probe run. `None` is a failed tick, not an
/// empty page. Section heads (unindented lines) are bold.
fn doctor(app: &App) -> Paragraph<'static> {
    let Some(report) = app.snapshot().doctor.as_ref() else {
        return empty("doctor did not answer this tick — `rtok doctor` has the details");
    };
    let lines = report
        .to_text()
        .lines()
        .map(|l| {
            if l.starts_with(' ') {
                Line::from(l.to_owned())
            } else {
                Line::styled(l.to_owned(), Style::new().bold())
            }
        })
        .collect::<Vec<_>>();
    Paragraph::new(lines)
}

/// The model's Stats page (T227), verbatim: `rtok stats --price`'s table plus `rtok
/// stats --cache`'s table, from the same scan the snapshot already carries (D27) — a
/// rendering, not a second scan. `None` is a failed tick, not an empty page.
fn stats(app: &App) -> Paragraph<'static> {
    let Some(text) = app.snapshot().stats.as_ref() else {
        return empty("stats did not answer this tick — `rtok stats` has the details");
    };
    Paragraph::new(text.clone())
}

/// The Graph page: the project registry with its keys (T476) above the index health text, and
/// the live calls pane (T480, D27) below. With no registered project there is nothing for the
/// keys to act on, so only the text and the pane show.
fn render_graph(frame: &mut Frame, app: &App, area: Rect) {
    // The pane takes half the body, capped, so the index health stays readable.
    let live = (area.height / 2).min(18);
    let [area, calls] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(live)]).areas(area);
    let (rows, state) = app.projects();
    let scope_rows: Vec<_> = rows
        .iter()
        .map(|r| ScopeRow {
            id: r.id,
            name: &r.name,
            selected: r.selected,
            links: r.links.iter().map(|l| l.0).collect(),
        })
        .collect();
    // The Graph tab has no drill-down, so the scope is the selected project's.
    app.live()
        .render(frame, calls, scope_names(&scope_rows, None).as_ref());
    if rows.is_empty() {
        return frame.render_widget(graph_page(app), area);
    }
    // Seven rows and the header: the table scrolls to the cursor beyond that.
    let height = (rows.len() as u16 + 1).min(8);
    let detail = health::detail(&rows[state.cursor(&rows)]);
    // Three hint lines: the project keys (T476), the live pane's (T480) and compare's (T485)
    // overflow two at 128 columns.
    let [table, breakdown, note, hints, text] = Layout::vertical([
        Constraint::Length(height),
        Constraint::Length(detail.len() as u16),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .areas(area);
    let body = rows.iter().map(|p| {
        Row::new(vec![
            Cell::from(if p.selected { "*" } else { "" }),
            Cell::from(p.id.to_string()),
            Cell::from(p.name.clone()),
            Cell::from(p.state),
            Cell::from(p.links.len().to_string()),
            health::cell(p),
            health::scope_cell(p),
        ])
    });
    let widths = [
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Min(16),
        Constraint::Length(12),
        Constraint::Length(5),
        Constraint::Length(8),
        Constraint::Length(5),
    ];
    let mut cursor = TableState::default().with_selected(Some(state.cursor(&rows)));
    frame.render_stateful_widget(
        Table::new(body, widths)
            .header(theme::header([
                "", "id", "project", "state", "links", "health", "scope",
            ]))
            .row_highlight_style(theme::selected()),
        table,
        &mut cursor,
    );
    frame.render_widget(Paragraph::new(detail), breakdown);
    frame.render_widget(
        Paragraph::new(Line::styled(state.note(&rows), Style::new().fg(WARN))),
        note,
    );
    frame.render_widget(
        Paragraph::new(status_line("graph", "")).wrap(Wrap { trim: true }),
        hints,
    );
    frame.render_widget(graph_page(app), text);
}

/// The model's Graph page (T230), verbatim: `rtok graph status`'s index health plus
/// `rtok graph dead`'s unreferenced-definition list, from the same store read the
/// snapshot already carries (D27) — a rendering, not a second read. `None` is a
/// failed tick or the `graph` feature being off, not an empty page.
fn graph_page(app: &App) -> Paragraph<'static> {
    let Some(text) = app.snapshot().graph.as_ref() else {
        return empty("graph did not answer this tick — `rtok graph status` has the details");
    };
    Paragraph::new(text.clone())
}

/// The model's Hosts page (T231), verbatim: `rtok agents list`'s blocks — kind,
/// detected version, installed surfaces, config path — from the cached probe the
/// snapshot already carries (D27), never a second spawn per tick.
fn hosts_page(app: &App) -> Paragraph<'static> {
    Paragraph::new(app.snapshot().hosts.clone())
}

/// The model's Config page (T228), from the snapshot (D27). `/` filters through
/// [`crate::expand::filter_lines`], the Calls expand pane's filter (T60.4).
fn config_page(app: &App) -> Paragraph<'static> {
    let Some(text) = app.snapshot().config.as_ref() else {
        return empty("config did not answer this tick — `rtok config show` has the details");
    };
    let (filtering, filter) = app.config_filter();
    if !filtering && filter.is_empty() {
        return Paragraph::new(text.clone());
    }
    let filtered = crate::expand::filter_lines(text, None, Some(filter), 0)
        .ok()
        .map(|v| v.join("\n"))
        .unwrap_or_else(|| text.clone());
    Paragraph::new(format!("/{filter}\n{filtered}"))
}

/// The model's Services page (T229), from the snapshot (D27): `demon status`'s
/// per-service rows plus `otel status`'s exporter health, verbatim like [`graph_page`]
/// — no filter, since the page is a handful of rows rather than a scrollable log.
fn services_page(app: &App) -> Paragraph<'static> {
    let Some(text) = app.snapshot().services.as_ref() else {
        return empty(
            "services did not answer this tick — `rtok demon status`/`rtok otel status` \
             have the details",
        );
    };
    Paragraph::new(text.clone())
}

/// The model's Worktrees page (T232): `worktree list`'s table, verbatim, like
/// [`graph_page`] — path, branch, owner, state, age and `target/` size; `gc`/`clean`
/// stay CLI-only. `None` only when the current directory could not be read.
fn worktrees_page(app: &App) -> Paragraph<'static> {
    let Some(text) = app.snapshot().worktrees.as_ref() else {
        return empty("worktrees did not answer this tick — `rtok worktree list` has the details");
    };
    Paragraph::new(text.clone())
}

/// The model's Usage page (T358.5): `rtok agents usage`'s screen, verbatim — the snapshot
/// carries the report's own text, so the tui sums nothing. Reads happen off the tick: the
/// text says "reading usage…" until the first one lands, and why when one fails.
fn usage_page(app: &App) -> Paragraph<'static> {
    Paragraph::new(app.snapshot().agent_usage.text.clone())
}

/// The model's Tasks page (T441.13.7): `rtok task list --all`'s table, verbatim, or the line
/// that says why there is none (no `[tasks]` section, a failed adapter, still reading).
fn tasks_page(app: &App) -> Paragraph<'static> {
    Paragraph::new(app.snapshot().tasks.text.clone())
}

/// The model's Calls page (T15.5): the ledger's recent rows, newest first — surface,
/// kind, session, latency and the linked usage tokens — with `Enter`/`z` expanding the
/// selected row's full fields below the list. Every value is the snapshot's row (D23),
/// never a second query.
fn render_calls(frame: &mut Frame, app: &App, area: Rect) {
    let rows = &app.snapshot().calls;
    if rows.is_empty() {
        let msg = if !app.snapshot().usage.alerts.is_empty() {
            "proxy disabled — live passthrough (not recorded); no traffic yet"
        } else {
            "no calls yet (the ledger fills as hooks, MCP and the proxy run)"
        };
        frame.render_widget(empty(msg), area);
        return;
    }
    let expand = app.calls_expand();
    let (list, detail, expand_area) = match (app.calls_detail(), expand.is_some()) {
        (true, true) => {
            let [l, d, e] = Layout::vertical([
                Constraint::Min(0),
                Constraint::Length(8),
                Constraint::Min(6),
            ])
            .areas(area);
            (l, Some(d), Some(e))
        }
        (true, false) => {
            let [l, d] = Layout::vertical([Constraint::Min(0), Constraint::Length(9)]).areas(area);
            (l, Some(d), None)
        }
        (false, true) => {
            let [l, e] = Layout::vertical([Constraint::Min(0), Constraint::Min(8)]).areas(area);
            (l, None, Some(e))
        }
        (false, false) => (area, None, None),
    };
    let selected = app.calls_selected();
    frame.render_widget(calls_table(rows, selected), list);
    if let Some(area) = detail {
        frame.render_widget(call_detail(&rows[selected], app), area);
    }
    if let (Some(area), Some((id, text, filtering, filter, scroll))) = (expand_area, expand) {
        frame.render_widget(expand_pane(id, &text, filtering, filter, scroll), area);
    }
}

/// The list: one row per call, the selected one as the cursor row, a failed call in
/// red. Tokens are the linked usage row's four counters summed; a dash says the ledger
/// carries none there.
fn calls_table(rows: &[CallRow], selected: usize) -> Table<'static> {
    let table_rows = rows.iter().enumerate().map(|(i, c)| {
        let row = Row::new([
            time_of(c.ts),
            c.surface.clone(),
            c.kind.clone(),
            c.name.clone().unwrap_or_else(|| "-".into()),
            c.session.clone(),
            c.ms.map_or_else(|| "-".into(), |ms| format!("{ms:.1}")),
            model::call_size_label(c),
        ]);
        if i == selected {
            row.style(theme::selected())
        } else if c.ok == 0 {
            row.style(Style::new().fg(ERR))
        } else {
            row
        }
    });
    let live = rows.iter().filter(|c| c.kind == "live_passthrough").count();
    let title = if live > 0 {
        format!(
            "calls (last {n}, {live} live passthrough — not recorded)",
            n = rows.len()
        )
    } else {
        format!("calls (last {}, newest first)", rows.len())
    };
    Table::new(
        table_rows,
        [
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Length(11),
            Constraint::Min(10),
            Constraint::Min(12),
            Constraint::Length(8),
            Constraint::Length(10),
        ],
    )
    .header(theme::header([
        "when", "surf", "kind", "name", "session", "ms", "tok",
    ]))
    .block(theme::section(title))
}

/// The selected row's full fields: every column the ledger keeps, the slugs its ids
/// point at, and the usage/api linkage when the call recorded one.
fn call_detail(c: &CallRow, app: &App) -> Paragraph<'static> {
    let dash = |v: Option<&str>| v.unwrap_or("-").to_string();
    let mut lines = vec![
        Line::from(format!(
            "ts {} UTC · ok {} · error {}",
            crate::log::stamp(c.ts.max(0) as u64),
            c.ok,
            dash(c.error.as_deref())
        )),
        Line::from(format!(
            "session {} · surface {} · kind {}",
            c.session, c.surface, c.kind
        )),
        Line::from(format!(
            "name {} · plugin {} · parent {}",
            dash(c.name.as_deref()),
            dash(c.plugin.as_deref()),
            c.parent_id.map_or_else(|| "-".into(), |p| p.to_string())
        )),
        Line::from(format!(
            "host {} · provider {} · model {}",
            dash(c.host.as_deref()),
            dash(c.provider.as_deref()),
            dash(c.model.as_deref())
        )),
        Line::from(format!(
            "ms {}",
            c.ms.map_or_else(|| "-".into(), |ms| ms.to_string())
        )),
        Line::from(format!(
            "ref_id {}",
            app.snapshot()
                .ref_ids
                .get(&c.id)
                .map(String::as_str)
                .unwrap_or("-")
        )),
    ];
    lines.push(Line::from(match (c.api.as_deref(), c.input) {
        (Some(api), Some(input)) => format!(
            "usage ({api}) input {input} cache create {} cache read {} output {} = {} tok",
            c.cache_create.unwrap_or(0),
            c.cache_read.unwrap_or(0),
            c.output.unwrap_or(0),
            model::call_linked_tokens(c)
        ),
        _ => "usage no row linked (only an api request records one)".to_string(),
    }));
    Paragraph::new(lines).block(theme::section(format!("call {}", c.id)))
}

fn expand_pane(
    id: &str,
    text: &str,
    filtering: bool,
    filter: &str,
    scroll: u16,
) -> Paragraph<'static> {
    let title = if filtering {
        format!("expand {id}  /{filter}")
    } else {
        format!("expand {id}")
    };
    Paragraph::new(text.to_string())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0))
        .block(theme::section(title))
}

/// `HH:MM:SS` — `log::stamp`'s time half; the full date is in the detail view.
pub(super) fn time_of(ts: i64) -> String {
    crate::log::stamp(ts.max(0) as u64)
        .rsplit_once(' ')
        .map_or_else(|| "-".into(), |(_, t)| t.to_string())
}

/// The model's Sessions page (T25.1 / D23), now with the same row model as Calls
/// (T60.10): a cursor (`↑/↓`), the selected row as the cursor row, live rows green,
/// `l` toggling the live-only filter the CLI exposes as a flag, the table scrolled so
/// the cursor row stays visible on a store taller than the terminal, and `Enter`
/// expanding the selected row through [`model::session_detail`] (T60.3).
fn render_sessions(frame: &mut Frame, app: &App, area: Rect) {
    let live_only = app.sessions_live_only();
    let rows: Vec<&crate::store::SessionTotals> = app
        .snapshot()
        .sessions
        .iter()
        .filter(|s| !live_only || s.ended_at.is_none())
        .collect();
    let [body, status] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
    if rows.is_empty() {
        let msg = if live_only {
            "nothing is running"
        } else {
            "no sessions yet"
        };
        frame.render_widget(empty(msg), body);
        frame.render_widget(Paragraph::new(sessions_status_line(live_only)), status);
        return;
    }
    let (table, detail) = if app.sessions_detail() {
        let [t, d] = Layout::vertical([Constraint::Min(0), Constraint::Length(8)]).areas(body);
        (t, Some(d))
    } else {
        (body, None)
    };
    let selected = app.sessions_selected();
    // Derived scroll: the smallest offset that keeps the cursor row on screen —
    // one body row per terminal line below the header, like the CLI table.
    let visible = table.height.saturating_sub(2).max(1) as usize;
    let offset = selected.saturating_sub(visible - 1);
    let shown: Vec<_> = rows
        .iter()
        .skip(offset)
        .take(visible)
        .zip(offset..)
        .map(|(s, i)| sessions_row(s, i == selected))
        .collect();
    frame.render_widget(sessions_table(shown), table);
    if let Some(area) = detail {
        let id = rows[selected].id.clone();
        frame.render_widget(session_pane(app.snapshot(), &id), area);
    }
    frame.render_widget(Paragraph::new(sessions_status_line(live_only)), status);
}

/// The selected session's fields the list hides, plus the snapshot's calls for
/// that id — [`model::session_detail`], never a second query (T60.3 / D23).
fn session_pane(snapshot: &model::Snapshot, id: &str) -> Paragraph<'static> {
    let Some((s, calls)) = model::session_detail(snapshot, id) else {
        return empty("no session");
    };
    let dash = |v: Option<&str>| v.unwrap_or("-").to_string();
    let ended = s
        .ended_at
        .map_or_else(|| "live".into(), |t| crate::log::stamp(t.max(0) as u64));
    let api = dash(s.api.as_deref());
    let mut lines = vec![
        Line::from(format!(
            "project {} · api {api}",
            dash(s.project.as_deref())
        )),
        Line::from(format!(
            "started {} · last {} · ended {ended}",
            crate::log::stamp(s.started_at.max(0) as u64),
            crate::log::stamp(s.last_activity.max(0) as u64),
        )),
        Line::from(format!(
            "usage ({api}) input {} cache create {} cache read {} output {}",
            s.input, s.cache_create, s.cache_read, s.output
        )),
        Line::from(format!("calls {}", calls.len())),
    ];
    for c in calls {
        lines.push(Line::styled(
            format!(
                "{} {} {} {}",
                time_of(c.ts),
                c.surface,
                c.kind,
                c.name.as_deref().unwrap_or("-")
            ),
            theme::muted(),
        ));
    }
    Paragraph::new(lines).block(theme::section(format!("session {}", s.id)))
}

fn sessions_row(s: &crate::store::SessionTotals, selected: bool) -> Row<'static> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let run = match s.ended_at {
        Some(end) => end - s.started_at,
        None => now - s.started_at,
    };
    let row = Row::new([
        s.host.clone().unwrap_or_else(|| "-".into()),
        s.provider
            .clone()
            .or_else(|| s.api.clone())
            .unwrap_or_else(|| "-".into()),
        s.model.clone().unwrap_or_else(|| "-".into()),
        s.input.to_string(),
        s.output.to_string(),
        s.cache_read.to_string(),
        s.cache_create.to_string(),
        crate::log::stamp(s.started_at.max(0) as u64),
        crate::render::duration(run),
    ]);
    if selected {
        row.style(theme::selected())
    } else if s.ended_at.is_none() {
        row.style(Style::new().fg(OK))
    } else {
        row
    }
}

fn sessions_table(rows: Vec<Row<'static>>) -> Table<'static> {
    Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(11),
            Constraint::Min(6),
        ],
    )
    .header(theme::header([
        "agent",
        "provider",
        "model",
        "input",
        "output",
        "cache_read",
        "cache_create",
        "started",
        "run",
    ]))
}

fn render_skills(frame: &mut Frame, app: &App, area: Rect) {
    let never_only = app.skills_never_only();
    let rows: Vec<&model::SkillPageRow> = app
        .snapshot()
        .skills
        .rows
        .iter()
        .filter(|r| !never_only || r.never)
        .collect();
    let [head, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Line::styled(
            app.snapshot().skills.header.clone(),
            theme::muted(),
        )),
        head,
    );
    if rows.is_empty() {
        let msg = if never_only {
            "no never-invoked skills"
        } else {
            "no skills listed"
        };
        frame.render_widget(empty(msg), body);
        frame.render_widget(Paragraph::new(skills_status_line(never_only)), status);
        return;
    }
    let selected = app.skills_selected();
    let visible = body.height.saturating_sub(2).max(1) as usize;
    let offset = selected.saturating_sub(visible - 1);
    let shown: Vec<_> = rows
        .iter()
        .skip(offset)
        .take(visible)
        .zip(offset..)
        .map(|(r, i)| skills_row(r, i == selected))
        .collect();
    frame.render_widget(skills_table(shown), body);
    frame.render_widget(Paragraph::new(skills_status_line(never_only)), status);
}

/// A never-invoked skill is the finding the page exists for, so its row is yellow.
fn skills_row(r: &model::SkillPageRow, selected: bool) -> Row<'static> {
    let row = Row::new([
        r.name.clone(),
        r.source.clone(),
        r.desc_chars.to_string(),
        r.body_bytes.to_string(),
        r.invocations.to_string(),
        r.resident.to_string(),
        r.last_invoked.clone(),
    ]);
    if selected {
        row.style(theme::selected())
    } else if r.never {
        row.style(Style::new().fg(WARN))
    } else {
        row
    }
}

fn skills_table(rows: Vec<Row<'static>>) -> Table<'static> {
    Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Length(8),
            Constraint::Min(5),
        ],
    )
    .header(theme::header([
        "name", "source", "desc", "body", "calls", "resident", "last",
    ]))
}

fn skills_status_line(never_only: bool) -> Line<'static> {
    status_line("skills", if never_only { "n again shows all" } else { "" })
}

fn sessions_status_line(live_only: bool) -> Line<'static> {
    status_line("sessions", if live_only { "l again shows all" } else { "" })
}

/// The model's Logs page (T15.7): the snapshot's log lines verbatim, newest first —
/// the selection (last `[log] lines`, newest first) is the model's, the same one
/// `rtok logs` screens; the numbering is the CLI's, not a second table here. A line
/// that names an error is red, a warning yellow. Nothing scrolls yet (T15.2 owns the
/// keys): the page shows the newest lines the bound allows, long lines truncated by
/// the terminal's width.
fn logs_text(app: &App) -> Paragraph<'static> {
    let logs = &app.snapshot().logs;
    if logs.is_empty() {
        return empty("no logs yet");
    }
    let lines = logs
        .iter()
        .map(|l| {
            let lower = l.to_ascii_lowercase();
            let style = if lower.contains("error") {
                Style::new().fg(ERR)
            } else if lower.contains("warn") {
                Style::new().fg(WARN)
            } else {
                Style::new()
            };
            Line::styled(l.clone(), style)
        })
        .collect::<Vec<_>>();
    Paragraph::new(lines)
}

/// The global key hints, `key description` pairs off the KEYS table (T60.8) — never
/// hand-written text. The tick moved to the header (T226).
fn footer_line() -> Line<'static> {
    theme::hints(&keys_for(""))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::config::Config;
    use crate::plugin::{Measurement, Runtime};
    use crate::tui::app::tests::config;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::backend::TestBackend;
    use rstest::rstest;

    /// What the loop would put on a real terminal, rendered into a buffer instead.
    /// 98 wide: at 90, 10 tabs' padded titles and dividers (T231's `hosts` was the
    /// tenth) no longer fit the tab bar's `body.width - 4` (T15.2's border cols),
    /// so `hosts` fell off screen and `shell_paints_the_model_tabs_hints_and_tick`
    /// failed on this file, not on a rendering bug — a little headroom for the next
    /// page too.
    pub(in crate::tui) fn screen(app: &App) -> String {
        // T228 widened the tab bar to 11 tabs, T229/T232 to 13 — narrower widths clip the
        // last one off the pane's border before its text ever hits the buffer; the tasks tab
        // (T441.13.7) made it 15.
        let mut terminal = ratatui::Terminal::new(TestBackend::new(140, 24)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().to_string())
            .collect()
    }

    /// The shell paints every tab the model offers, plus the hints and the tick —
    /// the T15.2 chrome — without a terminal.
    #[test]
    fn shell_paints_the_model_tabs_hints_and_tick() {
        let app = App::new(&config());
        let screen = screen(&app);
        for name in app.tab_names() {
            assert!(screen.contains(name), "tab {name} is on screen");
        }
        assert!(
            screen.contains("q/Esc"),
            "the footer hints are generated (T60.8)"
        );
        assert!(screen.contains("updated"));
        assert!(screen.contains("UTC"));
    }

    /// Switching tabs changes the body; both pages the model serves have one.
    #[test]
    fn body_follows_the_selected_tab() {
        let mut app = App::new(&config());
        assert!(screen(&app).contains("usage totals"));
        app.key(KeyCode::Right, KeyModifiers::NONE);
        let screen = screen(&app);
        assert!(screen.contains("id"));
        assert!(screen.contains("title"));
    }

    /// T15.3: the Overview tab shows the snapshot's numbers — totals, CTT, one bar per
    /// measured plugin, the sparkline — so the tab, `rtok stats --json` and the web
    /// Overview agree on one store (Gate P15). The values are five digits, so the match
    /// is the number, not a digit the chrome happens to contain.
    #[test]
    fn overview_tab_renders_the_snapshot_numbers() {
        let app = App::new(&seeded());
        let snap = app.snapshot();
        assert_eq!(snap.usage.totals.input, 11_540);
        assert_eq!(snap.usage.ctt, 1_500);
        let screen = screen(&app);
        for n in [
            snap.usage.totals.input,
            snap.usage.totals.output,
            snap.usage.totals.cache_create,
            snap.usage.totals.cache_read,
            snap.usage.ctt,
        ] {
            assert!(screen.contains(&n.to_string()), "{n} is on screen");
        }
        assert!(
            screen.contains("ctx tokens per turn"),
            "the sparkline title"
        );
        assert!(
            "▁▂▃▄▅▆▇█".chars().any(|c| screen.contains(c)),
            "the sparkline draws blocks"
        );
        // Two measured plugins: `cmd` saved 30 (the full bar), `read` 15 (half).
        for (id, count, blocks) in [
            ("cmd", "30 tok", BAR_WIDTH),
            ("read", "15 tok", BAR_WIDTH / 2),
        ] {
            assert!(screen.contains(id), "{id} has a bar");
            assert!(screen.contains(count), "{id} saved {count}");
            assert!(
                screen.contains(&"█".repeat(blocks)),
                "{id} draws {blocks} blocks"
            );
        }
    }

    /// T15.6: the Doctor tab shows what `rtok doctor` reports — hooks, MCP, proxy —
    /// rendered off the snapshot the model served, never a second probe run.
    #[test]
    fn doctor_tab_shows_what_rtok_doctor_reports() {
        let mut app = App::new(&config());
        while app.page() != "doctor" {
            app.key(KeyCode::Right, KeyModifiers::NONE);
        }
        let screen = screen(&app);
        let text = app
            .snapshot()
            .doctor
            .as_ref()
            .map_or_else(String::new, |report| report.to_text());
        assert!(text.contains("hooks"), "the page is the doctor report");
        for line in text.lines().take(3) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            assert!(screen.contains(line), "line `{line}` is on screen");
        }
    }

    /// T478: `f` on the Doctor tab swaps the report for the fix checklist, Esc closes it
    /// before it can quit, and a declined confirm leaves the fixture machine untouched.
    #[cfg(unix)]
    #[test]
    fn doctor_tab_fix_checklist_replaces_the_report_and_esc_closes_it_first() {
        use crate::tui::doctor_fix::tests::engine_on;
        let m = std::rc::Rc::new(crate::doctor::fix::tests::machine(
            crate::doctor::fix::tests::BROKEN,
        ));
        let mut app = App::new(&config());
        app.set_doctor_engine(engine_on(&m));
        while app.page() != "doctor" {
            app.key(KeyCode::Right, KeyModifiers::NONE);
        }
        let none = KeyModifiers::NONE;
        assert!(!app.key(KeyCode::Char('f'), none));
        let shown = screen(&app);
        assert!(shown.contains("doctor --fix checklist"), "{shown}");
        assert!(shown.contains("broken-hook"), "{shown}");
        assert!(!app.key(KeyCode::Enter, none));
        assert!(screen(&app).contains("y = apply"));
        assert!(
            !app.key(KeyCode::Char('q'), none),
            "q declines, it does not quit"
        );
        assert!(!app.key(KeyCode::Esc, none), "Esc closes the checklist");
        assert!(app.doctor_fix().paragraph().is_none());
        assert!(app.key(KeyCode::Esc, none), "the next Esc quits");
        let settings = crate::doctor::fix::tests::text(&m, crate::doctor::fix::tests::SETTINGS);
        assert_eq!(settings, crate::doctor::fix::tests::BROKEN);
    }

    /// T358.5: the Usage tab shows the snapshot's `agent_usage.text`, the CLI's own screen.
    #[test]
    fn usage_tab_renders_the_cli_screen() {
        let cfg = config();
        let mut app = App::new(&cfg);
        while app.page() != "usage" {
            app.key(KeyCode::Right, KeyModifiers::NONE);
        }
        let mut snap = crate::model::snapshot(&cfg);
        snap.agent_usage.text = "rtok agents usage: logs from 2 agents\n  1.2M tokens\n".into();
        app.refresh(snap);
        let screen = screen(&app);
        assert!(screen.contains("logs from 2 agents"), "{screen}");
        assert!(screen.contains("1.2M tokens"), "{screen}");
    }

    /// T441.13.7: the Tasks tab shows the snapshot's `tasks.text`, the CLI's own list.
    #[test]
    fn tasks_tab_renders_the_task_list() {
        let cfg = config();
        let mut app = App::new(&cfg);
        while app.page() != "tasks" {
            app.key(KeyCode::Right, KeyModifiers::NONE);
        }
        let mut snap = crate::model::snapshot(&cfg);
        snap.tasks.text = "R1  open  Ship the board\n".into();
        app.refresh(snap);
        assert!(screen(&app).contains("Ship the board"));
    }

    /// The bar's guards: nothing saved is no bar.
    #[test]
    fn bar_draws_nothing_for_nothing_saved() {
        assert_eq!(bar(0, 10), "");
        assert_eq!(bar(-3, 10), "");
        assert_eq!(bar(5, 0), "");
        assert_eq!(bar(1, 1_000_000), "█", "any positive saving draws");
    }

    /// T15.4: the Plugins tab lists every row the model serves, marks the cursor row,
    /// and hints the toggle keys — so the tab, `rtok plugins` and the web Plugins page
    /// agree on one catalogue (D23).
    #[test]
    fn plugins_tab_lists_rows_marks_the_cursor_and_hints_the_toggle() {
        let mut app = App::new(&config());
        app.key(KeyCode::Right, KeyModifiers::NONE);
        let first = screen(&app);
        for plugin in &app.snapshot().plugins {
            assert!(first.contains(plugin.id), "{} is on screen", plugin.id);
        }
        assert!(
            first.contains("Space/Enter"),
            "the key hint comes from the KEYS table"
        );
        assert!(
            first.contains("▸ measure"),
            "the cursor marks the first row"
        );
        app.key(KeyCode::Down, KeyModifiers::NONE);
        assert!(screen(&app).contains("▸ cmd"), "the cursor follows Down");
    }

    /// T419: the memory row's fields line shows the checkpoint prompt counts the store
    /// holds, read through the same model query as the web Plugins page.
    #[test]
    fn plugins_tab_shows_the_cursor_rows_checkpoint_counts() {
        let (cfg, store) = fresh_store("plugin-fields");
        store
            .kv_set(
                &crate::plugin::plugin_state_key("memory", "checkpoint:s1"),
                r#"{"typed":4,"skipped":9}"#,
            )
            .unwrap();
        drop(store);
        let app = crate::tui::app::tests::cursor_on_plugin(&cfg, "memory");
        let screen = screen(&app);
        assert!(screen.contains("checkpoint prompts typed 4"), "{screen}");
        assert!(
            screen.contains("checkpoint host records skipped 9"),
            "{screen}"
        );
    }

    /// T15.4: a toggle writes `<home>/config.toml` through `config set`'s writer and
    /// the tab shows the outcome in the row and the status line — both off the re-read
    /// model, never the key press's hope. `toon` starts on, so the toggle turns it off. Its
    /// own temp home, like the toggle test in `app`.
    #[test]
    fn plugins_toggle_shows_in_the_row_and_the_status_line() {
        let dir = std::env::temp_dir().join(format!("rtok-tui-plugins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = crate::testutil::config_file_in(&dir);
        let mut app = crate::tui::app::tests::cursor_on_plugin(&cfg, "toon");
        app.key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(
            !Config::load_from(&dir)
                .unwrap()
                .plugin_enabled("toon", false),
            "the file, not just the row"
        );
        let screen = screen(&app);
        assert!(screen.contains("toon off"), "the status names the outcome");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A config plus its store, in the caller's own numbered temp dir — seeding writes
    /// while other tests hold the shared `config()` store open, and two seeding tests
    /// running concurrently must not delete each other's store mid-migration (T15.3's
    /// `seeded` and T15.5's `calls_seeded` share this, so the shape lives once).
    fn fresh_store(name: &str) -> (Config, crate::store::Store) {
        static DIR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rtok-tui-{name}-{}-{}",
            std::process::id(),
            DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = crate::testutil::config_file_in(&dir);
        let store = crate::store::Store::open(&cfg.core.db_path).expect("seed store");
        (cfg, store)
    }

    fn expand_seeded() -> Config {
        let (mut cfg, store) = fresh_store("expand-calls");
        store.upsert_session("s", None, None, None, None).unwrap();
        let call = store
            .insert_call(
                "s",
                "hook",
                "plugin_run",
                None,
                None,
                None,
                Some("cmd"),
                None,
            )
            .unwrap();
        let id = store
            .put_archive("s", b"other-line\nNEEDLELINE\n", &cfg.core.archive_dir)
            .unwrap();
        let rt = Runtime::open(cfg.clone(), "seed").expect("seed runtime");
        rt.record(&Measurement {
            plugin: "cmd",
            kind: "filter",
            before_bytes: 20,
            after_bytes: 8,
            est_before: 5,
            est_after: 2,
            ref_id: Some(id),
            call_id: Some(call),
        })
        .unwrap();
        drop(rt);
        cfg.tui.tab = "calls".into();
        cfg
    }

    /// A config whose store holds two sessions, three turns and two measured plugins,
    /// so the Overview tab has numbers worth rendering.
    fn seeded() -> Config {
        let (cfg, store) = fresh_store("overview");
        {
            store.insert_proxy_turn("a", 1_500, 0, 0, 1_111).unwrap();
            store.insert_proxy_turn("a", 40, 0, 1_200, 2_222).unwrap();
            store
                .insert_proxy_turn("b", 10_000, 2_000, 0, 3_333)
                .unwrap();
        }
        let rt = Runtime::open(cfg.clone(), "seed").expect("seed runtime");
        for (plugin, before, after) in [("cmd", 30, 0), ("read", 30, 15)] {
            rt.record(&Measurement {
                plugin,
                kind: "filter",
                before_bytes: 100,
                after_bytes: 40,
                est_before: before,
                est_after: after,
                ref_id: None,
                call_id: None,
            })
            .unwrap();
        }
        cfg
    }

    /// T15.7: the Logs tab shows the model's log lines — the same selection `rtok logs`
    /// screens — newest first, and says so in the CLI's words when nothing is logged.
    #[test]
    fn logs_tab_renders_the_model_lines_newest_first() {
        let cfg = logged(0, &[]);
        let mut app = App::new(&cfg);
        select(&mut app, "logs");
        assert!(screen(&app).contains("no logs yet"), "the empty state");

        let cfg = logged(3, &["oldest", "middle", "newest"]);
        let mut app = App::new(&cfg);
        select(&mut app, "logs");
        let screen = screen(&app);
        for marker in ["oldest", "middle", "newest"] {
            assert!(screen.contains(marker), "{marker} is on screen");
        }
        assert!(
            screen.find("newest").unwrap() < screen.find("oldest").unwrap(),
            "newest is the first line on screen"
        );
    }

    /// The page's bound is the model's: `[log] lines` lines ride the snapshot, so the
    /// tab shows that many and not the file.
    #[test]
    fn logs_tab_honors_the_log_lines_bound() {
        let cfg = logged(2, &["one", "two", "three"]);
        let mut app = App::new(&cfg);
        assert_eq!(
            app.snapshot().logs.len(),
            2,
            "the snapshot carries the bound"
        );
        select(&mut app, "logs");
        let screen = screen(&app);
        assert!(
            screen.contains("three") && screen.contains("two"),
            "the two newest"
        );
        assert!(!screen.contains("one"), "the bound dropped the oldest");
    }

    /// Jump the app to a named tab with the digit key the shell owns (tabs count
    /// from one).
    fn select(app: &mut App, page: &str) {
        let idx = app
            .tab_names()
            .iter()
            .position(|name| *name == page)
            .expect("the model offers the page");
        assert!(idx < 9, "digits reach the first nine tabs");
        app.key(
            KeyCode::Char((b'1' + idx as u8) as char),
            KeyModifiers::NONE,
        );
    }

    /// A config whose log file holds one line per marker, oldest first, and
    /// `[log] lines` set to `lines` — `0` for the empty-state case. Its own temp dir
    /// (keyed by the bound, so the parallel tests never share one), like `seeded`:
    /// the log path is the config's, so the store stays untouched.
    fn logged(lines: usize, markers: &[&str]) -> Config {
        let dir =
            std::env::temp_dir().join(format!("rtok-tui-logs-{lines}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = crate::testutil::config_file_in(&dir);
        cfg.log.lines = lines;
        std::fs::create_dir_all(cfg.log.path.parent().expect("log dir")).unwrap();
        let body = markers
            .iter()
            .map(|m| format!("2026-09-10 12:00:00 info tui/test: {m}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&cfg.log.path, format!("{body}\n")).unwrap();
        cfg
    }

    #[rstest]
    fn live_passthrough_rows_show_bytes_not_tokens() {
        let _ring = crate::proxy::live::test_lock();
        crate::proxy::live::clear();
        crate::proxy::live::push(crate::proxy::LiveCall {
            ts: 1,
            method: "POST".into(),
            path: "/v1/messages".into(),
            provider: None,
            model: None,
            status: 200,
            request_bytes: 100,
            response_bytes: 50,
            ms: 5.0,
        });
        let mut cfg = calls_seeded();
        cfg.proxy.enabled = false;
        let mut app = App::new(&cfg);
        app.key(KeyCode::Char('3'), KeyModifiers::NONE);
        let screen = screen(&app);
        assert!(
            screen.contains("150 B"),
            "live passthrough sums request+response bytes"
        );
        assert!(
            screen.contains("16 tok"),
            "linked api_request still shows tokens"
        );
        crate::proxy::live::clear();
    }

    /// T15.5: the Calls tab lists the ledger's rows newest first — surface, kind,
    /// session, latency, the linked usage tokens — straight off the snapshot (D23), and
    /// the detail pane is closed until a key opens it.
    #[test]
    fn calls_tab_lists_rows_newest_first() {
        let mut app = App::new(&calls_seeded());
        app.key(KeyCode::Char('3'), KeyModifiers::NONE);
        assert_eq!(app.page(), "calls");
        let screen = screen(&app);
        for what in ["api_request", "mcp_call", "plugin_run", "PostToolUse"] {
            assert!(screen.contains(what), "{what} is on screen");
        }
        assert!(screen.contains("12.5"), "the recorded latency");
        assert!(screen.contains("16 tok"), "the linked usage summed");
        // Newest first: the api_request row (inserted last) sits above the plugin_run
        // one, which sits above the hook row.
        let (api, run, hook) = (
            screen.find("api_request").unwrap(),
            screen.find("plugin_run").unwrap(),
            screen.find("PostToolUse").unwrap(),
        );
        assert!(api < run && run < hook);
        assert!(!screen.contains("usage ("), "no detail until Enter/z");
    }

    /// T15.5: `Enter`/`z` expand the selected row — the full ledger fields plus the
    /// usage/api linkage — and collapse it again; the selection walks with `Up`/`Down`
    /// and the detail follows it.
    #[test]
    fn enter_expands_the_selected_row_and_z_collapses_it() {
        let mut app = App::new(&calls_seeded());
        app.key(KeyCode::Char('3'), KeyModifiers::NONE);
        app.key(KeyCode::Enter, KeyModifiers::NONE);
        let open = screen(&app);
        assert!(open.contains("input 10"), "the linkage's counters");
        assert!(open.contains("cache read 2"));
        assert!(open.contains("usage (anthropic)"));
        assert!(open.contains("claude-x"), "the model slug joined");
        app.key(KeyCode::Char('z'), KeyModifiers::NONE);
        assert!(!screen(&app).contains("input 10"), "z closes the pane");

        // The selection walks to the mcp row; the detail follows it and says there is
        // no usage linkage to show.
        app.key(KeyCode::Down, KeyModifiers::NONE);
        app.key(KeyCode::Enter, KeyModifiers::NONE);
        let open = screen(&app);
        assert!(open.contains("mcp_call"), "the mcp row is the selection");
        assert!(open.contains("no row linked"));
    }

    /// T60.4: `e` opens the selected call's archive through `expand_payload`; `/`
    /// filters the pane with grep parity.
    #[test]
    fn e_opens_the_archive_pane_and_slash_filters_it() {
        let mut cfg = expand_seeded();
        cfg.tui.tab = "calls".into();
        let mut app = App::new(&cfg);
        assert_eq!(app.page(), "calls");
        app.key(KeyCode::Enter, KeyModifiers::NONE);
        let detail = screen(&app);
        assert!(detail.contains("ref_id "), "detail prints the archive id");
        app.key(KeyCode::Char('e'), KeyModifiers::NONE);
        let open = screen(&app);
        assert!(
            open.contains("NEEDLELINE"),
            "payload is in the pane: {open}"
        );
        assert!(open.contains("expand "), "the pane is titled");
        app.key(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "NEEDLE".chars() {
            app.key(KeyCode::Char(c), KeyModifiers::NONE);
        }
        let filtered = screen(&app);
        assert!(filtered.contains("NEEDLELINE"), "{filtered}");
        assert!(!filtered.contains("other-line"), "{filtered}");
    }

    /// An empty ledger is an empty page, and its keys do nothing — not a panic.
    #[test]
    fn calls_keys_do_nothing_on_an_empty_ledger() {
        let mut app = App::new(&config());
        app.key(KeyCode::Char('3'), KeyModifiers::NONE);
        app.key(KeyCode::Down, KeyModifiers::NONE);
        app.key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(!app.calls_detail());
        assert!(screen(&app).contains("no calls yet"));
    }

    /// A config whose store holds four calls across two sessions — a hook, a plugin run
    /// nested under it, an mcp call, and a proxied api request with its usage row and
    /// host/provider/model slugs — so the Calls tab has rows worth listing and a
    /// linkage worth expanding.
    fn calls_seeded() -> Config {
        let (cfg, store) = fresh_store("calls");
        {
            let claude = store.host_id("claude").unwrap().expect("0002 seeds claude");
            store
                .upsert_session("older", None, None, None, Some("hook"))
                .unwrap();
            store
                .upsert_session("newer", None, None, None, Some("proxy"))
                .unwrap();
            let hook = store
                .insert_call(
                    "older",
                    "hook",
                    "hook",
                    None,
                    None,
                    None,
                    None,
                    Some("PostToolUse"),
                )
                .unwrap();
            let run = store
                .insert_call(
                    "older",
                    "hook",
                    "plugin_run",
                    None,
                    None,
                    None,
                    Some("cmd"),
                    None,
                )
                .unwrap();
            store.set_call_parent(run, hook).unwrap();
            let mcp = store
                .insert_call(
                    "newer",
                    "mcp",
                    "mcp_call",
                    None,
                    None,
                    None,
                    Some("toon"),
                    Some("plan_next"),
                )
                .unwrap();
            store.set_call_ms(mcp, 3.0).unwrap();
            let (pid, mid) = store.upsert_model("anthropic", "claude-x").unwrap();
            let api = store
                .insert_call(
                    "newer",
                    "proxy",
                    "api_request",
                    Some(claude),
                    Some(pid),
                    Some(mid),
                    None,
                    Some("/v1/messages"),
                )
                .unwrap();
            store.set_call_ms(api, 12.5).unwrap();
            store
                .insert_usage("newer", Some("claude-x"), "anthropic", 10, 1, 2, 3, api)
                .unwrap();
        }
        cfg
    }

    #[test]
    fn header_names_the_project_and_the_window() {
        let line = header_line(Rect::new(0, 0, 120, 40)).to_string();
        assert!(line.contains("rtok"), "line: {line}");
        assert!(line.ends_with("120×40"), "line: {line}");
    }

    #[test]
    fn footer_hints_the_keys_and_the_last_tick() {
        let app = App::new(&config());
        let line = footer_line().to_string();
        assert!(line.contains("q/Esc"), "line: {line}");
        assert!(
            line.contains("?") && line.contains("r"),
            "the footer names the T60.8 keys: {line}"
        );
        let status = header_status(&app).to_string();
        assert!(status.contains("UTC"), "status: {status}");
    }

    /// A session total whose model column carries the marker — the row field the
    /// sessions assertions grep on screen.
    fn session_row(n: usize, ended: Option<i64>) -> crate::store::SessionTotals {
        crate::store::SessionTotals {
            id: format!("sess-{n}"),
            host: Some("claude".into()),
            project: None,
            provider: None,
            api: None,
            model: Some(format!("m{n}")),
            input: 0,
            cache_create: 0,
            cache_read: 0,
            output: 0,
            started_at: 0,
            last_activity: 0,
            ended_at: ended,
        }
    }

    fn three_skills() -> model::SkillsPage {
        model::skills_from(
            Some(&crate::doctor::SkillsAudit {
                rows: vec![
                    crate::doctor::SkillRow {
                        name: "hot".into(),
                        source: "user".into(),
                        desc_chars: 40,
                        body_bytes: 100,
                        invocations: Some(3),
                        warn_desc: false,
                        warn_body: false,
                        warn_never: false,
                    },
                    crate::doctor::SkillRow {
                        name: "cold".into(),
                        source: "project".into(),
                        desc_chars: 10,
                        body_bytes: 20,
                        invocations: Some(0),
                        warn_desc: false,
                        warn_body: false,
                        warn_never: true,
                    },
                    crate::doctor::SkillRow {
                        name: "plug".into(),
                        source: "plugin:x".into(),
                        desc_chars: 8,
                        body_bytes: 50,
                        invocations: Some(1),
                        warn_desc: false,
                        warn_body: false,
                        warn_never: false,
                    },
                ],
                desc_bytes: 58,
                ..crate::doctor::SkillsAudit::default()
            }),
            None,
            10_000,
            true,
        )
    }

    #[test]
    fn skills_tab_lists_three_rows_and_n_hides_invoked() {
        let mut cfg = config();
        cfg.tui.tab = "skills".into();
        let mut app = App::new(&cfg);
        app.set_skills(model::SkillsPage::default());
        assert!(
            screen(&app).contains("no skills listed"),
            "listing-empty, not stats-empty"
        );
        app.set_skills(three_skills());
        let shown = screen(&app);
        assert!(shown.contains("hot"), "{shown}");
        assert!(shown.contains("cold"), "{shown}");
        assert!(shown.contains("plug"), "{shown}");
        assert!(shown.contains("never"), "never-invoked marked: {shown}");
        assert!(shown.contains("resident"), "{shown}");
        app.key(KeyCode::Char('n'), KeyModifiers::NONE);
        let filtered = screen(&app);
        assert!(filtered.contains("cold"), "{filtered}");
        assert!(!filtered.contains("hot"), "n hides invoked: {filtered}");
        assert!(filtered.contains("n again shows all"), "{filtered}");
    }

    /// T60.10: the Sessions tab has the Calls row model — an empty state, both rows
    /// listed, `l` narrowing to live-only and back.
    #[test]
    fn sessions_tab_lists_rows_and_l_filters_live_only() {
        let cfg = config();
        let mut app = App::new(&cfg);
        select(&mut app, "sessions");
        assert!(screen(&app).contains("no sessions yet"), "the empty state");
        app.refresh({
            let mut snap = model::snapshot(&cfg);
            snap.sessions = vec![session_row(0, None), session_row(1, Some(9))];
            snap
        });
        let rendered = screen(&app);
        assert!(
            rendered.contains("m0") && rendered.contains("m1"),
            "{rendered}"
        );
        assert!(
            rendered.contains("↑/↓"),
            "the status comes from the KEYS table"
        );
        app.key(KeyCode::Char('l'), KeyModifiers::NONE);
        let rendered = screen(&app);
        assert!(rendered.contains("m0"), "the live row stays");
        assert!(!rendered.contains("m1"), "the ended row is filtered off");
        app.key(KeyCode::Char('l'), KeyModifiers::NONE);
        assert!(screen(&app).contains("m1"), "l toggles the filter back");
    }

    /// T60.3: Enter opens a detail pane from `model::session_detail` — project, api,
    /// timestamps, the API usage row, and this session's snapshot calls only.
    #[test]
    fn enter_opens_the_session_detail_pane() {
        let cfg = config();
        let mut app = App::new(&cfg);
        select(&mut app, "sessions");
        app.refresh({
            let mut snap = model::snapshot(&cfg);
            let mut row = session_row(0, None);
            row.id = "a".into();
            row.project = Some("rtok".into());
            row.api = Some("anthropic".into());
            row.started_at = 1;
            row.last_activity = 2;
            row.input = 30;
            row.cache_create = 1;
            row.cache_read = 7;
            row.output = 7;
            snap.sessions = vec![row];
            snap.calls = vec![
                crate::store::CallRow {
                    id: 1,
                    ts: 3661,
                    session: "a".into(),
                    surface: "proxy".into(),
                    kind: "api_request".into(),
                    plugin: None,
                    name: Some("/v1/messages".into()),
                    parent_id: None,
                    ms: Some(12.5),
                    ok: 1,
                    error: None,
                    host: None,
                    provider: None,
                    model: None,
                    api: Some("anthropic".into()),
                    input: Some(10),
                    cache_create: Some(1),
                    cache_read: Some(2),
                    output: Some(3),
                },
                crate::store::CallRow {
                    id: 2,
                    ts: 3,
                    session: "other".into(),
                    surface: "hook".into(),
                    kind: "hook".into(),
                    plugin: None,
                    name: Some("Skip".into()),
                    parent_id: None,
                    ms: None,
                    ok: 1,
                    error: None,
                    host: None,
                    provider: None,
                    model: None,
                    api: None,
                    input: None,
                    cache_create: None,
                    cache_read: None,
                    output: None,
                },
            ];
            snap
        });
        assert!(
            !screen(&app).contains("project rtok"),
            "no detail until Enter"
        );
        app.key(KeyCode::Enter, KeyModifiers::NONE);
        let open = screen(&app);
        assert!(open.contains("project rtok"), "{open}");
        assert!(open.contains("api anthropic"), "{open}");
        assert!(open.contains("started"), "{open}");
        assert!(open.contains("last"), "{open}");
        assert!(open.contains("ended live"), "{open}");
        assert!(open.contains("usage (anthropic)"), "{open}");
        assert!(open.contains("/v1/messages"), "{open}");
        assert!(
            !open.contains("Skip"),
            "other session's calls stay off: {open}"
        );
        app.key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            !screen(&app).contains("project rtok"),
            "Enter closes the pane"
        );
    }

    /// T60.10: a store taller than the terminal scrolls to keep the cursor row on
    /// screen — the last row of 200 is visible after walking all the way down.
    #[test]
    fn sessions_tab_scrolls_to_keep_the_cursor_visible() {
        let cfg = config();
        let mut app = App::new(&cfg);
        select(&mut app, "sessions");
        app.refresh({
            let mut snap = model::snapshot(&cfg);
            snap.sessions = (0..200).map(|n| session_row(n, None)).collect();
            snap
        });
        for _ in 0..199 {
            app.key(KeyCode::Down, KeyModifiers::NONE);
        }
        let screen = screen(&app);
        assert!(screen.contains("m199"), "the cursor row is on screen");
        assert!(!screen.contains("m0 "), "the top scrolled off");
    }

    /// T60.8: `?` opens the help overlay listing the global keys plus the current
    /// page's, generated from the KEYS table; `?` again closes it.
    #[test]
    fn help_overlay_lists_the_keys_and_toggles() {
        let cfg = config();
        let mut app = App::new(&cfg);
        select(&mut app, "sessions");
        let closed = screen(&app);
        // T226: the status line hints `l live-only filter` itself, so the overlay is
        // told apart by its title.
        assert!(!closed.contains("keys —"), "closed until ?: {closed}");
        app.key(KeyCode::Char('?'), KeyModifiers::NONE);
        let open = screen(&app);
        assert!(open.contains("keys — sessions"), "{open}");
        // Globals and the page's own rows, off the one table.
        assert!(open.contains("refresh now"), "{open}");
        assert!(open.contains("live-only filter"), "{open}");
        assert!(
            !open.contains("toggle plugin"),
            "another page's rows stay off: {open}"
        );
        app.key(KeyCode::Char('?'), KeyModifiers::NONE);
        assert!(!screen(&app).contains("keys —"), "? closes the overlay");
    }

    /// T60.6: an unreadable store renders an error line instead of a silent empty page.
    #[test]
    fn unreadable_store_shows_an_error_banner() {
        let dir = std::env::temp_dir().join(format!("rtok-tui-store-err-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = crate::testutil::config_file_in(&dir);
        cfg.core.db_path = dir.join("not-a-db");
        std::fs::create_dir_all(&cfg.core.db_path).unwrap();
        let app = App::new(&cfg);
        assert!(app.snapshot().error.is_some(), "{:?}", app.snapshot().error);
        let screen = screen(&app);
        assert!(screen.contains('✕'), "error banner: {screen}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T60.8: `r` re-reads the model immediately, before the next tick.
    #[test]
    fn r_refreshes_the_snapshot_immediately() {
        let cfg = config();
        let mut app = App::new(&cfg);
        let before = app.updated();
        app.key(KeyCode::Char('r'), KeyModifiers::NONE);
        assert!(app.updated() >= before, "the stamp moved");
        assert_eq!(
            app.snapshot().plugins.len(),
            model::snapshot(&cfg).plugins.len()
        );
        // The shell's keys still work; `r` is not a quit.
        assert!(!app.key(KeyCode::Char('r'), KeyModifiers::NONE));
    }
}

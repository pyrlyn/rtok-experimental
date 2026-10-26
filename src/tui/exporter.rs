// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! The Graph page's export and saved-export views (T329.41, D27): the terminal counterpart of
//! `rtok graph export`. The write is [`export::write`], the function the command's `--output`
//! ends in, with the command's defaults (redaction on, the 200 best connected nodes in a picture),
//! so the file is the CLI's bytes. This module is the form, the background run and the read-only
//! view of a saved export, which opens the file with [`export::read`] and writes nothing. The
//! panel replaces the page body while it is open, as compare does.

use crossterm::event::KeyCode;
use ratatui::style::Style;
use ratatui::text::Line;

use super::panel::{self, Answer, Panel, Stage, error_lines};
use super::theme;
use crate::config::Config;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Format {
    Json,
    Svg,
    Png,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Level {
    Overview,
    Symbols,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Format,
    Level,
    Redact,
    Path,
}

const FIELDS: [Field; 4] = [Field::Format, Field::Level, Field::Redact, Field::Path];

#[derive(Clone)]
struct Form {
    format: Format,
    level: Level,
    redact: bool,
    path: String,
    at: usize,
    note: String,
}

/// The value after `cur`, wrapping.
fn next<T: Copy + PartialEq>(all: &[T], cur: T) -> T {
    let i = all.iter().position(|x| *x == cur).unwrap_or(0);
    all[(i + 1) % all.len()]
}

impl Default for Form {
    fn default() -> Self {
        Self {
            format: Format::Json,
            level: Level::Overview,
            redact: true,
            path: String::new(),
            at: FIELDS.len() - 1,
            note: String::new(),
        }
    }
}

impl Form {
    fn cycle(&mut self) {
        match FIELDS[self.at] {
            Field::Format => {
                self.format = next(&[Format::Json, Format::Svg, Format::Png], self.format)
            }
            Field::Level => self.level = next(&[Level::Overview, Level::Symbols], self.level),
            Field::Redact => self.redact = !self.redact,
            Field::Path => {}
        }
    }

    fn lines(&self, project: &str) -> Vec<Line<'static>> {
        let rows = [
            ("format", format!("{:?}", self.format).to_lowercase()),
            ("level", format!("{:?}", self.level).to_lowercase()),
            (
                "redact",
                if self.redact {
                    "on".to_owned()
                } else {
                    "off: home, user name and absolute paths are written".to_owned()
                },
            ),
            (
                "file",
                if FIELDS[self.at] == Field::Path {
                    format!("{}▏", self.path)
                } else {
                    self.path.clone()
                },
            ),
        ];
        let mut out = vec![Line::styled(
            format!("export the graph of {project}"),
            Style::new().bold(),
        )];
        for (i, (name, value)) in rows.into_iter().enumerate() {
            let mark = if i == self.at { "▸ " } else { "  " };
            out.push(Line::from(format!("{mark}{name:<7}{value}")));
        }
        out.push(Line::styled(
            "svg and png draw the 200 best connected nodes. File and symbol names are included; source text never is.",
            theme::muted(),
        ));
        if !self.note.is_empty() {
            out.push(Line::styled(
                self.note.clone(),
                Style::new().fg(theme::WARN),
            ));
        }
        out
    }
}

/// What the panel is asking for while a field has the keys.
#[derive(Clone, Copy)]
pub(super) enum Ask {
    Form,
    /// Typing the path of a saved export to view.
    Open,
}

pub(super) type Exporter = Panel<Export>;

#[derive(Default)]
pub(super) struct Export {
    form: Form,
    input: String,
    /// The registry id handed to the export, and the name the panel calls it by.
    project: (String, String),
}

impl panel::View for Export {
    type Ask = Ask;
    const LABEL: &'static str = "export";

    fn open(&mut self, code: KeyCode, target: Option<(i32, &str)>) -> Option<Stage<Ask>> {
        match code {
            KeyCode::Char('e') => Some(match target {
                Some((id, name)) => {
                    self.project = (id.to_string(), name.to_owned());
                    // The choices stay, as a second export of the same graph differs in one
                    // field; the name does not, or typing a new one would append to the last.
                    self.form.at = FIELDS.len() - 1;
                    self.form.path.clear();
                    self.form.note.clear();
                    Stage::Prompt(Ask::Form)
                }
                None => Stage::Shown(error_lines("no project to export")),
            }),
            KeyCode::Char('v') => {
                self.input.clear();
                Some(Stage::Prompt(Ask::Open))
            }
            _ => None,
        }
    }

    fn prompt(
        &mut self,
        ask: Ask,
        code: KeyCode,
        cfg: &Config,
        background: bool,
    ) -> Option<Stage<Ask>> {
        match ask {
            Ask::Form => self.form_key(code, cfg, background),
            Ask::Open => match code {
                KeyCode::Esc => Some(Stage::Closed),
                KeyCode::Enter => self.view(background),
                _ => {
                    panel::edit(&mut self.input, code);
                    None
                }
            },
        }
    }

    fn prompt_lines(&self, ask: Ask) -> Vec<Line<'static>> {
        match ask {
            Ask::Form => self.form.lines(&self.project.1),
            Ask::Open => vec![
                Line::from(format!("view the saved export: {}▏", self.input)),
                Line::styled(
                    "the path of a JSON file `rtok graph export` wrote; it is only read",
                    theme::muted(),
                ),
            ],
        }
    }

    fn running_lines(&self) -> Vec<Line<'static>> {
        vec![Line::styled("working …", theme::muted())]
    }
}

impl Export {
    fn form_key(&mut self, code: KeyCode, cfg: &Config, background: bool) -> Option<Stage<Ask>> {
        let f = &mut self.form;
        match code {
            KeyCode::Esc => return Some(Stage::Closed),
            KeyCode::Up => f.at = f.at.saturating_sub(1),
            KeyCode::Down => f.at = (f.at + 1).min(FIELDS.len() - 1),
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if FIELDS[f.at] != Field::Path => {
                f.cycle();
            }
            KeyCode::Backspace if FIELDS[f.at] == Field::Path => {
                f.path.pop();
            }
            KeyCode::Char(c) if FIELDS[f.at] == Field::Path => f.path.push(c),
            KeyCode::Enter if f.path.trim().is_empty() => {
                f.note = "name the file to write, for example graph.json".into();
            }
            KeyCode::Enter => {
                let (cfg, project, form) = (cfg.clone(), self.project.0.clone(), f.clone());
                return Some(Stage::run(
                    <Self as panel::View>::LABEL,
                    background,
                    move || body::write(&cfg, &project, &form),
                ));
            }
            _ => {}
        }
        None
    }

    fn view(&self, background: bool) -> Option<Stage<Ask>> {
        let path = self.input.trim().to_owned();
        (!path.is_empty()).then(|| {
            Stage::run(<Self as panel::View>::LABEL, background, move || {
                body::view(&path)
            })
        })
    }
}

#[cfg(not(feature = "graph"))]
mod body {
    use super::{Answer, Form};
    use crate::config::Config;

    pub(super) fn write(_: &Config, _: &str, _: &Form) -> Answer {
        super::panel::no_graph()
    }

    pub(super) fn view(_: &str) -> Answer {
        super::panel::no_graph()
    }
}

#[cfg(feature = "graph")]
mod body {
    use std::path::Path;

    use ratatui::style::Style;
    use ratatui::text::Line;

    use super::{Answer, Form, Format, Level};
    use crate::config::Config;
    use crate::plugin::Runtime;
    use crate::plugins::graph::export::{self, Export};
    use crate::plugins::graph::scope;
    use crate::tui::theme::{self, ACCENT, OK, WARN};

    /// Rows of nodes the view lists; the file has them all, `rtok graph export` prints them.
    const NODES_SHOWN: usize = 40;

    /// The command's own call with its defaults: scale 1, an opaque background, pretty JSON.
    pub(super) fn write(cfg: &Config, project: &str, job: &Form) -> Answer {
        let run = || -> anyhow::Result<Vec<Line<'static>>> {
            let rt = Runtime::open(cfg.clone(), "tui-export")?;
            let members = scope::resolve(&rt.store, Some(project), Path::new(""))?;
            let n = export::write(
                &rt,
                &members,
                &export::Query {
                    level: match job.level {
                        Level::Overview => export::Level::Overview,
                        Level::Symbols => export::Level::Symbols,
                    },
                    focus: None,
                    depth: 2,
                    redact: job.redact,
                    pretty: true,
                    from: None,
                },
                &export::Image {
                    format: match job.format {
                        Format::Json => export::Format::Json,
                        Format::Svg => export::Format::Svg,
                        Format::Png => export::Format::Png,
                    },
                    transparent: false,
                    scale: 1,
                },
                Path::new(job.path.trim()),
            )?;
            let mut out = vec![theme::banner(
                '✓',
                &format!("wrote {n} bytes to {}", job.path.trim()),
                OK,
            )];
            if !job.redact {
                out.push(Line::styled(
                    "redaction was off: the file holds the home directory, the user name and absolute paths.",
                    Style::new().fg(WARN),
                ));
            }
            Ok(out)
        };
        run().map_err(|e| format!("{e:#}"))
    }

    /// Reads the file with the importer the page and `--from` use; nothing is written, and the
    /// registry and the index are not opened.
    pub(super) fn view(path: &str) -> Answer {
        export::read(Path::new(path))
            .map(|e| lines(&e, path))
            .map_err(|e| format!("{e:#}"))
    }

    fn lines(e: &Export, path: &str) -> Vec<Line<'static>> {
        let name = |id: i32| {
            e.projects
                .iter()
                .find(|p| p.id == id)
                .map_or_else(|| id.to_string(), |p| p.name.clone())
        };
        let m = &e.meta;
        let yes = |b: bool| if b { "yes" } else { "no" };
        let head =
            |title: &str, n: usize| Line::styled(format!("{title} ({n})"), Style::new().bold());
        let focus = m
            .focus
            .as_ref()
            .map_or_else(String::new, |f| format!(" of {f}"));
        let mut out = vec![
            theme::banner(
                'ℹ',
                &format!("viewing export from {path} (read-only)"),
                ACCENT,
            ),
            Line::from(format!(
                "{}{focus} · scope {} · exported {} UTC · rtok {} · redacted {} · partial {}",
                m.level,
                m.scope.join(", "),
                crate::log::stamp(m.exported_at),
                m.rtok_version,
                yes(m.redacted),
                yes(m.partial),
            )),
            head("projects", e.projects.len()),
        ];
        out.extend(e.projects.iter().map(|p| {
            Line::from(format!(
                "  {}  {}  {}  {}",
                p.name, p.backend, p.health, p.root
            ))
        }));
        out.push(head("links", e.links.len()));
        out.extend(e.links.iter().map(|l| {
            let to = name(l.to);
            Line::from(format!(
                "  {} → {to} ({}, {} references)",
                name(l.from),
                l.kind,
                l.references
            ))
        }));
        out.push(head("nodes", e.nodes.len()));
        out.extend(e.nodes.iter().take(NODES_SHOWN).map(|n| {
            Line::from(format!(
                "  {}  {}  {}:{}  ({})",
                n.kind,
                n.name,
                n.path,
                n.line,
                name(n.project)
            ))
        }));
        let more = e.nodes.len().saturating_sub(NODES_SHOWN);
        if more > 0 {
            out.push(Line::styled(
                format!(
                    "{more} more nodes are in the file; `rtok graph export --from` draws them."
                ),
                theme::muted(),
            ));
        }
        out.push(head("edges", e.edges.len()));
        out.extend(
            m.notes
                .iter()
                .map(|n| Line::styled(n.clone(), theme::muted())),
        );
        out
    }
}

#[cfg(all(test, feature = "graph"))]
mod tests {
    use std::path::{Path, PathBuf};

    use crossterm::event::KeyModifiers;

    use super::*;
    use crate::plugin::Runtime;
    use crate::plugins::graph::{export, scope};
    use crate::store::Origin;
    use crate::tui::app::App;
    use crate::tui::app::tests::{config, render};

    const SRC: &str = "pub fn caller() {\n    callee();\n}\n\npub fn callee() {}\n";

    /// A registered project `p` in a temp dir; never the real home or repo.
    fn fixture() -> (App, Config, PathBuf) {
        let mut cfg = config();
        cfg.tui.tab = "graph".into();
        let base = crate::testutil::tmp_dir("tui-export");
        let root = base.join("p");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("lib.rs"), SRC).unwrap();
        let rt = Runtime::open(cfg.clone(), "t32941-fixture").unwrap();
        rt.store.register_project(&root, Origin::Manual).unwrap();
        (App::new(&cfg), cfg, base)
    }

    fn press(app: &mut App, code: KeyCode) {
        assert!(!app.key(code, KeyModifiers::NONE), "{code:?} quit");
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    /// Opens the form, picks `format` and `level` with Space, names the file and writes it.
    fn export_to(
        app: &mut App,
        format_steps: usize,
        level_steps: usize,
        redact_steps: usize,
        file: &Path,
    ) {
        press(app, KeyCode::Char('e'));
        for (steps, up) in [(format_steps, 3), (level_steps, 2), (redact_steps, 1)] {
            for _ in 0..up {
                press(app, KeyCode::Up);
            }
            for _ in 0..steps {
                press(app, KeyCode::Char(' '));
            }
            for _ in 0..up {
                press(app, KeyCode::Down);
            }
        }
        type_text(app, file.to_str().unwrap());
        press(app, KeyCode::Enter);
    }

    /// What `rtok graph export` writes for the same scope, called with the command's own defaults.
    fn cli_bytes(
        cfg: &Config,
        format: export::Format,
        level: export::Level,
        redact: bool,
    ) -> Vec<u8> {
        let rt = Runtime::open(cfg.clone(), "t32941-cli").unwrap();
        let members = scope::resolve(&rt.store, Some("1"), Path::new("")).unwrap();
        export::render(
            &rt,
            &members,
            &export::Query {
                level,
                focus: None,
                depth: 2,
                redact,
                pretty: true,
                from: None,
            },
            &export::Image {
                format,
                transparent: false,
                scale: 1,
            },
        )
        .unwrap()
    }

    /// Every format carries the export time to the second (`exported_at` in the JSON, the footer
    /// of a picture) and the index time, which every export refreshes, which a loaded machine can move between the key and the command, so that
    /// stamp is blanked on both sides. A PNG holds the stamp as pixels and is held to its
    /// signature and size, the header.
    fn same_as_cli(
        app: &mut App,
        cfg: &Config,
        steps: (usize, usize, usize),
        want: (export::Format, export::Level, bool),
        file: &Path,
    ) {
        let blank = |bytes: Vec<u8>| {
            if want.0 == export::Format::Png {
                return bytes[..24].to_vec();
            }
            let re = regex::Regex::new(
                r#"\d{4}-\d\d-\d\d \d\d:\d\d:\d\d UTC|"(exported|indexed)_at": \d+"#,
            )
            .unwrap();
            re.replace_all(&String::from_utf8(bytes).unwrap(), "<time>")
                .into_owned()
                .into_bytes()
        };
        export_to(app, steps.0, steps.1, steps.2, file);
        let cli = cli_bytes(cfg, want.0, want.1, want.2);
        let got = std::fs::read(file).unwrap();
        assert!(!got.is_empty());
        let (got, cli) = (blank(got), blank(cli));
        assert!(
            got == cli,
            "{}\n{}",
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(&cli)
        );
    }

    #[test]
    fn the_key_writes_the_bytes_the_cli_writes_for_json_svg_and_png() {
        let (mut app, cfg, base) = fixture();
        // The first symbols export indexes the project, which changes its health and `indexed_at`;
        // after this every run sees the same index.
        cli_bytes(&cfg, export::Format::Json, export::Level::Symbols, true);
        // Default form: json, overview, redact on.
        same_as_cli(
            &mut app,
            &cfg,
            (0, 0, 0),
            (export::Format::Json, export::Level::Overview, true),
            &base.join("a.json"),
        );
        assert!(render(&app, 100).contains("wrote"));
        // The result stays up; `e` opens the form again with the last choices.
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Esc);
        same_as_cli(
            &mut app,
            &cfg,
            (1, 1, 0),
            (export::Format::Svg, export::Level::Symbols, true),
            &base.join("a.svg"),
        );
        assert!(
            std::fs::read_to_string(base.join("a.svg"))
                .unwrap()
                .starts_with("<svg")
        );
        // PNG needs the rasteriser, which only the `graph-png` feature links.
        #[cfg(feature = "graph-png")]
        {
            same_as_cli(
                &mut app,
                &cfg,
                (1, 0, 0),
                (export::Format::Png, export::Level::Symbols, true),
                &base.join("a.png"),
            );
            assert_eq!(
                &std::fs::read(base.join("a.png")).unwrap()[..8],
                b"\x89PNG\r\n\x1a\n"
            );
        }
    }

    #[test]
    fn redaction_is_on_until_the_user_turns_it_off() {
        let (mut app, _, base) = fixture();
        let on = base.join("on.json");
        export_to(&mut app, 0, 0, 0, &on);
        let on: serde_json::Value = serde_json::from_slice(&std::fs::read(on).unwrap()).unwrap();
        assert_eq!(on["meta"]["redacted"], true);
        // The root field, not the serialised text: JSON escapes Windows backslashes.
        let root = |v: &serde_json::Value| v["projects"][0]["root"].as_str().unwrap().to_owned();
        assert!(
            !std::path::Path::new(&root(&on)).is_absolute(),
            "{}",
            root(&on)
        );

        let off = base.join("off.json");
        export_to(&mut app, 0, 0, 1, &off);
        let shown = render(&app, 100);
        assert!(shown.contains("redaction was off"), "{shown}");
        let off: serde_json::Value = serde_json::from_slice(&std::fs::read(off).unwrap()).unwrap();
        assert_eq!(off["meta"]["redacted"], false);
        let raw = root(&off);
        assert!(
            std::path::Path::new(&raw).is_absolute() && raw.ends_with('p'),
            "{raw}"
        );
    }

    #[test]
    fn typed_keys_stay_in_the_file_name_and_an_empty_name_writes_nothing() {
        let (mut app, _, base) = fixture();
        let page = render(&app, 128);
        assert!(
            page.contains("e export graph") && page.contains("v view saved export"),
            "{page}"
        );
        press(&mut app, KeyCode::Char('e'));
        // `?`, `r`, `q`, `e` and a digit would be help, refresh, quit, reopen and a tab jump.
        type_text(&mut app, "r?qe3");
        assert!(render(&app, 100).contains("r?qe3▏"));
        assert!(!app.help_open());
        assert_eq!(app.page(), "graph");
        for _ in 0..5 {
            press(&mut app, KeyCode::Backspace);
        }
        press(&mut app, KeyCode::Enter);
        assert!(render(&app, 100).contains("name the file to write"));
        press(&mut app, KeyCode::Esc);
        assert!(render(&app, 100).contains("e export graph"));
        assert_eq!(
            std::fs::read_dir(&base).unwrap().count(),
            1,
            "only the project dir"
        );
        assert!(
            app.key(KeyCode::Esc, KeyModifiers::NONE),
            "the shell's Esc is back"
        );
    }

    #[test]
    fn a_failed_write_is_a_line_not_a_crash() {
        let (mut app, _, base) = fixture();
        export_to(&mut app, 0, 0, 0, &base.join("no-such-dir").join("x.json"));
        assert!(render(&app, 100).contains('✕'));
    }

    #[test]
    fn the_saved_export_view_shows_the_file_and_writes_nothing() {
        let (mut app, _, base) = fixture();
        let file = base.join("saved.json");
        export_to(&mut app, 0, 1, 0, &file);
        let before = std::fs::read(&file).unwrap();
        let listing = || {
            let mut names: Vec<_> = std::fs::read_dir(&base)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            names.sort();
            names
        };
        let names = listing();

        press(&mut app, KeyCode::Char('v'));
        type_text(&mut app, "r?q3");
        assert!(render(&app, 100).contains("r?q3▏"));
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, file.to_str().unwrap());
        press(&mut app, KeyCode::Enter);
        for width in [100, 60] {
            let shown = render(&app, width);
            for want in [
                "viewing export from",
                "(read-only)",
                "projects (1)",
                "nodes (",
                "caller",
                "callee",
            ] {
                assert!(shown.contains(want), "{width} cols lack {want}:\n{shown}");
            }
        }
        assert_eq!(
            std::fs::read(&file).unwrap(),
            before,
            "the file is untouched"
        );
        assert_eq!(listing(), names, "nothing was created");

        // A file that is not an export is refused with the reason.
        let junk = base.join("junk.json");
        std::fs::write(&junk, "{}").unwrap();
        press(&mut app, KeyCode::Char('v'));
        type_text(&mut app, junk.to_str().unwrap());
        press(&mut app, KeyCode::Enter);
        assert!(render(&app, 100).contains('✕'));
    }

    #[test]
    fn no_project_row_means_no_export_but_a_saved_file_can_still_be_viewed() {
        let mut cfg = config();
        cfg.tui.tab = "graph".into();
        let mut app = App::new(&cfg);
        press(&mut app, KeyCode::Char('e'));
        assert!(render(&app, 100).contains("no project to export"));
        press(&mut app, KeyCode::Char('v'));
        assert!(render(&app, 100).contains("view the saved export"));
    }

    #[test]
    fn the_background_run_answers_through_poll_and_esc_drops_it() {
        let (_, cfg, base) = fixture();
        let mut x = Exporter::default();
        let go = |x: &mut Exporter, code| x.key(code, Some((1, "p")), &cfg, true);
        go(&mut x, KeyCode::Char('e'));
        for c in base.join("bg.json").to_str().unwrap().chars() {
            go(&mut x, KeyCode::Char(c));
        }
        assert!(go(&mut x, KeyCode::Enter));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while matches!(x.stage, Stage::Running(_)) {
            assert!(std::time::Instant::now() < deadline, "no answer");
            std::thread::sleep(std::time::Duration::from_millis(20));
            x.poll();
        }
        assert!(base.join("bg.json").is_file());
        go(&mut x, KeyCode::Char('e'));
        go(&mut x, KeyCode::Esc);
        assert!(!x.is_open());
    }
}

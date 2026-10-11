//! Parsers that turn a tool's progress lines into [`Update`]s.

/// What an [`Update`]'s `pos` and `total` count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Bytes,
    Percent,
    Items,
}

/// One measurable step. A `total` of `None` carries only a message: the indicator stays a spinner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub pos: u64,
    pub total: Option<u64>,
    pub unit: Unit,
    pub msg: Option<String>,
}

/// Which of the child's streams a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Reads a child's output line by line. `Send` because both streams are read on their own thread.
pub trait ProgressParser: Send {
    fn feed(&mut self, stream: Stream, line: &str) -> Option<Update>;
}

/// Splits bytes into lines on `\n` and `\r`: `git`, `curl` and `cargo` redraw a progress line
/// with a bare `\r`, so a newline-only split would deliver the whole transfer as one line.
#[derive(Default)]
pub(crate) struct Lines(Vec<u8>);

/// A line longer than this is a binary blob, not progress; the rest of it is dropped.
const LINE_CAP: usize = 8192;

impl Lines {
    pub(crate) fn push(&mut self, chunk: &[u8], mut each: impl FnMut(&str)) {
        for &b in chunk {
            if b == b'\n' || b == b'\r' {
                self.finish(&mut each);
            } else if self.0.len() < LINE_CAP {
                self.0.push(b);
            }
        }
    }

    /// The unterminated tail at end of stream.
    pub(crate) fn finish(&mut self, each: &mut impl FnMut(&str)) {
        if !self.0.is_empty() {
            each(&String::from_utf8_lossy(&self.0));
            self.0.clear();
        }
    }
}

/// The first `NN%` in `line`: the value and the byte range of the digits and `%` sign.
fn percent_in(line: &str) -> Option<(u64, usize, usize)> {
    let at = line.find('%')?;
    let start = line[..at]
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .len();
    let value = line[start..at].parse().ok().filter(|v| *v <= 100)?;
    Some((value, start, at + 1))
}

/// Any line holding `NN%` becomes a 0-100 bar.
pub struct Percent;

impl ProgressParser for Percent {
    fn feed(&mut self, _: Stream, line: &str) -> Option<Update> {
        let (pos, ..) = percent_in(line)?;
        Some(Update {
            pos,
            total: Some(100),
            unit: Unit::Percent,
            msg: None,
        })
    }
}

/// `git`'s `Receiving objects:  45% (120/267), 1.2 MiB | 3 MiB/s` and `Updating files: 50% (1/2)`:
/// the object or file count as the bar, the phase as the message. Needs `--progress` where the
/// subcommand has it, because git only draws these on a terminal.
pub struct GitProgress;

impl ProgressParser for GitProgress {
    fn feed(&mut self, _: Stream, line: &str) -> Option<Update> {
        let (pct, start, end) = percent_in(line)?;
        let phase = line[..start]
            .trim_end()
            .trim_end_matches(':')
            .trim_start_matches("remote:")
            .trim();
        let msg = (!phase.is_empty()).then(|| phase.to_string());
        let counts = line[end..].trim_start().strip_prefix('(').and_then(|r| {
            let (done, rest) = r.split_once('/')?;
            let (total, _) = rest.split_once(')')?;
            Some((done.parse().ok()?, total.parse().ok()?))
        });
        Some(match counts {
            Some((pos, total)) => Update {
                pos,
                total: Some(total),
                unit: Unit::Items,
                msg,
            },
            None => Update {
                pos: pct,
                total: Some(100),
                unit: Unit::Percent,
                msg,
            },
        })
    }
}

/// `cargo`'s own bar, `    Building [====>      ] 120/300: serde, rtok(bin)`: the crate count as the bar,
/// the crates being built as the message. Cargo draws it only on a terminal, so a caller that
/// pipes stderr sets [`CargoProgress::ENV`] on the child.
pub struct CargoProgress;

impl CargoProgress {
    /// Makes cargo draw the bar into a pipe (`term.progress.when` and `.width`).
    pub const ENV: [(&'static str, &'static str); 2] = [
        ("CARGO_TERM_PROGRESS_WHEN", "always"),
        ("CARGO_TERM_PROGRESS_WIDTH", "80"),
    ];
}

impl ProgressParser for CargoProgress {
    fn feed(&mut self, _: Stream, line: &str) -> Option<Update> {
        let open = line.find('[')?;
        let (bar, after) = line[open + 1..].split_once(']')?;
        if !bar.chars().all(|c| matches!(c, '=' | '>' | ' ')) {
            return None;
        }
        let (counts, crates) = after.split_once(':').unwrap_or((after, ""));
        let (done, total) = counts.trim().split_once('/')?;
        let total: u64 = total.trim().parse().ok().filter(|t| *t > 0)?;
        let crates = crates.trim();
        Some(Update {
            pos: done.trim().parse().ok()?,
            total: Some(total),
            unit: Unit::Items,
            msg: (!crates.is_empty()).then(|| crates.to_string()),
        })
    }
}

/// One JSON object per line, `{"progress":{"pos":3,"total":10,"unit":"items","msg":"x"}}`: how
/// rtok's own child processes and plugin CLIs report. `total` and `msg` are optional, `unit` is
/// `items` (the default), `bytes` or `percent`; any other line, or an unknown unit, is ignored.
pub struct JsonLines;

impl ProgressParser for JsonLines {
    fn feed(&mut self, _: Stream, line: &str) -> Option<Update> {
        let line = line.trim();
        if !line.starts_with('{') {
            return None;
        }
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        let p = value.get("progress")?;
        let unit = match p.get("unit").and_then(|u| u.as_str()) {
            None | Some("items") => Unit::Items,
            Some("bytes") => Unit::Bytes,
            Some("percent") => Unit::Percent,
            Some(_) => return None,
        };
        Some(Update {
            pos: p.get("pos")?.as_u64()?,
            total: p.get("total").and_then(|t| t.as_u64()),
            unit,
            msg: p.get("msg").and_then(|m| m.as_str()).map(String::from),
        })
    }
}

/// A closure that reads lines like a [`ProgressParser`], for [`Progress::Callback`](super::Progress::Callback).
pub type ProgressCallback = Box<dyn FnMut(Stream, &str) -> Option<Update> + Send>;

pub(crate) struct Callback(pub(crate) ProgressCallback);

impl ProgressParser for Callback {
    fn feed(&mut self, stream: Stream, line: &str) -> Option<Update> {
        (self.0)(stream, line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(p: &mut dyn ProgressParser, line: &str) -> Option<Update> {
        p.feed(Stream::Stderr, line)
    }

    #[test]
    fn git_progress_reads_the_count_and_the_phase() {
        let got = feed(
            &mut GitProgress,
            "Receiving objects:  45% (120/267), 1.2 MiB | 3.0 MiB/s",
        );
        assert_eq!(
            got,
            Some(Update {
                pos: 120,
                total: Some(267),
                unit: Unit::Items,
                msg: Some("Receiving objects".into()),
            })
        );
        let remote = feed(
            &mut GitProgress,
            "remote: Compressing objects: 100% (27/27), done.",
        );
        assert_eq!(
            remote.and_then(|u| u.msg).as_deref(),
            Some("Compressing objects")
        );
        let files = feed(&mut GitProgress, "Updating files:  50% (1/2)").unwrap();
        assert_eq!((files.pos, files.total), (1, Some(2)));
    }

    #[test]
    fn git_progress_falls_back_to_the_percentage_and_ignores_other_lines() {
        let got = feed(&mut GitProgress, "Resolving deltas:  80%").unwrap();
        assert_eq!(
            (got.pos, got.total, got.unit),
            (80, Some(100), Unit::Percent)
        );
        assert_eq!(
            feed(&mut GitProgress, "From github.com:o/r  * branch main"),
            None
        );
        assert_eq!(
            feed(&mut GitProgress, "100% sure"),
            Some(Update {
                pos: 100,
                total: Some(100),
                unit: Unit::Percent,
                msg: None,
            })
        );
    }

    #[test]
    fn percent_takes_the_first_number_before_a_percent_sign() {
        let got = feed(&mut Percent, "  7%|█▌  | 12.0M/170M [00:01<00:12]").unwrap();
        assert_eq!(
            (got.pos, got.total, got.unit),
            (7, Some(100), Unit::Percent)
        );
        assert_eq!(feed(&mut Percent, "no number %"), None);
        assert_eq!(feed(&mut Percent, "250% over"), None);
        assert_eq!(feed(&mut Percent, "nothing here"), None);
    }

    #[test]
    fn lines_split_on_carriage_returns_and_keep_the_tail() {
        let mut lines = Lines::default();
        let mut seen = Vec::new();
        lines.push(b"Receiving 1%\rReceiving 2", |l| seen.push(l.to_string()));
        lines.push(b"%\r\nlast", |l| seen.push(l.to_string()));
        lines.finish(&mut |l: &str| seen.push(l.to_string()));
        assert_eq!(seen, ["Receiving 1%", "Receiving 2%", "last"]);
    }

    #[test]
    fn an_overlong_line_is_cut_instead_of_growing_without_bound() {
        let mut lines = Lines::default();
        let mut seen = Vec::new();
        lines.push(&vec![b'x'; LINE_CAP * 3], |_| {});
        lines.finish(&mut |l: &str| seen.push(l.len()));
        assert_eq!(seen, [LINE_CAP]);
    }

    #[test]
    fn cargo_progress_reads_a_recorded_bar_line() {
        // Recorded from cargo 1.98 with CARGO_TERM_PROGRESS_WHEN=always, then the same shape midway.
        let start =
            "    Building [                                                  ]  0/1: a(bin)       ";
        let got = feed(&mut CargoProgress, start).unwrap();
        assert_eq!(
            (got.pos, got.total, got.unit, got.msg.as_deref()),
            (0, Some(1), Unit::Items, Some("a(bin)"))
        );
        let mid = "    Building [=========>                     ] 120/300: serde, rtok(bin)";
        let got = feed(&mut CargoProgress, mid).unwrap();
        assert_eq!(
            (got.pos, got.total, got.msg.as_deref()),
            (120, Some(300), Some("serde, rtok(bin)"))
        );
        let bare = feed(&mut CargoProgress, "Fetching [===>  ] 3/9").unwrap();
        assert_eq!((bare.pos, bare.total, bare.msg), (3, Some(9), None));
    }

    #[test]
    fn cargo_progress_ignores_every_other_line() {
        for line in [
            "   Compiling serde v1.0.0",
            "    Finished `dev` profile in 3.2s",
            "warning: unused [x] 1/2",
            "Building [==>] 0/0: none",
            "Building [==>] x/9",
            "",
        ] {
            assert_eq!(feed(&mut CargoProgress, line), None, "{line}");
        }
    }

    #[test]
    fn json_lines_reads_the_progress_object() {
        let line = r#"{"progress":{"pos":3,"total":10,"unit":"bytes","msg":"x"}}"#;
        assert_eq!(
            feed(&mut JsonLines, line),
            Some(Update {
                pos: 3,
                total: Some(10),
                unit: Unit::Bytes,
                msg: Some("x".into()),
            })
        );
        let minimal = feed(&mut JsonLines, r#" {"progress":{"pos":1}} "#).unwrap();
        assert_eq!(
            (minimal.total, minimal.unit, minimal.msg),
            (None, Unit::Items, None)
        );
        let pct = feed(
            &mut JsonLines,
            r#"{"progress":{"pos":5,"total":100,"unit":"percent"}}"#,
        );
        assert_eq!(pct.map(|u| u.unit), Some(Unit::Percent));
    }

    #[test]
    fn json_lines_ignores_what_is_not_a_progress_object() {
        for line in [
            "plain text",
            "{not json",
            r#"{"other":1}"#,
            r#"{"progress":{"total":3}}"#,
            r#"{"progress":{"pos":-1}}"#,
            r#"{"progress":{"pos":1,"unit":"furlongs"}}"#,
            "[1,2]",
        ] {
            assert_eq!(feed(&mut JsonLines, line), None, "{line}");
        }
    }
}

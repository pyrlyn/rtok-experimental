// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `[log]` — rtok's own log (plan T24.0, decision D26).
//!
//! One rotating text file: the thing an operator reads and `rtok logs` prints. The bytes are
//! written by [`rtok_log`]; the `logs` table keeps the same lines as rows for `rtok otel`.
//! [`record`] is the one funnel both come out of.

use rtok_sys::proc::{Output, ProgressRunner};

use crate::config::Config;
use crate::store::Store;
use same_file::Handle;
use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::thread;
use std::time::Duration;

pub use crate::stdio::{on_stderr, on_stdout, stderr, stderr_ln, stdout, stdout_ln};

/// Whether `[log] level` lets this line through — checked before any I/O.
pub fn enabled(cfg: &Config, level: &str) -> bool {
    rtok_log::enabled(&cfg.log.level, level)
}

pub fn now() -> u64 {
    rtok_log::now()
}

/// `2026-09-09 15:04:05`, UTC. The store keeps unix seconds everywhere; a log line is the one
/// place in the binary that has to spell a date out, and std has no calendar.
pub fn stamp(secs: u64) -> String {
    rtok_log::stamp(secs)
}

/// One line: `<ts> <level> <source>/<name>: <message>`. Newlines in the message would make one
/// event look like several, so they become spaces — `rtok logs` counts lines.
pub fn line(secs: u64, level: &str, source: &str, name: &str, message: &str) -> String {
    rtok_log::line(secs, level, source, name, message)
}

/// The one path a log line takes (T24.1): [`append`] for the file, then the same line as a
/// `logs` row unless `[log] to_db` is false — the file is then the only sink, which is the
/// reason the key exists. One `[log] level` decision gates both; both writes fail open (D1).
#[allow(clippy::too_many_arguments)]
pub fn record(
    cfg: &Config,
    store: &Store,
    session: Option<&str>,
    call_id: Option<i32>,
    level: &str,
    source: &str,
    name: &str,
    message: &str,
) {
    append(cfg, level, source, name, message);
    if cfg.log.to_db && enabled(cfg, level) {
        let _ = store.insert_log(level, source, name, message, session, call_id, None);
    }
}

/// Log the warnings a config load returned, and hand the same line to the stderr surface.
/// Config itself does neither; `cli::run` calls this from the load hook.
pub fn emit_config_notes(cfg: &Config) {
    for note in &cfg.notes.0 {
        stderr_ln(&format!("rtok: {}", note.message));
        crate::logfile::append(
            &cfg.log.path,
            cfg.log.max_bytes,
            cfg.log.files,
            &cfg.log.level,
            "warn",
            "config",
            note.event,
            &note.log,
        );
    }
}

/// Append one line, rotating first when it would take the file past `[log] max_bytes`.
///
/// Never fails upward: a log that cannot be written is not something the caller can act on, and a
/// hook must exit 0 in 10 ms whatever the disk is doing (D1). The bytes and the stderr mirror
/// are [`crate::logfile::append`].
pub fn append(cfg: &Config, level: &str, source: &str, name: &str, message: &str) {
    crate::logfile::append(
        &cfg.log.path,
        cfg.log.max_bytes,
        cfg.log.files,
        &cfg.log.level,
        level,
        source,
        name,
        message,
    );
}

/// Colour a stored log line's level (`<date> <time> <level> <source>/<name>: <message>`, T24.0's
/// `log::line`): red error, yellow warn, dim debug, info plain. `rtok logs export` prints the same
/// line through no such call, so piping stays byte-plain.
pub fn log_line(text: &str) -> String {
    use owo_colors::{OwoColorize, Stream};
    let mut parts = text.splitn(4, ' ');
    let (Some(date), Some(time), Some(level), Some(rest)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return text.to_string();
    };
    let level = match level {
        "error" => level
            .if_supports_color(Stream::Stdout, |t| t.red())
            .to_string(),
        "warn" => level
            .if_supports_color(Stream::Stdout, |t| t.yellow())
            .to_string(),
        "debug" => level
            .if_supports_color(Stream::Stdout, |t| t.dimmed())
            .to_string(),
        _ => level.to_string(),
    };
    format!("{date} {time} {level} {rest}")
}

/// The debug log on stderr (T225): the `log` facade behind `env_logger`, on only while
/// `RUST_LOG` is set (`RUST_LOG=rtok=debug`; `RUST_LOG_STYLE=never` drops colour). Unset, the
/// filter is `off` — not env_logger's `error` default — so a hook, `mcp` or `proxy` prints
/// nothing it did not print before, and the cost is one atomic store (D1). `RUST_LOG` rather
/// than `RTOK_LOG`: the config env layer owns `RTOK_<SECTION>_<KEY>`, where `RTOK_LOG=debug`
/// would read as a string in place of the `[log]` table. Called first thing in `cli::run`; a
/// second call (a library user, a test) is a no-op.
pub fn init_stderr() {
    let env = env_logger::Env::default().default_filter_or("off");
    let _ = env_logger::Builder::from_env(env).try_init();
}

/// `rtok.log` → `rtok.log.<i>`. The suffix lives in `rtok-log`; tail still walks it.
fn nth(path: &Path, i: u32) -> PathBuf {
    rtok_log::rotated_path(path, i)
}

/// The last `n` lines across `path`, then `.1`, `.2`, … newest first (plan T24.2). `n` is
/// `[log] lines` unless the caller overrides it. Stops at the first file that does not exist —
/// rotation is contiguous, so nothing sits behind a gap. Empty when nothing has been logged.
pub fn tail(cfg: &Config, n: Option<usize>) -> Vec<String> {
    let n = n.unwrap_or(cfg.log.lines);
    let live = whole_lines_of(&cfg.log.path);
    tail_with(&live, &cfg.log.path, n)
}

/// `tail`'s walk with the live file's lines supplied by the caller (T24.3): `logs watch` reads
/// the live file once and feeds both its first screen and its follow state from that one read,
/// so a line written as the watch starts shows exactly once — in the screen if it made the
/// read, from the loop if it did not. The rotated siblings are read as `tail` always read them.
fn tail_with(live: &[String], path: &Path, n: usize) -> Vec<String> {
    let mut out: Vec<String> = live.iter().rev().take(n).cloned().collect();
    let mut i = 1u32;
    while out.len() < n {
        let Ok(text) = fs::read_to_string(nth(path, i)) else {
            break; // rotation is contiguous: nothing sits behind a gap
        };
        out.extend(text.lines().rev().map(str::to_string).take(n - out.len()));
        i += 1;
    }
    out
}

/// `rtok logs`: the tail numbered (`1` newest) with the level coloured through
/// [`log_line`] — one colour table, not a second one here. Pure rendering:
/// the lines come from the operator model (T15.11), which owns the selection.
pub fn screen(lines: &[String]) -> Vec<String> {
    let coloured: Vec<String> = lines.iter().map(|l| log_line(l)).collect();
    numbered(&coloured)
}

/// The tail numbered `1` newest and nothing else: what [`screen`] colours, and what a viewer
/// (T225.1) gets as-is so it can colour it its own way.
pub fn numbered(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{} {line}", i + 1))
        .collect()
}

// ── T225.1: `rtok logs` through tailspin ─────────────────────────────────────────

/// A running `tspin --print` that `rtok logs` writes its plain rows into. Print mode, no
/// pager: the numbering and the stream shape stay rtok's, tailspin only colours.
pub struct Tspin(Child);

impl Tspin {
    /// `[log] tspin`: `auto` starts the viewer when stdout is a terminal, `always` regardless
    /// (a pipe then carries tailspin's colours), `off` never. `None` also when `tspin` is not on
    /// `PATH` or will not start: the caller prints with [`screen`]'s own colours instead, so a
    /// missing viewer never costs a line (D1).
    pub fn start(cfg: &Config, tty: bool) -> Option<Self> {
        let wanted = match cfg.log.tspin.as_str() {
            "always" => true,
            "auto" => tty,
            _ => false,
        };
        if !wanted {
            return None;
        }
        ProgressRunner::quiet("tspin", ["--print"])
            .output(Output::Inherit)
            .stdin(Stdio::piped())
            .spawn()
            .ok()
            .map(Self)
    }

    /// The viewer's stdin: where [`watch`] streams to.
    pub fn sink(&mut self) -> &mut ChildStdin {
        self.0.stdin.as_mut().expect("spawned with a piped stdin")
    }

    /// Hand the viewer every row, then [`finish`](Self::finish). A viewer that dies mid-stream
    /// is not rtok's error: the rows were the log, not a result.
    pub fn print(mut self, rows: &[String]) {
        for row in rows {
            if writeln!(self.sink(), "{row}").is_err() {
                break;
            }
        }
        self.finish();
    }

    /// Close the pipe and wait for tailspin to flush what it has coloured.
    pub fn finish(mut self) {
        drop(self.0.stdin.take());
        let _ = self.0.wait();
    }
}

// ── T24.3: `rtok logs watch` — follow the live file, newest first ─────────────────

/// How often a watch loop looks (T24.3; T25.3's table repaints on the same cadence). A constant
/// so a test can time a line's arrival against the interval it was promised.
pub const WATCH_POLL: Duration = Duration::from_millis(200);

/// One step of a running watch (T24.3): `fresh` is what a pipe must print now — new lines for a
/// stream, the whole body again for a state table (T25.3) — and `screen` is what a terminal must
/// be showing once the step lands. The loop owns the difference: a TTY repaints `screen` in
/// place, a pipe appends `fresh` and nothing else.
pub struct WatchTick {
    pub fresh: Vec<String>,
    pub screen: Vec<String>,
}

/// The poll-and-print loop every `watch` command runs (T24.3 `logs watch`; T25.3's `agent
/// sessions watch` repaints state on the same loop — do not grow a second one). Every
/// `interval`, `step` says what changed; `None` ends the watch. On a TTY the screen is redrawn
/// in place — cursor up, clear below — and nothing else: no raw mode, no alternate screen, no
/// hidden cursor, so Ctrl-C under the default signal handling leaves the terminal as it found
/// it, the way `tail -f` does. Piped, only `fresh` rows are appended, as plain text.
pub fn watch_loop<W: Write>(
    out: &mut W,
    tty: bool,
    interval: Duration,
    mut step: impl FnMut() -> Option<WatchTick>,
) -> std::io::Result<()> {
    let mut prev = 0usize; // rows the TTY is holding above the cursor
    // T235.3: a watch left behind by a terminal or agent that went away (SIGHUP ignored,
    // stdout on a file that never errors) would poll forever. Being reparented ends it — to
    // a subreaper (the pid changes) or to init, which also covers a parent gone before we
    // read it.
    let parent = rtok_sys::parent_pid();
    while let Some(tick) = step() {
        let now = rtok_sys::parent_pid();
        if now != parent || now == Some(1) {
            break;
        }
        if !tick.fresh.is_empty() || tick.screen.len() != prev {
            if tty {
                repaint(out, prev, &tick.screen)?;
            } else {
                for row in &tick.fresh {
                    writeln!(out, "{row}")?;
                }
            }
            out.flush()?;
            prev = tick.screen.len();
        }
        thread::sleep(interval);
    }
    Ok(())
}

/// Redraw in place: to the start of the row `prev` rows up, clear to the end of the screen,
/// print `rows`, each on its own line — the cursor ends one row past the screen, which is where
/// the next repaint counts from. The cursor moves by rows, not wrapped rows: a line wider than
/// the terminal makes the real display one row longer than `prev` and the redraw drifts down a
/// row. Knowing the width would take an ioctl and a dependency, for that one case.
fn repaint<W: Write>(out: &mut W, prev: usize, rows: &[String]) -> std::io::Result<()> {
    if prev > 0 {
        write!(out, "\x1b[{prev}F")?; // CPL: up `prev` rows, to column 1
    }
    write!(out, "\x1b[J")?; // ED: clear from the cursor down, erasing the old screen
    for row in rows {
        writeln!(out, "{row}")?;
    }
    Ok(())
}

/// `rtok logs watch` (T24.3): the same last-`n` screen `rtok logs` prints, then follow the live
/// file — every new line lands above the ones before it, so newest-first holds while it runs.
/// `tty` decides repaint-in-place over plain appending; the caller reads it off stdout so the
/// tests can pin both halves. Runs until killed; a pipe closing under it (`| head`) is a reader
/// that left, not an error.
pub fn watch<W: Write>(
    cfg: &Config,
    n: Option<usize>,
    out: &mut W,
    tty: bool,
) -> std::io::Result<()> {
    let n = n.unwrap_or(cfg.log.lines);
    // One read of the live file feeds both halves of the start: the first screen (through
    // `tail_with`, with the rotated history above it) and the follow state. A line written
    // after this read is new to both; a line in it is old to both — never shown twice, never
    // skipped.
    let (mut follow, live) = Follow::primed(&cfg.log.path);
    let mut window: Vec<String> = tail_with(&live, &cfg.log.path, n); // newest first
    let mut shown = window.len(); // rows already printed; piped rows keep counting past them
    let mut first = true;
    let run = watch_loop(out, tty, WATCH_POLL, move || {
        if first {
            first = false;
            let rows = screen(&window);
            return Some(WatchTick {
                fresh: rows.clone(),
                screen: rows,
            });
        }
        let fresh = follow.poll(&cfg.log.path); // arrival order, oldest first
        if fresh.is_empty() {
            return Some(WatchTick {
                fresh: Vec::new(),
                screen: screen(&window),
            });
        }
        for line in fresh.iter().rev() {
            window.insert(0, line.clone()); // newest goes to the top
        }
        window.truncate(n);
        // On a TTY the repaint renumbers everything, `1` newest. A pipe cannot renumber what it
        // already printed, so its rows keep counting past the screen: each line once, each
        // number once.
        let rows = fresh
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{} {}", shown + i + 1, log_line(line)))
            .collect();
        shown += fresh.len();
        Some(WatchTick {
            fresh: rows,
            screen: screen(&window),
        })
    });
    match run {
        Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

/// Incremental reader for the live log file across rotations (T24.3). Reads only `path` — the
/// rotated siblings are history `rtok logs` already shows. It keeps the file it was reading
/// open (T589): a rotation (T24.0: `path` renamed to `path.1`, the next write recreating
/// `path`) is `path` no longer naming that file, by file identity and not by what the lines say
/// — two generations can open with the same line — and the unread tail of the old file comes
/// off the open handle, wherever the rename left it or even once retention has deleted it.
#[derive(Default)]
pub struct Follow {
    reading: Option<Handle>, // the file we were reading
    consumed: usize,         // of its lines, the ones already returned
}

impl Follow {
    pub fn new() -> Self {
        Self::default()
    }

    /// A follow already caught up with `path`, and the lines it holds: every one of them counts
    /// as shown, matching the first screen built from the same read — one read for both, so
    /// the two cannot disagree about what was on disk and repeat or lose the line written
    /// between two reads. A file that is not there yet primes an empty follow.
    pub fn primed(path: &Path) -> (Self, Vec<String>) {
        let Some((handle, live)) = open_lines(path) else {
            return (Self::new(), Vec::new());
        };
        let consumed = live.len();
        let follow = Self {
            reading: Some(handle),
            consumed,
        };
        (follow, live)
    }

    /// The lines of `path` written since the last call, oldest first. A file renamed away and
    /// not yet recreated reads as empty and keeps its state, so whatever appears at `path`
    /// next is read as the new file it is.
    pub fn poll(&mut self, path: &Path) -> Vec<String> {
        // A file that is not there (renamed away, T24.0) or cannot be read now is a delay,
        // not a rotation.
        let Some((handle, mut lines)) = open_lines(path) else {
            return Vec::new();
        };
        if self.reading.as_ref() == Some(&handle) {
            // Fewer lines than consumed: the same file was truncated in place; start over.
            let at = if lines.len() >= self.consumed {
                self.consumed
            } else {
                0
            };
            self.consumed = lines.len();
            return lines.split_off(at); // `split_off` truncates, so the count comes first
        }
        // A rotation: carry the tail of the file we were reading — a rotation between two polls
        // must cost no line — then read the newcomer from its start. `nth` runs newest-old
        // first, so the generations that rotated past us between polls reverse into chronology.
        let mut pieces: Vec<Vec<String>> = Vec::new();
        for i in 1.. {
            // Rotation is contiguous (`tail` counts on that too).
            let Some((sibling, sibling_lines)) = open_lines(&nth(path, i)) else {
                break;
            };
            if self.reading.as_ref() == Some(&sibling) {
                break; // ours: older files were fully returned before they rotated
            }
            pieces.push(sibling_lines); // rotated past us between polls: all of it is new
        }
        pieces.reverse();
        let mut fresh = Vec::new();
        if let Some(old) = self.reading.as_mut()
            && let Some(mut tail) = read_lines(old.as_file_mut())
        {
            fresh = tail.split_off(self.consumed.min(tail.len())); // only what we had not returned
        }
        fresh.extend(pieces.concat());
        // The newcomer is returned whole, so all of it is consumed.
        self.consumed = lines.len();
        self.reading = Some(handle);
        fresh.extend(lines);
        fresh
    }
}

/// Open `path` and read its whole lines through that one handle, so the identity and the lines
/// describe the same file even if `path` is renamed between the two.
fn open_lines(path: &Path) -> Option<(Handle, Vec<String>)> {
    let mut handle = Handle::from_path(path).ok()?;
    let lines = read_lines(handle.as_file_mut())?;
    Some((handle, lines))
}

fn read_lines(file: &mut fs::File) -> Option<Vec<String>> {
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_end(&mut bytes).ok()?;
    Some(whole_lines(&bytes))
}

/// Whole lines of a byte string, dropping a half-written tail: the sink's `writeln!` lands as
/// two writes, and a line is a line only once its `\n` is on disk.
fn whole_lines(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    lines.pop(); // the piece after the last `\n` is not a line yet
    lines
}

/// Whole lines of the file at `path`, empty when it cannot be read — the one read `tail` and
/// `Follow` share. Lossy rather than empty, so a stray byte costs a character, not the log.
fn whole_lines_of(path: &Path) -> Vec<String> {
    whole_lines(&fs::read(path).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-log-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn cfg_at(dir: &Path, max_bytes: u64, files: u32) -> Config {
        let mut cfg = Config::default();
        cfg.log.path = dir.join("rtok.log");
        cfg.log.max_bytes = max_bytes;
        cfg.log.files = files;
        cfg.log.level = "debug".into();
        cfg
    }

    /// A facade sink the T225 mirror test can read back; `set_logger` is process-wide, so this
    /// is the one test in the crate that installs a logger.
    struct Capture(std::sync::Mutex<Vec<String>>);

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, r: &log::Record<'_>) {
            let line = format!("{} {} {}", r.level(), r.target(), r.args());
            self.0.lock().unwrap().push(line);
        }
        fn flush(&self) {}
    }

    static CAPTURE: Capture = Capture(std::sync::Mutex::new(Vec::new()));

    #[test]
    fn stderr_mirror_ignores_the_file_level_and_keeps_source_and_name() {
        log::set_logger(&CAPTURE).unwrap();
        log::set_max_level(log::LevelFilter::Debug);
        let dir = std::env::temp_dir().join(format!("rtok-t225-mirror-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut cfg = cfg_at(&dir, 1_048_576, 5);
        cfg.log.level = "error".into();
        append(&cfg, "debug", "demon", "proxy", "mirrored\nline");
        assert!(
            !cfg.log.path.exists(),
            "the [log] level gate on the file still holds"
        );
        // An unknown level ranks most severe, so this one does reach the file (and the mirror).
        append(&cfg, "bogus", "demon", "proxy", "unknown level");
        let lines = CAPTURE.0.lock().unwrap();
        // Other tests in this process may log too; the mirror's lines are what matters.
        assert!(
            lines
                .iter()
                .any(|l| l == "DEBUG rtok::log demon/proxy: mirrored\nline"),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l == "ERROR rtok::log demon/proxy: unknown level"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_known_epoch_reads_as_a_date() {
        assert_eq!(stamp(0), "1970-01-01 00:00:00");
        // `date -u -r <secs>` for each: an epoch in a leap year, one just after a leap day,
        // and one on a century that is not a leap year.
        assert_eq!(stamp(1_788_966_245), "2026-09-09 15:04:05");
        assert_eq!(stamp(1_709_209_845), "2024-02-29 12:30:45");
        assert_eq!(stamp(4_107_542_400), "2100-03-01 00:00:00");
    }

    #[test]
    fn the_level_is_a_floor_and_an_unknown_level_always_passes() {
        let mut cfg = Config::default();
        cfg.log.level = "warn".into();
        assert!(enabled(&cfg, "error") && enabled(&cfg, "warn"));
        assert!(!enabled(&cfg, "info") && !enabled(&cfg, "debug"));
        assert!(enabled(&cfg, "PANIC"), "an unreadable level is not dropped");
    }

    #[test]
    fn an_error_is_copied_to_errors_log_and_a_warning_is_not() {
        let dir = tmp("split");
        let cfg = cfg_at(&dir, 1 << 20, 2);
        append(&cfg, "warn", "test", "split", "careful");
        append(&cfg, "info", "test", "split", "note");
        append(&cfg, "debug", "test", "split", "trace");
        append(&cfg, "error", "test", "split", "broken");
        let main = fs::read_to_string(&cfg.log.path).unwrap();
        let errs = fs::read_to_string(rtok_log::error_path(&cfg.log.path)).unwrap();
        assert!(
            main.contains("careful") && main.contains("note") && main.contains("trace"),
            "{main}"
        );
        assert!(main.contains("broken"), "{main}");
        assert!(
            errs.contains("broken") && errs.lines().count() == 1,
            "{errs}"
        );
        assert!(
            !errs.contains("careful") && !errs.contains("note"),
            "{errs}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_keeps_files_plus_the_current_one_and_the_newest_line_is_in_it() {
        let dir = tmp("rotate");
        let cfg = cfg_at(&dir, 200, 2);
        for i in 0..50 {
            append(&cfg, "info", "test", "rotate", &format!("line {i}"));
        }
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 4, "{names:?}");
        for want in [
            "rtok.log",
            "rtok.log.1",
            "rtok.log.2",
            "rtok.log.rotate.lock",
        ] {
            assert!(names.iter().any(|n| n == want), "{names:?}");
        }
        let live = fs::read_to_string(dir.join("rtok.log")).unwrap();
        assert!(live.contains("line 49"), "{live}");
        assert!(
            live.len() <= 200,
            "the live file is bounded: {}",
            live.len()
        );
    }

    #[test]
    fn a_message_with_newlines_stays_one_line() {
        let dir = tmp("newline");
        let cfg = cfg_at(&dir, 1 << 20, 1);
        append(&cfg, "warn", "test", "multi", "first\nsecond");
        let text = fs::read_to_string(&cfg.log.path).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains("first second"), "{text}");
    }

    #[test]
    fn a_line_below_the_level_touches_no_disk() {
        let dir = tmp("quiet");
        let mut cfg = cfg_at(&dir, 1 << 20, 1);
        cfg.log.level = "error".into();
        append(&cfg, "debug", "test", "quiet", "nothing");
        assert!(!cfg.log.path.exists());
    }

    /// 4 lines in each of the live file and 3 rotated siblings, newest last within each file —
    /// the shape `rotate` leaves behind. `tail` must walk backwards across all four.
    fn seed_rotated(dir: &Path) -> Config {
        fs::create_dir_all(dir).unwrap();
        let cfg = cfg_at(dir, 1 << 20, 3);
        fs::write(&cfg.log.path, "live-1\nlive-2\nlive-3\nlive-4\n").unwrap();
        fs::write(nth(&cfg.log.path, 1), "r1-1\nr1-2\nr1-3\nr1-4\n").unwrap();
        fs::write(nth(&cfg.log.path, 2), "r2-1\nr2-2\nr2-3\nr2-4\n").unwrap();
        fs::write(nth(&cfg.log.path, 3), "r3-1\nr3-2\nr3-3\nr3-4\n").unwrap();
        cfg
    }

    #[test]
    fn tail_reads_backwards_across_the_rotation_boundary() {
        let dir = tmp("tail");
        let cfg = seed_rotated(&dir);
        let got = tail(&cfg, Some(10));
        assert_eq!(
            got,
            vec![
                "live-4", "live-3", "live-2", "live-1", "r1-4", "r1-3", "r1-2", "r1-1", "r2-4",
                "r2-3",
            ]
        );
    }

    #[test]
    fn tail_and_screen_are_empty_when_nothing_has_been_logged() {
        let dir = tmp("empty");
        let cfg = cfg_at(&dir, 1 << 20, 3);
        let lines = tail(&cfg, Some(10));
        assert!(lines.is_empty());
        assert!(screen(&lines).is_empty());
    }

    #[test]
    fn screen_numbers_newest_first_and_strips_to_the_same_lines_as_tail() {
        let dir = tmp("screen");
        let cfg = seed_rotated(&dir);
        let plain = tail(&cfg, Some(10));
        let numbered = screen(&plain);
        assert_eq!(numbered.len(), plain.len());
        for (i, (n, p)) in numbered.iter().zip(plain.iter()).enumerate() {
            // No terminal in a test run, so `log_line` added no ANSI codes: stripping the
            // leading "<n> " is the whole job.
            assert_eq!(n, &format!("{} {p}", i + 1));
        }
    }

    // ── T24.1: the funnel — one call, one line, one row ─────────────────────────────

    use crate::plugin::Runtime;

    #[test]
    fn a_funnel_call_is_one_file_line_and_one_row() {
        let dir = tmp("funnel");
        let cfg = cfg_at(&dir, 1 << 20, 1);
        let store = Store::open_in_memory().unwrap();
        record(
            &cfg,
            &store,
            Some("s1"),
            None,
            "info",
            "plugin",
            "read",
            "hello",
        );
        assert_eq!(
            fs::read_to_string(&cfg.log.path).unwrap().lines().count(),
            1
        );
        let rows = store.logs_after(0, 10).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].message, "hello");
        assert_eq!(rows[0].session.as_deref(), Some("s1"));
    }

    #[test]
    fn to_db_false_leaves_the_file_as_the_only_sink() {
        let dir = tmp("nodb");
        let mut cfg = cfg_at(&dir, 1 << 20, 1);
        cfg.log.to_db = false;
        let store = Store::open_in_memory().unwrap();
        record(&cfg, &store, Some("s1"), None, "warn", "plugin", "read", "");
        assert_eq!(
            fs::read_to_string(&cfg.log.path).unwrap().lines().count(),
            1
        );
        assert!(store.logs_after(0, 10).unwrap().is_empty());
    }

    #[test]
    fn a_read_only_log_directory_changes_nothing_about_the_call() {
        let dir = tmp("readonly");
        fs::create_dir_all(&dir).unwrap();
        let close = |ro: bool| {
            let mut p = fs::metadata(&dir).unwrap().permissions();
            p.set_readonly(ro);
            fs::set_permissions(&dir, p).unwrap();
        };
        close(true);
        // Root writes through the permission bit: the probe has to fail to proceed.
        if fs::write(dir.join("probe"), b"").is_ok() {
            close(false);
            return;
        }
        let cfg = cfg_at(&dir, 1 << 20, 1);
        let store = Store::open_in_memory().unwrap();
        record(
            &cfg,
            &store,
            Some("s1"),
            None,
            "error",
            "plugin",
            "read",
            "boom",
        );
        close(false);
        assert_eq!(store.logs_after(0, 10).unwrap().len(), 1, "row still lands");
        assert!(!cfg.log.path.exists(), "file write failed silently");
    }

    #[test]
    fn a_plugin_call_is_one_file_line_and_one_row() {
        let dir = tmp("plugin-funnel");
        let mut cx = Runtime::in_memory("s1").unwrap();
        cx.config.log.path = dir.join("rtok.log");
        cx.log("warn", "plugin", "read", "through the funnel");
        assert_eq!(
            fs::read_to_string(&cx.config.log.path)
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert_eq!(cx.store.logs_after(0, 10).unwrap().len(), 1);
    }

    // ── T24.3: follow, rotate, degrade ───────────────────────────────────────────

    use std::time::Duration;

    /// `Follow::primed` over the same read `watch` makes: everything on disk now is the first
    /// screen's to show, not the loop's to emit.
    fn primed_at(path: &Path) -> Follow {
        Follow::primed(path).0
    }

    #[test]
    fn follow_returns_only_lines_written_after_the_last_poll() {
        let dir = tmp("follow-new");
        let cfg = cfg_at(&dir, 1 << 20, 3);
        append(&cfg, "info", "test", "f", "first");
        let mut f = primed_at(&cfg.log.path);
        append(&cfg, "warn", "test", "f", "second");
        append(&cfg, "error", "test", "f", "third");
        let got = f.poll(&cfg.log.path);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(
            got[0].contains("second") && got[1].contains("third"),
            "{got:?}"
        );
        assert!(
            f.poll(&cfg.log.path).is_empty(),
            "nothing new, nothing repeated"
        );
    }

    #[test]
    fn follow_reopens_path_when_rotation_renames_the_file_it_was_reading() {
        let dir = tmp("follow-rotate");
        let cfg = cfg_at(&dir, 1 << 20, 3);
        append(&cfg, "info", "test", "f", "before");
        let mut f = primed_at(&cfg.log.path); // the seed line is the first screen's, not ours
        // T24.0's rotation: `path` → `.1`, and the next write recreates `path`.
        fs::rename(&cfg.log.path, nth(&cfg.log.path, 1)).unwrap();
        assert!(
            f.poll(&cfg.log.path).is_empty(),
            "renamed away, not yet recreated"
        );
        append(&cfg, "info", "test", "f", "after");
        let got = f.poll(&cfg.log.path);
        assert_eq!(got.len(), 1, "{got:?}"); // the old line lives on in `.1`: no repeat
        assert!(got[0].contains("after"), "{got:?}");
        assert!(
            f.poll(&cfg.log.path).is_empty(),
            "the stream continues past the rotation"
        );
    }

    #[test]
    fn a_rotation_between_polls_carries_the_lines_not_yet_seen() {
        let dir = tmp("follow-carry");
        let cfg = cfg_at(&dir, 1 << 20, 3);
        append(&cfg, "info", "test", "f", "already-emitted-1");
        append(&cfg, "info", "test", "f", "already-emitted-2");
        let mut f = primed_at(&cfg.log.path); // consumes both
        append(&cfg, "info", "test", "f", "missed-by-poll"); // written, then rotated off unseen
        fs::rename(&cfg.log.path, nth(&cfg.log.path, 1)).unwrap();
        append(&cfg, "info", "test", "f", "after-rename"); // recreates `path`
        let got = f.poll(&cfg.log.path);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(
            got[0].contains("missed-by-poll") && got[1].contains("after-rename"),
            "{got:?}"
        );
        assert!(
            !got.iter().any(|l| l.contains("already-emitted")),
            "no line twice: {got:?}"
        );
    }

    fn append_raw(path: &Path, text: &str) {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    /// T589: a new generation that opens with the same line, and as many lines, is still a new file.
    #[test]
    fn a_rotation_is_not_missed_when_both_generations_start_alike() {
        let dir = tmp("follow-same-first");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rtok.log");
        append_raw(&path, "same\nold-shown\n");
        let mut f = primed_at(&path);
        append_raw(&path, "old-unseen\n");
        fs::rename(&path, nth(&path, 1)).unwrap();
        append_raw(&path, "same\nnew-unseen\n");
        assert_eq!(f.poll(&path), ["old-unseen", "same", "new-unseen"]);
        assert!(f.poll(&path).is_empty());
    }

    /// T589: an empty replacement between polls must not make the next poll replay the old file.
    #[test]
    fn an_empty_replacement_keeps_the_generation_boundary() {
        let dir = tmp("follow-empty-gen");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rtok.log");
        append_raw(&path, "same\nold-shown\n");
        let mut f = primed_at(&path);
        append_raw(&path, "old-unseen\n");
        fs::rename(&path, nth(&path, 1)).unwrap();
        fs::write(&path, "").unwrap();
        assert_eq!(f.poll(&path), ["old-unseen"]);
        assert!(f.poll(&path).is_empty());
        append_raw(&path, "same\n");
        assert_eq!(f.poll(&path), ["same"], "no replay of the rotated file");
        append_raw(&path, "half-written");
        assert!(
            f.poll(&path).is_empty(),
            "a line is a line once its newline lands"
        );
    }

    /// T589: the unread tail of the old file survives retention deleting it, and a file
    /// truncated in place is read again from its start.
    #[test]
    fn the_old_tail_survives_deletion_and_truncation_restarts() {
        let dir = tmp("follow-deleted");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rtok.log");
        append_raw(&path, "a\nb\n");
        let mut f = primed_at(&path);
        append_raw(&path, "c\n");
        fs::remove_file(&path).unwrap(); // rotated off and deleted before the next poll
        append_raw(&path, "d\n");
        assert_eq!(f.poll(&path), ["c", "d"]);
        append_raw(&path, "d2\n");
        assert_eq!(f.poll(&path), ["d2"]);
        fs::write(&path, "e\n").unwrap(); // truncated in place: same file, fewer lines
        assert_eq!(f.poll(&path), ["e"]);
    }

    #[test]
    fn a_rotation_storm_between_polls_prints_every_line_once() {
        let dir = tmp("follow-storm");
        let cfg = cfg_at(&dir, 150, 5); // a line is ~40 B: the burst rotates every few writes
        let mut f = primed_at(&cfg.log.path); // prime on a not-yet-existing file
        for i in 0..12 {
            append(&cfg, "info", "test", "f", &format!("wave-{i}"));
        }
        assert!(nth(&cfg.log.path, 1).exists(), "the burst actually rotated");
        let got = f.poll(&cfg.log.path);
        assert_eq!(got.len(), 12, "{got:?}");
        for i in 0..12 {
            let times = got
                .iter()
                .filter(|l| l.ends_with(&format!("wave-{i}")))
                .count();
            assert_eq!(times, 1, "wave-{i} appeared {times} times");
        }
        assert!(
            got.last().unwrap().ends_with("wave-11"),
            "arrival order: {got:?}"
        );
    }

    #[test]
    fn a_tty_watch_repaints_the_screen_in_place() {
        let mut out = Vec::new();
        let mut ticks = vec![
            Some(WatchTick {
                fresh: vec!["1 a".into()],
                screen: vec!["1 a".into()],
            }),
            Some(WatchTick {
                fresh: vec!["1 b".into()],
                screen: vec!["1 b".into(), "2 a".into()],
            }),
            None,
        ];
        watch_loop(&mut out, true, Duration::from_millis(1), || ticks.remove(0)).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.starts_with("\x1b[J1 a\n"),
            "the first screen prints at the cursor: {text:?}"
        );
        assert!(
            text.ends_with("\x1b[1F\x1b[J1 b\n2 a\n"),
            "up one row, cleared, newest printed above: {text:?}"
        );
    }

    #[test]
    fn a_piped_watch_appends_plain_rows_and_no_escape_codes() {
        let mut out = Vec::new();
        let mut ticks = vec![
            Some(WatchTick {
                fresh: vec!["1 a".into()],
                screen: vec!["1 a".into()],
            }),
            Some(WatchTick {
                fresh: vec!["2 b".into()],
                screen: vec!["2 b".into(), "1 a".into()],
            }),
            Some(WatchTick {
                fresh: Vec::new(), // nothing changed: nothing printed
                screen: vec!["2 b".into(), "1 a".into()],
            }),
            None,
        ];
        watch_loop(&mut out, false, Duration::from_millis(1), || {
            ticks.remove(0)
        })
        .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "1 a\n2 b\n");
    }
}

//! The one way rtok runs an external program (T276): a [`ProgressRunner`] that shows a spinner
//! or a progress bar on a terminal while the child works, and nothing anywhere else.
//!
//! Every `Command::new` in the workspace belongs in this module: `clippy.toml` bans it
//! everywhere else (T276.6).
#![allow(clippy::disallowed_methods)]

mod indicator;
mod parse;

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

pub use indicator::{Hidden, Indicator, TtyIndicator, disable_progress, suspend};
pub use parse::{
    CargoProgress, GitProgress, JsonLines, Percent, ProgressCallback, ProgressParser, Stream, Unit,
    Update,
};

/// How long a run keeps reading once the child has exited. A descendant it left running
/// (`git fsmonitor--daemon`, a backgrounded server) can hold a pipe's write end for good, so
/// waiting for EOF would hang; what the pipe already holds is still collected.
const DRAIN_AFTER_EXIT: Duration = Duration::from_millis(200);

/// What a finished `.run()` hands back; the same shape as `Command::output`.
pub type Captured = std::process::Output;

/// Where the child's output goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// Keep stdout and stderr; an indicator shows until the child exits.
    Capture,
    /// Forward both streams live, hiding the indicator around each write.
    Stream,
    /// The child owns the terminal (`tspin`, `ketch`); no indicator.
    Inherit,
    /// A background child with null stdio; no indicator. `.spawn()` only.
    Detached,
}

/// How the wait is drawn.
pub enum Progress {
    /// No indicator: a quick read nobody waits for.
    Off,
    /// A spinner: the wait cannot be measured.
    Unknown,
    /// A spinner that turns into a bar when the parser reports a total.
    Parse(Box<dyn ProgressParser>),
    /// [`Progress::Parse`] with a closure, for a one-off format that needs no parser type.
    Callback(ProgressCallback),
}

pub struct ProgressRunner {
    label: String,
    cmd: Command,
    output: Output,
    progress: Progress,
    timeout: Option<Duration>,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
    shim: bool,
}

impl ProgressRunner {
    /// `label` is the message beside the spinner: the action and its target, present tense,
    /// ending in `…`.
    pub fn new<I, S>(label: &str, program: impl AsRef<OsStr>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new(program);
        cmd.args(args);
        Self {
            label: label.to_string(),
            cmd,
            output: Output::Capture,
            progress: Progress::Unknown,
            timeout: None,
            stdin: None,
            stdout: None,
            stderr: None,
            shim: false,
        }
    }

    /// A bar over `total` items that rtok works through itself (worktrees to remove, runs to
    /// time). Commands run inside it keep their own spinner under the bar.
    pub fn batch(label: &str, total: u64, unit: Unit) -> Batch {
        Batch::new(indicator::pick_bar(label, total, unit), total, unit)
    }

    /// A spinner labelled `label` while rtok works in-process (a migration, a report), cleared
    /// before it returns so the line the caller prints next takes its place.
    pub fn wait<T>(label: &str, f: impl FnOnce() -> T) -> T {
        wait_on(indicator::pick(label), f)
    }

    /// A call with no indicator.
    pub fn quiet<I, S>(program: impl AsRef<OsStr>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Self::new("", program, args).progress(Progress::Off)
    }

    pub fn cwd(mut self, dir: impl AsRef<Path>) -> Self {
        self.cmd.current_dir(dir);
        self
    }

    pub fn env(mut self, key: impl AsRef<OsStr>, val: impl AsRef<OsStr>) -> Self {
        self.cmd.env(key, val);
        self
    }

    /// Without this a piped or detached child gets a closed stdin; an inherited one keeps ours.
    pub fn stdin(mut self, stdin: Stdio) -> Self {
        self.stdin = Some(stdin);
        self
    }

    /// `.spawn()` only: where the child's stdout goes instead of what its [`Output`] mode gives
    /// (a pipe for a language server's protocol stream while its stderr is a file or ours).
    pub fn stdout(mut self, stdout: Stdio) -> Self {
        self.stdout = Some(stdout);
        self
    }

    /// `.spawn()` only: see [`ProgressRunner::stdout`].
    pub fn stderr(mut self, stderr: Stdio) -> Self {
        self.stderr = Some(stderr);
        self
    }

    /// One argument put on the Windows command line as is, for `cmd.exe /C` bodies that
    /// `Command::arg` would re-quote with MSVC rules (`\"`) which `cmd.exe` does not parse.
    #[cfg(windows)]
    pub fn raw_arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        use std::os::windows::process::CommandExt;
        self.cmd.raw_arg(arg);
        self
    }

    /// Windows CLIs installed through npm (`npx`, `claude`) ship as `.cmd` or `.bat` shims, which
    /// `CreateProcess` cannot start and `PATHEXT` never reaches; run those through
    /// `cmd.exe /D /C`, as the shell does. A no-op elsewhere and for anything that is an `.exe`
    /// or a path.
    pub fn windows_shim(mut self) -> Self {
        self.shim = true;
        self
    }

    /// Kill the child after `limit` and fail with [`io::ErrorKind::TimedOut`]. Only the child is
    /// killed, not what it started: a grandchild that keeps the pipes open delays `.run()`.
    pub fn timeout(mut self, limit: Duration) -> Self {
        self.timeout = Some(limit);
        self
    }

    pub fn progress(mut self, progress: Progress) -> Self {
        self.progress = progress;
        self
    }

    pub fn output(mut self, output: Output) -> Self {
        self.output = output;
        self
    }

    /// Run to the end. The indicator is cleared before this returns, so the caller's own
    /// output or error lands on a clean line.
    pub fn run(self) -> io::Result<Captured> {
        self.run_capped(u64::MAX).map(|(done, _)| done)
    }

    /// [`ProgressRunner::run`] that keeps at most `cap` bytes of stdout and stderr together.
    /// Bytes past it are still read, so the child never blocks on a full pipe and exits on its
    /// own, but are counted in the second value instead of stored: `yes | head -c 10G` must
    /// not take memory the unwrapped command would not.
    pub fn run_capped(mut self, cap: u64) -> io::Result<(Captured, u64)> {
        match self.output {
            Output::Capture | Output::Stream => self.run_piped(cap),
            Output::Inherit => {
                self.prepare(false);
                // `Command::status` cannot be bounded, so spawn and share the piped path's wait.
                let status = wait(&mut self.cmd.spawn()?, self.timeout)?;
                Ok((
                    Captured {
                        status,
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                    },
                    0,
                ))
            }
            Output::Detached => Err(use_spawn("a detached child")),
        }
    }

    /// Start a child the caller owns. `Capture` pipes stdout and stderr without an indicator;
    /// `Detached` nulls them; `Inherit` shares ours.
    pub fn spawn(mut self) -> io::Result<Child> {
        self.prepare(self.output != Output::Inherit);
        match self.output {
            Output::Stream => return Err(use_spawn("a streamed child")),
            Output::Capture => {
                self.cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            }
            Output::Detached => {
                self.cmd.stdout(Stdio::null()).stderr(Stdio::null());
            }
            Output::Inherit => {}
        }
        if let Some(stdout) = self.stdout.take() {
            self.cmd.stdout(stdout);
        }
        if let Some(stderr) = self.stderr.take() {
            self.cmd.stderr(stderr);
        }
        self.cmd.spawn()
    }

    /// Settings that wait for the last moment: the shim replaces the command and `Command` cannot
    /// hand its stdio over, so stdin is applied here and stdout and stderr only after this.
    /// `closed` is the stdin of a child nobody feeds.
    fn prepare(&mut self, closed: bool) {
        if self.shim && cfg!(windows) && needs_cmd_host(&self.cmd.get_program().to_string_lossy()) {
            self.cmd = via_cmd_host(&self.cmd);
        }
        match self.stdin.take() {
            Some(stdin) => self.cmd.stdin(stdin),
            None if closed => self.cmd.stdin(Stdio::null()),
            None => &mut self.cmd,
        };
    }

    fn run_piped(mut self, cap: u64) -> io::Result<(Captured, u64)> {
        self.prepare(true);
        self.cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = self.cmd.spawn()?;
        let ind: Arc<dyn Indicator> = match self.progress {
            Progress::Off => Arc::new(Hidden),
            _ => indicator::pick(&self.label).into(),
        };
        let parser = Arc::new(Mutex::new(match self.progress {
            Progress::Parse(p) => Some(p),
            Progress::Callback(f) => Some(Box::new(parse::Callback(f)) as Box<dyn ProgressParser>),
            _ => None,
        }));
        let pump = Pump {
            ind: ind.clone(),
            parser,
            budget: Arc::new(Budget::new(cap)),
            forward: self.output == Output::Stream,
        };
        let (tx, rx) = mpsc::channel();
        let out = pump.start(child.stdout.take(), Stream::Stdout, tx.clone());
        let err = pump.start(child.stderr.take(), Stream::Stderr, tx);
        let status = wait(&mut child, self.timeout);
        let deadline = Instant::now() + DRAIN_AFTER_EXIT;
        for _ in 0..2 {
            if rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .is_err()
            {
                break;
            }
        }
        ind.finish();
        let take = |b: &Mutex<Vec<u8>>| std::mem::take(&mut *lock(b));
        let done = Captured {
            status: status?,
            stdout: take(&out),
            stderr: take(&err),
        };
        Ok((done, pump.budget.dropped.load(Ordering::Relaxed)))
    }
}

/// The code a shell would report for `status`: its own code, or `128 + signal` for a Unix
/// signal death, where `ExitStatus::code()` is `None` and a caller that defaults it to 1 hides
/// an OOM kill (137) or a timeout (143) behind an ordinary failure.
pub fn exit_code(status: ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    if let Some(sig) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return Some(128 + sig);
    }
    status.code()
}

/// True when `command` is a Windows batch or `PATHEXT` shim that `CreateProcess` cannot run: a
/// `.cmd` or `.bat` path, or a bare name (`npx`, `uvx`) that `PATHEXT` may resolve to one.
pub fn needs_cmd_host(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    if lower.ends_with(".exe") || lower.ends_with(".com") {
        return false;
    }
    if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        return true;
    }
    !command.contains(['/', '\\', ':'])
}

/// `cmd.exe /D /C <program> <args>` with the directory and environment of `old`.
fn via_cmd_host(old: &Command) -> Command {
    let mut cmd = Command::new("cmd.exe");
    cmd.arg("/D")
        .arg("/C")
        .arg(old.get_program())
        .args(old.get_args());
    if let Some(dir) = old.get_current_dir() {
        cmd.current_dir(dir);
    }
    for (key, val) in old.get_envs() {
        match val {
            Some(val) => cmd.env(key, val),
            None => cmd.env_remove(key),
        };
    }
    cmd
}

/// A bar over a known number of items, from [`ProgressRunner::batch`]. It counts finished
/// items: [`Batch::step`] finishes the previous one and names the next.
pub struct Batch {
    ind: Box<dyn Indicator>,
    total: u64,
    pos: u64,
    unit: Unit,
    running: bool,
}

fn wait_on<T>(ind: Box<dyn Indicator>, f: impl FnOnce() -> T) -> T {
    let out = f();
    ind.finish();
    out
}

impl Batch {
    /// A batch that draws nothing, for a caller that does not own the terminal (the MCP server,
    /// a background watcher) but shares the code path of one that does.
    pub fn hidden(total: u64, unit: Unit) -> Self {
        Self::new(Box::new(Hidden), total, unit)
    }

    fn new(ind: Box<dyn Indicator>, total: u64, unit: Unit) -> Self {
        Self {
            ind,
            total,
            pos: 0,
            unit,
            running: false,
        }
    }

    /// Items finished so far.
    pub fn position(&self) -> u64 {
        self.pos
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    /// The previous item is done; `msg` names the one that starts now.
    pub fn step(&mut self, msg: &str) {
        self.complete();
        self.running = true;
        self.draw(Some(msg.to_string()));
    }

    /// The last item is done; the bar is cleared.
    pub fn finish(mut self) {
        self.complete();
        self.draw(None);
        self.ind.finish();
    }

    fn complete(&mut self) {
        if std::mem::take(&mut self.running) {
            self.pos = (self.pos + 1).min(self.total);
        }
    }

    fn draw(&self, msg: Option<String>) {
        self.ind.set(&Update {
            pos: self.pos,
            total: Some(self.total),
            unit: self.unit,
            msg,
        });
    }
}

fn use_spawn(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{what} does not fit this call: see ProgressRunner::run and ::spawn"),
    )
}

fn wait(child: &mut Child, limit: Option<Duration>) -> io::Result<ExitStatus> {
    let Some(limit) = limit else {
        return child.wait();
    };
    let end = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the command timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the two pipe readers of one run share: a byte budget for stdout and stderr together,
/// and the count of bytes that did not fit.
struct Budget {
    left: AtomicU64,
    dropped: AtomicU64,
}

impl Budget {
    fn new(cap: u64) -> Self {
        Self {
            left: AtomicU64::new(cap),
            dropped: AtomicU64::new(0),
        }
    }

    /// Take up to `n` bytes from the budget; the rest of `n` is counted as dropped.
    fn take(&self, n: usize) -> usize {
        let n = n as u64;
        let got = self
            .left
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                Some(left - left.min(n))
            })
            .map_or(0, |left| left.min(n));
        self.dropped.fetch_add(n - got, Ordering::Relaxed);
        got as usize
    }
}

type SharedParser = Mutex<Option<Box<dyn ProgressParser>>>;

/// What both reader threads of one run share. The threads own it (not a scope) so a reader
/// blocked on a pipe a descendant holds open can be left behind after [`DRAIN_AFTER_EXIT`].
#[derive(Clone)]
struct Pump {
    ind: Arc<dyn Indicator>,
    parser: Arc<SharedParser>,
    budget: Arc<Budget>,
    forward: bool,
}

impl Pump {
    /// Drain one stream on its own thread into the returned buffer, as far as the budget
    /// allows (or forward it live), and feed its lines to the parser; `done` fires at EOF.
    fn start(
        &self,
        reader: Option<impl Read + Send + 'static>,
        which: Stream,
        done: mpsc::Sender<()>,
    ) -> Arc<Mutex<Vec<u8>>> {
        let kept = Arc::new(Mutex::new(Vec::new()));
        let (sink, this) = (kept.clone(), self.clone());
        std::thread::spawn(move || {
            if let Some(reader) = reader {
                this.drain(reader, which, &sink);
            }
            let _ = done.send(());
        });
        kept
    }

    fn drain(&self, mut reader: impl Read, which: Stream, sink: &Mutex<Vec<u8>>) {
        let mut buf = [0u8; 8192];
        let mut lines = parse::Lines::default();
        let mut feed = |line: &str| {
            let mut slot = lock(&self.parser);
            if let Some(update) = slot.as_mut().and_then(|p| p.feed(which, line)) {
                self.ind.set(&update);
            }
        };
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            if self.forward {
                self.ind.suspend(&mut || echo(which, &buf[..n]));
            } else {
                let keep = self.budget.take(n);
                if keep > 0 {
                    lock(sink).extend_from_slice(&buf[..keep]);
                }
            }
            lines.push(&buf[..n], &mut feed);
        }
        lines.finish(&mut feed);
    }
}

fn echo(which: Stream, bytes: &[u8]) {
    let _ = match which {
        Stream::Stdout => io::stdout()
            .write_all(bytes)
            .and_then(|()| io::stdout().flush()),
        Stream::Stderr => io::stderr()
            .write_all(bytes)
            .and_then(|()| io::stderr().flush()),
    };
}

/// T276.3: a shimmed `.cmd` host once printed its `--version` straight to rtok's stdout, because
/// the stdio set before the shim swapped the command was lost with it.
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    fn cmd_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-sys-shim-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("host.cmd");
        std::fs::write(&file, "@echo off\r\necho shimmed %1\r\n").unwrap();
        file
    }

    #[test]
    fn a_shimmed_cmd_host_is_captured_not_printed() {
        let file = cmd_file("run");
        let got = ProgressRunner::quiet(&file, ["--version"])
            .windows_shim()
            .run()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&got.stdout).trim(),
            "shimmed --version"
        );
    }

    #[test]
    fn a_spawned_shimmed_cmd_host_writes_to_its_pipe() {
        let file = cmd_file("spawn");
        let got = ProgressRunner::quiet(&file, ["x"])
            .windows_shim()
            .spawn()
            .unwrap()
            .wait_with_output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&got.stdout).trim(), "shimmed x");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn sh(script: &str) -> ProgressRunner {
        ProgressRunner::new("testing…", "sh", ["-c", script])
    }

    #[test]
    fn capture_returns_what_command_output_would() {
        let got = sh("printf out; printf err >&2; exit 3").run().unwrap();
        let want = Command::new("sh")
            .args(["-c", "printf out; printf err >&2; exit 3"])
            .output()
            .unwrap();
        assert_eq!(got, want);
        assert_eq!(got.status.code(), Some(3));
    }

    #[test]
    fn a_missing_program_is_the_spawn_error() {
        let err = ProgressRunner::quiet("rtok-no-such-program", ["x"])
            .run()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn stdin_is_closed_unless_given_and_cwd_and_env_reach_the_child() {
        let closed = sh("cat").run().unwrap();
        assert!(closed.status.success() && closed.stdout.is_empty());
        let dir = std::env::temp_dir();
        let got = sh("printf '%s|%s' \"$PWD\" \"$T276\"")
            .cwd(&dir)
            .env("T276", "v")
            .run()
            .unwrap();
        let text = String::from_utf8_lossy(&got.stdout).into_owned();
        assert!(text.ends_with("|v"), "{text}");
    }

    #[test]
    fn a_slow_child_is_killed_at_the_timeout() {
        let start = Instant::now();
        let err = sh("exec sleep 5")
            .timeout(Duration::from_millis(100))
            .run()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(
            sh("exit 0")
                .timeout(Duration::from_secs(5))
                .run()
                .unwrap()
                .status
                .success()
        );
    }

    /// T651: an inherited child used to be waited on without the limit.
    #[test]
    fn an_inherited_slow_child_is_killed_at_the_timeout() {
        let start = Instant::now();
        let err = sh("exec sleep 2")
            .output(Output::Inherit)
            .timeout(Duration::from_millis(100))
            .run()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn an_inherited_child_inside_its_timeout_keeps_its_own_status() {
        let done = sh("exit 3")
            .output(Output::Inherit)
            .timeout(Duration::from_secs(5))
            .run()
            .unwrap();
        assert_eq!(
            (done.status.code(), done.stdout.len(), done.stderr.len()),
            (Some(3), 0, 0)
        );
        let err = sh("exec sleep 2")
            .output(Output::Inherit)
            .timeout(Duration::ZERO)
            .run()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn an_inherited_child_that_cannot_start_fails_to_spawn_not_to_time_out() {
        let err = ProgressRunner::quiet("/nonexistent/rtok-t651", ["x"])
            .output(Output::Inherit)
            .timeout(Duration::from_secs(5))
            .run()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    struct Record(Arc<Mutex<Vec<(Stream, String)>>>);

    impl ProgressParser for Record {
        fn feed(&mut self, stream: Stream, line: &str) -> Option<Update> {
            self.0.lock().unwrap().push((stream, line.to_string()));
            None
        }
    }

    #[test]
    fn the_parser_sees_both_streams_split_on_carriage_returns() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let done = sh("printf 'a\\rb\\n' >&2; printf 'c\\n'")
            .progress(Progress::Parse(Box::new(Record(seen.clone()))))
            .run()
            .unwrap();
        assert_eq!(done.stdout, b"c\n");
        assert_eq!(done.stderr, b"a\rb\n");
        let mut got = seen.lock().unwrap().clone();
        got.sort_by_key(|(_, l)| l.clone());
        let want = [
            (Stream::Stderr, "a"),
            (Stream::Stderr, "b"),
            (Stream::Stdout, "c"),
        ];
        assert_eq!(got, want.map(|(s, l)| (s, l.to_string())));
    }

    #[test]
    fn stream_and_inherit_leave_nothing_to_capture() {
        let streamed = sh("exit 4").output(Output::Stream).run().unwrap();
        assert_eq!(
            (streamed.status.code(), streamed.stdout.len()),
            (Some(4), 0)
        );
        let inherited = sh("exit 0").output(Output::Inherit).run().unwrap();
        assert!(inherited.status.success());
    }

    #[test]
    fn spawn_takes_explicit_stdout_and_stderr_over_the_mode_defaults() {
        let mut child = sh("cat; printf err >&2")
            .output(Output::Inherit)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"in").unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!((out.stdout.as_slice(), out.stderr.len()), (&b"in"[..], 0));
    }

    #[test]
    fn spawn_and_run_refuse_the_modes_that_do_not_fit() {
        let child = sh("exit 0").output(Output::Detached).spawn().unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success() && out.stdout.is_empty());
        let err = sh("exit 0").output(Output::Detached).run().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let err = sh("exit 0").output(Output::Stream).spawn().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let piped = sh("printf x").spawn().unwrap().wait_with_output().unwrap();
        assert_eq!(piped.stdout, b"x");
    }

    struct Seen(Arc<Mutex<Vec<Update>>>);

    impl Indicator for Seen {
        fn set(&self, update: &Update) {
            self.0.lock().unwrap().push(update.clone());
        }
        fn message(&self, _: &str) {}
        fn suspend(&self, f: &mut dyn FnMut()) {
            f();
        }
        fn finish(&self) {
            self.0.lock().unwrap().push(Update {
                pos: u64::MAX,
                total: None,
                unit: Unit::Items,
                msg: None,
            });
        }
    }

    #[test]
    fn a_batch_counts_finished_items_and_names_the_running_one() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut batch = Batch::new(Box::new(Seen(seen.clone())), 2, Unit::Items);
        assert_eq!((batch.position(), batch.total()), (0, 2));
        batch.step("a");
        assert_eq!(batch.position(), 0);
        batch.step("b");
        assert_eq!(batch.position(), 1);
        batch.finish();
        let got: Vec<_> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|u| (u.pos, u.total, u.msg.clone()))
            .collect();
        let want = [
            (0, Some(2), Some("a")),
            (1, Some(2), Some("b")),
            (2, Some(2), None),
            (u64::MAX, None, None),
        ];
        let want: Vec<_> = want
            .map(|(p, t, m)| (p, t, m.map(String::from)))
            .into_iter()
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_batch_never_counts_past_its_total_and_a_hidden_one_draws_nothing() {
        let mut batch = Batch::new(Box::new(Hidden), 1, Unit::Items);
        for item in ["a", "b", "c"] {
            batch.step(item);
        }
        assert_eq!(batch.position(), 1);
        batch.finish();
        // Under test stderr is not a terminal, so the runner's own pick is Hidden too.
        ProgressRunner::batch("x…", 3, Unit::Items).finish();
    }

    #[test]
    fn a_wait_clears_its_spinner_after_the_work_and_passes_the_result_through() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let got = wait_on(Box::new(Seen(seen.clone())), || {
            assert!(
                seen.lock().unwrap().is_empty(),
                "cleared before the work ran"
            );
            Err::<(), _>("boom")
        });
        assert_eq!(got, Err("boom"));
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "cleared once, after the work"
        );
        assert_eq!(ProgressRunner::wait("x…", || 7), 7);
    }

    #[test]
    fn a_batch_bar_and_a_command_spinner_share_one_target() {
        let multi =
            indicatif::MultiProgress::with_draw_target(indicatif::ProgressDrawTarget::hidden());
        let bar = TtyIndicator::bar(&multi, "removing…", 3, Unit::Items);
        let spinner = TtyIndicator::new(&multi, "git…");
        bar.set(&Update {
            pos: 1,
            total: Some(3),
            unit: Unit::Items,
            msg: Some("x".into()),
        });
        spinner.finish();
        bar.finish();
    }

    #[test]
    fn a_callback_reads_both_streams_and_can_end_the_spinner_phase() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let cb = move |s: Stream, line: &str| {
            log.lock().unwrap().push((s, line.to_string()));
            None
        };
        let done = sh("printf 'one\\n'; printf 'two\\n' >&2")
            .progress(Progress::Callback(Box::new(cb)))
            .run()
            .unwrap();
        assert_eq!(
            (done.stdout.as_slice(), done.stderr.as_slice()),
            (&b"one\n"[..], &b"two\n"[..])
        );
        let mut got = seen.lock().unwrap().clone();
        got.sort_by_key(|(_, l)| l.clone());
        assert_eq!(
            got,
            [
                (Stream::Stdout, "one".to_string()),
                (Stream::Stderr, "two".to_string())
            ]
        );
    }

    #[test]
    fn the_cmd_host_wraps_npm_shims_not_exes() {
        assert!(needs_cmd_host("npx"));
        assert!(needs_cmd_host("uvx"));
        assert!(needs_cmd_host(r"C:\Program Files\nodejs\npx.cmd"));
        assert!(needs_cmd_host("tool.bat"));
        assert!(!needs_cmd_host("rtok.exe"));
        assert!(!needs_cmd_host(r"C:\Users\u\.ketch\bin\rtok.exe"));
        assert!(!needs_cmd_host("/usr/bin/uvx"));
    }

    #[test]
    fn the_cmd_host_command_keeps_args_directory_and_environment() {
        let mut old = Command::new("npx");
        old.args(["-y", "pkg"]).current_dir("/tmp").env("A", "1");
        old.env_remove("B");
        let new = via_cmd_host(&old);
        assert_eq!(new.get_program(), "cmd.exe");
        let args: Vec<_> = new.get_args().collect();
        assert_eq!(args, ["/D", "/C", "npx", "-y", "pkg"]);
        assert_eq!(new.get_current_dir(), Some(Path::new("/tmp")));
        let envs: Vec<_> = new.get_envs().collect();
        assert_eq!(
            envs,
            [
                (OsStr::new("A"), Some(OsStr::new("1"))),
                (OsStr::new("B"), None)
            ]
        );
    }

    /// T638: a descendant that outlives the child keeps the pipe open; the drain after the
    /// child's exit is finite, so a run neither stalls nor loses the child's output.
    #[test]
    fn a_descendant_holding_the_pipe_does_not_stall_the_run() {
        let start = Instant::now();
        let got = sh("echo hi; sleep 20 & exit 4").run().unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(
            (got.stdout.as_slice(), got.status.code()),
            (&b"hi\n"[..], Some(4))
        );
    }

    #[test]
    fn a_cap_keeps_the_first_bytes_and_counts_the_rest() {
        let (got, dropped) = sh("head -c 5000 /dev/zero | tr '\\0' x; exit 2")
            .run_capped(1024)
            .unwrap();
        assert_eq!(
            (got.stdout.len(), dropped, got.status.code()),
            (1024, 3976, Some(2))
        );
        // The budget is shared by both streams, and the child still exits with its own code.
        let (got, dropped) = sh("head -c 50000 /dev/zero; head -c 7000 /dev/zero >&2; exit 3")
            .run_capped(1000)
            .unwrap();
        assert_eq!(got.stdout.len() + got.stderr.len(), 1000);
        assert_eq!((dropped, got.status.code()), (56_000, Some(3)));
        let (got, dropped) = sh("printf hello").run_capped(5).unwrap();
        assert_eq!((got.stdout.as_slice(), dropped), (&b"hello"[..], 0));
    }

    #[test]
    fn the_exit_code_is_128_plus_the_signal_of_a_killed_child() {
        for (script, code) in [
            ("kill -TERM $$", 143),
            ("kill -KILL $$", 137),
            ("exit 7", 7),
        ] {
            let got = sh(script).run().unwrap();
            assert_eq!(exit_code(got.status), Some(code), "{script}");
        }
    }

    #[test]
    fn the_shim_is_a_no_op_off_windows() {
        let got = ProgressRunner::quiet("sh", ["-c", "cat; printf ok"])
            .windows_shim()
            .stdin(Stdio::null())
            .run()
            .unwrap();
        assert_eq!(got.stdout, b"ok");
    }
}

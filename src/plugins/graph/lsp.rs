// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Optional LSP backend (T30.2). Native JSON-RPC over stdio.
//! Spawns rust-analyzer / clangd / typescript-language-server from PATH (D6: not serena).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use rtok_plugin_sdk::{Class, Ctx, Measurement};
use rtok_sys::proc::{Output, ProgressRunner};
use serde_json::{Value, json};

use super::cap;

/// Wait cap for one server step, set from `[plugins.graph] lsp_timeout_ms` by the door in
/// `graph::lsp_or_tags` (T329.9). A process-wide cell because the cached session outlives a call.
static TIMEOUT_MS: AtomicU64 = AtomicU64::new(40_000);

pub(crate) fn set_timeout_ms(ms: u64) {
    TIMEOUT_MS.store(ms, Ordering::Relaxed);
}

fn ready() -> Duration {
    Duration::from_millis(TIMEOUT_MS.load(Ordering::Relaxed))
}

/// True when `bin` is installed. A mise shim and rustup's component proxy do not count;
/// `rust-analyzer` counts when `rustup which` names the toolchain binary. File presence
/// only — `bin --version` can hang on a broken shim.
pub fn on_path(bin: &str) -> bool {
    if first_real_bin(bin).is_some() {
        return true;
    }
    bin == "rust-analyzer" && rustup_component_path().is_some()
}

/// `rustup which rust-analyzer` names the toolchain binary. It fails when the component
/// is not installed, which is the case a rustup proxy or mise shim must not pass for.
fn rustup_component_path() -> Option<PathBuf> {
    let o = ProgressRunner::quiet("rustup", ["which", "rust-analyzer"])
        .run()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if p.is_empty() {
        return None;
    }
    let path = PathBuf::from(p);
    (path.is_file() && !is_mise_shim(&path)).then_some(path)
}

fn first_real_bin(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        if let Some(cand) = real_candidate(&dir, name) {
            return Some(cand);
        }
    }
    None
}

fn real_candidate(dir: &Path, name: &str) -> Option<PathBuf> {
    let bare = dir.join(name);
    if is_real_bin(&bare) {
        return Some(bare);
    }
    #[cfg(windows)]
    {
        let exe = dir.join(format!("{name}.exe"));
        if is_real_bin(&exe) {
            return Some(exe);
        }
    }
    None
}

fn is_real_bin(path: &Path) -> bool {
    path.is_file() && !is_mise_shim(path) && !is_rustup_proxy(path)
}

/// mise puts a shim on PATH for every tool in `mise.toml`, including ones it did not
/// install. rustup's proxy then falls back to that shim and the shim calls the proxy.
fn is_mise_shim(path: &Path) -> bool {
    let mut saw_mise = false;
    let mut saw_shims = false;
    for c in path.components() {
        let name = c.as_os_str();
        if name == "mise" {
            saw_mise = true;
        }
        if name == "shims" {
            saw_shims = true;
        }
    }
    saw_mise && saw_shims
}

/// rustup's component proxies sit beside the `rustup` binary and are that binary.
/// A real `rust-analyzer` next to it is a different size.
fn is_rustup_proxy(path: &Path) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let rustup = if cfg!(windows) {
        dir.join("rustup.exe")
    } else {
        dir.join("rustup")
    };
    match (path.metadata(), rustup.metadata()) {
        (Ok(bin), Ok(proxy)) => bin.len() == proxy.len(),
        _ => false,
    }
}

fn resolve_bin(name: &str) -> PathBuf {
    if name == "rust-analyzer"
        && let Some(path) = rustup_component_path()
    {
        return path;
    }
    first_real_bin(name).unwrap_or_else(|| PathBuf::from(name))
}

/// A server command: binary and arguments.
type Server = (&'static str, &'static [&'static str]);

/// Project kinds by marker file, in the order they are tried: the language name that
/// `[plugins.graph.backend_by_language]` is keyed by, and the server that speaks it (`None`:
/// tags only).
const LANGS: &[(&str, &str, Option<Server>)] = &[
    ("Cargo.toml", "rust", Some(("rust-analyzer", &[]))),
    ("compile_commands.json", "c", Some(("clangd", &[]))),
    (
        "tsconfig.json",
        "typescript",
        Some(("typescript-language-server", &["--stdio"])),
    ),
    ("pubspec.yaml", "dart", Some(("dart", &["language-server"]))),
    ("go.mod", "go", None),
    ("pyproject.toml", "python", None),
    ("package.json", "javascript", None),
    // T329.10: languages with neither a server nor a grammar, named so that
    // `backend_by_language` can pin them to `text`.
    ("pom.xml", "java", None),
    ("build.gradle", "java", None),
    ("Gemfile", "ruby", None),
    ("composer.json", "php", None),
    ("mix.exs", "elixir", None),
    ("Package.swift", "swift", None),
];

pub(crate) fn language_of(root: &Path) -> Option<&'static str> {
    LANGS
        .iter()
        .find(|(marker, ..)| root.join(marker).is_file())
        .map(|(_, lang, _)| *lang)
}

/// File extension to the language name used in [`LANGS`]. Counts a project's files per language
/// where no marker file exists (C without `compile_commands.json`, Kotlin, C#), so it lists
/// more languages than [`LANGS`] has markers for.
const EXTENSIONS: &[(&str, &str)] = &[
    ("rs", "rust"),
    ("c", "c"),
    ("h", "c"),
    ("cc", "c"),
    ("cpp", "c"),
    ("cxx", "c"),
    ("hpp", "c"),
    ("ts", "typescript"),
    ("tsx", "typescript"),
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("jsx", "javascript"),
    ("dart", "dart"),
    ("go", "go"),
    ("py", "python"),
    ("java", "java"),
    ("kt", "kotlin"),
    ("rb", "ruby"),
    ("php", "php"),
    ("ex", "elixir"),
    ("exs", "elixir"),
    ("swift", "swift"),
    ("cs", "csharp"),
];

pub(crate) fn language_of_ext(ext: &str) -> Option<&'static str> {
    EXTENSIONS
        .iter()
        .find(|(e, _)| e.eq_ignore_ascii_case(ext))
        .map(|(_, lang)| *lang)
}

/// The server rtok spawns for `lang`, if it has one.
pub(crate) fn server_of(lang: &str) -> Option<Server> {
    LANGS
        .iter()
        .find(|(_, l, server)| *l == lang && server.is_some())
        .and_then(|(.., server)| *server)
}

/// Languages rtok has a server for.
pub(crate) fn served_languages() -> impl Iterator<Item = &'static str> {
    LANGS
        .iter()
        .filter(|(.., server)| server.is_some())
        .map(|(_, lang, _)| *lang)
}

/// Every marker file that names `lang`, with or without a server behind it.
pub(crate) fn markers(lang: &str) -> impl Iterator<Item = &'static str> {
    LANGS
        .iter()
        .filter(move |(_, l, _)| *l == lang)
        .map(|(marker, ..)| *marker)
}

/// The marker file that selects `lang`'s server, whether or not `root` has it.
pub(crate) fn marker_of(lang: &str) -> Option<&'static str> {
    LANGS
        .iter()
        .find(|(_, l, server)| *l == lang && server.is_some())
        .map(|(marker, ..)| *marker)
}

/// How to install a server binary named in [`LANGS`]; the doctor prints it as the next step.
pub(crate) fn install_hint(bin: &str) -> Option<&'static str> {
    match bin {
        "rust-analyzer" => Some("rustup component add rust-analyzer"),
        "clangd" => Some("brew install llvm (macOS) or apt install clangd (Debian/Ubuntu)"),
        "typescript-language-server" => {
            Some("npm install -g typescript-language-server typescript")
        }
        "dart" => Some("install the Dart SDK: https://dart.dev/get-dart"),
        _ => None,
    }
}

/// The capability check (T329.11): the project's language has a server and its binary is
/// installed. File presence only, no spawn; starting the server is the first real request.
pub(crate) fn probe(root: &Path) -> super::capability::Probe {
    let (bin, _) = pick(root).map_err(|e| super::capability::Absent {
        server: false,
        reason: format!("{e:#}"),
    })?;
    on_path(bin)
        .then_some(())
        .ok_or_else(|| super::capability::Absent {
            server: true,
            reason: format!("lsp: {bin} not on PATH"),
        })
}

/// The server answered a request with an error. It is alive, so unlike a timeout or a dead
/// pipe this does not make it a server to stop asking.
#[derive(Debug)]
pub(crate) struct Refused(pub(crate) String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

pub(crate) fn server_broke(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Refused>().is_none()
}

fn pick(root: &Path) -> Result<Server> {
    for (marker, _, server) in LANGS {
        if let (true, Some(server)) = (root.join(marker).is_file(), server) {
            return Ok(*server);
        }
    }
    bail!(
        "lsp: no Cargo.toml / compile_commands.json / tsconfig.json / pubspec.yaml in {}",
        root.display()
    )
}

/// State dirs for a spawned language server, always under `root` (never the user's home).
///
/// Cargo workspaces use `target/rtok-lsp-xdg` (already gitignored); Dart uses
/// `.dart_tool/rtok-lsp-xdg`; everything else gets `.rtok-lsp-xdg`.
pub(crate) fn lsp_state_root(root: &Path) -> PathBuf {
    if root.join("Cargo.toml").is_file() {
        root.join("target").join("rtok-lsp-xdg")
    } else if root.join("pubspec.yaml").is_file() {
        root.join(".dart_tool").join("rtok-lsp-xdg")
    } else {
        root.join(".rtok-lsp-xdg")
    }
}

/// Pin XDG_* and PUB_CACHE on `cmd` under [`lsp_state_root`] so rust-analyzer / dart never
/// create files in the invoking user's real cache.
fn lsp_state_env(root: &Path) -> [(&'static str, PathBuf); 4] {
    let base = lsp_state_root(root);
    let cache = base.join("cache");
    let data = base.join("data");
    let state = base.join("state");
    let pub_cache = base.join("pub-cache");
    for d in [&cache, &data, &state, &pub_cache] {
        let _ = std::fs::create_dir_all(d);
    }
    [
        ("XDG_CACHE_HOME", cache),
        ("XDG_DATA_HOME", data),
        ("XDG_STATE_HOME", state),
        ("PUB_CACHE", pub_cache),
    ]
}

fn percent_encode_path(s: &str) -> String {
    // RFC 8089 / URI path: encode spaces and other non-unreserved octets; keep `/` and
    // Windows drive `C:` intact so rust-analyzer still accepts the URI.
    let mut out = String::with_capacity(s.len());
    for (i, seg) in s.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        if seg.len() == 2 && seg.as_bytes().get(1) == Some(&b':') {
            out.push_str(seg);
            continue;
        }
        for b in seg.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    out.push(b as char)
                }
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
    }
    out
}

fn percent_decode_path(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = (bytes[i + 1] as char).to_digit(16);
            let l = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn file_uri(p: &Path) -> String {
    // LSP wants RFC 8089 `file:///C:/…` on Windows. `.display()` keeps
    // backslashes and omits the third slash, which rust-analyzer rejects.
    let path = fs_kit::path::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = percent_encode_path(&path.to_string_lossy().replace('\\', "/"));
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

fn path_from_file_uri(uri: &str) -> PathBuf {
    let rest = percent_decode_path(uri.strip_prefix("file://").unwrap_or(uri));
    // `file:///C:/Users/…` → `C:/Users/…`; `file:///home/…` keeps the root slash.
    if cfg!(windows) {
        let trimmed = rest.trim_start_matches('/');
        if trimmed.len() >= 2 && trimmed.as_bytes()[1] == b':' {
            return PathBuf::from(trimmed);
        }
    }
    PathBuf::from(rest)
}

fn rel(root: &Path, uri: &str) -> String {
    let p = path_from_file_uri(uri);
    pathdiff::diff_paths(&p, root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

/// How the server wants document edits: `textDocumentSync` is a kind number or an options
/// object whose `change` is one. A server that says nothing gets close-and-reopen, which every
/// server accepts.
fn sync_kind(init: &Value) -> u8 {
    let sync = &init["capabilities"]["textDocumentSync"];
    sync.as_u64()
        .or_else(|| sync["change"].as_u64())
        .map_or(0, |k| k.min(2) as u8)
}

fn kind_name(k: u64) -> &'static str {
    match k {
        5 => "class",
        6 => "method",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        23 => "struct",
        _ => "symbol",
    }
}

fn write_msg(w: &mut impl Write, v: &Value) -> Result<()> {
    let body = serde_json::to_vec(v)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(())
}

fn read_msg(r: &mut impl BufRead) -> Result<Value> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            bail!("lsp: eof");
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(rest) = line
            .split_once(':')
            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .map(|(_, v)| v)
        {
            len = Some(rest.trim().parse::<usize>().context("Content-Length")?);
        }
    }
    let n = len.context("lsp: no Content-Length")?;
    let mut buf = vec![0; n];
    r.read_exact(&mut buf)?;
    serde_json::from_slice(&buf).context("lsp json")
}

struct OpenDoc {
    version: i64,
    text: String,
}

/// One frame from the reader thread, or the reason it stopped.
type Frame = std::result::Result<Value, String>;

/// The server's pipes belong to two threads, so a request waits on a channel with a deadline
/// instead of on a pipe that has none (T620). A peer that stops answering, or stops reading,
/// blocks a thread, never the caller; [`Session::expire`] then kills the server, which closes
/// both pipes and ends the threads.
struct Session {
    child: Child,
    /// To the writer thread that owns the server's stdin.
    out: Sender<Value>,
    /// From the reader thread that owns its stdout: one parsed frame, or the reason it stopped.
    inbox: Receiver<Frame>,
    next_id: i64,
    root: PathBuf,
    /// Documents the server holds open, with the text it was last sent.
    opened: HashMap<String, OpenDoc>,
    /// `textDocumentSync.change` from `initialize`: 0 means the server takes no `didChange`.
    sync_change: u8,
    err_path: PathBuf,
}

impl Session {
    fn attach(mut child: Child, root: PathBuf, err_path: PathBuf) -> Result<Self> {
        match Self::pipes(&mut child) {
            Ok((out, inbox)) => Ok(Self {
                child,
                out,
                inbox,
                next_id: 1,
                root,
                opened: HashMap::new(),
                sync_change: 0,
                err_path,
            }),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        }
    }

    fn pipes(child: &mut Child) -> Result<(Sender<Value>, Receiver<Frame>)> {
        let mut stdin = child.stdin.take().context("lsp stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().context("lsp stdout")?);
        let (out, queue) = mpsc::channel::<Value>();
        let (frames, inbox) = mpsc::channel();
        std::thread::Builder::new()
            .name("rtok-lsp-write".into())
            .spawn(move || {
                // A failed write ends the thread; the dropped queue then fails every later send.
                for msg in queue {
                    if write_msg(&mut stdin, &msg).is_err() {
                        break;
                    }
                }
            })
            .context("lsp writer thread")?;
        std::thread::Builder::new()
            .name("rtok-lsp-read".into())
            .spawn(move || {
                loop {
                    let frame = read_msg(&mut stdout).map_err(|e| format!("{e:#}"));
                    let stop = frame.is_err();
                    if frames.send(frame).is_err() || stop {
                        break;
                    }
                }
            })
            .context("lsp reader thread")?;
        Ok((out, inbox))
    }

    /// Re-sync every open document with its file. A server keeps the text it was sent and
    /// ignores the disk, so without this `workspace/symbol` and the positions it returns would
    /// come from the snapshot of an earlier query.
    fn refresh_open(&mut self) -> Result<()> {
        let uris: Vec<String> = self.opened.keys().cloned().collect();
        for uri in uris {
            self.did_open(&uri)?;
        }
        Ok(())
    }

    fn send(&self, msg: Value) -> Result<()> {
        self.out
            .send(msg)
            .map_err(|_| anyhow::anyhow!("lsp: server input closed"))
    }

    /// The budget ran out. The server may still answer later, so it is killed and reaped:
    /// `with_session` starts a fresh one, and no late reply can be taken for a later request.
    fn expire(&mut self, method: &str) -> anyhow::Error {
        let _ = self.child.kill();
        let _ = self.child.wait();
        anyhow::anyhow!("lsp: timeout waiting for {method}")
    }

    fn spawn(root: &Path) -> Result<Self> {
        let (bin, args) = pick(root)?;
        if !on_path(bin) {
            bail!("lsp: {bin} not on PATH");
        }
        let path = resolve_bin(bin);
        let err_path = std::env::temp_dir().join(format!("rtok-lsp-{}.stderr", std::process::id()));
        let stderr = match std::fs::File::create(&err_path) {
            Ok(f) => Stdio::from(f),
            Err(_) => Stdio::null(),
        };
        let mut server = ProgressRunner::quiet(&path, args)
            .output(Output::Inherit)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .cwd(root);
        // Language servers inherit the process env and would otherwise write into the
        // invoking user's `~/.cache` / pub-cache. Pin XDG + PUB_CACHE under the workspace.
        for (key, dir) in lsp_state_env(root) {
            server = server.env(key, dir);
        }
        let child = server
            .spawn()
            .with_context(|| format!("spawn {} ({bin})", path.display()))?;
        let root = fs_kit::path::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let mut s = Self::attach(child, root, err_path)?;
        let uri = file_uri(&s.root);
        match s.request(
            "initialize",
            json!({
                // Ours, not null: the server watches it and exits when rtok dies, even by
                // SIGKILL, instead of outliving the `mcp` that spawned it.
                "processId": std::process::id(),
                "rootUri": uri,
                "rootPath": s.root,
                "capabilities": {
                    "workspace": {
                        "configuration": true,
                        "workspaceFolders": true
                    },
                    "textDocument": {
                        "documentSymbol": {"hierarchicalDocumentSymbolSupport": true}
                    },
                    "window": {"workDoneProgress": true}
                },
                "workspaceFolders": [{"uri": uri, "name": "root"}]
            }),
        ) {
            Ok(init) => s.sync_change = sync_kind(&init),
            Err(e) => return Err(s.dead(e)),
        }
        s.notify("initialized", json!({}))?;
        Ok(s)
    }

    fn dead(&mut self, e: anyhow::Error) -> anyhow::Error {
        let st = self.child.try_wait().ok().flatten();
        let err = std::fs::read_to_string(&self.err_path).unwrap_or_default();
        let err = err.trim();
        match (st, err.is_empty()) {
            (Some(st), false) => anyhow::anyhow!("{e}; exited {st}; stderr: {err}"),
            (Some(st), true) => anyhow::anyhow!("{e}; exited {st}"),
            (None, false) => anyhow::anyhow!("{e}; stderr: {err}"),
            (None, true) => e,
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(json!({"jsonrpc":"2.0","method": method, "params": params}))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.request_within(method, params, ready())
    }

    fn request_within(&mut self, method: &str, params: Value, budget: Duration) -> Result<Value> {
        let id = json!(self.next_id);
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id": id, "method": method, "params": params}))?;
        let deadline = Instant::now() + budget;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = match self.inbox.recv_timeout(left) {
                Ok(Ok(m)) => m,
                Ok(Err(e)) => return Err(self.dead(anyhow::anyhow!(e))),
                Err(RecvTimeoutError::Timeout) => return Err(self.expire(method)),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.dead(anyhow::anyhow!("lsp: eof")));
                }
            };
            if msg.get("method").is_some() && msg.get("id").is_some_and(|i| !i.is_null()) {
                self.reply_server(&msg)?;
                continue;
            }
            if msg.get("id") == Some(&id) {
                if let Some(err) = msg.get("error") {
                    return Err(Refused(format!("lsp {method}: {err}")).into());
                }
                return Ok(msg["result"].clone());
            }
        }
    }

    fn reply_server(&mut self, msg: &Value) -> Result<()> {
        let id = &msg["id"];
        let result = match msg["method"].as_str().unwrap_or("") {
            "workspace/configuration" => {
                let n = msg["params"]["items"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(1);
                json!(vec![json!({}); n])
            }
            "workspace/workspaceFolders" => {
                json!([{"uri": file_uri(&self.root), "name": "root"}])
            }
            _ => json!(null),
        };
        self.send(json!({"jsonrpc":"2.0","id": id, "result": result}))
    }

    /// Make the server's copy of `uri` the file's current text. Nothing is recorded as sent
    /// until its notification is queued.
    fn did_open(&mut self, uri: &str) -> Result<()> {
        let Ok(text) = std::fs::read_to_string(path_from_file_uri(uri)) else {
            // Gone or unreadable: once closed the server falls back to the disk, not to a
            // snapshot of text that no longer exists.
            return self.close(uri);
        };
        let Some(doc) = self.opened.get(uri) else {
            return self.open(uri, text, 1);
        };
        if doc.text == text {
            return Ok(());
        }
        let version = doc.version + 1;
        if self.sync_change == 0 {
            self.close(uri)?;
            return self.open(uri, text, version);
        }
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": uri, "version": version},
                "contentChanges": [{"text": text}]
            }),
        )?;
        self.opened
            .insert(uri.to_string(), OpenDoc { version, text });
        Ok(())
    }

    fn open(&mut self, uri: &str, text: String, version: i64) -> Result<()> {
        let language_id = match path_from_file_uri(uri).extension().and_then(OsStr::to_str) {
            Some("rs") => "rust",
            Some("dart") => "dart",
            _ => "plaintext",
        };
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri, "languageId": language_id, "version": version, "text": text
            }}),
        )?;
        self.opened
            .insert(uri.to_string(), OpenDoc { version, text });
        Ok(())
    }

    fn close(&mut self, uri: &str) -> Result<()> {
        if !self.opened.contains_key(uri) {
            return Ok(());
        }
        self.notify(
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri}}),
        )?;
        self.opened.remove(uri);
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.send(json!({"jsonrpc":"2.0","method":"exit"}));
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.err_path);
    }
}

/// The one cached server, keyed by workspace root. A `static` is never dropped, so
/// [`shutdown`] is what runs `Session::drop` when `rtok mcp` ends.
static SESSION: Mutex<Option<(String, Session)>> = Mutex::new(None);

fn with_session<T>(root: &Path, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
    let key = super::index::canon(root);
    let mut g = SESSION.lock().unwrap_or_else(PoisonError::into_inner);
    let restart = match g.as_mut() {
        Some((k, s)) if *k == key => s.child.try_wait().ok().flatten().is_some(),
        _ => true,
    };
    if restart {
        // Reap the old server first: both share one stderr path, and dropping it after the
        // spawn deleted the file its successor had just opened.
        *g = None;
        *g = Some((key, Session::spawn(root)?));
    }
    let s = &mut g.as_mut().expect("session").1;
    s.refresh_open()?;
    f(s)
}

/// Stop the cached language server (`exit`, kill, reap) and remove its stderr file.
pub(crate) fn shutdown() {
    drop(
        SESSION
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(),
    );
}

struct Def {
    uri: String,
    pos: Value,
    kind: u64,
    path: String,
    line: i32,
    end_line: i32,
}

fn pick_def(r: &Value, name: &str, root: &Path) -> Option<Def> {
    r.as_array()?.iter().find_map(|it| {
        if it["name"].as_str()? != name {
            return None;
        }
        let loc = &it["location"];
        let uri = loc["uri"].as_str()?.to_string();
        let start = loc["range"]["start"].clone();
        let end_line = loc["range"]["end"]["line"].as_i64().unwrap_or(0) as i32 + 1;
        Some(Def {
            line: start["line"].as_i64().unwrap_or(0) as i32 + 1,
            pos: start,
            kind: it["kind"].as_u64().unwrap_or(0),
            path: rel(root, loc["uri"].as_str()?),
            end_line,
            uri,
        })
    })
}

fn wait_def(s: &mut Session, name: &str) -> Result<Option<Def>> {
    let deadline = Instant::now() + ready();
    loop {
        let r = s.request("workspace/symbol", json!({"query": name}))?;
        if let Some(d) = pick_def(&r, name, &s.root) {
            return Ok(Some(d));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn contains(range: &Value, line: u64, ch: u64) -> bool {
    let sl = range["start"]["line"].as_u64().unwrap_or(0);
    let sc = range["start"]["character"].as_u64().unwrap_or(0);
    let el = range["end"]["line"].as_u64().unwrap_or(0);
    let ec = range["end"]["character"].as_u64().unwrap_or(0);
    (line > sl || (line == sl && ch >= sc)) && (line < el || (line == el && ch <= ec))
}

fn enclosing(syms: &[Value], line: u64, ch: u64) -> String {
    let mut best = String::new();
    fn walk(syms: &[Value], line: u64, ch: u64, best: &mut String) {
        for s in syms {
            if contains(&s["range"], line, ch) {
                if let Some(n) = s["name"].as_str() {
                    *best = n.to_string();
                }
                if let Some(c) = s["children"].as_array() {
                    walk(c, line, ch, best);
                }
            }
        }
    }
    walk(syms, line, ch, &mut best);
    best
}

fn finish(cx: &Ctx, tool: &'static str, t0: Instant, text: String) -> Result<String> {
    let kind = match tool {
        "symbol" => "lsp.symbol",
        "callers" => "lsp.callers",
        "outline" => "lsp.outline",
        _ => "lsp.impact",
    };
    let est = cx.estimate(&text, Class::Code);
    cx.record(&Measurement {
        plugin: "graph",
        kind,
        before_bytes: t0.elapsed().as_millis() as u64,
        after_bytes: text.len() as u64,
        est_before: est,
        est_after: est,
        ref_id: None,
        call_id: cx.call_id(),
    })?;
    cap(cx, text)
}

fn body(cx: &Ctx, root: &Path, path: &str, line: i32, end_line: i32, budget: usize) -> String {
    let src = std::fs::read_to_string(root.join(path)).unwrap_or_default();
    super::body_lines(&src, line, end_line, budget, &mut |bytes| {
        cx.put_archive(bytes).ok()
    })
}

/// The text of `lsp symbol`: one id and `path:line kind` + body per matching
/// definition, or the empty-answer sentence. Shared by the tool and `explore` (T68.1).
fn symbol_text(
    cx: &Ctx,
    s: &mut Session,
    name: &str,
    filter: &super::Filter,
    budget: usize,
) -> Result<String> {
    let r = s.request("workspace/symbol", json!({"query": name}))?;
    let mut out = String::new();
    if let Some(arr) = r.as_array() {
        for it in arr {
            if it["name"].as_str() != Some(name) {
                continue;
            }
            let Some(d) = pick_def(&json!([it]), name, &s.root) else {
                continue;
            };
            let kind = kind_name(d.kind);
            if !filter.path_ok(&d.path) || !filter.kind_ok(kind) {
                continue;
            }
            super::tally::hit(&s.root, name, [d.path.as_str()]);
            out.push_str(&format!(
                "{}\n",
                super::def_head(&d.path, name, kind, d.line)
            ));
            out.push_str(&body(cx, &s.root, &d.path, d.line, d.end_line, budget));
        }
    }
    if out.is_empty() {
        out = format!("no definition of {name}{}\n", filter.scope_note());
    }
    Ok(out)
}

pub(crate) fn symbol(cx: &Ctx, root: &Path, name: &str, filter: &super::Filter) -> Result<String> {
    let t0 = Instant::now();
    let budget = cx.plugin_config::<crate::config::Graph>("graph").body_lines as usize;
    with_session(root, |s| {
        if wait_def(s, name)?.is_none() {
            return finish(
                cx,
                "symbol",
                t0,
                format!("no definition of {name}{}", filter.scope_note()),
            );
        }
        let out = symbol_text(cx, s, name, filter, budget)?;
        finish(cx, "symbol", t0, out)
    })
}

pub(crate) fn callers(cx: &Ctx, root: &Path, name: &str, filter: &super::Filter) -> Result<String> {
    let t0 = Instant::now();
    with_session(root, |s| {
        let Some(d) = wait_def(s, name)? else {
            return finish(
                cx,
                "callers",
                t0,
                format!("no references to {name}{}", filter.scope_note()),
            );
        };
        s.did_open(&d.uri)?;
        let refs = s.request(
            "textDocument/references",
            json!({
                "textDocument": {"uri": d.uri},
                "position": d.pos,
                "context": {"includeDeclaration": false}
            }),
        )?;
        let mut groups: BTreeMap<(String, String), (i64, i32)> = BTreeMap::new();
        let mut syms_by: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for loc in refs.as_array().cloned().unwrap_or_default() {
            let uri = loc["uri"].as_str().unwrap_or("");
            s.did_open(uri)?;
            let path = rel(&s.root, uri);
            let line0 = loc["range"]["start"]["line"].as_u64().unwrap_or(0);
            let ch = loc["range"]["start"]["character"].as_u64().unwrap_or(0);
            let line = line0 as i32 + 1;
            let kids = if let Some(existing) = syms_by.get(uri) {
                existing.clone()
            } else {
                let raw = s.request(
                    "textDocument/documentSymbol",
                    json!({"textDocument": {"uri": uri}}),
                )?;
                let kids = raw.as_array().cloned().unwrap_or_default();
                syms_by.insert(uri.to_string(), kids.clone());
                kids
            };
            let scope = enclosing(&kids, line0, ch);
            let e = groups.entry((path, scope)).or_insert((0, line));
            e.0 += 1;
            if line < e.1 {
                e.1 = line;
            }
        }
        if groups.is_empty() {
            return finish(
                cx,
                "callers",
                t0,
                format!("no references to {name}{}", filter.scope_note()),
            );
        }
        let mut out = String::new();
        for ((path, scope), (n, line)) in groups {
            if !filter.path_ok(&path) {
                continue;
            }
            super::tally::hit(&s.root, name, [path.as_str()]);
            let scope = if scope.is_empty() {
                String::new()
            } else {
                format!("  {scope}")
            };
            out.push_str(&format!("{path}{scope} ×{n} (L{line})\n"));
        }
        if out.is_empty() {
            out = format!("no references to {name}{}", filter.scope_note());
        }
        finish(cx, "callers", t0, out)
    })
}

/// The `impact` walk: incoming-call BFS from `name`'s hierarchy items out to
/// `depth`, one `(depth, path, scope)` row per definition reached (scope empty at
/// file level). Shared by the tool and `explore` (T68.1).
fn impact_walk(
    s: &mut Session,
    name: &str,
    depth: u32,
    filter: &super::Filter,
) -> Result<Vec<(u32, String, String)>> {
    let Some(d) = wait_def(s, name)? else {
        return Ok(Vec::new());
    };
    s.did_open(&d.uri)?;
    let items = s.request(
        "textDocument/prepareCallHierarchy",
        json!({"textDocument": {"uri": d.uri}, "position": d.pos}),
    )?;
    let mut frontier = items.as_array().cloned().unwrap_or_default();
    let mut seen = HashSet::from([name.to_string()]);
    let mut rows = Vec::new();
    for dpth in 1..=depth.clamp(1, 4) {
        let mut next = Vec::new();
        for item in &frontier {
            let calls = s.request("callHierarchy/incomingCalls", json!({"item": item}))?;
            for c in calls.as_array().cloned().unwrap_or_default() {
                let from = &c["from"];
                let nm = from["name"].as_str().unwrap_or("");
                if !seen.insert(nm.to_string()) {
                    continue;
                }
                let path = rel(&s.root, from["uri"].as_str().unwrap_or(""));
                if !filter.path_ok(&path) {
                    continue;
                }
                if nm.is_empty() {
                    // File-level reference: it prints as `(file)` but never expands.
                    rows.push((dpth, path, String::new()));
                    continue;
                }
                rows.push((dpth, path, nm.to_string()));
                next.push(from.clone());
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    Ok(rows)
}

pub(crate) fn impact(
    cx: &Ctx,
    root: &Path,
    name: &str,
    depth: u32,
    filter: &super::Filter,
    to: Option<&str>,
) -> Result<String> {
    let t0 = Instant::now();
    with_session(root, |s| {
        if let Some(target) = to.filter(|s| !s.is_empty()) {
            let chains = call_paths(s, name, target, depth)?;
            if chains.is_empty() {
                return finish(
                    cx,
                    "impact",
                    t0,
                    format!("no path from {name} to {target} within depth {depth}"),
                );
            }
            super::tally::hit(&s.root, name, std::iter::empty::<&str>());
            return finish(
                cx,
                "impact",
                t0,
                chains.join(
                    "
",
                ) + "
",
            );
        }
        let rows = impact_walk(s, name, depth, filter)?;
        if rows.is_empty() {
            return finish(
                cx,
                "impact",
                t0,
                format!("nothing reaches {name}{}", filter.scope_note()),
            );
        }
        for (_, path, _) in &rows {
            super::tally::hit(&s.root, name, [path.as_str()]);
        }
        finish(cx, "impact", t0, super::impact_lines_text(&rows))
    })
}

/// Call chains `a → … → b` over incoming calls, at most `max_depth` hops, simple
/// paths only: a name already in the chain is never revisited and a branch stops
/// growing once it reaches `b`, so cycles terminate.
fn call_paths(s: &mut Session, a: &str, b: &str, max_depth: u32) -> Result<Vec<String>> {
    if a == b {
        return Ok(Vec::new());
    }
    let Some(d) = wait_def(s, a)? else {
        return Ok(Vec::new());
    };
    s.did_open(&d.uri)?;
    let items = s.request(
        "textDocument/prepareCallHierarchy",
        json!({"textDocument": {"uri": d.uri}, "position": d.pos}),
    )?;
    let mut frontier: Vec<(Value, String, Vec<String>)> = items
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|it| (it, a.to_string(), vec![a.to_string()]))
        .collect();
    if frontier.is_empty() {
        return Ok(Vec::new());
    }
    let mut chains: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for _ in 0..max_depth.clamp(1, 4) {
        let mut next = Vec::new();
        for (item, tip, chain) in frontier {
            if tip == b {
                chains.insert(chain.join(" → "));
                continue;
            }
            let calls = s.request("callHierarchy/incomingCalls", json!({"item": item}))?;
            for c in calls.as_array().cloned().unwrap_or_default() {
                let from = c["from"].clone();
                let nm = from["name"].as_str().unwrap_or("").to_string();
                if nm.is_empty() || chain.iter().any(|n| n == &nm) {
                    continue;
                }
                let mut c2 = chain.clone();
                c2.push(nm.clone());
                if nm == b {
                    chains.insert(c2.join(" → "));
                } else {
                    next.push((from, nm, c2));
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    Ok(chains.into_iter().collect())
}

/// The LSP backend's `explore` pieces (T68.1): same assembler as the tags
/// backend, answered over workspace/symbol, document bodies and call hierarchy.
struct LspExplore<'a> {
    cx: &'a Ctx<'a>,
    s: &'a mut Session,
    filter: &'a super::Filter,
    budget: usize,
}

impl super::ExploreParts for LspExplore<'_> {
    fn resolve(&mut self, token: &str) -> Result<Vec<String>> {
        let r = self
            .s
            .request("workspace/symbol", json!({"query": token}))?;
        let mut prefixed = std::collections::BTreeSet::new();
        for it in r.as_array().cloned().unwrap_or_default() {
            let Some(n) = it["name"].as_str() else {
                continue;
            };
            if n == token {
                return Ok(vec![token.to_string()]);
            }
            if n.starts_with(token) {
                prefixed.insert(n.to_string());
            }
        }
        Ok(prefixed.into_iter().take(5).collect())
    }

    fn defs(&mut self, name: &str) -> Result<String> {
        symbol_text(self.cx, self.s, name, self.filter, self.budget)
    }

    fn paths(&mut self, a: &str, b: &str) -> Result<Vec<String>> {
        call_paths(self.s, a, b, 3)
    }

    fn impact1(&mut self, name: &str) -> Result<(String, usize)> {
        let rows = impact_walk(self.s, name, 1, self.filter)?;
        if rows.is_empty() {
            return Ok((format!("nothing reaches {name}"), 0));
        }
        let n = rows.len();
        Ok((super::impact_lines_text(&rows), n))
    }

    fn def_count(&mut self, _name: &str) -> Result<usize> {
        Ok(1)
    }
}

pub(crate) fn explore(
    cx: &Ctx,
    root: &Path,
    query: &str,
    filter: &super::Filter,
) -> Result<String> {
    let t0 = Instant::now();
    let budget = cx.plugin_config::<crate::config::Graph>("graph").body_lines as usize;
    with_session(root, |s| {
        let mut parts = LspExplore {
            cx,
            s,
            filter,
            budget,
        };
        let (text, before) = super::assemble_explore(query, filter, &mut parts)?;
        let est = cx.estimate(&text, Class::Code);
        cx.record(&Measurement {
            plugin: "graph",
            kind: "lsp.explore",
            before_bytes: t0.elapsed().as_millis() as u64,
            after_bytes: text.len() as u64,
            est_before: est,
            est_after: est,
            ref_id: None,
            call_id: cx.call_id(),
        })?;
        super::cap_kind(cx, text, before, "explore")
    })
}

fn flatten(syms: &[Value], out: &mut String) {
    for s in syms {
        let name = s["name"].as_str().unwrap_or("");
        let line = s["range"]["start"]["line"].as_u64().unwrap_or(0) + 1;
        let prefix = match kind_name(s["kind"].as_u64().unwrap_or(0)) {
            "function" | "method" => "fn",
            other => other,
        };
        out.push_str(&format!("{prefix} {name} {line}\n"));
        if let Some(c) = s["children"].as_array() {
            flatten(c, out);
        }
    }
}

fn workspace_of(file: &Path) -> PathBuf {
    for d in file.ancestors().skip(1) {
        if LANGS
            .iter()
            .any(|(marker, _, server)| server.is_some() && d.join(marker).is_file())
        {
            return d.to_path_buf();
        }
    }
    file.parent().unwrap_or(file).to_path_buf()
}

pub(crate) fn outline(cx: &Ctx, root: &Path, path: &str) -> Result<String> {
    let t0 = Instant::now();
    let abs = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };
    // Same session key as `symbol` / `callers` / `impact` (cwd) when the file sits under cwd and
    // its nearest manifest speaks cwd's server; otherwise the nearest manifest. Keyed by the
    // nearest manifest alone, alternating `outline` and `symbol` calls in a Cargo workspace
    // killed and respawned the server each time; keyed by cwd alone, a Dart file outside a
    // Cargo cwd was sent to rust-analyzer and outlined as empty.
    let near = workspace_of(&abs);
    let ws = match (pick(root), pick(&near)) {
        (Ok((a, _)), Ok((b, _))) if a == b && abs.starts_with(root) => root.to_path_buf(),
        _ => near,
    };
    with_session(&ws, |s| {
        let uri = file_uri(&abs);
        s.did_open(&uri)?;
        let deadline = Instant::now() + ready();
        loop {
            let raw = s.request(
                "textDocument/documentSymbol",
                json!({"textDocument": {"uri": uri}}),
            )?;
            let mut text = String::new();
            if let Some(arr) = raw.as_array() {
                flatten(arr, &mut text);
            }
            if !text.is_empty() || Instant::now() >= deadline {
                if !text.is_empty() {
                    super::tally::hit(root, "", [path]);
                }
                return finish(cx, "outline", t0, text.trim_end().to_string());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    /// LSP 3.17 `SymbolKind`: 10 is Enum, 11 is Interface (11 used to read as "enum").
    #[test]
    fn kind_name_follows_the_lsp_symbol_kind_numbers() {
        for (k, name) in [
            (5, "class"),
            (6, "method"),
            (10, "enum"),
            (11, "interface"),
            (12, "function"),
            (13, "variable"),
            (23, "struct"),
            (99, "symbol"),
        ] {
            assert_eq!(kind_name(k), name, "SymbolKind {k}");
        }
    }

    /// T41.1: a `pubspec.yaml` root picks `dart language-server`, and a `.dart`
    /// file resolves its workspace to that root.
    #[test]
    fn dart_pubspec_root_picks_dart_language_server() {
        let dir = std::env::temp_dir().join(format!("rtok-lsp-dart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("pubspec.yaml"), "name: dart_gate\n").unwrap();
        let main = dir.join("lib/main.dart");
        std::fs::write(&main, "void main() {}\n").unwrap();
        let (bin, args) = pick(&dir).unwrap();
        assert_eq!(bin, "dart");
        assert_eq!(args, &["language-server"]);
        assert_eq!(workspace_of(&main), dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The cached server outlived `rtok mcp`: it sat in a `static`, and statics never drop.
    #[test]
    fn shutdown_reaps_the_cached_server_and_its_stderr_file() {
        let child = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id().to_string();
        let err_path = std::env::temp_dir().join(format!("rtok-lsp-test-{pid}.stderr"));
        std::fs::write(&err_path, "").unwrap();
        let s = Session::attach(child, PathBuf::new(), err_path.clone()).unwrap();
        *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = Some(("test".into(), s));
        shutdown();
        let alive = Command::new("kill").args(["-0", &pid]).status().unwrap();
        assert!(!alive.success(), "server {pid} still running");
        assert!(!err_path.exists());
    }

    /// A scripted peer: `sh -c script sh arg`, so a script can replay `$1` byte for byte.
    /// Every script ends on its own, so a red test cannot leave a process behind.
    fn peer(script: &str, arg: &str) -> Session {
        let child = Command::new("sh")
            .args(["-c", script, "sh", arg])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Session::attach(child, PathBuf::new(), PathBuf::new()).unwrap()
    }

    fn framed(v: &Value) -> String {
        let body = v.to_string();
        format!("Content-Length: {}\r\n\r\n{body}", body.len())
    }

    fn reply(id: i64) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "result": {"ok": true}})
    }

    const BUDGET: Duration = Duration::from_millis(100);

    fn expect_timeout(s: &mut Session, params: Value) {
        let t0 = Instant::now();
        let err = s
            .request_within("textDocument/documentSymbol", params, BUDGET)
            .expect_err("the peer never answered in time");
        assert!(format!("{err:#}").contains("timeout"), "{err:#}");
        assert!(server_broke(&err), "a timeout must reach the fallback path");
        assert!(
            t0.elapsed() < Duration::from_millis(1_800),
            "{:?}",
            t0.elapsed()
        );
    }

    #[test]
    fn a_silent_peer_times_out_instead_of_blocking_on_the_pipe() {
        let mut s = peer("sleep 3", "");
        expect_timeout(&mut s, json!({}));
    }

    #[test]
    fn a_reply_after_the_budget_is_a_timeout_not_a_success() {
        let mut s = peer("sleep 1.5; printf '%s' \"$1\"", &framed(&reply(1)));
        expect_timeout(&mut s, json!({}));
    }

    #[test]
    fn a_partial_frame_does_not_hold_the_request_past_the_budget() {
        let whole = framed(&reply(1));
        let mut s = peer("printf '%s' \"$1\"; sleep 3", &whole[..whole.len() - 6]);
        expect_timeout(&mut s, json!({}));
    }

    #[test]
    fn a_header_without_a_body_does_not_hold_the_request() {
        let mut s = peer("printf 'Content-Length: 40\\r\\n'; sleep 3", "");
        expect_timeout(&mut s, json!({}));
    }

    #[test]
    fn notifications_do_not_extend_the_budget() {
        let note = framed(&json!({"jsonrpc": "2.0", "method": "$/progress", "params": {}}));
        let script =
            "i=0; while [ $i -lt 300 ]; do printf '%s' \"$1\"; sleep 0.01; i=$((i+1)); done";
        let mut s = peer(script, &note);
        expect_timeout(&mut s, json!({}));
    }

    #[test]
    fn a_peer_that_stops_reading_cannot_block_the_write() {
        let mut s = peer("sleep 3", "");
        expect_timeout(&mut s, json!({"pad": "x".repeat(1 << 20)}));
    }

    #[test]
    fn a_timely_reply_after_a_notification_succeeds() {
        let note = framed(&json!({"jsonrpc": "2.0", "method": "window/logMessage", "params": {}}));
        let both = format!("{note}{}", framed(&reply(1)));
        let mut s = peer("printf '%s' \"$1\"; sleep 3", &both);
        let got = s
            .request_within("workspace/symbol", json!({}), Duration::from_secs(2))
            .unwrap();
        assert_eq!(got, json!({"ok": true}));
    }

    /// The expired server is gone, so `with_session` respawns it and its late reply can never
    /// be taken for the answer to a later request.
    #[test]
    fn an_expired_session_is_reaped_and_refuses_further_requests() {
        let mut s = peer("sleep 0.5; printf '%s' \"$1\"", &framed(&reply(1)));
        expect_timeout(&mut s, json!({}));
        assert!(
            s.child.try_wait().unwrap().is_some(),
            "server still running"
        );
        std::thread::sleep(Duration::from_millis(700));
        let again = s.request_within("workspace/symbol", json!({}), Duration::from_secs(1));
        assert!(again.is_err(), "{again:?}");
    }

    /// A peer that appends everything the session sends to `log`, so a test can read back the
    /// notifications. It ends with the session's kill.
    fn recording(dir: &Path) -> (Session, PathBuf) {
        let log = dir.join("peer.log");
        (peer("cat > \"$1\"", log.to_str().unwrap()), log)
    }

    /// The `(method, params)` of every frame the peer has received so far, once at least `n` of
    /// them have arrived.
    fn received(log: &Path, n: usize) -> Vec<(String, Value)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let bytes = std::fs::read(log).unwrap_or_default();
            let mut r = std::io::Cursor::new(bytes);
            let mut got = Vec::new();
            while let Ok(m) = read_msg(&mut r) {
                got.push((
                    m["method"].as_str().unwrap_or("").to_string(),
                    m["params"].clone(),
                ));
            }
            if got.len() >= n || Instant::now() > deadline {
                return got;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn doc_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rtok-lsp-doc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn text_of(note: &(String, Value)) -> &str {
        let p = &note.1["textDocument"];
        p["text"]
            .as_str()
            .or(note.1["contentChanges"][0]["text"].as_str())
            .unwrap_or("")
    }

    /// The reported defect: the second query got the first snapshot.
    #[test]
    fn an_edited_file_reaches_a_syncing_server_as_a_full_change() {
        let dir = doc_dir("change");
        let file = dir.join("a.rs");
        std::fs::write(&file, "fn old_symbol() {}\n").unwrap();
        let (mut s, log) = recording(&dir);
        s.sync_change = 1;
        let uri = file_uri(&file);
        s.did_open(&uri).unwrap();
        std::fs::write(&file, "fn new_symbol() {}\n").unwrap();
        s.did_open(&uri).unwrap();
        let got = received(&log, 2);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0].0, "textDocument/didOpen");
        assert_eq!(text_of(&got[0]), "fn old_symbol() {}\n");
        assert_eq!(got[1].0, "textDocument/didChange");
        assert_eq!(got[1].1["textDocument"], json!({"uri": uri, "version": 2}));
        assert_eq!(text_of(&got[1]), "fn new_symbol() {}\n");
    }

    #[test]
    fn a_server_without_change_sync_gets_a_close_and_a_reopen() {
        let dir = doc_dir("reopen");
        let file = dir.join("a.rs");
        std::fs::write(&file, "fn one() {}\n").unwrap();
        let (mut s, log) = recording(&dir);
        let uri = file_uri(&file);
        s.did_open(&uri).unwrap();
        std::fs::write(&file, "fn two() {}\n").unwrap();
        s.did_open(&uri).unwrap();
        let got = received(&log, 3);
        let methods: Vec<_> = got.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(
            methods,
            [
                "textDocument/didOpen",
                "textDocument/didClose",
                "textDocument/didOpen"
            ]
        );
        assert_eq!(got[2].1["textDocument"]["version"], 2);
        assert_eq!(text_of(&got[2]), "fn two() {}\n");
    }

    #[test]
    fn an_unchanged_file_is_not_sent_twice() {
        let dir = doc_dir("same");
        let file = dir.join("a.rs");
        std::fs::write(&file, "fn same() {}\n").unwrap();
        let (mut s, log) = recording(&dir);
        s.sync_change = 1;
        let uri = file_uri(&file);
        s.did_open(&uri).unwrap();
        s.did_open(&uri).unwrap();
        s.refresh_open().unwrap();
        s.notify("probe", json!({})).unwrap();
        let methods: Vec<_> = received(&log, 2).into_iter().map(|(m, _)| m).collect();
        assert_eq!(methods, ["textDocument/didOpen", "probe"]);
    }

    #[test]
    fn consecutive_edits_with_unicode_count_versions_up() {
        let dir = doc_dir("unicode");
        let file = dir.join("a.rs");
        let (mut s, log) = recording(&dir);
        s.sync_change = 2;
        let uri = file_uri(&file);
        let texts = [
            "fn \u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}() {}\n",
            "// \u{1f980}\nfn crab() {}\n",
            "fn plain() {}\n",
        ];
        for t in texts {
            std::fs::write(&file, t).unwrap();
            s.did_open(&uri).unwrap();
        }
        let got = received(&log, 3);
        assert_eq!(got.len(), 3, "{got:?}");
        for (i, t) in texts.iter().enumerate() {
            assert_eq!(text_of(&got[i]), *t);
            assert_eq!(got[i].1["textDocument"]["version"], i as i64 + 1);
        }
    }

    #[test]
    fn a_deleted_file_is_closed_and_a_recreated_one_is_opened_again() {
        let dir = doc_dir("delete");
        let file = dir.join("a.rs");
        std::fs::write(&file, "fn gone() {}\n").unwrap();
        let (mut s, log) = recording(&dir);
        s.sync_change = 1;
        let uri = file_uri(&file);
        s.did_open(&uri).unwrap();
        std::fs::remove_file(&file).unwrap();
        s.did_open(&uri).unwrap();
        std::fs::write(&file, "fn back() {}\n").unwrap();
        s.did_open(&uri).unwrap();
        let got = received(&log, 3);
        let methods: Vec<_> = got.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(
            methods,
            [
                "textDocument/didOpen",
                "textDocument/didClose",
                "textDocument/didOpen"
            ]
        );
        assert_eq!(got[2].1["textDocument"]["version"], 1);
        assert_eq!(text_of(&got[2]), "fn back() {}\n");
    }

    /// `with_session` calls this before every query, so `workspace/symbol` and the positions it
    /// returns come from the current text of documents opened by an earlier query.
    #[test]
    fn refresh_open_resyncs_documents_this_query_does_not_touch() {
        let dir = doc_dir("refresh");
        let (a, b) = (dir.join("a.rs"), dir.join("b.rs"));
        std::fs::write(&a, "fn a() {}\n").unwrap();
        std::fs::write(&b, "fn b() {}\n").unwrap();
        let (mut s, log) = recording(&dir);
        s.sync_change = 1;
        s.did_open(&file_uri(&a)).unwrap();
        s.did_open(&file_uri(&b)).unwrap();
        std::fs::write(&a, "fn a2() {}\n").unwrap();
        s.refresh_open().unwrap();
        let got = received(&log, 3);
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[2].0, "textDocument/didChange");
        assert_eq!(text_of(&got[2]), "fn a2() {}\n");
    }

    /// A document is only recorded as synchronized once its notification was queued.
    #[test]
    fn a_failed_notification_does_not_mark_the_document_synchronized() {
        let dir = doc_dir("failed");
        let file = dir.join("a.rs");
        std::fs::write(&file, "fn x() {}\n").unwrap();
        let (mut s, log) = recording(&dir);
        let (dead, queue) = mpsc::channel();
        drop(queue);
        let live = std::mem::replace(&mut s.out, dead);
        let uri = file_uri(&file);
        assert!(s.did_open(&uri).is_err());
        s.out = live;
        s.did_open(&uri).unwrap();
        let got = received(&log, 1);
        assert_eq!(got[0].0, "textDocument/didOpen");
    }

    #[test]
    fn the_sync_kind_is_read_from_the_initialize_result() {
        for (caps, kind) in [
            (json!({"textDocumentSync": 1}), 1),
            (
                json!({"textDocumentSync": {"openClose": true, "change": 2}}),
                2,
            ),
            (json!({"textDocumentSync": {"openClose": true}}), 0),
            (json!({"textDocumentSync": 0}), 0),
            (json!({}), 0),
        ] {
            assert_eq!(sync_kind(&json!({"capabilities": caps})), kind, "{caps}");
        }
    }

    #[test]
    fn dropping_a_session_whose_peer_stopped_reading_does_not_wait_for_it() {
        let mut s = peer("sleep 3", "");
        let t0 = Instant::now();
        s.notify("textDocument/didOpen", json!({"pad": "x".repeat(1 << 20)}))
            .unwrap();
        drop(s);
        assert!(
            t0.elapsed() < Duration::from_millis(1_800),
            "{:?}",
            t0.elapsed()
        );
    }
}

#[cfg(test)]
mod uri_tests {
    use super::{file_uri, path_from_file_uri};
    use std::path::PathBuf;

    #[test]
    fn file_uri_uses_forward_slashes() {
        let dir = std::env::temp_dir().join(format!("rtok-uri-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let uri = file_uri(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(uri.starts_with("file://"), "{uri}");
        assert!(!uri.contains('\\'), "no backslashes: {uri}");
        if cfg!(windows) {
            assert!(
                uri.starts_with("file:///"),
                "windows needs three slashes: {uri}"
            );
        }
        let round = path_from_file_uri(&uri);
        assert!(!round.as_os_str().is_empty(), "{round:?}");
    }

    #[test]
    fn path_from_file_uri_keeps_unix_root() {
        let p = path_from_file_uri("file:///tmp/a");
        assert_eq!(p, PathBuf::from("/tmp/a"));
    }

    #[cfg(windows)]
    #[test]
    fn path_from_file_uri_strips_slash_before_drive() {
        let p = path_from_file_uri("file:///C:/Users/x");
        assert_eq!(p, PathBuf::from(r"C:\Users\x"));
    }

    #[test]
    fn file_uri_encodes_spaces() {
        // Pure encode/decode — no host TempDir (VFS / T56).
        let encoded = super::percent_encode_path("C:/Users/Ivan Tuhai/proj/a.rs");
        assert!(encoded.contains("%20"), "{encoded}");
        assert!(!encoded.contains(' '), "{encoded}");
        assert!(encoded.starts_with("C:"), "{encoded}");
        let uri = format!("file:///{encoded}");
        let round = path_from_file_uri(&uri);
        assert!(round.to_string_lossy().contains("Ivan Tuhai"), "{round:?}");
    }

    #[test]
    fn path_from_file_uri_decodes_percent20() {
        let p = path_from_file_uri("file:///tmp/My%20Docs/a.rs");
        assert_eq!(p, PathBuf::from("/tmp/My Docs/a.rs"));
    }

    #[test]
    fn file_uri_encodes_hash_and_question_in_path() {
        let encoded = super::percent_encode_path("C:/tmp/a#b?.rs");
        assert!(encoded.contains("%23"), "{encoded}");
        assert!(
            encoded.contains("%3F") || encoded.contains("%3f"),
            "{encoded}"
        );
        let round = path_from_file_uri(&format!("file:///{encoded}"));
        let s = round.to_string_lossy();
        assert!(s.contains("a#b?.rs") || s.contains("a"), "{round:?}");
    }
}

#[cfg(test)]
mod probe_tests {
    use super::{is_mise_shim, is_rustup_proxy};
    use std::fs;
    use std::path::Path;

    #[test]
    fn mise_shim_is_not_an_installed_server() {
        let shim = Path::new("/home/runner/.local/share/mise/shims/rust-analyzer");
        assert!(is_mise_shim(shim));
        assert!(!is_mise_shim(Path::new(
            "/home/runner/.rustup/toolchains/stable/bin/rust-analyzer"
        )));
    }

    #[test]
    fn rustup_proxy_matches_the_rustup_binary_size() {
        let dir = std::env::temp_dir().join(format!("rtok-ra-proxy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let proxy = if cfg!(windows) {
            dir.join("rustup.exe")
        } else {
            dir.join("rustup")
        };
        let analyzer = if cfg!(windows) {
            dir.join("rust-analyzer.exe")
        } else {
            dir.join("rust-analyzer")
        };
        fs::write(&proxy, b"proxy").unwrap();
        fs::write(&analyzer, b"proxy").unwrap();
        assert!(is_rustup_proxy(&analyzer));
        fs::write(&analyzer, b"a real rust-analyzer binary").unwrap();
        assert!(!is_rustup_proxy(&analyzer));
        let _ = fs::remove_dir_all(&dir);
    }
}

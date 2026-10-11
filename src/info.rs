// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok info` (plan T43): version, effective paths and disk usage in one place.
//!
//! `doctor` reports the host chain and `config show` reports every key, but neither
//! answers "what does rtok use and how much disk does it take". Everything here is
//! fail-open: a missing file or an unreadable store renders as `-`, never an error.

use crate::config::{CATALOGUE, Config};
use crate::store::Store;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One file on disk: its path and bytes, or `None` when it is missing.
#[derive(Debug, Clone, Serialize)]
pub struct FileInfo {
    pub path: String,
    pub bytes: Option<u64>,
}

/// A directory on disk: its path, file count and summed bytes.
#[derive(Debug, Clone, Serialize)]
pub struct DirInfo {
    pub path: String,
    pub files: u64,
    pub bytes: u64,
}

/// The regular log (`[log]` settings, T43): its bytes plus the line and
/// error-line counts. Errors are lines whose level is `error` — rtok keeps no
/// separate `errors.log`; the same settings cover both.
#[derive(Debug, Clone, Serialize)]
pub struct LogInfo {
    pub path: String,
    pub bytes: u64,
    pub lines: u64,
    pub errors: u64,
}

/// The proxy server: its configured shape plus whether it answers.
#[derive(Debug, Clone, Serialize)]
pub struct ProxyInfo {
    pub enabled: bool,
    pub mode: String,
    pub bind: String,
    pub port: u16,
    pub upstream: String,
    /// `disabled` when `[proxy] enabled` is false, else `up`/`down` from one
    /// `/health` probe (same client `doctor` uses).
    pub status: String,
}

/// OpenTelemetry export: off until an endpoint resolves (default), else its URL.
/// Headers are never printed — they carry ingestion keys.
#[derive(Debug, Clone, Serialize)]
pub struct OtelInfo {
    pub enabled: bool,
    pub endpoint: String,
}

/// Row counts from the store; `None` when it will not open.
#[derive(Debug, Clone, Serialize)]
pub struct StoreInfo {
    pub calls: u64,
    pub measurements: u64,
    pub usage: u64,
    pub sessions: u64,
}

/// Everything `rtok info` prints, as data. Byte counts stay numbers here;
/// [`Info::to_text`] renders them human-readable.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub version: String,
    pub binary: FileInfo,
    pub home: String,
    pub config: FileInfo,
    pub db: FileInfo,
    pub db_wal: FileInfo,
    pub db_shm: FileInfo,
    pub archive: DirInfo,
    pub log: LogInfo,
    pub proxy: ProxyInfo,
    pub otel: OtelInfo,
    pub store: Option<StoreInfo>,
    /// Sum of the known files above (config, db + wal + shm, archive, log).
    pub total_bytes: u64,
}

use change_preview::human_bytes;

fn file_of(path: &Path) -> FileInfo {
    FileInfo {
        path: path.display().to_string(),
        bytes: std::fs::metadata(path).ok().map(|m| m.len()),
    }
}

/// `<db path>` with a WAL-style suffix (`-wal`, `-shm`): SQLite's own names.
fn sibling(path: &Path, suffix: &str) -> FileInfo {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    file_of(&PathBuf::from(s))
}

/// Recursive file count and bytes; symlinks are not followed, unreadable
/// entries are skipped — a diagnostic must not fail on permissions.
fn dir_usage(path: &Path) -> (u64, u64) {
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                files += 1;
                bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (files, bytes)
}

/// Lines and error-lines in `lines`: the level is the third whitespace token
/// (`2026-09-09 15:04:05 error t/n: …`), the shape [`crate::log::line`] writes.
fn count_errors(lines: &[String]) -> (u64, u64) {
    let mut errors = 0u64;
    for l in lines {
        if l.split_whitespace().nth(2) == Some("error") {
            errors += 1;
        }
    }
    (lines.len() as u64, errors)
}

fn log_info(cfg: &Config) -> LogInfo {
    // Live file plus rotated siblings, matched by name so no format is
    // duplicated with `log::nth`: `rtok.log`, `rtok.log.1`, …
    let name = cfg
        .log
        .path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned());
    let mut bytes = file_of(&cfg.log.path).bytes.unwrap_or(0);
    if let (Some(dir), Some(name)) = (cfg.log.path.parent(), name)
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n != name && !n.starts_with(&format!("{name}.")) {
                continue;
            }
            if e.path() == cfg.log.path {
                continue;
            }
            bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    // One bounded read: rotation caps the total at `max_bytes × (files + 1)`.
    let (lines, errors) = count_errors(&crate::log::tail(cfg, Some(usize::MAX)));
    LogInfo {
        path: cfg.log.path.display().to_string(),
        bytes,
        lines,
        errors,
    }
}

fn proxy_info(cfg: &Config) -> ProxyInfo {
    let status = if !cfg.proxy.enabled {
        "disabled".to_string()
    } else {
        // `0.0.0.0` never answers a connect; the loopback does on its behalf.
        let host = if cfg.proxy.bind == "0.0.0.0" {
            "127.0.0.1"
        } else {
            cfg.proxy.bind.as_str()
        };
        let timeout = Duration::from_millis(cfg.doctor.probe_timeout_ms.max(100));
        let up = crate::doctor::http_get(
            &format!("http://{host}:{}", cfg.proxy.port),
            "/health",
            timeout,
        )
        .is_some();
        if up { "up" } else { "down" }.to_string()
    };
    ProxyInfo {
        enabled: cfg.proxy.enabled,
        mode: cfg.proxy.mode.clone(),
        bind: cfg.proxy.bind.clone(),
        port: cfg.proxy.port,
        upstream: cfg.proxy.upstream.clone(),
        status,
    }
}

fn store_info(cfg: &Config) -> Option<StoreInfo> {
    let store = Store::open(&cfg.core.db_path).ok()?;
    let calls = store.calls_after(0, i64::MAX).ok()?.len() as u64;
    let mut measurements = 0u64;
    for (id, _) in CATALOGUE {
        measurements += store.list_measurements(id).ok()?.len() as u64;
    }
    let mut usage = 0u64;
    let mut sessions = 0u64;
    for s in store.usage_sessions().ok()? {
        sessions += 1;
        usage += store.usage_rows(&s).ok()?.len() as u64;
    }
    sessions = sessions.max(store.session_totals(0).ok()?.len() as u64);
    Some(StoreInfo {
        calls,
        measurements,
        usage,
        sessions,
    })
}

/// Collect every row. The DB stat is taken before the store is opened for its
/// counts — opening creates the file, and a missing DB must still print `-`.
pub fn collect(cfg: &Config, config_file: Option<&Path>) -> Info {
    let binary = std::env::current_exe().ok();
    let binary = FileInfo {
        path: binary
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string()),
        bytes: binary
            .as_deref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len()),
    };
    let config = file_of(&Config::user_path(&cfg.home, config_file));
    let db = file_of(&cfg.core.db_path);
    let db_wal = sibling(&cfg.core.db_path, "-wal");
    let db_shm = sibling(&cfg.core.db_path, "-shm");
    let (archive_files, archive_bytes) = dir_usage(&cfg.core.archive_dir);
    let archive = DirInfo {
        path: cfg.core.archive_dir.display().to_string(),
        files: archive_files,
        bytes: archive_bytes,
    };
    let log = log_info(cfg);
    let proxy = proxy_info(cfg);
    let endpoint = cfg.otel.resolve().map(|e| e.url).unwrap_or_default();
    let otel = OtelInfo {
        enabled: !endpoint.is_empty(),
        endpoint,
    };
    let store = store_info(cfg);
    let total_bytes = config.bytes.unwrap_or(0)
        + db.bytes.unwrap_or(0)
        + db_wal.bytes.unwrap_or(0)
        + db_shm.bytes.unwrap_or(0)
        + archive.bytes
        + log.bytes;
    Info {
        version: crate::VERSION.to_string(),
        binary,
        home: cfg.home.display().to_string(),
        config,
        db,
        db_wal,
        db_shm,
        archive,
        log,
        proxy,
        otel,
        store,
        total_bytes,
    }
}

fn show(file: &FileInfo) -> String {
    match file.bytes {
        Some(n) => format!("{} ({})", file.path, human_bytes(n)),
        None => format!("{} (-)", file.path),
    }
}

impl Info {
    /// The `rtok info` text: one `key value` line per row, `-` for missing.
    pub fn to_text(&self) -> String {
        let mut out = format!("rtok {}\n", self.version);
        out.push_str(&format!("binary {}\n", show(&self.binary)));
        out.push_str(&format!("home {}\n", self.home));
        out.push_str(&format!("config {}\n", show(&self.config)));
        let wal = self
            .db_wal
            .bytes
            .map(human_bytes)
            .unwrap_or_else(|| "-".into());
        let shm = self
            .db_shm
            .bytes
            .map(human_bytes)
            .unwrap_or_else(|| "-".into());
        out.push_str(&format!("db {} (wal {wal}; shm {shm})\n", show(&self.db)));
        out.push_str(&format!(
            "archive {} ({} files, {})\n",
            self.archive.path,
            self.archive.files,
            human_bytes(self.archive.bytes)
        ));
        out.push_str(&format!(
            "log {} ({}, {} lines, {} errors)\n",
            self.log.path,
            human_bytes(self.log.bytes),
            self.log.lines,
            self.log.errors
        ));
        out.push_str(&format!(
            "proxy {} {}:{} ({}) upstream {}\n",
            self.proxy.mode,
            self.proxy.bind,
            self.proxy.port,
            self.proxy.status,
            self.proxy.upstream
        ));
        if self.otel.enabled {
            out.push_str(&format!("otel {}\n", self.otel.endpoint));
        } else {
            out.push_str("otel off\n");
        }
        match &self.store {
            Some(s) => out.push_str(&format!(
                "store calls {}, measurements {}, usage {}, sessions {}\n",
                s.calls, s.measurements, s.usage, s.sessions
            )),
            None => out.push_str("store - (unreadable)\n"),
        }
        out.push_str(&format!("disk {}\n", human_bytes(self.total_bytes)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_count_reads_the_level_not_the_text() {
        let lines = vec![
            "2026-09-09 15:04:05 error t/n: boom".to_string(),
            "2026-09-09 15:04:06 info t/n: mentions error loudly".to_string(),
            "2026-09-09 15:04:07 warn t/n: an error occurred".to_string(),
            "garbage".to_string(),
        ];
        assert_eq!(count_errors(&lines), (4, 1));
    }

    #[test]
    fn missing_files_render_dash_and_json_keeps_nulls() {
        let dir = std::env::temp_dir().join(format!("rtok-info-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = Config::load_from(&dir).unwrap();
        // Fresh home: config was just written, the DB was never opened before
        // `collect` stats it — so it still prints `-`.
        let info = collect(&cfg, None);
        assert!(!info.version.is_empty());
        assert_eq!(info.home, dir.display().to_string());
        assert!(info.config.bytes.is_some());
        assert_eq!(info.db.bytes, None);
        assert!(!info.otel.enabled);
        let text = info.to_text();
        assert!(text.contains("rtok ") && text.contains("errors"));
        let v = serde_json::to_value(&info).unwrap();
        assert!(v["db"]["bytes"].is_null());
        assert_eq!(v["proxy"]["port"], cfg.proxy.port);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn error_lines_in_the_log_are_counted() {
        let dir = std::env::temp_dir().join(format!("rtok-info-err-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = Config::load_from(&dir).unwrap();
        std::fs::create_dir_all(cfg.log.path.parent().unwrap()).unwrap();
        std::fs::write(
            &cfg.log.path,
            "2026-09-09 15:04:05 error t/n: boom\n2026-09-09 15:04:06 info t/n: ok\n",
        )
        .unwrap();
        let info = collect(&cfg, None);
        assert_eq!((info.log.lines, info.log.errors), (2, 1));
        assert!(info.to_text().contains("1 errors"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

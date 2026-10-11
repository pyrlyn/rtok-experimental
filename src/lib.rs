// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! rtok — token reduction for AI coding agents, one plugin per method.
//!
//! Library layout (see `architecture.md`):
//! - [`cli`]     — clap subcommand tree (`rtok config`, `rtok hook`, …)
//! - [`config`]  — `~/.rtok/config.toml`, `RTOK_HOME`
//! - [`store`]   — one SQLite file: events, measurements, archive, notes (FTS5), usage
//! - [`tokens`]  — chars-per-token estimator (uncalibrated heuristic, T221)
//! - [`plugin`]  — the `Plugin` trait, `Manifest`, `Runtime`, `Measurement`
//! - [`plugins`] — the registry and one module per catalogue plugin
//! - [`hooks`]   — Claude Code hook I/O types
//! - [`doctor`]  — `rtok doctor`
//! - [`measure`] — JSONL ingest, `rtok stats` (P1)
//! - [`modes`]   — terse compress + YAGNI ladder helpers (D6/D7)
//! - [`tui`]     — `rtok tui`, the terminal rendering of the operator model (D23)
//! - [`worktree`] — git worktree inventory: records, owners, states (T150)

#![forbid(unsafe_code)]

pub mod agent_view;
pub mod agents;
pub mod batch;
pub mod bench;
pub mod bytes;
pub mod cli;
pub mod completions;
pub mod config;
pub mod demon;
pub mod disk_usage;
pub mod doctor;
pub mod expand;
pub mod fs;
/// `cargo fuzz` entry points (`fuzz/`); absent from every normal build.
#[cfg(fuzzing)]
#[doc(hidden)]
pub mod fuzzing;
pub mod hooks;
pub mod info;
pub mod lane;
pub mod log;
pub mod logfile;
pub mod man;
pub mod mcp;
pub mod measure;
pub mod memory_status;
pub mod model;
pub mod modes;
pub mod names;
pub mod otel;
pub mod plugin;
pub mod plugins;
pub mod proc;
pub mod progress;
pub mod project;
pub mod proxy;
pub mod removal_preview;
pub mod render;
pub mod report;
pub mod sanitize;
pub mod shell_agent;
pub mod since;
pub mod skill_path;
pub mod stat_preview;
pub mod stdio;
pub mod store;
pub mod task_id;
pub mod tasks;
/// Test helpers, also for the integration tests under `tests/`; not part of the public API.
#[doc(hidden)]
pub mod testutil;
pub mod tls;
pub mod tokens;
pub mod tui;
pub mod ui;
pub mod web;
pub mod worktree;

/// `0.1.0 (1a2b3c4d5)` — the sha comes from `build.rs` (T10.4).
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("RTOK_GIT_SHA"), ")");

/// The plugin contract at the crate root, so an external plugin crate writes
/// `use rtok::{Runtime, Manifest, Plugin, Surface};`.
pub use plugin::*;

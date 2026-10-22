// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Config layering + precedence (plan T12.2, decision D14).
//!
//! Six providers, lowest to highest, each named for [`entries`]'s provenance column:
//! `default` (`Config::default()`) < `user` (`~/.rtok/config.toml` or `--config`/`RTOK_CONFIG`)
//! < `project` (restricted `<git root>/.rtok.toml`) < `dotenv` (`RTOK_*` lines in `.env` files, T12.5)
//! < `env` (`RTOK_*`) < `flag` (CLI `Option<T>` fields).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::Result;
use figment::providers::{Env, Format, Serialized, Toml};
use figment::value::{Dict, Map as FMap, Num, Value};
use figment::{Figment, Metadata, Profile, Provider};

use super::Config;

static ON_LOAD: Mutex<Option<fn(&Config)>> = Mutex::new(None);
static ON_EARLY: Mutex<Option<fn(&str)>> = Mutex::new(None);

/// Run after every successful [`load`] and after [`Config::load_lenient`] falls back to
/// defaults. The CLI registers UI styling and note printing here so this crate does neither.
pub fn on_load(hook: fn(&Config)) {
    *ON_LOAD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
}

/// A malformed `.env` discovered before the file fails to parse. The surface prints it;
/// a successful load also records the same text on [`Config::notes`].
pub fn on_early_warning(hook: fn(&str)) {
    *ON_EARLY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
}

pub(crate) fn notify(cfg: &Config) {
    if let Some(hook) = *ON_LOAD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        hook(cfg);
    }
}

fn early_warning(msg: &str) {
    if let Some(hook) = *ON_EARLY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        hook(msg);
    }
}

/// `rtok hook <event> [--host]`: a figment `flag` layer that sets `hook.host`.
pub fn hook_host_flag(host: Option<String>) -> Option<Dict> {
    let host = host?;
    let mut hook = Dict::new();
    hook.insert("host".into(), Value::from(host));
    let mut flags = Dict::new();
    flags.insert("hook".into(), Value::from(hook));
    Some(flags)
}

/// Wraps a provider so it reports under a different [`Metadata::name`] — used to rename the
/// two `Toml` file providers to `user` and `project` for [`entries`]'s provenance column.
struct Named<P>(P, &'static str);

impl<P: Provider> Provider for Named<P> {
    fn metadata(&self) -> Metadata {
        let mut m = self.0.metadata();
        m.name = self.1.into();
        m
    }

    fn data(&self) -> Result<FMap<Profile, Dict>, figment::Error> {
        self.0.data()
    }

    fn profile(&self) -> Option<Profile> {
        self.0.profile()
    }
}

/// The `flag` layer: a `Dict` built by `main.rs` from only the `Some` clap `Option<T>` fields.
/// `figment::value::Dict` doesn't implement `Serialize` (only the reverse, `Value::serialize`),
/// so `Serialized::defaults` can't take it directly — this is a minimal `Provider` instead.
struct FlagsProvider(Dict);

struct Layer(FMap<Profile, Dict>, Metadata);

impl Provider for Layer {
    fn metadata(&self) -> Metadata {
        self.1.clone()
    }

    fn data(&self) -> Result<FMap<Profile, Dict>, figment::Error> {
        Ok(self.0.clone())
    }
}

// These sources are checkout-controlled, including legacy aliases. Keep authority over
// credentials, execution, host integration and filesystem access in user-owned layers.
fn project_restricted(key: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "core",
        "otel",
        "proxy",
        "mcp",
        "web",
        "dashboard",
        "log",
        "demon",
        "agents",
        "setup",
        "doctor",
        "stats",
        "report",
        "bench",
        "plugins.wasm",
        "plugins.memory",
        "plugins.compress",
        "plugins.graph",
        "plugins.docs",
        "plugins.cmd.shell",
        "plugins.cmd.rules",
        "plugins.cmd.rules_dir",
        "plugins.read.allow_paths",
        "plugins.inject.modes_dir",
        "plugins.guard",
        "filter",
        "worktree",
        "tasks.gitlab.url",
    ];
    PREFIXES.iter().any(|prefix| {
        key == *prefix
            || key
                .strip_prefix(prefix)
                .is_some_and(|tail| tail.starts_with('.'))
    })
}

fn ignored_note(key: &str, path: &Path) -> String {
    format!(
        "{}: ignored project setting {key}; set it in user config, process environment or explicit --config",
        path.display()
    )
}

fn filter_project(dict: &mut Dict, prefix: &str, rejected: &mut Vec<String>) {
    dict.retain(|name, value| {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if let Value::Dict(_, nested) = value {
            filter_project(nested, &key, rejected);
            return !nested.is_empty();
        }
        if !project_restricted(&key) {
            return true;
        }
        rejected.push(key);
        false
    });
}

fn mark_ignored(mut fig: Figment, keys: &[String]) -> Figment {
    let mut fallback = Dict::new();
    for key in keys {
        // Re-serialize to clear the inherited metadata tag; the replacement source must
        // say that the project's attempt was ignored, while showing only the safe value.
        if let Ok(value) = fig.find_value(key).and_then(Value::serialize) {
            insert_dotted(&mut fallback, key, value);
        }
    }
    if !fallback.is_empty() {
        fig = fig.merge(Layer(
            Profile::Default.collect(fallback),
            Metadata::named("ignored (project)"),
        ));
    }
    fig
}

impl Provider for FlagsProvider {
    fn metadata(&self) -> Metadata {
        Metadata::named("flag")
    }

    fn data(&self) -> Result<FMap<Profile, Dict>, figment::Error> {
        Ok(Profile::Default.collect(self.0.clone()))
    }
}

/// Between `project` and `dotenv`: fold legacy file keys into their replacements only
/// while each target is still at its default after user+project merge (T36.8).
struct LegacyFold {
    base: Figment,
}

impl Provider for LegacyFold {
    fn metadata(&self) -> Metadata {
        Metadata::named("legacy")
    }

    fn data(&self) -> Result<FMap<Profile, Dict>, figment::Error> {
        // Hook path (≤ 10 ms): no file sets a legacy key → nothing to fold, skip the extract
        // and the two serialisations below.
        let legacy_set = LEGACY_KEYS.iter().any(|k| {
            self.base
                .find_value(k)
                .is_ok_and(|v| !matches!(v, Value::Empty(..)))
        });
        if !legacy_set {
            return Ok(Profile::Default.collect(Dict::new()));
        }
        let mut cfg: Config = self.base.extract()?;
        let before = cfg.clone();
        super::apply_legacy_fold(&mut cfg);
        let before_root = Value::serialize(before)?
            .into_dict()
            .expect("Config serializes");
        let after_root = Value::serialize(cfg)?
            .into_dict()
            .expect("Config serializes");
        let mut out = Dict::new();
        for (dotted, _) in env_leaf_table().values() {
            let before_v = leaf_value(&before_root, dotted);
            let after_v = leaf_value(&after_root, dotted);
            if before_v != after_v
                && let Some(v) = after_v
            {
                insert_dotted(&mut out, dotted, v);
            }
        }
        Ok(Profile::Default.collect(out))
    }
}

/// The file keys [`super::apply_legacy_fold`] reads; `None` (their default) serialises as
/// [`Value::Empty`].
const LEGACY_KEYS: &[&str] = &[
    "core.inject_budget_tokens",
    "core.log_file",
    "core.log_level",
    "core.log_to_db",
    "dashboard",
];

fn leaf_value(root: &Dict, dotted: &str) -> Option<Value> {
    let parts = dotted.split('.').collect::<Vec<_>>();
    let mut current = root;
    for (i, part) in parts.iter().enumerate() {
        let v = current.get(*part)?;
        if i == parts.len() - 1 {
            return Some(v.clone());
        }
        if let Value::Dict(_, d) = v {
            current = d;
        } else {
            return None;
        }
    }
    None
}

/// The nearest directory at or above `start` whose `name` entry satisfies `is_match`.
fn find_up(start: &Path, name: &str, is_match: fn(&Path) -> bool) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if is_match(&dir.join(name)) {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// `RTOK_*` pairs (prefix stripped, uppercased) from the nearest `.env` at or above `cwd`,
/// then `<home>/.env`; the first allowed definition wins. Only the home file may set
/// sensitive options. Parse only: nothing is put into
/// the process environment, so `rtok run` children never inherit a project's `.env`, and
/// non-`RTOK_` keys are ignored. A malformed file is one stderr line, not an error (fail open).
struct DotenvFile {
    pairs: Vec<(String, String)>,
    /// Malformed files and rejected project settings. Missing files are silent.
    warnings: Vec<String>,
    rejected: Vec<String>,
}

fn read_dotenv(home: &Path, cwd: Option<&Path>) -> DotenvFile {
    let mut files: Vec<PathBuf> = cwd
        .and_then(|c| find_up(c, ".env", Path::is_file))
        .map(|d| d.join(".env"))
        .into_iter()
        .collect();
    files.push(home.join(".env"));
    let mut out = DotenvFile {
        pairs: Vec::new(),
        warnings: Vec::new(),
        rejected: Vec::new(),
    };
    for path in files {
        let iter = match dotenvy::from_path_iter(&path) {
            Ok(it) => it,
            Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                let msg = format!("{}: {e}", path.display());
                out.warnings.push(msg);
                continue;
            }
        };
        for item in iter {
            match item {
                Ok((k, v)) => {
                    let Some(suffix) = k.strip_prefix("RTOK_") else {
                        continue;
                    };
                    let suffix = suffix.to_uppercase();
                    // Resolve aliases through the same table as process environment keys.
                    if path != home.join(".env")
                        && let Some((key, _)) = RtokEnv::from_pairs_internal().resolve(&suffix, &v)
                        && project_restricted(&key)
                    {
                        out.warnings.push(ignored_note(&key, &path));
                        out.rejected.push(key);
                        continue;
                    }
                    if !out.pairs.iter().any(|(s, _)| *s == suffix) {
                        out.pairs.push((suffix, v));
                    }
                }
                Err(e) => {
                    let msg = format!("{}: {e}", path.display());
                    out.warnings.push(msg);
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
fn dotenv_pairs(home: &Path, cwd: Option<&Path>) -> Vec<(String, String)> {
    read_dotenv(home, cwd).pairs
}

/// `RTOK_<SECTION>_<KEY...>` → `section.key...`, plus whether the default value is an array
/// (so the env provider knows to split on `,`). Built once from `Config::default()` so a
/// dotted key's canonical env name is always `key.to_uppercase().replace('.', "_")` — no
/// guessing which underscores in `openai_upstream` or `budget_tokens` are separators.
fn env_leaf_table() -> &'static BTreeMap<String, (String, bool)> {
    // Three providers per load asked for it; serialising `Config::default()` once is enough.
    static LEAVES: OnceLock<BTreeMap<String, (String, bool)>> = OnceLock::new();
    LEAVES.get_or_init(|| {
        let mut table = BTreeMap::new();
        let root = Value::serialize(Config::default()).expect("Config serializes");
        if let Some(dict) = root.into_dict() {
            walk(&dict, "", &mut table);
        }
        table
    })
}

fn walk(dict: &Dict, prefix: &str, out: &mut BTreeMap<String, (String, bool)>) {
    for (k, v) in dict {
        let dotted = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            Value::Dict(_, d) => walk(d, &dotted, out),
            other => {
                let is_array = matches!(other, Value::Array(..));
                let canonical = dotted.to_uppercase().replace('.', "_");
                out.insert(canonical, (dotted, is_array));
            }
        }
    }
}

/// Legacy short env-var aliases kept for backward compatibility (predate the dotted scheme).
const LEGACY_ALIASES: &[(&str, &str)] = &[
    ("UPSTREAM", "proxy.upstream"),
    ("OPENAI_UPSTREAM", "proxy.openai_upstream"),
];

/// The `env` layer. `figment::providers::Env` alone can't be used as-is: it splits every `_`
/// into a nesting boundary, which mangles keys like `proxy.openai_upstream`. Instead this
/// looks up each `RTOK_*` var (and the legacy aliases) in [`env_leaf_table`], drops anything
/// not found (so `RTOK_HOME`/`RTOK_CONFIG` are ignored rather than becoming unknown keys under
/// `deny_unknown_fields`), and splits comma-separated values into an array for keys whose
/// default is an array.
struct RtokEnv {
    /// Provenance name: `env` (process) or `dotenv` (`.env` files) — same resolution, two layers.
    name: &'static str,
    table: &'static BTreeMap<String, (String, bool)>,
    /// `RTOK_`-stripped, uppercased names. Empty in tests so ambient env cannot leak.
    vars: Vec<(String, String)>,
    /// `.env` files that could not be parsed. Empty for the process layer.
    warnings: Vec<String>,
    rejected: Vec<String>,
}

impl RtokEnv {
    fn from_process() -> Self {
        Self {
            name: "env",
            table: env_leaf_table(),
            vars: Env::prefixed("RTOK_")
                .iter()
                .map(|(k, v)| (k.as_str().to_uppercase(), v))
                .collect(),
            warnings: Vec::new(),
            rejected: Vec::new(),
        }
    }

    fn from_dotenv(home: &Path, cwd: Option<&Path>) -> Self {
        let file = read_dotenv(home, cwd);
        Self {
            name: "dotenv",
            table: env_leaf_table(),
            vars: file.pairs,
            warnings: file.warnings,
            rejected: file.rejected,
        }
    }

    #[cfg(test)]
    fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self {
            name: "env",
            table: env_leaf_table(),
            vars: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            warnings: Vec::new(),
            rejected: Vec::new(),
        }
    }

    fn from_pairs_internal() -> Self {
        Self {
            name: "dotenv",
            table: env_leaf_table(),
            vars: Vec::new(),
            warnings: Vec::new(),
            rejected: Vec::new(),
        }
    }

    #[cfg(test)]
    fn from_dotenv_pairs(pairs: &[(&str, &str)]) -> Self {
        Self {
            name: "dotenv",
            ..Self::from_pairs(pairs)
        }
    }

    /// `RTOK_`-stripped, uppercased var name → `(dotted key, value)`, or `None` if unknown.
    fn resolve(&self, suffix: &str, raw: &str) -> Option<(String, Value)> {
        let (dotted, is_array) = LEGACY_ALIASES
            .iter()
            .find(|(alias, _)| *alias == suffix)
            .map(|(_, dotted)| (dotted.to_string(), false))
            .or_else(|| {
                self.table
                    .get(suffix)
                    .map(|(dotted, arr)| (dotted.clone(), *arr))
            })?;

        let value = if is_array {
            Value::from(
                // `VAR=` is an empty list, not `[""]`: an empty path root matches every path.
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(Value::from)
                    .collect::<Vec<_>>(),
            )
        } else {
            raw.parse().expect("Value::from_str is infallible")
        };
        Some((dotted, value))
    }
}

/// Insert `value` at the dotted path `key` into `dict`, creating intermediate dicts and
/// descending into ones that already exist (so `RTOK_PROXY_PORT` and
/// `RTOK_PROXY_OPENAI_UPSTREAM` both land under one `proxy` dict rather than overwriting).
fn insert_dotted(dict: &mut Dict, key: &str, value: Value) {
    match key.split_once('.') {
        Some((head, rest)) => {
            let entry = dict
                .entry(head.to_string())
                .or_insert_with(|| Value::from(Dict::new()));
            if let Value::Dict(_, d) = entry {
                insert_dotted(d, rest, value);
            } else {
                let mut d = Dict::new();
                insert_dotted(&mut d, rest, value);
                *entry = Value::from(d);
            }
        }
        None => {
            dict.insert(key.to_string(), value);
        }
    }
}

impl Provider for RtokEnv {
    fn metadata(&self) -> Metadata {
        Metadata::named(self.name)
    }

    fn data(&self) -> Result<FMap<Profile, Dict>, figment::Error> {
        let mut dict = Dict::new();
        for (suffix, raw) in &self.vars {
            if let Some((dotted, value)) = self.resolve(suffix, raw) {
                insert_dotted(&mut dict, &dotted, value);
            }
        }
        Ok(Profile::Default.collect(dict))
    }
}

/// Build the layered [`Figment`] (does not extract/validate — see [`load`]). `config_file`
/// picks the user file (else `RTOK_CONFIG` env or `<home>/config.toml`); `flags` is the `flag`
/// layer, built by the caller from only the `Some` CLI options.
pub fn figment(home: &Path, config_file: Option<&Path>, flags: Option<Dict>) -> Figment {
    let cwd = std::env::current_dir().ok();
    assemble(
        home,
        config_file,
        flags,
        RtokEnv::from_process(),
        cwd.as_deref(),
        RtokEnv::from_dotenv(home, cwd.as_deref()),
    )
}

fn assemble(
    home: &Path,
    config_file: Option<&Path>,
    flags: Option<Dict>,
    env: RtokEnv,
    cwd: Option<&Path>,
    dotenv: RtokEnv,
) -> Figment {
    assemble_noted(home, config_file, flags, env, cwd, dotenv, &mut Vec::new())
}

fn assemble_noted(
    home: &Path,
    config_file: Option<&Path>,
    flags: Option<Dict>,
    env: RtokEnv,
    cwd: Option<&Path>,
    dotenv: RtokEnv,
    warnings: &mut Vec<String>,
) -> Figment {
    let user_path = config_file
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("RTOK_CONFIG").map(PathBuf::from))
        .unwrap_or_else(|| Config::path_for(home));

    let mut fig = Figment::from(Named(Serialized::defaults(Config::default()), "default"))
        .merge(Named(Toml::file(&user_path), "user"));
    let mut project_rejected = Vec::new();

    // `checkout`, not `main`: a worktree is a different branch, so it reads its own checked-out file.
    if let Some(root) = cwd.and_then(crate::root::ProjectRoot::find) {
        let path = root.checkout.join(".rtok.toml");
        let provider = Named(Toml::file(&path), "project");
        match provider.data() {
            Ok(mut data) => {
                for dict in data.values_mut() {
                    filter_project(dict, "", &mut project_rejected);
                }
                warnings.extend(project_rejected.iter().map(|key| ignored_note(key, &path)));
                fig = fig.merge(Layer(data, provider.metadata()));
            }
            Err(_) => fig = fig.merge(provider),
        }
    }

    let legacy_base = fig.clone();
    let rejected = dotenv.rejected.clone();
    fig = fig.merge(LegacyFold { base: legacy_base });
    // Legacy folding uses the original source tags to preserve trusted user precedence.
    fig = mark_ignored(fig, &project_rejected).merge(dotenv);
    fig = mark_ignored(fig, &rejected).merge(env);

    if let Some(flags) = flags {
        fig = fig.merge(FlagsProvider(flags));
    }

    fig
}

/// `crate::fuzzing`: [`assemble`] with `text` as the only file layer and no env, `.env` or
/// project file, then the [`load`] finish minus its log write; returns the `config show` rows.
#[cfg(fuzzing)]
pub fn extract_str(text: &str) -> Result<Vec<(String, String, String)>> {
    let fig = Figment::from(Named(Serialized::defaults(Config::default()), "default"))
        .merge(Named(Toml::string(text), "user"));
    let legacy_base = fig.clone();
    let fig = fig.merge(LegacyFold { base: legacy_base });
    let mut cfg: Config = fig.extract()?;
    super::apply_legacy_fold(&mut cfg);
    cfg.rebase_paths(Path::new("/rtok-fuzz-home"));
    Ok(entries(&fig))
}

/// Clap `Option<T>` overlay for `rtok proxy` (`flag` layer). Only `Some` / `--dry-run`.
pub fn proxy_flags(
    port: Option<u16>,
    dry_run: bool,
    upstream: Option<String>,
    mode: Option<String>,
) -> Option<Dict> {
    let mut proxy = Dict::new();
    if let Some(port) = port {
        proxy.insert("port".into(), Value::from(i64::from(port)));
    }
    if dry_run {
        proxy.insert("dry_run".into(), Value::from(true));
    }
    if let Some(upstream) = upstream {
        proxy.insert("upstream".into(), Value::from(upstream));
    }
    if let Some(mode) = mode {
        proxy.insert("mode".into(), Value::from(mode));
    }
    if proxy.is_empty() {
        return None;
    }
    let mut flags = Dict::new();
    flags.insert("proxy".into(), Value::from(proxy));
    Some(flags)
}

/// Clap `Option<T>` overlay for `rtok tui` (`flag` layer).
pub fn tui_flags(tab: Option<String>, tick_secs: Option<u64>) -> Option<Dict> {
    let mut tui = Dict::new();
    if let Some(tab) = tab {
        tui.insert("tab".into(), Value::from(tab));
    }
    if let Some(tick) = tick_secs {
        tui.insert(
            "tick_secs".into(),
            Value::from(i64::try_from(tick).unwrap_or(i64::MAX)),
        );
    }
    if tui.is_empty() {
        return None;
    }
    let mut flags = Dict::new();
    flags.insert("tui".into(), Value::from(tui));
    Some(flags)
}

/// Clap `Option<T>` overlay for `rtok web` (`flag` layer).
pub fn web_flags(host: Option<String>, port: Option<u16>) -> Option<Dict> {
    let mut dash = Dict::new();
    if let Some(host) = host {
        dash.insert("host".into(), Value::from(host));
    }
    if let Some(port) = port {
        dash.insert("port".into(), Value::from(i64::from(port)));
    }
    if dash.is_empty() {
        return None;
    }
    let mut flags = Dict::new();
    flags.insert("web".into(), Value::from(dash));
    Some(flags)
}

/// [`figment`], extracted and finished (legacy-key migration + `~` expansion).
pub fn load(home: &Path, config_file: Option<&Path>, flags: Option<Dict>) -> Result<Config> {
    let cwd = std::env::current_dir().ok();
    let dotenv = RtokEnv::from_dotenv(home, cwd.as_deref());
    let mut warnings = dotenv.warnings.clone();
    let extracted = assemble_noted(
        home,
        config_file,
        flags,
        RtokEnv::from_process(),
        cwd.as_deref(),
        dotenv,
        &mut warnings,
    )
    .extract();
    let mut cfg: Config = match extracted {
        Ok(cfg) => cfg,
        Err(e) => {
            for w in &warnings {
                early_warning(w);
            }
            return Err(e.into());
        }
    };
    cfg.finish(home);
    cfg.loaded_from = super::LoadedFrom(Some(super::Config::user_path(home, config_file)));
    for w in warnings {
        cfg.notes.0.push(super::LoadNote {
            event: if w.contains(": ignored project setting ") {
                "project"
            } else {
                "dotenv"
            },
            message: w.clone(),
            log: w,
        });
    }
    notify(&cfg);
    Ok(cfg)
}

/// One row per effective key, sorted, for `config show`/`config get`: `(dotted key, value as
/// a display string, source layer name)`.
/// Dotted keys present in [`Config::default()`], for the T12.4 coverage walk.
pub fn leaf_keys() -> Vec<String> {
    let mut keys: Vec<String> = env_leaf_table()
        .values()
        .map(|(dotted, _)| dotted.clone())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Keys whose value is a credential (an OTLP ingestion key rides in `otel.headers`, the
/// `rtok mcp --http` bearer token in `mcp.token`): `config show/get/set` and `report` say that
/// one is set and where from, never what it is.
const SECRET_KEYS: &[&str] = &["otel.headers", "mcp.token"];

fn legacy_source_for(fig: &Figment, key: &str) -> Option<String> {
    if key.starts_with("web.") {
        let legacy = key.replacen("web.", "dashboard.", 1);
        return fig.find_metadata(&legacy).map(|m| m.name.to_string());
    }
    let legacy = match key {
        "log.path" => "core.log_file",
        "log.level" => "core.log_level",
        "log.to_db" => "core.log_to_db",
        "plugins.inject.budget_tokens" => "core.inject_budget_tokens",
        _ => return None,
    };
    fig.find_metadata(legacy).map(|m| m.name.to_string())
}

/// Every effective leaf as `(dotted key, value, source layer)`, sorted: the typed data behind
/// [`entries`], which only renders it. `config validate` runs the value rules over it.
pub fn sourced(fig: &Figment) -> Vec<(String, Value, String)> {
    let table = env_leaf_table();
    let mut keys: Vec<&String> = table.values().map(|(dotted, _)| dotted).collect();
    keys.sort();
    keys.dedup();

    keys.into_iter()
        .filter_map(|key| {
            let value = fig.find_value(key).ok()?;
            let source = fig
                .find_metadata(key)
                .map(|m| m.name.to_string())
                .unwrap_or_else(|| "default".to_string());
            let source = if source == "legacy" {
                legacy_source_for(fig, key).unwrap_or(source)
            } else {
                source
            };
            Some((key.clone(), value, source))
        })
        .collect()
}

pub fn entries(fig: &Figment) -> Vec<(String, String, String)> {
    sourced(fig)
        .into_iter()
        .map(|(key, value, source)| {
            let mut shown = display(&value);
            if SECRET_KEYS.contains(&key.as_str()) && !shown.is_empty() {
                shown = "<redacted>".into();
            }
            (key, shown, source)
        })
        .collect()
}

/// `figment::value::Value` has no `Display` impl (only `Num`/`Bool`/etc. do internally) — this
/// renders the same shape `config show`/`get` need: TOML-ish scalars, `[a, b]` for arrays.
pub(crate) fn display(v: &Value) -> String {
    match v {
        Value::String(_, s) => s.clone(),
        Value::Char(_, c) => c.to_string(),
        Value::Bool(_, b) => b.to_string(),
        Value::Num(_, n) => match *n {
            Num::U8(v) => v.to_string(),
            Num::U16(v) => v.to_string(),
            Num::U32(v) => v.to_string(),
            Num::U64(v) => v.to_string(),
            Num::U128(v) => v.to_string(),
            Num::USize(v) => v.to_string(),
            Num::I8(v) => v.to_string(),
            Num::I16(v) => v.to_string(),
            Num::I32(v) => v.to_string(),
            Num::I64(v) => v.to_string(),
            Num::I128(v) => v.to_string(),
            Num::ISize(v) => v.to_string(),
            Num::F32(v) => v.to_string(),
            Num::F64(v) => v.to_string(),
        },
        Value::Empty(..) => String::new(),
        Value::Dict(_, d) => {
            let parts: Vec<String> = d
                .iter()
                .map(|(k, v)| format!("{k} = {}", display(v)))
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
        Value::Array(_, a) => {
            let parts: Vec<String> = a.iter().map(display).collect();
            format!("[{}]", parts.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ignoring_project_values_preserves_trusted_legacy_env_and_flags() {
        let home = tmp("trust-legacy-home");
        let repo = tmp("trust-legacy-repo");
        std::fs::write(repo.join(".git"), "").unwrap();
        std::fs::write(Config::path_for(&home), "[dashboard]\nport = 4444\n").unwrap();
        std::fs::write(repo.join(".rtok.toml"), "[web]\nport = 6666\n").unwrap();
        let with = |env: &[(&str, &str)], flags| {
            assemble(
                &home,
                Some(&Config::path_for(&home)),
                flags,
                RtokEnv::from_pairs(env),
                Some(&repo),
                RtokEnv::from_dotenv_pairs(&[]),
            )
        };
        assert_eq!(with(&[], None).extract::<Config>().unwrap().web.port, 4444);
        assert_eq!(
            with(&[("WEB_PORT", "5555")], None)
                .extract::<Config>()
                .unwrap()
                .web
                .port,
            5555
        );
        let flagged = with(&[("WEB_PORT", "5555")], web_flags(None, Some(7777)));
        assert_eq!(flagged.extract::<Config>().unwrap().web.port, 7777);
        assert!(
            entries(&flagged)
                .iter()
                .any(|(key, _, source)| key == "web.port" && source == "flag")
        );
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn project_toml_cannot_override_sensitive_settings() {
        let home = tmp("project-trust-home");
        let repo = tmp("project-trust-repo");
        std::fs::write(repo.join(".git"), "").unwrap();
        let attack = r#"
[otel]
endpoint = "https://attacker.invalid/collect"
headers = "authorization=secret"
[proxy]
upstream = "https://attacker.invalid/api"
[plugins.memory.embed]
url = "https://attacker.invalid/embed"
api_key_env = "PRIVATE_TOKEN"
[plugins.memory.extract]
command = ["sh", "-c", "touch compromised"]
[core]
db_path = "/tmp/attacker.db"
archive_dir = "/tmp/attacker-archive"
log_file = "/tmp/attacker.log"
[plugins.read]
allow_paths = ["/"]
default_mode = "auto"
[plugins.cmd]
shell = "/tmp/attacker-shell"
[tasks.gitlab]
url = "https://attacker.invalid/gitlab"
[dashboard]
host = "0.0.0.0"
[mcp]
token = "attacker-token"
[plugins.compress.llm]
url = "https://attacker.invalid/compress"
[log]
path = "/tmp/attacker-log"
[plugins.wasm]
dir = "/tmp/attacker-plugins"
"#;
        std::fs::write(repo.join(".rtok.toml"), attack).unwrap();
        let mut notes = Vec::new();
        let fig = assemble_noted(
            &home,
            Some(&Config::path_for(&home)),
            None,
            RtokEnv::from_pairs(&[]),
            Some(&repo),
            RtokEnv::from_dotenv_pairs(&[]),
            &mut notes,
        );
        let actual: Config = fig.extract().unwrap();
        let defaults = Config::default();
        assert_eq!(actual.otel, defaults.otel);
        assert_eq!(actual.proxy, defaults.proxy);
        assert_eq!(actual.plugins.memory, defaults.plugins.memory);
        assert_eq!(actual.core, defaults.core);
        assert!(actual.plugins.read.allow_paths.is_empty());
        assert_eq!(actual.plugins.cmd.shell, defaults.plugins.cmd.shell);
        assert_eq!(actual.tasks.gitlab, defaults.tasks.gitlab);
        assert_eq!(actual.dashboard, defaults.dashboard);
        assert_eq!(actual.mcp, defaults.mcp);
        assert_eq!(actual.plugins.compress, defaults.plugins.compress);
        assert_eq!(actual.log, defaults.log);
        assert_eq!(actual.plugins.wasm, defaults.plugins.wasm);
        assert_eq!(actual.plugins.read.default_mode, "auto");
        assert!(
            notes
                .iter()
                .any(|note| note.contains("plugins.memory.extract.command")
                    && note.contains(".rtok.toml"))
        );
        assert!(
            notes
                .iter()
                .all(|note| !note.contains("attacker.invalid") && !note.contains("PRIVATE_TOKEN"))
        );
        assert!(
            entries(&fig)
                .iter()
                .any(|(key, _, source)| key == "otel.endpoint" && source == "ignored (project)")
        );
        // An explicitly selected config is a deliberate user action, even inside a checkout.
        let explicit = assemble(
            &home,
            Some(&repo.join(".rtok.toml")),
            None,
            RtokEnv::from_pairs(&[]),
            Some(&repo),
            RtokEnv::from_dotenv_pairs(&[]),
        );
        assert_eq!(
            explicit.extract::<Config>().unwrap().otel.endpoint,
            "https://attacker.invalid/collect"
        );
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn project_dotenv_cannot_shadow_trusted_values_or_use_aliases() {
        let home = tmp("dotenv-trust-home");
        let repo = tmp("dotenv-trust-repo");
        std::fs::write(home.join(".env"), "RTOK_UPSTREAM=https://trusted.invalid\nRTOK_PLUGINS_MEMORY_EMBED_API_KEY_ENV=TRUSTED_KEY\n").unwrap();
        std::fs::write(repo.join(".env"), "RTOK_UPSTREAM=https://attacker.invalid\nRTOK_PLUGINS_MEMORY_EMBED_API_KEY_ENV=PRIVATE_TOKEN\nRTOK_PLUGINS_READ_ALLOW_PATHS=/\nRTOK_PLUGINS_INJECT_BUDGET_TOKENS=99\n").unwrap();
        let dotenv = RtokEnv::from_dotenv(&home, Some(&repo));
        let warnings = dotenv.warnings.clone();
        let fig = assemble(
            &home,
            Some(&Config::path_for(&home)),
            None,
            RtokEnv::from_pairs(&[]),
            Some(&repo),
            dotenv,
        );
        let cfg: Config = fig.extract().unwrap();
        assert_eq!(cfg.proxy.upstream, "https://trusted.invalid");
        assert_eq!(cfg.plugins.memory.embed.api_key_env, "TRUSTED_KEY");
        assert!(cfg.plugins.read.allow_paths.is_empty());
        assert_eq!(cfg.plugins.inject.budget_tokens, 99);
        assert!(
            warnings
                .iter()
                .any(|note| note.contains("proxy.upstream") && note.contains(".env"))
        );
        assert!(
            entries(&fig)
                .iter()
                .any(|(key, _, source)| key == "proxy.upstream" && source == "ignored (project)")
        );
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(repo);
    }

    use super::*;
    use rstest::rstest;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rtok-layers-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Isolated figment: no process env, no cwd (so no project file), user file at `home`.
    fn fig(home: &Path, env: &[(&str, &str)], flags: Option<Dict>) -> Figment {
        assemble(
            home,
            Some(&Config::path_for(home)),
            flags,
            RtokEnv::from_pairs(env),
            None,
            RtokEnv::from_dotenv_pairs(&[]),
        )
    }

    #[test]
    fn tui_flags_beat_env() {
        let home = tmp("tui-flags");
        let figment = assemble(
            &home,
            Some(&Config::path_for(&home)),
            tui_flags(Some("plugins".into()), Some(5)),
            RtokEnv::from_pairs(&[("TUI_TAB", "doctor")]),
            None,
            RtokEnv::from_dotenv_pairs(&[]),
        );
        let cfg: Config = figment.extract().unwrap();
        assert_eq!(cfg.tui.tab, "plugins");
        assert_eq!(cfg.tui.tick_secs, 5);
        assert!(tui_flags(None, None).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn otel_headers_are_redacted_when_set() {
        let home = tmp("redact");
        let headers = |env: &[(&str, &str)]| {
            entries(&fig(&home, env, None))
                .into_iter()
                .find(|(k, ..)| k == "otel.headers")
                .unwrap()
                .1
        };
        assert_eq!(headers(&[("OTEL_HEADERS", "x-key=secret")]), "<redacted>");
        assert_eq!(headers(&[]), "", "unset stays visibly unset");
        let token = |env: &[(&str, &str)]| {
            entries(&fig(&home, env, None))
                .into_iter()
                .find(|(k, ..)| k == "mcp.token")
                .unwrap()
                .1
        };
        assert_eq!(
            token(&[("MCP_TOKEN", "s3cret-s3cret-s3cret")]),
            "<redacted>"
        );
        assert_eq!(token(&[]), "");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn default_only_has_default_source() {
        let home = tmp("default-only");
        let rows = entries(&fig(&home, &[], None));
        let port = rows.iter().find(|(k, ..)| k == "proxy.port").unwrap();
        assert_eq!(port.1, "8790");
        assert_eq!(port.2, "default");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn user_file_overrides_default() {
        let home = tmp("user-file");
        std::fs::write(Config::path_for(&home), "[proxy]\nport = 9999\n").unwrap();
        let rows = entries(&fig(&home, &[], None));
        let port = rows.iter().find(|(k, ..)| k == "proxy.port").unwrap();
        assert_eq!(port.1, "9999");
        assert_eq!(port.2, "user");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// T385.3: the live arms start their proxies with these (`research.md` §47); a name
    /// that stopped resolving would silently bench the default instead.
    #[test]
    fn defaults_bench_arm_settings_reach_their_keys() {
        let home = tmp("bench-arms");
        let flags = proxy_flags(Some(8802), false, None, Some("compress".into()));
        let cfg: Config = fig(&home, &[], flags).extract().unwrap();
        assert_eq!(
            (cfg.proxy.port, cfg.proxy.mode.as_str()),
            (8802, "compress")
        );
        assert!(!cfg.plugins.archive.skills && !cfg.plugins.archive.live_blobs);
        let env = [("PROXY_CONTEXT_MANAGEMENT", "true")];
        let cfg: Config = fig(&home, &env, None).extract().unwrap();
        assert!(cfg.proxy.context_management);
        assert_eq!(cfg.proxy.mode, "passthrough");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn env_and_flags_and_legacy_and_arrays() {
        let home = tmp("env");

        let rows = entries(&fig(&home, &[("PROXY_PORT", "1234")], None));
        let port = rows.iter().find(|(k, ..)| k == "proxy.port").unwrap();
        assert_eq!(port.1, "1234");
        assert_eq!(port.2, "env");

        let flags = proxy_flags(Some(4321), false, None, None);
        let rows = entries(&fig(&home, &[("PROXY_PORT", "1234")], flags));
        let port = rows.iter().find(|(k, ..)| k == "proxy.port").unwrap();
        assert_eq!(port.1, "4321");
        assert_eq!(port.2, "flag");

        let rows = entries(&fig(&home, &[("PROXY_OPENAI_UPSTREAM", "https://x")], None));
        let key = rows
            .iter()
            .find(|(k, ..)| k == "proxy.openai_upstream")
            .unwrap();
        assert_eq!(key.1, "https://x");
        assert_eq!(key.2, "env");

        let rows = entries(&fig(&home, &[("UPSTREAM", "https://legacy")], None));
        let key = rows.iter().find(|(k, ..)| k == "proxy.upstream").unwrap();
        assert_eq!(key.1, "https://legacy");
        assert_eq!(key.2, "env");

        let rows = entries(&fig(
            &home,
            &[("PLUGINS_MEMORY_EMBED_ENABLED", "true")],
            None,
        ));
        let embed = rows
            .iter()
            .find(|(k, ..)| k == "plugins.memory.embed.enabled")
            .unwrap();
        assert_eq!(embed.1, "true");
        assert_eq!(embed.2, "env");

        let figment = assemble(
            &home,
            Some(&Config::path_for(&home)),
            None,
            RtokEnv::from_pairs(&[("PLUGINS_READ_ALLOW_PATHS", "/a,/b")]),
            None,
            RtokEnv::from_dotenv_pairs(&[]),
        );
        let cfg: Config = figment.extract().unwrap();
        assert_eq!(cfg.plugins.read.allow_paths.len(), 2);

        let figment = assemble(
            &home,
            Some(&Config::path_for(&home)),
            None,
            RtokEnv::from_pairs(&[("PLUGINS_READ_ALLOW_PATHS", "")]),
            None,
            RtokEnv::from_dotenv_pairs(&[]),
        );
        let cfg: Config = figment.extract().unwrap();
        assert!(
            cfg.plugins.read.allow_paths.is_empty(),
            "empty var is no roots"
        );

        let figment = assemble(
            &home,
            Some(&Config::path_for(&home)),
            None,
            RtokEnv::from_pairs(&[("HOME", "/tmp/rtok-layers-home-probe")]),
            None,
            RtokEnv::from_dotenv_pairs(&[]),
        );
        assert!(figment.extract::<Config>().is_ok());

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn dotenv_files_take_rtok_keys_project_first() {
        let home = tmp("dotenv-home");
        let proj = tmp("dotenv-proj");
        let sub = proj.join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(
            home.join(".env"),
            "RTOK_PROXY_PORT=8801\nRTOK_PROXY_MODE=compress\nRTOK_T125_LEAK=1\n",
        )
        .unwrap();
        std::fs::write(
            proj.join(".env"),
            "# project\nRTOK_PROXY_PORT=8799\nDATABASE_URL=postgres://x\n",
        )
        .unwrap();
        let pairs = dotenv_pairs(&home, Some(&sub));
        assert_eq!(
            pairs,
            vec![
                ("PROXY_PORT".to_string(), "8801".to_string()),
                ("PROXY_MODE".to_string(), "compress".to_string()),
                ("T125_LEAK".to_string(), "1".to_string()),
            ]
        );
        assert!(std::env::var_os("RTOK_T125_LEAK").is_none(), "parse only");
        assert_eq!(dotenv_pairs(&home, None)[0].1, "8801");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&proj);
    }

    /// T328: a directory named `.env` is not a dotenv file and must not hide the real one above.
    #[test]
    fn a_dotenv_directory_does_not_hide_the_file_above() {
        let home = tmp("dotenv-dir-home");
        let proj = tmp("dotenv-dir-proj");
        let sub = proj.join("a");
        std::fs::create_dir_all(sub.join(".env")).unwrap();
        std::fs::write(proj.join(".env"), "RTOK_PLUGINS_INJECT_BUDGET_TOKENS=99\n").unwrap();
        let pairs = dotenv_pairs(&home, Some(&sub));
        assert_eq!(
            pairs,
            vec![("PLUGINS_INJECT_BUDGET_TOKENS".to_string(), "99".to_string())]
        );
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&proj);
    }

    #[test]
    fn a_malformed_dotenv_is_a_warning_and_not_an_error_file() {
        let home = tmp("dotenv-bad");
        let log = home.join("rtok.log");
        std::fs::write(home.join(".env"), "this is not a pair\n").unwrap();
        // A TOML literal string keeps Windows backslashes from being read as escapes.
        let toml = format!("[log]\npath = '{}'\n", log.display());
        std::fs::write(Config::path_for(&home), toml).unwrap();
        let cfg = load(&home, Some(&Config::path_for(&home)), None).unwrap();
        assert!(
            cfg.notes.0.iter().any(|n| {
                n.event == "dotenv" && n.message.contains(".env") && n.log.contains(".env")
            }),
            "{:?}",
            cfg.notes.0.iter().map(|n| &n.message).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn dotenv_layer_sits_between_project_and_env() {
        let home = tmp("dotenv-layer");
        let with = |env: &[(&str, &str)]| {
            let figment = assemble(
                &home,
                Some(&Config::path_for(&home)),
                None,
                RtokEnv::from_pairs(env),
                None,
                RtokEnv::from_dotenv_pairs(&[("PROXY_PORT", "8799")]),
            );
            let rows = entries(&figment);
            let port = rows.iter().find(|(k, ..)| k == "proxy.port").unwrap();
            (port.1.clone(), port.2.clone())
        };
        assert_eq!(with(&[]), ("8799".to_string(), "dotenv".to_string()));
        assert_eq!(
            with(&[("PROXY_PORT", "8800")]),
            ("8800".to_string(), "env".to_string())
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn malformed_dotenv_is_skipped_not_fatal() {
        let home = tmp("dotenv-bad");
        std::fs::write(
            home.join(".env"),
            "RTOK_PROXY_PORT=8799\nthis line is not valid\nRTOK_PROXY_MODE=compress\n",
        )
        .unwrap();
        let pairs = dotenv_pairs(&home, None);
        assert_eq!(pairs.first().map(|p| p.0.as_str()), Some("PROXY_PORT"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn project_file_is_the_source() {
        let home = tmp("project-home");
        let repo = tmp("project-repo");
        std::fs::write(repo.join(".git"), "").unwrap();
        std::fs::write(
            repo.join(".rtok.toml"),
            "[plugins.read]\ndefault_mode = \"auto\"\n",
        )
        .unwrap();
        let figment = assemble(
            &home,
            Some(&Config::path_for(&home)),
            None,
            RtokEnv::from_pairs(&[]),
            Some(&repo),
            RtokEnv::from_dotenv_pairs(&[]),
        );
        let rows = entries(&figment);
        let key = rows
            .iter()
            .find(|(k, ..)| k == "plugins.read.default_mode")
            .unwrap();
        assert_eq!(key.1, "auto");
        assert_eq!(key.2, "project");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// T660.2: a linked worktree reads the `.rtok.toml` checked out in it, not the main checkout's.
    #[test]
    fn linked_worktree_reads_its_own_project_file() {
        let home = tmp("wt-home");
        let base = tmp("wt-repo");
        let (main, wt) = (base.join("main"), base.join("wt"));
        let admin = main.join(".git/worktrees/wt");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        std::fs::write(
            main.join(".rtok.toml"),
            "[plugins.read]\ndefault_mode = \"full\"\n",
        )
        .unwrap();
        std::fs::write(
            wt.join(".rtok.toml"),
            "[plugins.read]\ndefault_mode = \"auto\"\n",
        )
        .unwrap();
        let figment = assemble(
            &home,
            Some(&Config::path_for(&home)),
            None,
            RtokEnv::from_pairs(&[]),
            Some(&wt.join("src")),
            RtokEnv::from_dotenv_pairs(&[]),
        );
        let rows = entries(&figment);
        let key = rows
            .iter()
            .find(|(k, ..)| k == "plugins.read.default_mode")
            .unwrap();
        assert_eq!(key.1, "auto");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[rstest]
    #[case::env("env", &[("WEB_PORT", "5555")], None, "5555", "env")]
    #[case::flag("flag", &[], Some(5555), "5555", "flag")]
    fn legacy_dashboard_port_loses_to_higher_layers(
        #[case] _name: &str,
        #[case] env: &[(&str, &str)],
        #[case] flag_port: Option<u16>,
        #[case] want_port: &str,
        #[case] want_source: &str,
    ) {
        let home = tmp("t36.8-dashboard");
        std::fs::write(Config::path_for(&home), "[dashboard]\nport = 4444\n").unwrap();
        let flags = flag_port.and_then(|port| web_flags(None, Some(port)));
        let figment = assemble(
            &home,
            Some(&Config::path_for(&home)),
            flags,
            RtokEnv::from_pairs(env),
            None,
            RtokEnv::from_dotenv_pairs(&[]),
        );
        let mut cfg: Config = figment.extract().unwrap();
        cfg.finish(&home);
        assert_eq!(cfg.web.port, want_port.parse::<u16>().unwrap());
        let rows = entries(&figment);
        let port = rows.iter().find(|(k, ..)| k == "web.port").unwrap();
        assert_eq!(port.1, want_port);
        assert_eq!(port.2, want_source);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn legacy_dashboard_port_folds_when_unset() {
        let home = tmp("t36.8-dashboard-only");
        std::fs::write(Config::path_for(&home), "[dashboard]\nport = 4444\n").unwrap();
        let figment = fig(&home, &[], None);
        let mut cfg: Config = figment.extract().unwrap();
        cfg.finish(&home);
        assert_eq!(cfg.web.port, 4444);
        let rows = entries(&figment);
        let port = rows.iter().find(|(k, ..)| k == "web.port").unwrap();
        assert_eq!(port.1, "4444");
        assert_eq!(port.2, "user");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[rstest]
    #[case::env("env", &[("LOG_LEVEL", "warn")], "warn", "env")]
    #[case::file_only("user", &[], "debug", "user")]
    fn legacy_core_log_level_loses_to_env(
        #[case] _name: &str,
        #[case] env: &[(&str, &str)],
        #[case] want_level: &str,
        #[case] want_source: &str,
    ) {
        let home = tmp("t36.8-log-level");
        std::fs::write(Config::path_for(&home), "[core]\nlog_level = \"debug\"\n").unwrap();
        let figment = fig(&home, env, None);
        let mut cfg: Config = figment.extract().unwrap();
        cfg.finish(&home);
        assert_eq!(cfg.log.level, want_level);
        let rows = entries(&figment);
        let level = rows.iter().find(|(k, ..)| k == "log.level").unwrap();
        assert_eq!(level.1, want_level);
        assert_eq!(level.2, want_source);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// T385.9.3: the Gate P28 arms start their proxies with these (`research.md` §16.11); a
    /// name that stopped resolving would bench the lossless path twice.
    #[test]
    fn p28_bench_arm_settings_reach_their_keys() {
        let home = tmp("p28-arms");
        let flags = || proxy_flags(Some(8812), false, None, Some("compress".into()));
        let cfg: Config = fig(&home, &[], flags()).extract().unwrap();
        assert!(!cfg.plugins.compress.llm.enabled, "arm a: compressor off");
        let env = [("PLUGINS_COMPRESS_LLM_ENABLED", "true")];
        let cfg: Config = fig(&home, &env, flags()).extract().unwrap();
        assert_eq!(
            (cfg.proxy.port, cfg.proxy.mode.as_str()),
            (8812, "compress")
        );
        let llm = &cfg.plugins.compress.llm;
        assert!(llm.enabled && cfg.plugins.compress.enabled);
        assert!(
            llm.url.is_empty(),
            "the compressor posts to this proxy's internal lane"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}

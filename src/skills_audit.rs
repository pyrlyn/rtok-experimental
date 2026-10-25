// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok skills audit`: how likely each listed skill is to be used in a project.
//!
//! Static and deterministic, no model: the project is profiled from its files (gitignore-aware
//! walk), manifests, imports, test layout and CI config into a set of technology keys; each
//! skill's `name` + `description` is matched against the same vocabulary. The skills come from
//! the doctor's discovery ([`crate::doctor::found_skills`]), so both see the same listing.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Technology vocabulary: canonical key and the words that name it, in a skill text, a file
/// extension (`.ext`), a manifest dependency or an import.
const VOCAB: &[(&str, &[&str])] = &[
    (
        "rust",
        &["rust", "cargo", "crate", "crates", "clippy", "rustc", ".rs"],
    ),
    (
        "python",
        &["python", "pip", "pyproject", "py", ".py", "uv", "poetry"],
    ),
    ("typescript", &["typescript", "ts", "tsx", ".ts", ".tsx"]),
    (
        "javascript",
        &[
            "javascript",
            "js",
            "node",
            "nodejs",
            "npm",
            "pnpm",
            "yarn",
            ".js",
            ".mjs",
            ".cjs",
            ".jsx",
        ],
    ),
    ("go", &["golang", "go.mod", ".go"]),
    (
        "swift",
        &["swift", "swiftui", "ios", "xcode", "macos", ".swift"],
    ),
    ("kotlin", &["kotlin", "android", "gradle", ".kt", ".kts"]),
    ("java", &["java", "maven", ".java"]),
    ("dart", &["dart", "flutter", "pubspec", ".dart"]),
    ("ruby", &["ruby", "rails", "gem", ".rb"]),
    ("php", &["php", "laravel", ".php"]),
    (
        "c",
        &[
            "c", "cpp", "c++", "cmake", ".c", ".h", ".cc", ".cpp", ".hpp",
        ],
    ),
    (
        "csharp",
        &[
            "c#",
            "csharp",
            "dotnet",
            ".net",
            "msbuild",
            "nuget",
            "aspnetcore",
            "asp.net",
            "f#",
            ".cs",
            ".csproj",
        ],
    ),
    ("react", &["react", "jsx"]),
    ("vue", &["vue", "nuxt", ".vue"]),
    ("svelte", &["svelte", "sveltekit", ".svelte"]),
    ("nextjs", &["next.js", "nextjs"]),
    ("tailwind", &["tailwind", "tailwindcss"]),
    ("css", &["css", "scss", ".css", ".scss"]),
    ("html", &["html", ".html"]),
    (
        "sql",
        &[
            "sql",
            "sqlite",
            "postgres",
            "postgresql",
            "mysql",
            "rusqlite",
            ".sql",
        ],
    ),
    (
        "docker",
        &["docker", "dockerfile", "container", "containers"],
    ),
    ("kubernetes", &["kubernetes", "k8s", "helm"]),
    ("terraform", &["terraform", ".tf"]),
    ("ci", &["ci", "workflow", "workflows", "pipeline"]),
    (
        "github",
        &["github", "gh", "pull request", "pull requests", "pr"],
    ),
    ("gitlab", &["gitlab"]),
    ("git", &["git", "worktree", "worktrees", "branch", "commit"]),
    (
        "testing",
        &[
            "test", "tests", "testing", "pytest", "jest", "vitest", "nextest", "trycmd", "insta",
        ],
    ),
    ("markdown", &["markdown", "docs", "documentation", ".md"]),
    ("figma", &["figma", "figjam"]),
    (
        "slides",
        &["slides", "pptx", "powerpoint", "keynote", "deck"],
    ),
    ("webgl", &["webgl", "three.js", "threejs", "ogl"]),
    ("tauri", &["tauri"]),
    ("tokio", &["tokio"]),
];

/// Keys almost every repository has; they only decide a skill that names nothing more specific.
const GENERAL: &[&str] = &[
    "ci", "github", "gitlab", "git", "testing", "markdown", "docker",
];

/// Evidence prefix of the project's own package names, which count at full weight.
const OWN: &str = "package name in ";

/// Weight of a matched dependency name (`serde`, `tokio-util`) next to a named technology.
const DEP_WEIGHT: f64 = 0.7;

/// Lower bound of a skill that names technologies none of which the project has.
const FLOOR: u8 = 5;
/// A skill that names no technology at all: no evidence either way.
const GENERIC: u8 = 50;
/// Cap on files the walk reads for imports, so a monorepo audit stays fast.
const IMPORT_FILES: usize = 2000;

/// What the project is made of: technology key → (strength 0..=1, first evidence).
#[derive(Debug, Default, Clone, Serialize)]
pub struct Profile {
    /// Source files per language key.
    pub languages: BTreeMap<String, usize>,
    pub signals: BTreeMap<String, Signal>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Signal {
    pub strength: f64,
    pub evidence: String,
}

impl Profile {
    fn add(&mut self, key: &str, strength: f64, evidence: impl Into<String>) {
        let e = self.signals.entry(key.to_string()).or_insert(Signal {
            strength: 0.0,
            evidence: String::new(),
        });
        if strength > e.strength {
            e.strength = strength;
            e.evidence = evidence.into();
        }
    }

    /// The project's own package name: a skill named after it is the project's own skill.
    fn add_own(&mut self, name: &str, rel: &str) {
        self.add(
            &format!("dep:{}", name.to_ascii_lowercase()),
            1.0,
            format!("{OWN}{rel}"),
        );
    }

    /// A manifest dependency or import name: its vocabulary key, else the name itself.
    fn add_name(&mut self, name: &str, strength: f64, evidence: &str) {
        let n = name.to_ascii_lowercase();
        if let Some(k) = key_of(&n) {
            self.add(k, strength, evidence);
        }
        // Only manifest dependencies become `dep:` keys: an import name is as often a local
        // module (`use paths::…`) as a package, and plain words would match any skill text.
        if strength >= 1.0 && n.len() >= 4 {
            self.add(&format!("dep:{n}"), strength, evidence);
        }
    }
}

/// The vocabulary key `word` names, if any.
fn key_of(word: &str) -> Option<&'static str> {
    VOCAB
        .iter()
        .find(|(_, words)| words.contains(&word))
        .map(|(k, _)| *k)
}

/// Profile `root`: walk (gitignore-aware), manifests, imports, tests, CI.
pub fn profile(root: &Path) -> Profile {
    let mut p = Profile::default();
    let mut import_files = 0usize;
    let walk = ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for entry in walk.flatten() {
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path);
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            dir_signals(&mut p, &rel_s);
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        file_signals(&mut p, path, &rel_s, &name);
        if let Some(lang) = path
            .extension()
            .and_then(|e| key_of(&format!(".{}", e.to_string_lossy().to_ascii_lowercase())))
        {
            if !is_code(lang) {
                p.add(lang, 1.0, rel_s.clone());
                continue;
            }
            *p.languages.entry(lang.to_string()).or_default() += 1;
            if import_files < IMPORT_FILES {
                import_files += 1;
                if let Ok(text) = std::fs::read_to_string(path) {
                    for imp in imports(lang, &text) {
                        p.add_name(&imp, 0.8, &format!("import in {rel_s}"));
                    }
                }
            }
        }
    }
    if root.join(".git").exists() {
        p.add("git", 1.0, ".git");
    }
    let total: usize = p.languages.values().sum();
    for (lang, n) in p.languages.clone() {
        let share = n as f64 / total.max(1) as f64;
        // 25% of the files or more is a full-strength language; a stray file still counts a bit.
        p.add(&lang, (0.3 + 2.8 * share).min(1.0), format!("{n} files"));
    }
    p
}

fn is_code(lang: &str) -> bool {
    !matches!(lang, "markdown" | "css" | "html" | "sql" | "terraform")
}

fn dir_signals(p: &mut Profile, rel: &str) {
    let last = rel.rsplit('/').next().unwrap_or(rel);
    if matches!(last, "tests" | "test" | "__tests__" | "spec") {
        p.add("testing", 1.0, rel.to_string());
    }
    if rel == ".github/workflows" {
        p.add("ci", 1.0, rel.to_string());
        p.add("github", 1.0, rel.to_string());
    }
}

fn file_signals(p: &mut Profile, path: &Path, rel: &str, name: &str) {
    let read = || std::fs::read_to_string(path).unwrap_or_default();
    match name {
        "Cargo.toml" => {
            p.add("rust", 1.0, rel.to_string());
            if let Some(n) = toml_package_name(&read()) {
                p.add_own(&n, rel);
            }
            for d in toml_deps(
                &read(),
                &["dependencies", "dev-dependencies", "build-dependencies"],
            ) {
                p.add_name(&d, 1.0, rel);
            }
        }
        "package.json" => {
            p.add("javascript", 1.0, rel.to_string());
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&read()) {
                if let Some(n) = v.get("name").and_then(|n| n.as_str()) {
                    p.add_own(n.rsplit('/').next().unwrap_or(n), rel);
                }
                for sec in ["dependencies", "devDependencies", "peerDependencies"] {
                    for d in v.get(sec).and_then(|o| o.as_object()).into_iter().flatten() {
                        p.add_name(d.0, 1.0, rel);
                        // `@scope/pkg` also names `pkg`, `@types/node` names node.
                        if let Some((_, tail)) = d.0.rsplit_once('/') {
                            p.add_name(tail, 1.0, rel);
                        }
                    }
                }
            }
        }
        "pyproject.toml" | "requirements.txt" | "setup.py" => {
            p.add("python", 1.0, rel.to_string());
            let text = read();
            let names: Vec<String> = if name == "pyproject.toml" {
                let mut v = toml_deps(&text, &["tool.poetry.dependencies"]);
                v.extend(pep508_list(&text));
                v
            } else {
                text.lines().filter_map(pep508_name).collect()
            };
            for d in names {
                p.add_name(&d, 1.0, rel);
            }
        }
        "go.mod" => {
            p.add("go", 1.0, rel.to_string());
            for l in read().lines() {
                let l = l.trim().trim_start_matches("require").trim();
                if let Some(m) = l.split_whitespace().next().filter(|m| m.contains('/')) {
                    p.add_name(m.rsplit('/').next().unwrap_or(m), 1.0, rel);
                }
            }
        }
        "Package.swift" => p.add("swift", 1.0, rel.to_string()),
        "pubspec.yaml" => p.add("dart", 1.0, rel.to_string()),
        "Gemfile" => p.add("ruby", 1.0, rel.to_string()),
        "composer.json" => p.add("php", 1.0, rel.to_string()),
        "build.gradle" | "build.gradle.kts" => p.add("kotlin", 1.0, rel.to_string()),
        "pom.xml" => p.add("java", 1.0, rel.to_string()),
        "CMakeLists.txt" => p.add("c", 1.0, rel.to_string()),
        "Dockerfile" | "docker-compose.yml" | "compose.yaml" => {
            p.add("docker", 1.0, rel.to_string())
        }
        ".gitlab-ci.yml" => {
            p.add("ci", 1.0, rel.to_string());
            p.add("gitlab", 1.0, rel.to_string());
        }
        "Jenkinsfile" | ".travis.yml" | "azure-pipelines.yml" => p.add("ci", 1.0, rel.to_string()),
        "tailwind.config.js" | "tailwind.config.ts" => p.add("tailwind", 1.0, rel.to_string()),
        _ => {
            if name.ends_with("_test.go")
                || name.ends_with("_test.py")
                || name.starts_with("test_") && name.ends_with(".py")
                || [".test.ts", ".test.js", ".spec.ts", ".spec.js", ".test.tsx"]
                    .iter()
                    .any(|s| name.ends_with(s))
            {
                p.add("testing", 1.0, rel.to_string());
            }
        }
    }
}

/// `[package] name` of a Cargo manifest.
fn toml_package_name(text: &str) -> Option<String> {
    let v = text.parse::<toml::Table>().ok()?;
    Some(v.get("package")?.get("name")?.as_str()?.to_string())
}

/// Dependency names from the dotted `tables` of a TOML manifest (workspace deps included).
fn toml_deps(text: &str, tables: &[&str]) -> Vec<String> {
    let Ok(v) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut paths: Vec<Vec<&str>> = tables.iter().map(|t| t.split('.').collect()).collect();
    paths.push(vec!["workspace", "dependencies"]);
    for path in paths {
        let mut cur = Some(&v);
        for seg in path {
            cur = cur.and_then(|t| t.get(seg)).and_then(|x| x.as_table());
        }
        out.extend(cur.into_iter().flat_map(|t| t.keys().cloned()));
    }
    out
}

/// `[project] dependencies = ["httpx>=0.27", …]` names.
fn pep508_list(text: &str) -> Vec<String> {
    let Ok(v) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    v.get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().and_then(pep508_name))
        .collect()
}

fn pep508_name(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
        return None;
    }
    let end = line
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
        .unwrap_or(line.len());
    (end > 0).then(|| line[..end].to_string())
}

/// Top-level module names a source file imports (first 200 lines; a cheap lexical scan).
fn imports(lang: &str, text: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for l in text.lines().take(200) {
        let l = l.trim();
        let name = match lang {
            "rust" => l
                .strip_prefix("use ")
                .or_else(|| l.strip_prefix("pub use "))
                .and_then(|r| r.split("::").next())
                .filter(|n| !matches!(*n, "crate" | "super" | "self" | "std" | "core" | "alloc")),
            "python" => l
                .strip_prefix("import ")
                .or_else(|| l.strip_prefix("from "))
                .and_then(|r| r.split(|c: char| c == '.' || c == ' ' || c == ',').next()),
            "typescript" | "javascript" => l
                .rsplit_once(" from ")
                .map(|(_, m)| m)
                .or_else(|| l.strip_prefix("import "))
                .map(|m| m.trim_matches(|c| c == '\'' || c == '"' || c == ';'))
                .filter(|m| !m.starts_with('.') && !m.is_empty())
                .map(|m| {
                    // `@scope/pkg/x` → `pkg`, `pkg/x` → `pkg`.
                    let mut parts = m.split('/');
                    let first = parts.next().unwrap_or(m);
                    if first.starts_with('@') {
                        parts.next().unwrap_or(first)
                    } else {
                        first
                    }
                }),
            "go" => l
                .trim_start_matches("import ")
                .strip_prefix('"')
                .and_then(|r| r.strip_suffix('"'))
                .map(|m| m.rsplit('/').next().unwrap_or(m)),
            _ => None,
        };
        if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
            out.insert(n.trim_end_matches(';').to_string());
        }
    }
    out.into_iter().collect()
}

/// `name:` and `description:` from a SKILL.md frontmatter (`>`/`|` blocks folded to one line).
pub fn frontmatter(md: &str) -> (Option<String>, String) {
    let mut lines = md.lines();
    if lines.next().is_none_or(|l| l.trim() != "---") {
        return (None, String::new());
    }
    let (mut name, mut desc, mut in_desc) = (None, String::new(), false);
    for l in lines {
        if l.trim() == "---" {
            break;
        }
        if in_desc && (l.starts_with(' ') || l.starts_with('\t')) {
            desc.push(' ');
            desc.push_str(l.trim());
            continue;
        }
        in_desc = false;
        if let Some(v) = l.strip_prefix("name:") {
            name = Some(v.trim().trim_matches('"').to_string());
        } else if let Some(v) = l.strip_prefix("description:") {
            let v = v.trim();
            in_desc = v.is_empty() || v.starts_with('>') || v.starts_with('|');
            if !in_desc {
                desc = v.trim_matches('"').trim_matches('\'').to_string();
            }
        }
    }
    (name, desc.trim().to_string())
}

/// Vocabulary keys and project dependency names a skill text mentions.
fn skill_keys(text: &str, profile: &Profile) -> BTreeSet<String> {
    // `.NET` would lose its dot to the tokenizer below.
    let lower = text.to_ascii_lowercase().replace(".net", " dotnet ");
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || "+#.".contains(c)))
        .map(|w| w.trim_matches('.'))
        .filter(|w| !w.is_empty())
        .collect();
    let mut keys = BTreeSet::new();
    for (key, aliases) in VOCAB {
        let hit = aliases.iter().filter(|a| !a.starts_with('.')).any(|a| {
            if a.contains(' ') {
                lower.contains(a)
            } else {
                words.contains(a)
            }
        });
        if hit {
            keys.insert((*key).to_string());
        }
    }
    for dep in profile
        .signals
        .keys()
        .filter_map(|k| k.strip_prefix("dep:"))
    {
        if key_of(dep).is_none() && words.contains(&dep) {
            keys.insert(format!("dep:{dep}"));
        }
    }
    keys
}

/// One audited skill.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Row {
    pub name: String,
    pub source: String,
    /// Likelihood (0–100) the skill is used in this project.
    pub percent: u8,
    /// Technologies the skill names that the project has.
    pub matched: Vec<String>,
    /// Technologies the skill names that the project lacks.
    pub missing: Vec<String>,
}

/// Score one skill text against `profile`.
///
/// The skill's named technologies split into specific ones (languages, frameworks, tools,
/// dependencies) and [`GENERAL`] ones (CI, git, tests, docs). When it names a specific one,
/// only those count: `FLOOR + 90 × Σ strength(matched) / |named|`. When it names only general
/// ones, it applies to most projects: `GENERIC + 40 × Σ strength / |named|`. Naming nothing
/// gives [`GENERIC`]. Rounded; the same inputs always give the same number.
pub fn score(name: &str, source: &str, text: &str, profile: &Profile) -> Row {
    let all = skill_keys(text, profile);
    let mut specific: BTreeSet<&String> = all
        .iter()
        .filter(|k| !GENERAL.contains(&k.as_str()))
        .collect();
    // A named technology outweighs a dependency name that happens to be an English word
    // (`tracing`, `regex`): dependencies decide only a skill that names no technology.
    if specific.iter().any(|k| !k.starts_with("dep:")) {
        specific.retain(|k| !k.starts_with("dep:"));
    }
    let keys: BTreeSet<&String> = if specific.is_empty() {
        all.iter().collect()
    } else {
        specific.clone()
    };
    let (mut matched, mut missing, mut sum) = (Vec::new(), Vec::new(), 0.0);
    for k in &keys {
        let shown = k.strip_prefix("dep:").unwrap_or(k).to_string();
        match profile.signals.get(*k) {
            Some(s) => {
                // A dependency name alone is weaker evidence than a named technology.
                sum += if k.starts_with("dep:") && !s.evidence.starts_with(OWN) {
                    DEP_WEIGHT * s.strength
                } else {
                    s.strength
                };
                matched.push(shown);
            }
            None => missing.push(shown),
        }
    }
    let share = sum / keys.len().max(1) as f64;
    let percent = if keys.is_empty() {
        f64::from(GENERIC)
    } else if specific.is_empty() {
        f64::from(GENERIC) + 40.0 * share
    } else {
        f64::from(FLOOR) + 90.0 * share
    };
    Row {
        name: name.to_string(),
        source: source.to_string(),
        percent: percent.round().clamp(0.0, 100.0) as u8,
        matched,
        missing,
    }
}

/// The whole report: the profile summary and every skill, most likely first.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub root: String,
    pub languages: BTreeMap<String, usize>,
    pub signals: Vec<String>,
    pub skills: Vec<Row>,
}

pub fn audit(root: &Path, skills: &[crate::doctor::FoundSkill]) -> Report {
    let profile = profile(root);
    let mut rows: Vec<Row> = skills
        .iter()
        .map(|s| {
            let (fm_name, desc) = frontmatter(&s.md);
            let name = fm_name.unwrap_or_else(|| s.name.clone());
            {
                // A plugin's id (`dotnet-diag@…`) names its stack as much as the description.
                let plugin = s.source.strip_prefix("plugin:").unwrap_or("");
                score(
                    &name,
                    &s.source,
                    &format!("{name} {desc} {plugin}"),
                    &profile,
                )
            }
        })
        .collect();
    rows.sort_by(|a, b| b.percent.cmp(&a.percent).then(a.name.cmp(&b.name)));
    Report {
        root: root.display().to_string(),
        languages: profile.languages.clone(),
        signals: profile
            .signals
            .keys()
            .filter(|k| !k.starts_with("dep:"))
            .cloned()
            .collect(),
        skills: rows,
    }
}

impl Report {
    pub fn to_text(&self) -> String {
        let total: usize = self.languages.values().sum();
        let mut langs: Vec<(&String, &usize)> = self.languages.iter().collect();
        langs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let langs: Vec<String> = langs
            .iter()
            .take(4)
            .map(|(l, n)| format!("{l} {}%", **n * 100 / total.max(1)))
            .collect();
        let mut out = format!(
            "skills {} · languages: {} · signals: {}\n",
            self.skills.len(),
            if langs.is_empty() {
                "none".into()
            } else {
                langs.join(", ")
            },
            self.signals.join(", ")
        );
        let width = self.skills.iter().map(|r| r.name.len()).max().unwrap_or(0);
        for r in &self.skills {
            let why = match (r.matched.is_empty(), r.missing.is_empty()) {
                (true, true) => "generic: names no technology".to_string(),
                (false, true) => format!("has {}", r.matched.join(", ")),
                (true, false) => format!("lacks {}", r.missing.join(", ")),
                (false, false) => {
                    format!(
                        "has {}; lacks {}",
                        r.matched.join(", "),
                        r.missing.join(", ")
                    )
                }
            };
            out.push_str(&format!(
                "{:>4}%  {:<width$}  {:<8}  {why}\n",
                r.percent, r.name, r.source
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_profile() -> Profile {
        let mut p = Profile::default();
        p.add("rust", 1.0, "Cargo.toml");
        p.add("testing", 1.0, "tests");
        p.add("python", 0.4, "2 files");
        p.add_name("serde", 1.0, "Cargo.toml");
        p.add_own("acme", "Cargo.toml");
        p
    }

    #[test]
    fn a_skill_naming_only_present_tech_scores_high() {
        let r = score("x", "user", "Rust crates and their tests", &rust_profile());
        assert_eq!(r.percent, 95);
        // `testing` is general: a specific key decides.
        assert_eq!(r.matched, ["rust"]);
        let g = score("t", "user", "Run the tests before CI", &rust_profile());
        // testing 1.0 + ci 0 → 50 + 40 × 0.5 = 70.
        assert_eq!(g.percent, 70);
        let d = score(
            "dotnet-trace",
            "user",
            "Collect traces in Docker for .NET",
            &rust_profile(),
        );
        assert_eq!(
            (d.percent, d.missing.as_slice()),
            (FLOOR, ["csharp".to_string()].as_slice())
        );
    }

    #[test]
    fn a_skill_naming_absent_tech_scores_the_floor() {
        let r = score("figma-use", "user", "Edit Figma files", &rust_profile());
        assert_eq!(r.percent, FLOOR);
        assert_eq!(r.missing, ["figma"]);
    }

    #[test]
    fn a_mixed_skill_scores_by_strength_share() {
        // rust 1.0 + swift 0 → 5 + 90 × 0.5 = 50; python 0.4 alone → 5 + 36 = 41.
        assert_eq!(
            score("m", "user", "Rust and Swift", &rust_profile()).percent,
            50
        );
        assert_eq!(
            score("p", "user", "Python scripts", &rust_profile()).percent,
            41
        );
    }

    #[test]
    fn a_generic_skill_is_neutral_and_deps_count() {
        assert_eq!(
            score("g", "user", "Write good emails", &rust_profile()).percent,
            GENERIC
        );
        let r = score("s", "user", "serde derive tips", &rust_profile());
        // 5 + 90 × 0.7 = 68.
        assert_eq!(
            (r.percent, r.matched.as_slice()),
            (68, ["serde".to_string()].as_slice())
        );
        // The project's own name counts in full.
        assert_eq!(
            score("acme", "user", "acme workflows", &rust_profile()).percent,
            95
        );
        // A named technology the project lacks outweighs a dependency word.
        assert_eq!(
            score("d", "user", "serde for .NET", &rust_profile()).percent,
            FLOOR
        );
    }

    #[test]
    fn frontmatter_reads_inline_and_folded_descriptions() {
        let (n, d) = frontmatter("---\nname: a\ndescription: >\n  one\n  two\n---\nbody");
        assert_eq!((n.as_deref(), d.as_str()), (Some("a"), "one two"));
        let (_, d) = frontmatter("---\ndescription: \"x y\"\n---\n");
        assert_eq!(d, "x y");
    }

    #[test]
    fn imports_skip_relative_and_std() {
        assert_eq!(
            imports("rust", "use std::io;\nuse tokio::fs;\nuse crate::x;"),
            ["tokio"]
        );
        assert_eq!(
            imports(
                "typescript",
                "import x from './a';\nimport { y } from '@tanstack/query';"
            ),
            ["query"]
        );
        assert_eq!(
            imports("python", "import os.path\nfrom httpx import Client"),
            ["httpx", "os"]
        );
    }

    #[test]
    fn profile_reads_manifests_tests_and_ci() {
        let d = &crate::testutil::tmp_dir("skills-audit-profile");
        std::fs::create_dir_all(d.join(".github/workflows")).unwrap();
        std::fs::create_dir_all(d.join("tests")).unwrap();
        std::fs::write(
            d.join("Cargo.toml"),
            "[package]\nname='a'\n[dependencies]\nclap='4'\n",
        )
        .unwrap();
        std::fs::write(
            d.join("package.json"),
            r#"{"devDependencies":{"react":"18"}}"#,
        )
        .unwrap();
        std::fs::write(
            d.join("main.ts"),
            "import { z } from 'zod';\nimport x from 'vue';\n",
        )
        .unwrap();
        let p = profile(d);
        for k in [
            "rust",
            "javascript",
            "react",
            "ci",
            "github",
            "testing",
            "dep:clap",
            "typescript",
            "vue",
        ] {
            assert!(p.signals.contains_key(k), "{k}");
        }
        assert!(
            !p.signals.contains_key("dep:zod"),
            "imports are not dep keys"
        );
        assert_eq!(p.languages.get("typescript"), Some(&1));
    }
}

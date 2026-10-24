// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok review` (see `src/review/README.md`).

use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::batch::Provider;
use crate::config::Config;
use crate::review::{ContextMode, Options, build, provider, report};

#[derive(clap::Args)]
pub(super) struct ReviewArgs {
    /// `git diff` this range (`main...HEAD`, `HEAD~1`); default: stdin when piped, else `git diff HEAD`
    #[arg(long, conflicts_with = "diff")]
    range: Option<String>,
    /// Read the unified diff from this file (`-` = stdin)
    #[arg(long)]
    diff: Option<PathBuf>,
    /// Stop adding context once the prompt reaches this many estimated tokens
    #[arg(long)]
    budget: Option<u32>,
    /// Related files kept at most
    #[arg(long, default_value_t = 12)]
    max_related: usize,
    /// Context next to the diff: digests (default), none or full files (baselines)
    #[arg(long, value_enum, default_value_t = ContextMode::Digest)]
    context: ContextMode,
    /// Build the prompt and report its size, call no model
    #[arg(long)]
    dry_run: bool,
    /// Print the prompt itself (implies --dry-run)
    #[arg(long)]
    print_prompt: bool,
    /// JSON instead of text
    #[arg(long)]
    json: bool,
    /// Reviewer provider; default: the first of ANTHROPIC_API_KEY / OPENAI_API_KEY that is set
    #[arg(long, value_enum)]
    provider: Option<Provider>,
    /// Model id (default per provider, or RTOK_REVIEW_MODEL)
    #[arg(long)]
    model: Option<String>,
    /// API base URL (default per provider, or RTOK_REVIEW_BASE_URL)
    #[arg(long)]
    base_url: Option<String>,
}

fn read_diff(root: &std::path::Path, a: &ReviewArgs) -> Result<String> {
    let mut s = String::new();
    match (&a.diff, &a.range) {
        (Some(p), _) if p.as_os_str() == "-" => {
            std::io::stdin().read_to_string(&mut s)?;
        }
        (Some(p), _) => {
            s = std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?
        }
        (None, Some(r)) => s = git_diff(root, r)?,
        (None, None) if !std::io::stdin().is_terminal() => {
            std::io::stdin().read_to_string(&mut s)?;
        }
        (None, None) => s = git_diff(root, "HEAD")?,
    }
    Ok(s)
}

fn git_diff(root: &std::path::Path, range: &str) -> Result<String> {
    let out = crate::plugins::graph::git_stdout_result(
        root,
        &["diff", "--no-color", "--no-ext-diff", range, "--"],
    )?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

pub(super) fn run(config_file: &Option<PathBuf>, a: ReviewArgs) -> Result<()> {
    let cfg = Config::load_with(config_file.as_deref(), None)?;
    let rt = crate::plugin::Runtime::open(cfg, "review")?;
    let cx = crate::plugin::Ctx::new(&rt);
    let root = crate::plugins::graph::cli_root(None)?;
    let text = read_diff(&root, &a)?;
    if crate::review::diff::parse(&text).is_empty() {
        println!("no changes");
        return Ok(());
    }
    let opts = Options {
        budget: a.budget,
        max_related: a.max_related,
        mode: a.context,
    };
    let b = build(&cx, &root, &text, &opts)?;
    if a.print_prompt {
        print!("{}", b.prompt.text);
        eprintln!(
            "rtok review: ~{} tokens ({} diff, {} context)",
            b.prompt.tokens, b.prompt.diff_tokens, b.prompt.context_tokens
        );
        return Ok(());
    }
    let summary = |b: &crate::review::Built| {
        format!(
            "rtok review: {} file(s), {} related, ~{} tokens ({} diff, {} context), {} dropped by budget\n",
            b.files.len(),
            b.related.len(),
            b.prompt.tokens,
            b.prompt.diff_tokens,
            b.prompt.context_tokens,
            b.prompt.dropped.len()
        )
    };
    if a.dry_run {
        if a.json {
            println!("{}", serde_json::to_string_pretty(&report(&b, None))?);
        } else {
            print!("{}", summary(&b));
        }
        return Ok(());
    }
    let p = a
        .provider
        .or_else(provider::detect)
        .context("no reviewer: set ANTHROPIC_API_KEY or OPENAI_API_KEY, or use --dry-run")?;
    let model = a
        .model
        .clone()
        .or_else(|| std::env::var("RTOK_REVIEW_MODEL").ok())
        .unwrap_or_else(|| provider::default_model(p).to_string());
    let base = a
        .base_url
        .clone()
        .or_else(|| std::env::var("RTOK_REVIEW_BASE_URL").ok())
        .unwrap_or_else(|| provider::default_base(p).to_string());
    let r = provider::Http::new(p, model, base)?;
    use provider::Reviewer;
    let out = r.review(&b.prompt.text)?;
    if a.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report(&b, Some((&r.name(), &out))))?
        );
    } else {
        print!("{}", summary(&b));
        println!("{out}");
    }
    Ok(())
}

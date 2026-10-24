// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Reviewer model call for `rtok review`. Providers and key variables are the ones
//! `rtok batch` uses ([`crate::batch::Provider`]); the key is read from the environment and
//! never echoed.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::batch::Provider;

/// A reviewer: prompt in, review text out. Tests plug in a fake.
pub trait Reviewer {
    fn name(&self) -> String;
    fn review(&self, prompt: &str) -> Result<String>;
}

/// Default model per provider; `--model` or `RTOK_REVIEW_MODEL` overrides it.
pub fn default_model(p: Provider) -> &'static str {
    match p {
        Provider::Anthropic => "claude-sonnet-4-5",
        Provider::Openai => "gpt-4.1-mini",
    }
}

/// Default API base per provider; `--base-url` or `RTOK_REVIEW_BASE_URL` overrides it.
pub fn default_base(p: Provider) -> &'static str {
    match p {
        Provider::Anthropic => "https://api.anthropic.com",
        Provider::Openai => "https://api.openai.com",
    }
}

/// The first provider whose key variable is set: Anthropic, then OpenAI.
pub fn detect() -> Option<Provider> {
    [Provider::Anthropic, Provider::Openai]
        .into_iter()
        .find(|p| std::env::var_os(p.key_env()).is_some_and(|v| !v.is_empty()))
}

/// An HTTP reviewer for one provider.
pub struct Http {
    pub provider: Provider,
    pub model: String,
    pub base: String,
    key: String,
}

impl Http {
    pub fn new(provider: Provider, model: String, base: String) -> Result<Self> {
        let key = std::env::var(provider.key_env())
            .ok()
            .filter(|k| !k.is_empty())
            .with_context(|| format!("{} is not set", provider.key_env()))?;
        if !(base.starts_with("https://") || base.starts_with("http://")) {
            bail!("base url must start with http:// or https://: {base}");
        }
        Ok(Self {
            provider,
            model,
            base: base.trim_end_matches('/').to_string(),
            key,
        })
    }
}

/// Request body for one provider (pure, for tests).
pub fn body(p: Provider, model: &str, prompt: &str) -> Value {
    match p {
        Provider::Anthropic => json!({
            "model": model, "max_tokens": 4096,
            "messages": [{"role": "user", "content": prompt}],
        }),
        Provider::Openai => json!({
            "model": model, "temperature": 0,
            "messages": [{"role": "user", "content": prompt}],
        }),
    }
}

/// Review text from one provider's response body.
pub fn text_of(p: Provider, v: &Value) -> Option<String> {
    match p {
        Provider::Anthropic => v["content"].as_array().map(|parts| {
            parts
                .iter()
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        }),
        Provider::Openai => v["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string),
    }
}

impl Reviewer for Http {
    fn name(&self) -> String {
        format!("{:?}/{}", self.provider, self.model).to_lowercase()
    }

    fn review(&self, prompt: &str) -> Result<String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;
        let rb = match self.provider {
            Provider::Anthropic => client
                .post(format!("{}/v1/messages", self.base))
                .header("x-api-key", &self.key)
                .header("anthropic-version", "2023-06-01"),
            Provider::Openai => client
                .post(format!("{}/v1/chat/completions", self.base))
                .bearer_auth(&self.key),
        };
        let resp = rb
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body(self.provider, &self.model, prompt).to_string())
            .send()?;
        let status = resp.status();
        let raw = resp.text()?;
        if !status.is_success() {
            let snippet: String = raw.chars().take(300).collect();
            bail!("{} returned {status}: {snippet}", self.name());
        }
        let v: Value = serde_json::from_str(&raw).context("provider response is not JSON")?;
        text_of(self.provider, &v).context("provider response has no text")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_and_replies_per_provider() {
        let b = body(Provider::Openai, "m", "hi");
        assert_eq!(b["messages"][0]["content"], "hi");
        let v = json!({"choices":[{"message":{"content":"ok"}}]});
        assert_eq!(text_of(Provider::Openai, &v).as_deref(), Some("ok"));
        let v = json!({"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]});
        assert_eq!(text_of(Provider::Anthropic, &v).as_deref(), Some("ab"));
        assert!(body(Provider::Anthropic, "m", "x")["max_tokens"].is_u64());
    }
}

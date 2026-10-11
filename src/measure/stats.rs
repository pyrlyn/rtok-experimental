// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `rtok stats` (plan T1.2): per-tool sizes, Bash families, MCP groups, CTT.
//!
//! Tool-result tokens use 4 chars/token (`research.md` §2 heuristic) so this report is
//! comparable to that baseline. Usage counters are the API numbers from the transcript.

use super::jsonl::{self, Parsed};
use crate::config::Config;
use crate::render::{Col, table};
use crate::store::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, SystemTime};

const CHARS_PER_TOKEN: f64 = 4.0;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SizeRow {
    pub count: u64,
    pub total_bytes: u64,
    pub mean: u64,
    pub p95: u64,
    pub max: u64,
    pub est_tokens: u64,
    pub ctt: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub sessions: u64,
    /// Transcript compaction events (`subtype=compact_boundary`), T58.2.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub compact: u64,
    /// Transcript sessions that already have a `checkpoint:*` or `session:*` note (T71.2).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub checkpoint: u64,
    /// Transcript sessions with no such note.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub no_checkpoint: u64,
    /// File stems of counted sessions; matched against notes. Not in JSON.
    #[serde(skip)]
    session_stems: Vec<String>,
    pub lines: u64,
    pub malformed: u64,
    pub tools: BTreeMap<String, SizeRow>,
    pub bash_families: BTreeMap<String, SizeRow>,
    /// T50.1: `formatter`, named `rule`, or `default` per Bash stem (transcripts).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bash_filter: BTreeMap<String, String>,
    /// T50.1: `cmd` measurements with `kind = rule` on stems still on [`Rule::default()`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bash_default_rule: BTreeMap<String, SizeRow>,
    pub mcp_groups: BTreeMap<String, SizeRow>,
    pub usage_input: u64,
    pub usage_cache_create: u64,
    pub usage_cache_read: u64,
    pub usage_output: u64,
    pub cache_hit_rate: f64,
    pub median_final_context: u64,
    /// Gate P5 replay: context-token-turns as recorded, and as they would be with the
    /// `archive` policy applied to every tool result (estimate: bytes/4, pointer = head + tail lines).
    #[serde(default)]
    pub ctt_total: u64,
    #[serde(default)]
    pub ctt_archive: u64,
    #[serde(default)]
    pub archive_candidates: u64,
    #[serde(default)]
    pub api: BTreeMap<String, ApiRow>,
    /// T385.6: the same counters per proxy lane. Present only once traffic ran off the agent
    /// lane, so a store of agent turns alone prints exactly what it did before lanes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lanes: BTreeMap<String, ApiRow>,
    /// T385.12.2: the same counters per lane and service tier (`bulk/flex`, `agent/-`). Absent
    /// with `lanes`, unless some response named a tier other than `standard` or `default`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lane_tiers: BTreeMap<String, ApiRow>,
    /// `Some` only for `rtok stats --price`: the default report is byte-identical
    /// with and without the price table (T49.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostReport>,
    /// Absent from `--json` when no session edited anything, so the T15.11 goldens hold.
    #[serde(default, skip_serializing_if = "EditRow::is_empty")]
    pub edits: EditRow,
    /// T58.1: native Read of a path already read in-session with Edit/Write/MultiEdit
    /// of that path in between. Absent when none, so the goldens hold.
    #[serde(default, skip_serializing_if = "ReadDeltaRow::is_empty")]
    pub read_delta: ReadDeltaRow,
    /// T136: whole-file Reads an outline could have answered. Absent when none.
    #[serde(default, skip_serializing_if = "ReadWholeRow::is_empty")]
    pub read_whole: ReadWholeRow,
    /// T383: native Reads the read hook denied, and what followed. Absent when none.
    #[serde(default, skip_serializing_if = "ReadDenyRow::is_empty")]
    pub read_deny: ReadDenyRow,
    /// T65.1: tool_result bytes whose SHA-256 matches an earlier result in the
    /// same session. Absent when none, so the goldens hold.
    #[serde(default, skip_serializing_if = "RepeatRow::is_empty")]
    pub repeat: RepeatRow,
    /// T176: expands of an id this session was shown. Absent when none, so the goldens hold.
    #[serde(default, skip_serializing_if = "ExpandAfterRow::is_empty")]
    pub expand_after: ExpandAfterRow,
    /// T177: unmarked Bash bytes, "below size gate" vs "no rule matched". Absent when
    /// none, so the goldens hold.
    #[serde(default, skip_serializing_if = "BashUnmarkedRow::is_empty")]
    pub bash_unmarked: BashUnmarkedRow,
    /// T125: assistant thinking blocks in the session.
    #[serde(default, skip_serializing_if = "ThinkingRow::is_empty")]
    pub thinking: ThinkingRow,
    /// T137: `image` content blocks. Absent when none, so the goldens hold.
    #[serde(default, skip_serializing_if = "ImageRow::is_empty")]
    pub images: ImageRow,
    /// T403: repeated long paths and identifiers, and the net a legend would save. Absent
    /// when nothing repeated.
    #[serde(
        default,
        skip_serializing_if = "super::dictionary::DictionaryRow::is_empty"
    )]
    pub dictionary: super::dictionary::DictionaryRow,
    /// T61.1: skill bodies the transcripts inject as `isMeta` records, per skill
    /// name. Absent when none, so the goldens hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<BTreeMap<String, SkillRow>>,
    /// T128: sub-agent transcripts under `<session>/subagents/`, attributed to their
    /// parent sessions (`super::subagents`). Absent when none, so the goldens hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagents: Option<super::subagents::Subagents>,
    /// T179: same-session repeat native `Read`s, by class. Absent when none.
    #[serde(default, skip_serializing_if = "RepeatReadsRow::is_empty")]
    pub repeat_reads: RepeatReadsRow,
}

/// What the model re-types to edit (plan T58.3). `old_string` is the span `Edit` and every
/// `MultiEdit.edits[]` entry quote back verbatim; `tool_input_bytes` is every `tool_use`
/// input serialized — the "tool input" denominator of `research.md` §2.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditRow {
    pub calls: u64,
    pub old_bytes: u64,
    pub new_bytes: u64,
    pub tool_input_bytes: u64,
}

impl EditRow {
    fn is_empty(&self) -> bool {
        self.calls == 0
    }
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Re-reads that a unified diff could shorten (plan T58.1). `bytes` is those
/// tool-result payloads; `read_bytes` is every native `Read` result in the same
/// window — the denominator of "share of Read bytes".
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadDeltaRow {
    pub calls: u64,
    pub bytes: u64,
    pub read_bytes: u64,
}

impl ReadDeltaRow {
    fn is_empty(&self) -> bool {
        self.calls == 0
    }
}

/// T136, the gate for I-82: native `Read`s with no `offset`/`limit` of a file `outline`
/// has a grammar for, at or above `[plugins.read] native_max_bytes`. `edited` of them
/// were followed by an Edit/Write/MultiEdit of the same path within `[plugins.guard]
/// window_turns` — those needed the body; the rest is what an outline could have answered.
/// `read_bytes` and `result_bytes` are every Read / every tool_result — the denominators.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadWholeRow {
    pub calls: u64,
    pub bytes: u64,
    pub edited: u64,
    pub edited_bytes: u64,
    pub read_bytes: u64,
    pub result_bytes: u64,
}

impl ReadWholeRow {
    fn is_empty(&self) -> bool {
        self.calls == 0
    }
}

/// T383: native `Read`s the read hook denied. `tokens` is the estimated size of the deny
/// reasons (the cost side of the net per deny). `then_mcp` / `then_native`: denies whose
/// path was read through rtok's MCP `read` / again by a native `Read` within the next
/// [`DENY_FOLLOW_CALLS`] tool calls; the rest were abandoned or went elsewhere.
/// `then_toolsearch` (T384): denies followed by a `ToolSearch` before any other `Read` or rtok MCP
/// call in that window, the extra turn a deny that names the exact tool is meant to remove.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadDenyRow {
    pub calls: u64,
    pub tokens: u64,
    pub then_mcp: u64,
    pub then_native: u64,
    #[serde(default)]
    pub then_toolsearch: u64,
}

impl ReadDenyRow {
    fn is_empty(&self) -> bool {
        self.calls == 0
    }
}

/// Content-hash repeats within a session (plan T65.1). `bytes` is those
/// tool-result payloads; `result_bytes` is every tool_result in the same
/// window — the denominator of "share of result bytes".
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepeatRow {
    pub calls: u64,
    pub bytes: u64,
    pub result_bytes: u64,
}

impl RepeatRow {
    fn is_empty(&self) -> bool {
        self.calls == 0
    }
}

/// One class's calls and bytes in a [`RepeatReadsRow`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassCounts {
    pub calls: u64,
    pub bytes: u64,
}

/// T179: repeat native `Read`s of a path already read this session, by why
/// `read/dedup`+`read/delta` missed them: `fired` (marker present), `changed`
/// (Edit/Write/MultiEdit/Bash touched the path since), `ranged` (`offset`/`limit`
/// differ), `subagent` (another agent's earlier read, T127), `hook_absent` (no
/// marker anywhere this session — `rtok` likely off `PATH`), else `declined`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepeatReadsRow {
    pub fired: ClassCounts,
    pub changed: ClassCounts,
    pub ranged: ClassCounts,
    pub subagent: ClassCounts,
    pub hook_absent: ClassCounts,
    pub declined: ClassCounts,
}

impl RepeatReadsRow {
    fn is_empty(&self) -> bool {
        self.fired.calls == 0
            && self.changed.calls == 0
            && self.ranged.calls == 0
            && self.subagent.calls == 0
            && self.hook_absent.calls == 0
            && self.declined.calls == 0
    }
}

/// T176: `expand` calls on an id rtok printed earlier in the same session — the agent
/// needed what a cut dropped. `bytes` is those expand results; `shown_bytes` is the
/// results that first named each id, counted once per id.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandAfterRow {
    pub calls: u64,
    pub bytes: u64,
    pub shown_bytes: u64,
}

impl ExpandAfterRow {
    fn is_empty(&self) -> bool {
        self.calls == 0
    }
}

/// T177: `cmd` measurements that carry no rtok marker (`kind = raw` or `unmatched`),
/// split by why. `gate_bytes` never reached a rule — a tiny body below the trailer gate
/// or a T176 bounded passthrough, both by design. `unmatched_bytes` reached a picked
/// rule/formatter and shrank nothing — the actionable share the 2026-09-22 audit named
/// (large `cat`/`sed`/`grep` dumps, newline-joined scripts).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BashUnmarkedRow {
    pub gate_calls: u64,
    pub gate_bytes: u64,
    pub unmatched_calls: u64,
    pub unmatched_bytes: u64,
}

impl BashUnmarkedRow {
    fn is_empty(&self) -> bool {
        self.gate_calls == 0 && self.unmatched_calls == 0
    }
}

/// T125: thinking blocks (type="thinking" or type="redacted_thinking") in assistant messages.
/// `blocks` is the count of thinking content blocks; `bytes` is their total thinking text/data.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingRow {
    pub blocks: u64,
    pub bytes: u64,
}

impl ThinkingRow {
    fn is_empty(&self) -> bool {
        self.blocks == 0
    }
}

/// T137: `image` blocks in tool results and user messages. `tokens` uses
/// [`super::image::tokens`] (the provider's published formula); `resident` multiplies each
/// block's tokens by the API requests at or after its turn, like [`SkillRow`]'s. `no_size`
/// blocks are not PNG/JPEG base64 and add no tokens. `by_source` keys by tool name, `user`
/// for images pasted into a prompt.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRow {
    pub blocks: u64,
    pub bytes: u64,
    pub tokens: u64,
    pub resident: u64,
    pub no_size: u64,
    pub by_source: BTreeMap<String, ImageSource>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSource {
    pub blocks: u64,
    pub bytes: u64,
    pub tokens: u64,
}

impl ImageRow {
    fn is_empty(&self) -> bool {
        self.blocks == 0
    }
}

/// T61.1: one skill's injected bodies — the `isMeta` user records keyed to that
/// skill's `Skill` tool_use. `resident` is the number the context actually carried:
/// body bytes × the API requests of the session that came after the injection.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SkillRow {
    pub count: u64,
    pub bytes: u64,
    pub mean: u64,
    pub p95: u64,
    pub max: u64,
    pub est_tokens: u64,
    pub resident: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ApiRow {
    pub input: i64,
    pub cache_create: i64,
    pub cache_read: i64,
    pub output: i64,
    pub hit: f64,
}

/// One model's USD costs (`rtok stats --price`, T49.1). `cost`/`saved` are
/// `None` for models without a `[stats.prices]` entry: their token counts still
/// print, but no dollar figure is guessed.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CostRow {
    pub input: i64,
    pub cache_create: i64,
    pub cache_read: i64,
    pub output: i64,
    /// USD at the model's `$` per MTok row.
    pub cost: Option<f64>,
    /// USD the cache reads saved versus uncached input price.
    pub saved: Option<f64>,
}

/// USD costs over the proxy `usage` rows (`rtok stats --price`, T49.1). Models
/// without a `[stats.prices]` entry are named in `unknown` and priced nowhere —
/// never by guess.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CostReport {
    pub models: BTreeMap<String, CostRow>,
    pub unknown: Vec<String>,
    pub total_cost: f64,
    pub total_saved: f64,
    /// T385.10: USD per proxy lane, each lane costed at the models that served it. Empty — and
    /// absent from JSON — while every request ran on the agent lane.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lanes: BTreeMap<String, LaneCost>,
}

/// One lane's USD (`rtok stats --price`, T385.10).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LaneCost {
    /// USD over the lane's priced models.
    pub cost: f64,
    /// True when some model on the lane has no `[stats.prices]` row, so `cost` is a floor.
    pub partial: bool,
}

/// The config knobs the fold needs, so `collect` stays usable without a `Config`: the
/// `[plugins.archive]` replay, and the T136 `read_whole` thresholds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Replay {
    pub keep_turns: u64,
    pub min_tokens: u64,
    pub head_lines: usize,
    pub tail_lines: usize,
    /// `[plugins.read] native_max_bytes`: the size above which the hook denies a native Read.
    pub read_whole_min: u64,
    /// `[plugins.guard] window_turns`: an Edit this close after a Read needed its body.
    pub edit_window_turns: u64,
}

impl Replay {
    pub fn from_cfg(cfg: &Config) -> Self {
        let a = &cfg.plugins.archive;
        Self {
            keep_turns: u64::from(a.keep_turns),
            min_tokens: u64::from(a.min_tokens),
            head_lines: a.head_lines as usize,
            tail_lines: a.tail_lines as usize,
            read_whole_min: cfg.plugins.read.native_max_bytes,
            edit_window_turns: u64::from(cfg.plugins.guard.window_turns),
        }
    }
}

impl Report {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn to_table(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "sessions {}  compact {}  checkpoint {}  no_checkpoint {}  lines {}  malformed {}\n",
            self.sessions,
            self.compact,
            self.checkpoint,
            self.no_checkpoint,
            self.lines,
            self.malformed
        ));
        s.push_str(&format!(
            "usage input={} cache_create={} cache_read={} output={}  hit={:.1}%  median_context={}\n",
            self.usage_input,
            self.usage_cache_create,
            self.usage_cache_read,
            self.usage_output,
            self.cache_hit_rate * 100.0,
            self.median_final_context
        ));
        if let Some(sa) = &self.subagents {
            s.push_str(&format!(
                "sub-agents {} sessions / {} agents  results {} B vs parents {} B ({:.1}% of the tree)\n",
                sa.sessions,
                sa.count,
                sa.result_bytes,
                sa.parent_result_bytes,
                pct(sa.result_bytes, sa.result_bytes + sa.parent_result_bytes)
            ));
            s.push_str(&format!(
                "  reads {} B ({:.1}% of results)  re-reads: parent {} B, sibling {} B\n",
                sa.read_bytes,
                pct(sa.read_bytes, sa.result_bytes),
                sa.read_parent,
                sa.read_sibling
            ));
            s.push_str(&format!(
                "  re-read {} B = {:.1}% of reads, {:.1}% of results, {:.1}% of the tree\n",
                sa.reread_bytes, sa.share_read, sa.share_results, sa.share_tree
            ));
            s.push_str(&format!(
                "  usage input={} cache_read={} cache_write={} output={}\n",
                sa.usage_input, sa.usage_cache_read, sa.usage_cache_write, sa.usage_output
            ));
            if sa.with_brief.count > 0 || sa.without_brief.count > 0 {
                s.push_str(&format!(
                    "  brief: {} agents  re-read {} B ({:.1}% of reads)  input={}\n",
                    sa.with_brief.count,
                    sa.with_brief.reread_bytes,
                    sa.with_brief.share_read,
                    sa.with_brief.usage_input
                ));
                s.push_str(&format!(
                    "  no brief: {} agents  re-read {} B ({:.1}% of reads)  input={}\n",
                    sa.without_brief.count,
                    sa.without_brief.reread_bytes,
                    sa.without_brief.share_read,
                    sa.without_brief.usage_input
                ));
            }
            for r in &sa.by_type {
                s.push_str(&format!(
                    "  {} | {}  {} agents  results {} B  reads {} B  re-read {} B\n",
                    r.agent_type, r.model, r.count, r.result_bytes, r.read_bytes, r.reread_bytes
                ));
            }
        }
        s.push_str(&api_table("api", &self.api));
        s.push_str(&api_table("lane", &self.lanes));
        s.push_str(&api_table("lane/tier", &self.lane_tiers));
        if self.ctt_total > 0 {
            let pct =
                100.0 * (self.ctt_total as f64 - self.ctt_archive as f64) / self.ctt_total as f64;
            s.push_str(&format!(
                "archive replay (estimate) ctt {} → {}  -{pct:.1}%  over {} results\n",
                self.ctt_total, self.ctt_archive, self.archive_candidates
            ));
        }
        if let Some(cost) = &self.cost {
            s.push_str(&cost.to_table());
        }
        if self.edits.calls > 0 {
            let e = &self.edits;
            s.push_str(&format!(
                "edit calls {}  old_string {} B  new_string {} B  old/tool_input {:.1}%  old/output_tokens (est) {:.1}%\n",
                e.calls,
                e.old_bytes,
                e.new_bytes,
                pct(e.old_bytes, e.tool_input_bytes),
                pct(est_tokens(e.old_bytes), self.usage_output)
            ));
        }
        if self.read_delta.calls > 0 {
            let d = &self.read_delta;
            s.push_str(&format!(
                "read delta calls {}  bytes {}  of Read bytes {}  {:.1}%\n",
                d.calls,
                d.bytes,
                d.read_bytes,
                pct(d.bytes, d.read_bytes)
            ));
        }
        if self.read_whole.calls > 0 {
            let d = &self.read_whole;
            let plain = d.bytes - d.edited_bytes;
            s.push_str(&format!(
                "read whole calls {}  bytes {}  {:.1}% of Read  {:.1}% of results  \
                 not edited {} bytes {:.1}% of results  edited after {}\n",
                d.calls,
                d.bytes,
                pct(d.bytes, d.read_bytes),
                pct(d.bytes, d.result_bytes),
                d.calls - d.edited,
                pct(plain, d.result_bytes),
                d.edited
            ));
        }
        if self.read_deny.calls > 0 {
            let d = &self.read_deny;
            s.push_str(&format!(
                "read deny calls {}  tokens {}  then rtok read {}  then native Read {}  then ToolSearch {}\n",
                d.calls, d.tokens, d.then_mcp, d.then_native, d.then_toolsearch
            ));
        }
        if !self.repeat_reads.is_empty() {
            let r = &self.repeat_reads;
            let classes = [
                ("fired", r.fired),
                ("changed", r.changed),
                ("ranged", r.ranged),
                ("subagent", r.subagent),
                ("hook_absent", r.hook_absent),
                ("declined", r.declined),
            ];
            let total: u64 = classes.iter().map(|(_, c)| c.calls).sum();
            s.push_str(&format!("repeat reads  total {total}\n"));
            for (name, c) in classes {
                s.push_str(&format!("  {name} calls {} bytes {}\n", c.calls, c.bytes));
            }
        }
        if self.thinking.blocks > 0 {
            let est_toks = est_tokens(self.thinking.bytes);
            // Share of session input: uncached + cache writes + cache reads (research.md §2).
            let input = self.usage_input + self.usage_cache_create + self.usage_cache_read;
            let share = if input > 0 {
                100.0 * est_toks as f64 / input as f64
            } else {
                0.0
            };
            s.push_str(&format!(
                "thinking blocks {} bytes {} est. tokens {} {:.4}% of session input\n",
                self.thinking.blocks, self.thinking.bytes, est_toks, share
            ));
        }
        if self.images.blocks > 0 {
            let d = &self.images;
            let input = self.usage_input + self.usage_cache_create + self.usage_cache_read;
            s.push_str(&format!(
                "images blocks {}  bytes {}  est. tokens {}  resident {}  {:.2}% of session input  no_size {}\n",
                d.blocks, d.bytes, d.tokens, d.resident, pct(d.resident, input), d.no_size
            ));
            for (src, r) in &d.by_source {
                s.push_str(&format!(
                    "  images {src}  blocks {}  bytes {}  est. tokens {}\n",
                    r.blocks, r.bytes, r.tokens
                ));
            }
        }
        if !self.dictionary.is_empty() {
            let d = &self.dictionary;
            // Bytes to tokens at the report's chars-per-token, so a byte saving is compared
            // with the provider's token count (the shares are bytes saved, not tokens).
            let input = d.request_tokens.saturating_mul(CHARS_PER_TOKEN as u64);
            s.push_str(&format!(
                "dictionary requests {}  content bytes {}  billed input est. bytes {}\n",
                d.requests, d.input_bytes, input
            ));
            for (name, k) in [("paths", d.paths), ("idents", d.idents)] {
                s.push_str(&format!(
                    "  {name} repeated {} ({:.2}% of billed)  cache-safe saving {} ({:.2}% of billed, {:.2}% of content)\n",
                    k.repeated,
                    pct(k.repeated, input),
                    k.saved,
                    pct(k.saved, input),
                    pct(k.saved, d.input_bytes)
                ));
            }
        }
        if self.repeat.calls > 0 {
            let d = &self.repeat;
            s.push_str(&format!(
                "repeat calls {}  bytes {}  of result bytes {}  {:.1}%\n",
                d.calls,
                d.bytes,
                d.result_bytes,
                pct(d.bytes, d.result_bytes)
            ));
        }
        if self.expand_after.calls > 0 {
            let d = &self.expand_after;
            s.push_str(&format!(
                "expand right after  calls {}  bytes {}  shown bytes {}\n",
                d.calls, d.bytes, d.shown_bytes
            ));
        }
        s.push_str(&format_section("tool", &self.tools));
        s.push_str(&format_bash_section(&self.bash_families, &self.bash_filter));
        if !self.bash_default_rule.is_empty() {
            s.push_str(&format_section("bash_default", &self.bash_default_rule));
        }
        if !self.bash_unmarked.is_empty() {
            let d = &self.bash_unmarked;
            let total = d.gate_bytes + d.unmatched_bytes;
            s.push_str(&format!(
                "bash unmarked  below_gate calls {} bytes {} {:.1}%  no_rule calls {} bytes {} {:.1}%\n",
                d.gate_calls,
                d.gate_bytes,
                pct(d.gate_bytes, total),
                d.unmatched_calls,
                d.unmatched_bytes,
                pct(d.unmatched_bytes, total)
            ));
        }
        s.push_str(&format_section("mcp", &self.mcp_groups));
        if let Some(skills) = &self.skills {
            s.push_str(&skills_section(skills));
        }
        s
    }
}

/// The seven numeric size columns every `SizeRow` table prints, shared by the bash
/// and titled formatters.
fn numeric_cells(r: &SizeRow) -> [String; 7] {
    [
        r.count.to_string(),
        r.total_bytes.to_string(),
        r.mean.to_string(),
        r.p95.to_string(),
        r.max.to_string(),
        r.est_tokens.to_string(),
        r.ctt.to_string(),
    ]
}

fn format_bash_section(
    rows: &BTreeMap<String, SizeRow>,
    kinds: &BTreeMap<String, String>,
) -> String {
    let cols = [
        Col::left(24),
        Col::left(8),
        Col::right(7),
        Col::right(12),
        Col::right(8),
        Col::right(8),
        Col::right(8),
        Col::right(12),
        Col::right(12),
    ];
    let mut out = vec![vec![
        "bash".to_string(),
        "filter".into(),
        "count".into(),
        "bytes".into(),
        "mean".into(),
        "p95".into(),
        "max".into(),
        "est_tokens".into(),
        "ctt".into(),
    ]];
    for (name, r) in rows {
        let mut row = vec![
            name.clone(),
            kinds
                .get(name)
                .cloned()
                .unwrap_or_else(|| "default".to_string()),
        ];
        row.extend(numeric_cells(r));
        out.push(row);
    }
    table(&cols, &out)
}

/// T50.1: label each transcript Bash family and rank default-rule `cmd` savings.
/// Without the `cmd` plugin there are no rules to label a family against, so the
/// report keeps the families and leaves the filter column empty (T0.4: one plugin
/// feature must build alone).
#[cfg(not(feature = "cmd"))]
pub fn attach_bash_cmd(_report: &mut Report, _store: &Store) -> Result<()> {
    Ok(())
}

/// T50.1: label each transcript Bash family and rank default-rule `cmd` savings.
#[cfg(feature = "cmd")]
pub fn attach_bash_cmd(report: &mut Report, store: &Store) -> Result<()> {
    let settings = crate::plugins::cmd::rules::Settings::builtin();
    for name in report.bash_families.keys() {
        let kind = crate::plugins::cmd::formatters::filter_kind(&settings, name);
        report.bash_filter.insert(name.clone(), kind.to_string());
    }
    let mut samples: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for r in store.list_measurements("cmd")? {
        // T177: every raw/unmatched measurement's bytes, split by why there is no
        // marker — independent of the per-family `bash_default_rule` ranking below.
        match r.kind.as_str() {
            "raw" => {
                report.bash_unmarked.gate_calls += 1;
                report.bash_unmarked.gate_bytes += r.before_bytes.max(0) as u64;
            }
            "unmatched" => {
                report.bash_unmarked.unmatched_calls += 1;
                report.bash_unmarked.unmatched_bytes += r.before_bytes.max(0) as u64;
            }
            _ => {}
        }
        if r.kind != "rule" {
            continue;
        }
        let fam = r
            .ref_id
            .as_deref()
            .and_then(|id| id.split(':').next())
            .unwrap_or("");
        if fam.is_empty()
            || crate::plugins::cmd::formatters::filter_kind(&settings, fam) != "default"
        {
            continue;
        }
        let bytes = r.after_bytes.max(0) as u64;
        add(
            &mut report.bash_default_rule,
            &mut samples,
            fam,
            bytes,
            est_tokens(bytes),
            0,
        );
    }
    finish_rows(&mut report.bash_default_rule, &mut samples);
    Ok(())
}
fn format_section(title: &str, rows: &BTreeMap<String, SizeRow>) -> String {
    // The section's own title sits in the first column of its header line; the fixed
    // widths are floors now (`render::table`, T25.2), bytes unchanged.
    let cols = [
        Col::left(24),
        Col::right(7),
        Col::right(12),
        Col::right(8),
        Col::right(8),
        Col::right(8),
        Col::right(12),
        Col::right(12),
    ];
    let mut out = vec![vec![
        title.to_string(),
        "count".into(),
        "bytes".into(),
        "mean".into(),
        "p95".into(),
        "max".into(),
        "est_tokens".into(),
        "ctt".into(),
    ]];
    for (name, r) in rows {
        let mut row = vec![name.clone()];
        row.extend(numeric_cells(r));
        out.push(row);
    }
    table(&cols, &out)
}

/// T61.1: the injected skill bodies, one row per skill, `resident` = the bytes the
/// later API requests of the same session actually carried.
fn skills_section(skills: &BTreeMap<String, SkillRow>) -> String {
    let cols = [
        Col::left(24),
        Col::right(7),
        Col::right(12),
        Col::right(8),
        Col::right(8),
        Col::right(8),
        Col::right(12),
        Col::right(14),
    ];
    let mut out = vec![vec![
        "skill".to_string(),
        "count".into(),
        "bytes".into(),
        "mean".into(),
        "p95".into(),
        "max".into(),
        "est_tokens".into(),
        "resident".into(),
    ]];
    for (name, r) in skills {
        out.push(vec![
            name.clone(),
            r.count.to_string(),
            r.bytes.to_string(),
            r.mean.to_string(),
            r.p95.to_string(),
            r.max.to_string(),
            r.est_tokens.to_string(),
            r.resident.to_string(),
        ]);
    }
    table(&cols, &out)
}

/// The `api` / `lane` counters table; empty input prints nothing.
fn api_table(label: &str, rows: &BTreeMap<String, ApiRow>) -> String {
    if rows.is_empty() {
        return String::new();
    }
    // The old fixed widths ride along as column floors, so the bytes a golden
    // pinned do not move (T25.2 moved the padding into `render::table`).
    let cols = [
        Col::left(24),
        Col::right(8),
        Col::right(12),
        Col::right(10),
        Col::right(6),
        Col::right(6),
    ];
    let mut out = vec![vec![
        label.into(),
        "input".into(),
        "cache_create".into(),
        "cache_read".into(),
        "output".into(),
        "hit".into(),
    ]];
    for (name, r) in rows {
        out.push(vec![
            name.clone(),
            r.input.to_string(),
            r.cache_create.to_string(),
            r.cache_read.to_string(),
            r.output.to_string(),
            format!("{:.1}%", r.hit * 100.0),
        ]);
    }
    table(&cols, &out)
}

pub use crate::since::{parse_since, parse_since_from};

/// The counters of one bucket with its prompt-cache hit rate: cache reads over every input
/// token the provider billed (uncached, written and read).
fn api_row(input: i64, cache_create: i64, cache_read: i64, output: i64) -> ApiRow {
    let denom = cache_read
        .saturating_add(cache_create)
        .saturating_add(input);
    let hit = if denom == 0 {
        0.0
    } else {
        cache_read as f64 / denom as f64
    };
    ApiRow {
        input,
        cache_create,
        cache_read,
        output,
        hit,
    }
}

pub fn attach_api(report: &mut Report, store: &Store) -> Result<()> {
    for row in store.usage_by_api()? {
        report.api.insert(
            row.api,
            api_row(row.input, row.cache_create, row.cache_read, row.output),
        );
    }
    Ok(())
}

/// Counters summed over several store rows, keyed the way the tables print them.
fn fold_rows(rows: impl Iterator<Item = (String, [i64; 4])>) -> BTreeMap<String, ApiRow> {
    let mut sums: BTreeMap<String, [i64; 4]> = BTreeMap::new();
    for (key, legs) in rows {
        let sum = sums.entry(key).or_default();
        for (total, leg) in sum.iter_mut().zip(legs) {
            *total += leg;
        }
    }
    sums.into_iter()
        .map(|(key, [input, create, read, output])| (key, api_row(input, create, read, output)))
        .collect()
}

/// The per-lane and per-lane-and-tier counters (T385.6, T385.12.2), `lane/tier` keyed; a call
/// whose response named no tier prints `-`. Shared by `rtok stats` and `rtok report`.
pub fn lane_rows(store: &Store) -> Result<(BTreeMap<String, ApiRow>, BTreeMap<String, ApiRow>)> {
    let rows = store.usage_by_lane_tier()?;
    let legs = |r: &rtok_store::LaneUsage| [r.input, r.cache_create, r.cache_read, r.output];
    let lane = |r: &rtok_store::LaneUsage| crate::lane::lane_of_kind(&r.kind).to_string();
    let lanes = fold_rows(rows.iter().map(|r| (lane(r), legs(r))));
    let tiers = fold_rows(rows.iter().map(|r| {
        let tier = r.tier.as_deref().unwrap_or("-");
        (format!("{}/{tier}", lane(r)), legs(r))
    }));
    // Both tables stay empty while every request ran on the agent lane at the ordinary tier:
    // the `api` table already says the same, and every Anthropic and OpenAI response names
    // `standard` or `default`, which must not add a table to an untouched setup.
    let off_agent = lanes.keys().any(|lane| lane != "agent");
    let tiered = rows.iter().any(|r| {
        r.tier
            .as_deref()
            .is_some_and(|t| !matches!(t, "standard" | "default"))
    });
    Ok((
        if off_agent { lanes } else { BTreeMap::new() },
        if off_agent || tiered {
            tiers
        } else {
            BTreeMap::new()
        },
    ))
}

/// Per-lane cache-hit rows (T385.6) and the lane/tier breakdown (T385.12.2).
pub fn attach_lanes(report: &mut Report, store: &Store) -> Result<()> {
    (report.lanes, report.lane_tiers) = lane_rows(store)?;
    Ok(())
}

/// Match counted transcript stems to `checkpoint:<id>` / `session:<id>` notes (T71.2).
pub fn attach_checkpoint_notes(report: &mut Report, store: &Store) -> Result<()> {
    let ids: std::collections::BTreeSet<String> =
        store.checkpoint_session_ids()?.into_iter().collect();
    let mut with = 0u64;
    for stem in &report.session_stems {
        if ids.contains(stem) {
            with += 1;
        }
    }
    report.checkpoint = with;
    report.no_checkpoint = report.sessions.saturating_sub(with);
    Ok(())
}

/// Codex CLI sessions as one more `api` row (T49.2), read from `dir` with the same `since`
/// window as the Claude Code transcripts. Absent dir or no `token_count` line → no row.
pub fn attach_codex(report: &mut Report, dir: &Path, since: Duration) {
    let cutoff = SystemTime::now()
        .checked_sub(since)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    if let Some(row) = super::codex::collect(dir, cutoff) {
        report.api.insert("codex".into(), row);
    }
}

/// USD for one model's counters at its `$` per MTok row: `(cost, saved)`.
/// `saved` is what the cache reads saved versus uncached input price — the only
/// saving computable from the `usage` rows alone (T49.1). Dust below a tenth of
/// a microdollar rounds away so JSON goldens stay exact.
pub fn row_cost(
    input: i64,
    cache_create: i64,
    cache_read: i64,
    output: i64,
    price: &crate::config::ModelPrice,
) -> (f64, f64) {
    let leg = |n: i64, rate: f64| n as f64 / 1e6 * rate;
    let cost = leg(input, price.input)
        + leg(cache_create, price.cache_write)
        + leg(cache_read, price.cache_read)
        + leg(output, price.output);
    let saved = leg(cache_read, (price.input - price.cache_read).max(0.0));
    let round = |v: f64| (v * 1e6).round() / 1e6;
    (round(cost), round(saved))
}

/// Price the store's per-model `usage` into `report.cost` (`rtok stats --price`,
/// T49.1). Models without a `[stats.prices]` entry land in `unknown` and stay
/// out of the totals.
pub fn attach_costs(
    report: &mut Report,
    store: &Store,
    prices: &BTreeMap<String, crate::config::ModelPrice>,
) -> Result<()> {
    let mut costs = CostReport::default();
    for row in store.usage_by_model_tier()? {
        let priced = prices.get(&row.model).map(|price| {
            row_cost(
                row.input,
                row.cache_create,
                row.cache_read,
                row.output,
                price,
            )
        });
        if priced.is_none() {
            costs.unknown.push(row.model.clone());
        }
        let (cost, saved) = priced.unzip();
        costs.total_cost += cost.unwrap_or(0.0);
        costs.total_saved += saved.unwrap_or(0.0);
        costs.models.insert(
            row.model,
            CostRow {
                input: row.input,
                cache_create: row.cache_create,
                cache_read: row.cache_read,
                output: row.output,
                cost,
                saved,
            },
        );
    }
    costs.unknown.sort();
    costs.unknown.dedup();
    let round = |v: f64| (v * 1e6).round() / 1e6;
    // Same rule as the lane counters: a store of agent turns alone prints what it always did.
    let lane_rows = store.usage_by_lane_model_tier()?;
    let lane_of = |kind: &Option<String>| {
        kind.as_deref()
            .map_or("agent", crate::lane::lane_of_kind)
            .to_string()
    };
    if lane_rows.iter().any(|r| lane_of(&r.kind) != "agent") {
        for row in &lane_rows {
            let lane = costs.lanes.entry(lane_of(&row.kind)).or_default();
            let u = &row.usage;
            match prices.get(&u.model) {
                Some(price) => {
                    lane.cost += row_cost(u.input, u.cache_create, u.cache_read, u.output, price).0;
                }
                None => lane.partial = true,
            }
        }
        for lane in costs.lanes.values_mut() {
            lane.cost = round(lane.cost);
        }
    }
    costs.total_cost = round(costs.total_cost);
    costs.total_saved = round(costs.total_saved);
    report.cost = Some(costs);
    Ok(())
}

impl CostReport {
    fn to_table(&self) -> String {
        let mut s = String::from("cost (USD at [stats.prices] $/MTok; `-` = no price row)\n");
        if self.models.is_empty() && self.unknown.is_empty() {
            s.push_str("  no usage rows\n");
            return s;
        }
        let cols = [
            Col::left(24),
            Col::right(12),
            Col::right(12),
            Col::right(12),
            Col::right(12),
            Col::right(10),
            Col::right(10),
        ];
        let mut rows = vec![vec![
            "model".into(),
            "input".into(),
            "cache_create".into(),
            "cache_read".into(),
            "output".into(),
            "cost".into(),
            "saved".into(),
        ]];
        for (model, r) in &self.models {
            let money = |v: Option<f64>| v.map_or_else(|| "-".into(), |v| format!("{v:.2}"));
            rows.push(vec![
                model.clone(),
                r.input.to_string(),
                r.cache_create.to_string(),
                r.cache_read.to_string(),
                r.output.to_string(),
                money(r.cost),
                money(r.saved),
            ]);
        }
        s.push_str(&table(&cols, &rows));
        s.push_str(&format!(
            "cost total ${:.2} (cache reads saved ${:.2}; {} model(s) without a price)\n",
            self.total_cost,
            self.total_saved,
            self.unknown.len()
        ));
        if !self.lanes.is_empty() {
            s.push_str("cost by lane (USD; `*` = excludes models without a price)\n");
            for (lane, c) in &self.lanes {
                let star = if c.partial { "*" } else { "" };
                s.push_str(&format!("  {lane:<12} {:>10.2}{star}\n", c.cost));
            }
        }
        s
    }
}

pub fn collect(dir: &Path, since: Duration, plugin: &str, replay: Replay) -> Result<Report> {
    let cutoff = SystemTime::now()
        .checked_sub(since)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut report = Report::default();
    let mut finals = Vec::new();
    let mut parents = Vec::new();
    let mut samples = RowSamples::default();
    // `read_dir` order varies run to run; the totals are order-free but the
    // bounded p95 reservoir is not, so walk in path order.
    let mut paths = super::codex::jsonl_paths(dir, cutoff);
    paths.sort();
    for p in paths {
        // T128: a sub-agent transcript is attributed to its parent below, never counted
        // as a session of its own.
        if super::subagents::is_subagent(&p) {
            continue;
        }
        // One unreadable transcript is one malformed entry, not the end of the report.
        let Ok(parsed) = jsonl::parse_path(&p) else {
            report.malformed += 1;
            continue;
        };
        report.compact += compact_events(&p);
        if let Some(stem) = p.file_stem().and_then(OsStr::to_str) {
            report.session_stems.push(stem.to_string());
        }
        fold_session(
            &parsed,
            plugin,
            replay,
            &mut report,
            &mut samples,
            &mut finals,
        );
        parents.push(super::subagents::Parent::new(&p, &parsed));
    }
    report.no_checkpoint = report.sessions;
    finish_rows(&mut report.tools, &mut samples.tools);
    finish_rows(&mut report.bash_families, &mut samples.bash);
    finish_rows(&mut report.mcp_groups, &mut samples.mcp);
    if let Some(skills) = report.skills.as_mut() {
        finish_skills(skills, &mut samples.skills);
    }
    let denom = report.usage_cache_read + report.usage_cache_create + report.usage_input;
    report.cache_hit_rate = if denom == 0 {
        0.0
    } else {
        report.usage_cache_read as f64 / denom as f64
    };
    finals.sort_unstable();
    report.median_final_context = if finals.is_empty() {
        0
    } else {
        finals[finals.len() / 2]
    };
    report.subagents = super::subagents::collect(&parents, cutoff);
    // T179 `subagent` class: invisible to `fold_repeat_reads`'s per-session `Parsed`.
    if let Some(sa) = &report.subagents {
        report.repeat_reads.subagent.calls += sa.reread_calls;
        report.repeat_reads.subagent.bytes += sa.reread_bytes;
    }
    Ok(report)
}

fn compact_events(path: &Path) -> u64 {
    let Ok(text) = std::fs::read_to_string(path) else {
        return 0;
    };
    text.lines().filter(|line| is_compact_line(line)).count() as u64
}

fn is_compact_line(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    super::jsonl::is_compact_boundary(&v)
}

fn fold_session(
    parsed: &Parsed,
    plugin: &str,
    replay: Replay,
    report: &mut Report,
    samples: &mut RowSamples,
    finals: &mut Vec<u64>,
) {
    report.sessions += 1;
    report.lines += parsed.lines;
    report.malformed += parsed.malformed;
    let n = u64::from(parsed.turns);
    let mut id_name: BTreeMap<&str, &str> = BTreeMap::new();
    let mut id_family: BTreeMap<&str, String> = BTreeMap::new();
    let mut id_skill: BTreeMap<&str, String> = BTreeMap::new();
    for u in &parsed.tool_uses {
        id_name.insert(u.id.as_str(), u.name.as_str());
        if u.name == "Bash"
            && let Some(cmd) = u.input.get("command").and_then(|v| v.as_str())
        {
            id_family.insert(u.id.as_str(), bash_family(cmd));
        }
        if u.name == "Skill"
            && let Some(skill) = u.input.get("skill").and_then(|v| v.as_str())
        {
            id_skill.insert(u.id.as_str(), skill.to_string());
        }
        fold_edits(&mut report.edits, u);
    }
    fold_read_delta(&mut report.read_delta, parsed);
    fold_read_whole(&mut report.read_whole, parsed, replay);
    fold_read_deny(&mut report.read_deny, parsed);
    fold_repeat(&mut report.repeat, parsed);
    fold_repeat_reads(&mut report.repeat_reads, parsed);
    fold_expand_after(&mut report.expand_after, parsed);
    for r in &parsed.tool_results {
        let name = id_name
            .get(r.tool_use_id.as_str())
            .copied()
            .unwrap_or("unknown");
        if !plugin.is_empty() && plugin != name && !name.contains(plugin) {
            continue;
        }
        let bytes = r.content.len() as u64;
        let tokens = est_tokens(bytes);
        let remain = n.saturating_sub(u64::from(r.turn));
        let ctt = tokens.saturating_mul(remain);
        let after = replay_ctt(&r.content, tokens, remain, replay);
        report.ctt_total += ctt;
        report.ctt_archive += after;
        if after != ctt {
            report.archive_candidates += 1;
        }
        add(
            &mut report.tools,
            &mut samples.tools,
            name,
            bytes,
            tokens,
            ctt,
        );
        if name == "Bash" {
            let fam = id_family
                .get(r.tool_use_id.as_str())
                .map(String::as_str)
                .unwrap_or("other");
            add(
                &mut report.bash_families,
                &mut samples.bash,
                fam,
                bytes,
                tokens,
                ctt,
            );
        }
        if let Some(grp) = mcp_group(name) {
            add(
                &mut report.mcp_groups,
                &mut samples.mcp,
                grp,
                bytes,
                tokens,
                ctt,
            );
        }
    }

    fold_thinking(parsed, report);
    fold_images(parsed, &id_name, &mut report.images);
    super::dictionary::fold(parsed, &mut report.dictionary);

    fold_skills(parsed, &id_skill, report, &mut samples.skills);
    for u in &parsed.usages {
        report.usage_input += u64::from(u.input_tokens);
        report.usage_cache_create += u64::from(u.cache_creation_input_tokens);
        report.usage_cache_read += u64::from(u.cache_read_input_tokens);
        report.usage_output += u64::from(u.output_tokens);
    }
    if let Some(last) = parsed.usages.last() {
        finals.push(
            u64::from(last.input_tokens)
                + u64::from(last.cache_creation_input_tokens)
                + u64::from(last.cache_read_input_tokens),
        );
    }
}

/// T61.1: fold the session's injected skill bodies into one row per skill, keyed by
/// the `Skill` tool_use's `input.skill`. `resident` multiplies the body bytes by the
/// API requests of this session at or after the injection turn — what the context
/// actually carried.
/// T125: count thinking blocks per session.
fn fold_thinking(parsed: &Parsed, report: &mut Report) {
    report.thinking.blocks += parsed.thinking.len() as u64;
    report.thinking.bytes += parsed.thinking.iter().map(|t| t.bytes).sum::<u64>();
}

/// T137: fold the session's image blocks; see [`ImageRow`].
fn fold_images(parsed: &Parsed, id_name: &BTreeMap<&str, &str>, row: &mut ImageRow) {
    for img in &parsed.images {
        let later = parsed.usages.iter().filter(|u| u.turn >= img.turn).count() as u64;
        row.blocks += 1;
        row.bytes += img.bytes;
        row.tokens += img.tokens;
        row.resident += img.tokens.saturating_mul(later);
        row.no_size += u64::from(!img.sized);
        let src = match img.tool_use_id.as_str() {
            "" => "user",
            id => id_name.get(id).copied().unwrap_or("unknown"),
        };
        let r = row.by_source.entry(src.to_string()).or_default();
        r.blocks += 1;
        r.bytes += img.bytes;
        r.tokens += img.tokens;
    }
}

fn fold_skills(
    parsed: &Parsed,
    id_skill: &BTreeMap<&str, String>,
    report: &mut Report,
    samples: &mut BTreeMap<String, Vec<u64>>,
) {
    if parsed.injected.is_empty() {
        return;
    }
    let map = report.skills.get_or_insert_with(BTreeMap::new);
    for inj in &parsed.injected {
        let Some(name) = id_skill.get(inj.tool_use_id.as_str()) else {
            continue;
        };
        let later = parsed.usages.iter().filter(|u| u.turn >= inj.turn).count() as u64;
        let row = map.entry(name.clone()).or_default();
        row.count += 1;
        row.bytes += inj.bytes;
        row.max = row.max.max(inj.bytes);
        row.est_tokens += est_tokens(inj.bytes);
        row.resident += inj.bytes.saturating_mul(later);
        push_capped(samples.entry(name.clone()).or_default(), inj.bytes);
    }
}

/// Mean from totals; p95 nearest-rank over the sizes seen across every session.
fn finish_skills(map: &mut BTreeMap<String, SkillRow>, samples: &mut BTreeMap<String, Vec<u64>>) {
    for (name, row) in map.iter_mut() {
        row.mean = row.bytes.checked_div(row.count).unwrap_or(0);
        match samples.remove(name) {
            Some(mut v) if !v.is_empty() => row.p95 = nearest_p95(&mut v),
            _ => row.p95 = row.max,
        }
    }
}

/// What `ctt` becomes when the `archive` plugin (T5.3) swaps this result for its pointer
/// after `keep_turns`: full size while young, head + tail lines afterwards.
fn replay_ctt(content: &str, tokens: u64, remain: u64, rp: Replay) -> u64 {
    if tokens < rp.min_tokens || remain <= rp.keep_turns {
        return tokens.saturating_mul(remain);
    }
    let lines: Vec<&str> = content.lines().collect();
    // Mirrors `archive::pointer`: with `lines.len() <= head + tail` it dumps the whole body
    // once instead of separate head/tail slices (which used to overlap here and double-count
    // lines, so `kept` came out up to 2x too big), and every shown line is clipped.
    let n = lines.len();
    let (head, tail) = if n <= rp.head_lines + rp.tail_lines {
        (n, 0)
    } else {
        (rp.head_lines, rp.tail_lines)
    };
    let kept: usize = lines[..head]
        .iter()
        .chain(&lines[n - tail..])
        .map(|l| shown_len(l) + 1)
        .sum();
    let pointer = est_tokens(kept as u64 + 64); // + the `[archived …]` line itself
    tokens.saturating_mul(rp.keep_turns) + pointer.saturating_mul(remain - rp.keep_turns)
}

/// One line as the archive pointer shows it: clipped when the `archive` plugin is built.
#[cfg(feature = "archive")]
fn shown_len(line: &str) -> usize {
    crate::plugins::archive::clip(line).len()
}

#[cfg(not(feature = "archive"))]
fn shown_len(line: &str) -> usize {
    line.len()
}

/// T58.3: every tool input counts toward the denominator; `Edit` and each `MultiEdit.edits[]`
/// entry add the `old_string` the model had to re-type and the `new_string` it meant to write.
fn fold_edits(row: &mut EditRow, u: &jsonl::ToolUse) {
    row.tool_input_bytes += u.input.to_string().len() as u64;
    let edits: Vec<&Value> = match u.name.as_str() {
        "Edit" => vec![&u.input],
        "MultiEdit" => u
            .input
            .get("edits")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default(),
        _ => return,
    };
    for e in edits {
        let len = |k: &str| {
            e.get(k)
                .and_then(Value::as_str)
                .map_or(0, |s| s.len() as u64)
        };
        row.calls += 1;
        row.old_bytes += len("old_string");
        row.new_bytes += len("new_string");
    }
}

/// T65.1: SHA-256 of each tool_result in order. A later result whose digest
/// equals an earlier one in the same session is a repeat — its bytes are the
/// content-hash dedup surface. Guard's input-key cache is a different axis.
fn fold_repeat(row: &mut RepeatRow, parsed: &Parsed) {
    let mut seen = BTreeSet::new();
    for r in &parsed.tool_results {
        let bytes = r.content.len() as u64;
        row.result_bytes += bytes;
        let sha = crate::store::hex_sha256(r.content.as_bytes());
        if !seen.insert(sha) {
            row.calls += 1;
            row.bytes += bytes;
        }
    }
}

/// T176: walk tool uses in order. A result naming an archive id (`(expand <id>)`,
/// `expand: rtok expand <id>`) makes it shown; a later `rtok expand <id>` Bash call or MCP
/// `expand` of a shown id is a re-expand — the bytes the cut made the agent fetch again.
fn fold_expand_after(row: &mut ExpandAfterRow, parsed: &Parsed) {
    static NAMED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"expand:? (?:rtok expand )?([0-9a-f]{64})").expect("static regex")
    });
    static CALLED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"rtok expand ([0-9a-f]{64})").expect("static regex")
    });
    let results: BTreeMap<&str, &str> = parsed
        .tool_results
        .iter()
        .map(|r| (r.tool_use_id.as_str(), r.content.as_str()))
        .collect();
    // id → bytes of the result that showed it, `None` once counted.
    let mut shown: BTreeMap<String, Option<u64>> = BTreeMap::new();
    for u in &parsed.tool_uses {
        let content = results.get(u.id.as_str()).copied().unwrap_or("");
        let asked = if u.name.ends_with("expand") {
            u.input.get("id").and_then(Value::as_str)
        } else if u.name == "Bash" {
            let cmd = u.input.get("command").and_then(Value::as_str).unwrap_or("");
            CALLED
                .captures(cmd)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str())
        } else {
            None
        };
        if let Some(seen) = asked.and_then(|id| shown.get_mut(id)) {
            row.calls += 1;
            row.bytes += content.len() as u64;
            row.shown_bytes += seen.take().unwrap_or(0);
            continue;
        }
        for c in NAMED.captures_iter(content) {
            shown
                .entry(c[1].to_string())
                .or_insert(Some(content.len() as u64));
        }
    }
}

/// T58.1: walk this session's tool uses in order. A native `Read` of a path that
/// was already read, with an `Edit` / `Write` / `MultiEdit` of that path in
/// between, is a changed re-read — its result bytes are the delta-read surface.
fn fold_read_delta(row: &mut ReadDeltaRow, parsed: &Parsed) {
    let sizes: BTreeMap<&str, u64> = parsed
        .tool_results
        .iter()
        .map(|r| (r.tool_use_id.as_str(), r.content.len() as u64))
        .collect();
    let mut paths: Vec<(String, bool, bool)> = Vec::new();
    for u in &parsed.tool_uses {
        let Some(path) = tool_path(&u.input) else {
            continue;
        };
        match u.name.as_str() {
            "Read" => {
                let bytes = sizes.get(u.id.as_str()).copied().unwrap_or(0);
                row.read_bytes += bytes;
                if let Some((_, seen, dirty)) = paths.iter_mut().find(|(p, ..)| same_path(p, path))
                {
                    if *seen && *dirty {
                        row.calls += 1;
                        row.bytes += bytes;
                    }
                    *seen = true;
                    *dirty = false;
                } else {
                    paths.push((path.to_string(), true, false));
                }
            }
            "Edit" | "Write" | "MultiEdit" => {
                if let Some((_, _, dirty)) = paths.iter_mut().find(|(p, ..)| same_path(p, path)) {
                    *dirty = true;
                } else {
                    paths.push((path.to_string(), false, true));
                }
            }
            _ => {}
        }
    }
}

/// T136: see [`ReadWholeRow`]. The grammar list is `read::outline::supported`; without
/// the `read` feature nothing has an outline, so nothing counts.
fn fold_read_whole(row: &mut ReadWholeRow, parsed: &Parsed, rp: Replay) {
    let sizes: BTreeMap<&str, u64> = parsed
        .tool_results
        .iter()
        .map(|r| (r.tool_use_id.as_str(), r.content.len() as u64))
        .collect();
    row.result_bytes += sizes.values().sum::<u64>();
    for (i, u) in parsed.tool_uses.iter().enumerate() {
        if u.name != "Read" {
            continue;
        }
        let bytes = sizes.get(u.id.as_str()).copied().unwrap_or(0);
        row.read_bytes += bytes;
        let ranged = u.input.get("offset").is_some() || u.input.get("limit").is_some();
        let Some(path) = tool_path(&u.input) else {
            continue;
        };
        if ranged || bytes < rp.read_whole_min.max(1) || !has_outline(path) {
            continue;
        }
        row.calls += 1;
        row.bytes += bytes;
        let edited = parsed.tool_uses[i + 1..]
            .iter()
            .take_while(|e| u64::from(e.turn) <= u64::from(u.turn) + rp.edit_window_turns)
            .any(|e| {
                matches!(e.name.as_str(), "Edit" | "Write" | "MultiEdit")
                    && tool_path(&e.input).is_some_and(|p| same_path(p, path))
            });
        if edited {
            row.edited += 1;
            row.edited_bytes += bytes;
        }
    }
}

/// How many tool calls after a deny count as "what the agent did next".
const DENY_FOLLOW_CALLS: usize = 3;

/// T383: see [`ReadDenyRow`]. The result text is the hook's reason, possibly behind a host
/// prefix, so it is matched by substring; without the `read` plugin there is no hook to match.
fn fold_read_deny(row: &mut ReadDenyRow, parsed: &Parsed) {
    let results: BTreeMap<&str, &str> = parsed
        .tool_results
        .iter()
        .map(|r| (r.tool_use_id.as_str(), r.content.as_str()))
        .collect();
    for (i, u) in parsed.tool_uses.iter().enumerate() {
        let Some(text) = results.get(u.id.as_str()) else {
            continue;
        };
        if u.name != "Read" || !is_read_deny(text) {
            continue;
        }
        row.calls += 1;
        row.tokens += est_tokens(text.len() as u64);
        let window = parsed.tool_uses[i + 1..].iter().take(DENY_FOLLOW_CALLS);
        if window
            .clone()
            .take_while(|n| n.name != "Read" && mcp_group(&n.name) != Some("rtok"))
            .any(|n| n.name == "ToolSearch")
        {
            row.then_toolsearch += 1;
        }
        let Some(path) = tool_path(&u.input) else {
            continue;
        };
        let next = window.into_iter().find(|n| {
            (n.name == "Read" || is_rtok_mcp_read(&n.name))
                && tool_path(&n.input).is_some_and(|p| same_path(p, path))
        });
        match next {
            Some(n) if n.name == "Read" => row.then_native += 1,
            Some(_) => row.then_mcp += 1,
            None => {}
        }
    }
}

fn is_rtok_mcp_read(name: &str) -> bool {
    mcp_group(name) == Some("rtok") && name.ends_with("__read")
}

#[cfg(feature = "read")]
fn is_read_deny(content: &str) -> bool {
    use crate::plugins::read::hook::{DELTA_REASON, REASON};
    content.contains(REASON) || content.contains(DELTA_REASON)
}

#[cfg(not(feature = "read"))]
fn is_read_deny(_content: &str) -> bool {
    false
}

#[cfg(feature = "read")]
fn has_outline(path: &str) -> bool {
    crate::plugins::read::outline::supported(Path::new(path))
}

#[cfg(not(feature = "read"))]
fn has_outline(_path: &str) -> bool {
    false
}

/// A native `Read` result carrying rtok's own trace: a read-advice deny reason
/// (`plugins::read::hook`), an MCP `read` cache-hit message (unreachable from a
/// native Read, kept for the session-wide "any sign rtok saw it" check), or an
/// archive/cmd trailer.
#[cfg(feature = "read")]
fn has_rtok_marker(content: &str) -> bool {
    is_read_deny(content)
        || content.contains("unchanged since ")
        || content.starts_with("[archived ")
        || content.contains("[rtok ")
}

/// Path-named write signals (need the path in `cmd` too); words are whole shell
/// tokens so `rm` cannot match inside `warm`. `mv`/`cp` don't distinguish
/// source from destination — approximation, not a shell parser.
#[cfg(feature = "read")]
const WRITE_SYMBOLS: [&str; 5] = [">", "sed -i", "perl -i", "perl -pi", "tee "];
#[cfg(feature = "read")]
const WRITE_WORDS: [&str; 5] = ["mv", "cp", "rm", "touch", "patch"];
/// Blanket writers: can rewrite any tracked file without naming it.
#[cfg(feature = "read")]
const BLANKET_WRITERS: [&str; 13] = [
    "git checkout",
    "git restore",
    "git stash",
    "git reset",
    "git rebase",
    "git merge",
    "git pull",
    "git apply",
    "git cherry-pick",
    "cargo fmt",
    "rustfmt",
    "oxfmt",
    "prettier --write",
];

#[cfg(feature = "read")]
fn has_word(cmd: &str, w: &str) -> bool {
    cmd.split(|c: char| c.is_whitespace() || matches!(c, '&' | '|' | ';'))
        .any(|t| t == w)
}

/// T179 "changed" class, narrowed to a write signal so a `cat`/`grep`/`sed -n` of
/// the path doesn't count — only something that could have changed it.
#[cfg(feature = "read")]
fn bash_touches(cmd: &str, path: &str) -> bool {
    if BLANKET_WRITERS.iter().any(|w| cmd.contains(w)) {
        return true;
    }
    let named = cmd.contains(path)
        || Path::new(path)
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|f| cmd.contains(f));
    named
        && (WRITE_SYMBOLS.iter().any(|w| cmd.contains(w))
            || WRITE_WORDS.iter().any(|w| has_word(cmd, w)))
}

/// Per-path state since its last Read: `.1`/`.2` are its `offset`/`limit`, `.3`
/// whether an Edit/Write/MultiEdit/Bash touched it since.
#[cfg(feature = "read")]
type Last = (String, Option<u64>, Option<u64>, bool);

/// Classifies each repeat native `Read` this session; `subagent` is filled by the
/// caller ([`collect`]) from [`super::subagents`], the only place that can see it.
#[cfg(feature = "read")]
fn fold_repeat_reads(row: &mut RepeatReadsRow, parsed: &Parsed) {
    let results: BTreeMap<&str, &str> = parsed
        .tool_results
        .iter()
        .map(|r| (r.tool_use_id.as_str(), r.content.as_str()))
        .collect();
    let saw_rtok = results.values().any(|c| has_rtok_marker(c));
    let mut paths: Vec<Last> = Vec::new();
    for u in &parsed.tool_uses {
        match u.name.as_str() {
            "Edit" | "Write" | "MultiEdit" => {
                if let Some(path) = tool_path(&u.input)
                    && let Some(p) = paths.iter_mut().find(|p| same_path(&p.0, path))
                {
                    p.3 = true;
                }
            }
            "Bash" => {
                let cmd = u.input.get("command").and_then(Value::as_str).unwrap_or("");
                paths
                    .iter_mut()
                    .filter(|p| bash_touches(cmd, &p.0))
                    .for_each(|p| p.3 = true);
            }
            "Read" => {
                let Some(path) = tool_path(&u.input) else {
                    continue;
                };
                let content = results.get(u.id.as_str()).copied().unwrap_or("");
                let bytes = content.len() as u64;
                let offset = u.input.get("offset").and_then(Value::as_u64);
                let limit = u.input.get("limit").and_then(Value::as_u64);
                if let Some(p) = paths.iter_mut().find(|p| same_path(&p.0, path)) {
                    let class = if has_rtok_marker(content) {
                        &mut row.fired
                    } else if p.3 {
                        &mut row.changed
                    } else if p.1 != offset || p.2 != limit {
                        &mut row.ranged
                    } else if saw_rtok {
                        &mut row.declined
                    } else {
                        &mut row.hook_absent
                    };
                    class.calls += 1;
                    class.bytes += bytes;
                    (p.1, p.2, p.3) = (offset, limit, false);
                } else {
                    paths.push((path.to_string(), offset, limit, false));
                }
            }
            _ => {}
        }
    }
}

#[cfg(not(feature = "read"))]
fn fold_repeat_reads(_row: &mut RepeatReadsRow, _parsed: &Parsed) {}

pub(crate) fn tool_path(input: &Value) -> Option<&str> {
    input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty())
}

pub(crate) use crate::fs::same_spelling as same_path;

pub(crate) fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        100.0 * part as f64 / whole as f64
    }
}

fn add(
    map: &mut BTreeMap<String, SizeRow>,
    samples: &mut BTreeMap<String, Vec<u64>>,
    name: &str,
    bytes: u64,
    tokens: u64,
    ctt: u64,
) {
    let row = map.entry(name.to_string()).or_default();
    row.count += 1;
    row.total_bytes += bytes;
    row.max = row.max.max(bytes);
    row.est_tokens += tokens;
    row.ctt += ctt;
    push_capped(samples.entry(name.to_string()).or_default(), bytes);
}

/// Bounded per-row sizes for the p95 column (T219). 2048 u64s ≈ 16 KiB per row.
const P95_CAP: usize = 2048;

/// Deterministic thinning once a row exceeds the cap: keep the even-indexed
/// half, then take the new sample. No RNG anywhere in this path.
fn push_capped(v: &mut Vec<u64>, bytes: u64) {
    if v.len() < P95_CAP {
        v.push(bytes);
        return;
    }
    let mut i = 0;
    v.retain(|_| {
        let keep = i % 2 == 0;
        i += 1;
        keep
    });
    v.push(bytes);
}

/// Nearest-rank p95: the `ceil(0.95·n)`-th smallest sample, integer math so the
/// rank is exact. Sorts the sidecar, never the report row.
fn nearest_p95(samples: &mut [u64]) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let rank = (95 * samples.len()).div_ceil(100);
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}

/// Mean from totals; p95 nearest-rank over the sidecar (falls back to max when
/// a row somehow has no samples). The sidecar is consumed.
fn finish_rows(map: &mut BTreeMap<String, SizeRow>, samples: &mut BTreeMap<String, Vec<u64>>) {
    for (name, row) in map.iter_mut() {
        row.mean = row.total_bytes.checked_div(row.count).unwrap_or(0);
        match samples.remove(name) {
            Some(mut v) if !v.is_empty() => row.p95 = nearest_p95(&mut v),
            _ => row.p95 = row.max,
        }
    }
}

/// One sidecar per `SizeRow` map `collect` fills, so `add` stays allocation-free
/// past the cap and `finish_rows` needs no extra lookup.
#[derive(Debug, Default)]
struct RowSamples {
    tools: BTreeMap<String, Vec<u64>>,
    bash: BTreeMap<String, Vec<u64>>,
    mcp: BTreeMap<String, Vec<u64>>,
    skills: BTreeMap<String, Vec<u64>>,
}

pub(crate) fn est_tokens(bytes: u64) -> u64 {
    if bytes == 0 {
        0
    } else {
        ((bytes as f64) / CHARS_PER_TOKEN).ceil() as u64
    }
}

pub fn bash_family(cmd: &str) -> String {
    let mut s = cmd.trim();
    loop {
        let next = strip_prefix_env(s).or_else(|| strip_prefix_cd(s));
        match next {
            Some(rest) if rest != s => s = rest,
            _ => break,
        }
    }
    let first = s.split_whitespace().next().unwrap_or("other");
    // T386: `mise exec -- cargo test` buckets as `cargo`. The peel lives in `cmd`
    // (`visible_argv`); without that plugin the stem stays argv[0], as before.
    #[cfg(feature = "cmd")]
    {
        let words: Vec<String> = s.split_whitespace().map(str::to_string).collect();
        let vis = crate::plugins::cmd::formatters::visible_argv(&words);
        let stem = vis.first().map(String::as_str).unwrap_or(first);
        crate::names::cmd_stem(stem).to_string()
    }
    #[cfg(not(feature = "cmd"))]
    crate::names::cmd_stem(first).to_string()
}

fn strip_prefix_env(s: &str) -> Option<&str> {
    let t = s.trim_start();
    let ident_end = t.find(|c: char| !c.is_ascii_alphanumeric() && c != '_')?;
    if ident_end == 0 || !t.as_bytes().get(ident_end).is_some_and(|b| *b == b'=') {
        return None;
    }
    crate::plugins::skip_word(&t[ident_end + 1..])
}

fn strip_prefix_cd(s: &str) -> Option<&str> {
    let after = s.trim_start().strip_prefix("cd")?;
    if !after.starts_with([' ', '\t']) {
        return None;
    }
    let after = after.trim_start();
    let rest = if after.starts_with("&&") {
        after
    } else {
        crate::plugins::skip_word(after)?
    };
    Some(rest.strip_prefix("&&")?.trim_start())
}

fn mcp_group(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("mcp__")?;
    Some(rest.split("__").next().unwrap_or(rest))
}

// SQLite measurements for a catalogue plugin (`rtok stats --plugin cmd --json`) live in the
// operator model (`crate::model::plugin_stats`, T15.11): the command renders the page,
// it does not query the store.

#[cfg(test)]
mod tests {
    use super::*;
    use rtok_plugin_sdk::Measurement;
    use serde_json::json;
    use std::fs;
    use std::io::Write;

    #[test]
    fn since_60d_parses() {
        assert_eq!(parse_since("60d").unwrap(), Duration::from_secs(60 * 86400));
    }

    #[test]
    fn price_arithmetic_costs_legs_and_cache_saving() {
        let p = crate::config::ModelPrice {
            input: 2.0,
            cache_write: 2.5,
            cache_read: 0.2,
            output: 10.0,
        };
        assert_eq!(
            row_cost(2_000_000, 400_000, 8_000_000, 500_000, &p),
            (11.6, 14.4)
        );
        assert_eq!(row_cost(0, 0, 0, 0, &p), (0.0, 0.0));
        // A read price above input never yields a negative saving.
        let p = crate::config::ModelPrice {
            input: 1.0,
            cache_write: 1.0,
            cache_read: 2.0,
            output: 1.0,
        };
        assert_eq!(row_cost(0, 0, 1_000_000, 0, &p), (2.0, 0.0));
    }

    #[test]
    fn attach_costs_prices_known_and_dashes_unknown() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_session("s1", None, None, None, Some("proxy"))
            .unwrap();
        let id = store
            .insert_call(
                "s1",
                "proxy",
                "api_request",
                None,
                None,
                None,
                None,
                Some("/v1/messages"),
            )
            .unwrap();
        store
            .insert_usage(
                "s1",
                Some("claude-sonnet-5"),
                "anthropic",
                2_000_000,
                400_000,
                8_000_000,
                500_000,
                id,
            )
            .unwrap();
        store
            .insert_usage("s1", Some("mystery-1"), "anthropic", 30, 0, 0, 1, id)
            .unwrap();
        store
            .insert_usage("s1", None, "anthropic", 7, 0, 0, 0, id)
            .unwrap();
        let mut report = Report::default();
        attach_costs(
            &mut report,
            &store,
            &crate::config::Config::default().stats.prices,
        )
        .unwrap();
        let cost = report.cost.unwrap();
        assert_eq!(cost.total_cost, 11.6);
        assert_eq!(cost.total_saved, 14.4);
        assert_eq!(cost.unknown, ["mystery-1", "unknown"]);
        let m = &cost.models["mystery-1"];
        assert_eq!(m.input, 30);
        assert_eq!((m.cost, m.saved), (None, None));
        assert_eq!(cost.models["claude-sonnet-5"].cost, Some(11.6));
        let table = cost.to_table();
        assert!(table.contains("11.60"), "{table}");
        assert!(table.contains("mystery-1"), "{table}");
    }

    /// T385.12.1: Batch-lane usage is costed at the model's `@batch` row, and a model without
    /// one stays unpriced instead of borrowing the standard rate.
    #[test]
    fn batch_lane_usage_is_priced_at_its_batch_row() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_session("s1", None, None, None, Some("proxy"))
            .unwrap();
        for (kind, model) in [
            ("api_request", "gpt-5"),
            ("api_request:batch", "gpt-5"),
            ("api_request:batch", "mystery-1"),
        ] {
            let call = store
                .insert_call("s1", "proxy", kind, None, None, None, None, Some("/x"))
                .unwrap();
            store
                .insert_usage(
                    "s1",
                    Some(model),
                    "openai_chat",
                    1_000_000,
                    0,
                    0,
                    1_000_000,
                    call,
                )
                .unwrap();
        }
        let mut report = Report::default();
        let prices = crate::config::Config::default().stats.prices;
        attach_costs(&mut report, &store, &prices).unwrap();
        let cost = report.cost.unwrap();
        assert_eq!(cost.models["gpt-5"].cost, Some(11.25));
        assert_eq!(cost.models["gpt-5@batch"].cost, Some(5.625));
        assert_eq!(cost.unknown, ["mystery-1@batch"]);
        assert_eq!(cost.total_cost, 16.875);
    }

    /// T385.12.2: usage the provider reported serving on Flex is costed at the `@flex` row, and
    /// the lane/tier table lists it beside the Batch lane's untiered rows; the ordinary tiers
    /// alone add no table.
    #[test]
    fn flex_usage_is_priced_at_its_flex_row_and_listed_by_tier() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_session("s1", None, None, None, Some("proxy"))
            .unwrap();
        for (kind, tier) in [
            ("api_request", Some("default")),
            ("api_request:bulk", Some("flex")),
            ("api_request:bulk", Some("default")),
            ("api_request:batch", None),
        ] {
            let call = store
                .insert_call("s1", "proxy", kind, None, None, None, None, Some("/x"))
                .unwrap();
            if let Some(tier) = tier {
                store.set_call_service_tier(call, tier).unwrap();
            }
            store
                .insert_usage(
                    "s1",
                    Some("gpt-5"),
                    "openai_chat",
                    1_000_000,
                    0,
                    0,
                    1_000_000,
                    call,
                )
                .unwrap();
        }
        let mut report = Report::default();
        let prices = crate::config::Config::default().stats.prices;
        attach_costs(&mut report, &store, &prices).unwrap();
        attach_lanes(&mut report, &store).unwrap();
        let cost = report.cost.unwrap();
        assert_eq!(cost.models["gpt-5"].cost, Some(22.5));
        assert_eq!(cost.models["gpt-5@flex"].cost, Some(5.625));
        assert_eq!(cost.models["gpt-5@batch"].cost, Some(5.625));
        let keys: Vec<_> = report.lane_tiers.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["agent/default", "batch/-", "bulk/default", "bulk/flex"]
        );
        assert_eq!(report.lanes["bulk"].input, 2_000_000);

        let agent_only = Store::open_in_memory().unwrap();
        agent_only.insert_proxy_turn("s", 1, 0, 0, 1).unwrap();
        let call = agent_only.call_ids_of_kind("api_request").unwrap()[0];
        agent_only.set_call_service_tier(call, "standard").unwrap();
        let mut plain = Report::default();
        attach_lanes(&mut plain, &agent_only).unwrap();
        assert!(plain.lanes.is_empty() && plain.lane_tiers.is_empty());
    }

    /// T364: a bad value is blamed on the place it came from, not always on `--since`.
    #[test]
    fn parse_since_names_its_source() {
        for bad in ["7x", "d", "-1d", ""] {
            let flag = parse_since(bad).unwrap_err().to_string();
            assert!(
                flag.contains("--since") && !flag.contains("stats.since"),
                "{flag}"
            );
            let cfg = parse_since_from(bad, "stats.since")
                .unwrap_err()
                .to_string();
            assert!(
                cfg.contains("stats.since") && !cfg.contains("--since"),
                "{cfg}"
            );
        }
        let huge = parse_since_from("99999999999999999d", "stats.since").unwrap_err();
        assert!(huge.to_string().contains("stats.since"), "{huge}");
        assert_eq!(
            parse_since_from("12h", "stats.since").unwrap(),
            Duration::from_secs(12 * 3600)
        );
    }

    /// A window wider than the calendar is a typo, not a wrapped duration: the multiply
    /// used to panic in a debug build and overflow silently in a release one.
    #[test]
    fn an_absurd_since_is_refused_not_wrapped() {
        let err = parse_since("99999999999999999d").unwrap_err();
        assert!(err.to_string().contains("out of range"), "{err}");
    }

    #[test]
    fn bash_family_strips_cd_and_env() {
        assert_eq!(bash_family("cd /tmp && git status"), "git");
        assert_eq!(bash_family("FOO=1 grep x"), "grep");
        assert_eq!(bash_family("sed -n 1p"), "sed");
        assert_eq!(bash_family(r"C:\Git\cmd\git.exe status"), "git");
        assert_eq!(bash_family("cargo.exe test"), "cargo");
    }

    #[cfg(feature = "cmd")]
    #[test]
    fn bash_family_sees_through_mise_exec() {
        assert_eq!(bash_family("mise exec -- cargo test"), "cargo");
        assert_eq!(bash_family("mise x node@20 -- git status"), "git");
        assert_eq!(bash_family("just --command cargo test"), "cargo");
        assert_eq!(bash_family("cd /tmp && mise exec -- cargo test"), "cargo");
        assert_eq!(bash_family("mise run test"), "mise");
        assert_eq!(bash_family("just check"), "just");
        assert_eq!(bash_family("just test"), "just");
    }

    #[test]
    fn bash_family_strips_quoted_cd_paths() {
        assert_eq!(bash_family("cd 'My Documents' && git status"), "git");
        assert_eq!(
            bash_family(r#"cd "C:\Program Files\App" && npm test"#),
            "npm"
        );
        assert_eq!(
            bash_family("cd ~/'My Documents'/src && cargo build"),
            "cargo"
        );
        assert_eq!(bash_family(r#"FOO="a b" cd 'x y' && rg z"#), "rg");
        assert_eq!(bash_family("cd\t'a b'\t&& sed -n 1p"), "sed");
        // Malformed quotes fail open: nothing is stripped.
        assert_eq!(bash_family("cd 'unterminated && git status"), "cd");
        assert_eq!(bash_family("FOO='x cd y && git status"), "FOO='x");
        // `cd` followed by something other than a path separator is not `cd`.
        assert_eq!(bash_family("cdx && git status"), "cdx");
    }

    #[test]
    fn ctt_and_tool_totals_on_mini_session() {
        let dir = tempfile_dir();
        let path = dir.join("s.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        // 3 turns: assistant, user result, assistant. result at turn 1, N=3 → ctt = tokens*(3-1)
        let body = "hello world!!"; // 13 bytes → ceil(13/4)=4 tokens; ctt=8
        writeln!(
            f,
            "{}",
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cd x && sed -n p"}}],"usage":{"input_tokens":10,"output_tokens":1}}})
        )
        .unwrap();
        writeln!(
            f,
            "{}",
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":body}]}})
        )
        .unwrap();
        writeln!(
            f,
            "{}",
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"done"}],"usage":{"input_tokens":20,"cache_read_input_tokens":80,"output_tokens":2}}})
        )
        .unwrap();
        drop(f);
        // p95 fixture: twenty 1-byte Reads and one 1000-byte Read. Nearest-rank
        // p95 is the 20th of 21 samples, so 1 while max is 1000.
        let mut g = fs::File::create(dir.join("p.jsonl")).unwrap();
        for i in 0..20 {
            writeln!(
                g,
                "{}",
                json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":format!("r{i}"),"name":"Read","input":{"file_path":"a"}}]}})
            )
            .unwrap();
            writeln!(
                g,
                "{}",
                json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":format!("r{i}"),"content":"x"}]}})
            )
            .unwrap();
        }
        writeln!(
            g,
            "{}",
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"big","name":"Read","input":{"file_path":"b"}}]}})
        )
        .unwrap();
        writeln!(
            g,
            "{}",
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"big","content":"y".repeat(1000)}]}})
        )
        .unwrap();
        drop(g);
        let r = collect(
            &dir,
            Duration::from_secs(86400 * 60),
            "",
            Replay::from_cfg(&Config::default()),
        )
        .unwrap();
        assert_eq!(r.sessions, 2);
        let bash = r.tools.get("Bash").unwrap();
        assert_eq!(bash.count, 1);
        assert_eq!(bash.est_tokens, 4);
        assert_eq!(bash.ctt, 8);
        assert_eq!(r.bash_families.get("sed").unwrap().count, 1);
        assert_eq!(r.usage_cache_read, 80);
        let read = r.tools.get("Read").unwrap();
        assert_eq!(read.count, 21);
        assert_eq!(read.max, 1000);
        assert_eq!(read.p95, 1);
        assert_ne!(read.p95, read.max);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn p95_is_nearest_rank_not_max() {
        let mut map = BTreeMap::new();
        let mut samples = BTreeMap::new();
        for _ in 0..100 {
            add(&mut map, &mut samples, "Read", 1, 1, 0);
        }
        add(&mut map, &mut samples, "Read", 1000, 250, 0);
        finish_rows(&mut map, &mut samples);
        let row = map.get("Read").unwrap();
        assert_eq!(row.max, 1000);
        assert_eq!(row.p95, 1);
        assert_ne!(row.p95, row.max);
    }

    #[test]
    fn p95_survives_the_cap_without_rng() {
        let mut v = Vec::new();
        for i in 0..(P95_CAP + 100) {
            push_capped(&mut v, i as u64);
        }
        assert!(v.len() <= P95_CAP);
        let once = nearest_p95(&mut v.clone());
        let twice = nearest_p95(&mut v.clone());
        assert_eq!(once, twice);
        assert!(once <= (P95_CAP + 100) as u64);
    }

    #[test]
    fn thinking_blocks_counted_per_session() {
        let dir = tempfile_dir();
        let path = dir.join("t.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        let lines = vec![
            json!({"type": "assistant", "message": {"id": "m1", "content": [
                {"type": "thinking", "thinking": "reason 123"}], "usage": {"input_tokens": 10, "output_tokens": 1}}}),
            json!({"type": "assistant", "message": {"id": "m2", "content": [
                {"type": "redacted_thinking", "data": "x"}], "usage": {"input_tokens": 11, "output_tokens": 1}}}),
            json!({"type": "assistant", "message": {"id": "m1", "content": [
                {"type": "text", "text": "answer"}], "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        ];
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        drop(f);
        let r = collect(
            &dir,
            Duration::from_secs(86400 * 60),
            "",
            Replay {
                keep_turns: 0,
                min_tokens: 0,
                head_lines: 0,
                tail_lines: 0,
                ..Replay::default()
            },
        )
        .unwrap();
        assert_eq!(r.thinking.blocks, 2);
        assert_eq!(r.thinking.bytes, 11);
    }

    /// T403: the dictionary row prints against the provider-billed input, and stays out of
    /// the table and the JSON when no request was sampled.
    #[test]
    fn dictionary_row_prints_shares_of_billed_input() {
        use crate::measure::dictionary::{DictKind, DictionaryRow};
        let mut r = Report::default();
        assert!(!r.to_table().contains("dictionary"));
        r.dictionary = DictionaryRow {
            requests: 2,
            input_bytes: 4000,
            request_tokens: 2500,
            paths: DictKind {
                repeated: 400,
                saved: 100,
            },
            idents: DictKind::default(),
        };
        let table = r.to_table();
        assert!(
            table.contains(
                "dictionary requests 2  content bytes 4000  billed input est. bytes 10000"
            ),
            "{table}"
        );
        assert!(table.contains("paths repeated 400 (4.00% of billed)  cache-safe saving 100 (1.00% of billed, 2.50% of content)"), "{table}");
    }

    /// T137: a PNG screenshot in a tool_result and a JPEG pasted into a prompt, both before
    /// the second of two API requests: each is resident once.
    #[test]
    fn image_blocks_counted_with_size_and_tokens() {
        use crate::measure::image::tests::{jpeg, png};
        let dir = tempfile_dir();
        let img = |data: String, mt: &str| json!({"type": "image", "source": {"type": "base64", "media_type": mt, "data": data}});
        let lines = [
            json!({"type": "assistant", "message": {"id": "a1", "content": [
                {"type": "tool_use", "id": "s1", "name": "screenshot", "input": {}}],
                "usage": {"input_tokens": 1000, "output_tokens": 1}}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result",
                "tool_use_id": "s1", "content": [img(png(1920, 1080), "image/png")]}]}}),
            json!({"type": "user", "message": {"content": [img(jpeg(200, 200), "image/jpeg"),
                {"type": "text", "text": "look"}]}}),
            json!({"type": "assistant", "message": {"id": "a2", "content": [
                {"type": "text", "text": "ok"}], "usage": {"input_tokens": 9000, "output_tokens": 1}}}),
        ];
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        fs::write(dir.join("t.jsonl"), body).unwrap();
        let r = collect(&dir, Duration::from_secs(86400), "", Replay::default()).unwrap();
        let d = &r.images;
        assert_eq!((d.blocks, d.tokens, d.no_size), (2, 2691 + 64, 0));
        assert_eq!(d.resident, 2691 + 64);
        assert_eq!(d.by_source["screenshot"].tokens, 2691);
        assert_eq!(d.by_source["user"].blocks, 1);
        let text = r.to_table();
        assert!(text.contains("images blocks 2"), "{text}");
        assert!(text.contains("  images user  blocks 1"), "{text}");
    }

    /// T61.1: skill bodies ride as `isMeta` records keyed by the top-level
    /// `sourceToolUseID`; the fold keys them by the `Skill` tool_use's `input.skill`
    /// and `resident` multiplies the body bytes by the API requests at or after the
    /// injection turn. One 3-line and one 3,000-line body.
    #[test]
    fn skills_fold_injected_bodies_and_count_resident_requests() {
        let dir = tempfile_dir();
        let path = dir.join("sk.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        let tiny = "line one\nline two\nline three\n";
        let huge: String = (0..3000).map(|i| format!("body line {i}\n")).collect();
        let line = |v: serde_json::Value| v.to_string();
        // Turn 0: the Skill tool_use. Turn 1: its 22-byte tool_result. Turn 2: the
        // isMeta body — what every later request of the session re-carries.
        for l in [
            line(json!({"type":"assistant","message":{"id":"m1","content":[
                {"type":"tool_use","id":"tu-s1","name":"Skill","input":{"skill":"tiny-skill","args":""}}]},
                "usage":{"input_tokens":10,"output_tokens":1}})),
            line(json!({"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"tu-s1","content":"ideas.md"}]}})),
            line(
                json!({"type":"user","isMeta":true,"sourceToolUseID":"tu-s1",
                "message":{"role":"user","content":[{"type":"text","text":tiny}]}}),
            ),
            line(
                json!({"type":"assistant","message":{"id":"m2","content":[]},
                "usage":{"input_tokens":100,"output_tokens":2}}),
            ),
            line(json!({"type":"assistant","message":{"id":"m3","content":[
                {"type":"tool_use","id":"tu-s2","name":"Skill","input":{"skill":"huge-skill","args":""}}]},
                "usage":{"input_tokens":110,"output_tokens":2}})),
            line(
                json!({"type":"user","isMeta":true,"sourceToolUseID":"tu-s2",
                "message":{"role":"user","content":[{"type":"text","text":huge}]}}),
            ),
        ] {
            writeln!(f, "{l}").unwrap();
        }
        let r = collect(
            &dir,
            Duration::from_secs(86400 * 60),
            "",
            Replay::from_cfg(&Config::default()),
        )
        .unwrap();
        let skills = r.skills.as_ref().expect("skills folded");
        let tiny_row = skills.get("tiny-skill").unwrap();
        assert_eq!(tiny_row.count, 1);
        assert_eq!(tiny_row.bytes, tiny.len() as u64);
        assert_eq!(tiny_row.mean, tiny.len() as u64);
        assert_eq!(tiny_row.p95, tiny_row.max);
        assert_eq!(
            tiny_row.resident,
            tiny.len() as u64 * 2,
            "the m2 and m3 requests carried it"
        );
        let huge_row = skills.get("huge-skill").unwrap();
        assert_eq!(huge_row.count, 1);
        assert_eq!(huge_row.bytes, huge.len() as u64);
        assert_eq!(
            huge_row.resident, 0,
            "no API request rides after the last injection in this fixture"
        );
        // The 8-byte tool_result is all `tools` sees for Skill; the body lives in
        // the skills fold now.
        assert_eq!(
            r.tools.get("Skill").unwrap().total_bytes,
            "ideas.md".len() as u64
        );
        let table = r.to_table();
        assert!(
            table.contains("skill") && table.contains("resident"),
            "{table}"
        );
        let js = r.to_json().unwrap();
        assert!(js.contains("\"skills\""), "{js}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edit_row_sums_old_and_new_strings_over_edit_and_multiedit() {
        let dir = tempfile_dir();
        let path = dir.join("e.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        // One Edit (old 10 B, new 3 B), one MultiEdit with two edits (old 4+6 B, new 1+2 B),
        // one Bash that only feeds the denominator.
        let edit = json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"/a.rs","old_string":"0123456789","new_string":"abc"}}],"usage":{"input_tokens":1,"output_tokens":10}}});
        let multi = json!({"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t2","name":"MultiEdit","input":{"file_path":"/b.rs","edits":[{"old_string":"abcd","new_string":"x"},{"old_string":"abcdef","new_string":"xy"}]}}],"usage":{"input_tokens":1,"output_tokens":10}}});
        let bash = json!({"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"t3","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":1,"output_tokens":10}}});
        for line in [&edit, &multi, &bash] {
            writeln!(f, "{line}").unwrap();
        }
        let r = collect(
            &dir,
            Duration::from_secs(86400 * 60),
            "",
            Replay::from_cfg(&Config::default()),
        )
        .unwrap();
        let e = &r.edits;
        assert_eq!((e.calls, e.old_bytes, e.new_bytes), (3, 20, 6));
        let denom: u64 = [&edit, &multi, &bash]
            .iter()
            .map(|v| v["message"]["content"][0]["input"].to_string().len() as u64)
            .sum();
        assert_eq!(e.tool_input_bytes, denom);
        let table = r.to_table();
        assert!(
            table.contains("edit calls 3  old_string 20 B  new_string 6 B"),
            "{table}"
        );
        assert!(
            table.contains(&format!(
                "old/tool_input {:.1}%",
                100.0 * 20.0 / denom as f64
            )),
            "{table}"
        );
        // 20 B → 5 est. tokens of 30 output tokens.
        assert!(table.contains("old/output_tokens (est) 16.7%"), "{table}");
        fs::remove_dir_all(&dir).ok();
    }

    /// T136: of five Reads only the two whole, large, outline-able ones count; the one an
    /// Edit follows is split out as having needed its body.
    #[cfg(feature = "read")]
    #[test]
    fn read_whole_counts_large_unranged_reads_of_outlined_files() {
        let dir = tempfile_dir();
        let big = "x".repeat(40_000);
        let read = |id: &str, input: Value| json!({"type":"assistant","message":{"id":format!("m{id}"),"content":[{"type":"tool_use","id":id,"name":"Read","input":input}]}});
        let result = |id: &str, body: &str| json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":body}]}});
        let lines = [
            read("t1", json!({"file_path":"src/a.rs"})),
            result("t1", &big),
            read("t2", json!({"file_path":"src/b.rs"})),
            result("t2", &big),
            json!({"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"t3","name":"Edit","input":{"file_path":"src/b.rs","old_string":"x","new_string":"y"}}]}}),
            read("t4", json!({"file_path":"src/c.rs","limit":50})),
            result("t4", &big),
            read("t5", json!({"file_path":"notes.txt"})),
            result("t5", &big),
            read("t6", json!({"file_path":"src/small.rs"})),
            result("t6", "fn main() {}"),
        ];
        let r = write_and_collect(&dir, "w.jsonl", &lines);
        let d = &r.read_whole;
        assert_eq!(
            (d.calls, d.bytes, d.edited, d.edited_bytes),
            (2, 80_000, 1, 40_000)
        );
        assert_eq!(d.read_bytes, 160_012);
        let table = r.to_table();
        assert!(
            table.contains("read whole calls 2  bytes 80000  50.0% of Read  50.0% of results  not edited 1 bytes 25.0% of results  edited after 1"),
            "{table}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// T383: denied Reads are counted with the size of their reasons; each is classed by what
    /// the agent did within the next three calls — an rtok `read` of the path, a native
    /// `Read` of it, or neither (here: the path comes four calls too late).
    #[cfg(feature = "read")]
    #[test]
    fn read_deny_counts_denies_and_what_followed() {
        use crate::plugins::read::hook::REASON;
        let dir = tempfile_dir();
        let deny = format!("PreToolUse:Read hook error: {REASON}");
        let call = |id: &str, name: &str, input: Value| json!({"type":"assistant","message":{"id":format!("m{id}"),"content":[{"type":"tool_use","id":id,"name":name,"input":input}]}});
        let result = |id: &str, body: &str| json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":body}]}});
        let lines = [
            call("t1", "Read", json!({"file_path":"src/a.rs"})),
            result("t1", &deny),
            call("t2", "mcp__rtok__read", json!({"path":"/proj/src/a.rs"})),
            result("t2", "body"),
            call("t3", "Read", json!({"file_path":"src/b.rs"})),
            result("t3", &deny),
            call("t4", "Read", json!({"file_path":"src/b.rs","limit":50})),
            result("t4", "body"),
            call("t5", "Read", json!({"file_path":"src/c.rs"})),
            result("t5", &deny),
            call(
                "t6",
                "ToolSearch",
                json!({"query":"select:mcp__rtok__read"}),
            ),
            call("t7", "Bash", json!({"command":"ls"})),
            call("t8", "Bash", json!({"command":"ls"})),
            call("t9", "mcp__rtok__read", json!({"path":"src/c.rs"})),
            result("t9", "body"),
            call("t10", "Read", json!({"file_path":"src/d.rs"})),
            result("t10", "fine"),
        ];
        let r = write_and_collect(&dir, "deny.jsonl", &lines);
        let d = &r.read_deny;
        let tokens = est_tokens(deny.len() as u64);
        assert_eq!(
            (
                d.calls,
                d.tokens,
                d.then_mcp,
                d.then_native,
                d.then_toolsearch
            ),
            (3, 3 * tokens, 1, 1, 1)
        );
        // T384: the pre-T384 reason and a Claude Code deny that names the exact tool both count.
        assert!(is_read_deny(
            "use rtok read; before Edit run native Read(limit=1) — it satisfies the edit gate"
        ));
        assert!(is_read_deny(
            "file changed since last read; use mcp__rtok__read(mode=diff) vs ab12cd34"
        ));
        assert!(has_rtok_marker(&format!(
            "use mcp__rtok__read (ToolSearch select:mcp__rtok__read loads it); {REASON}"
        )));
        let table = r.to_table();
        assert!(
            table.contains(&format!(
                "read deny calls 3  tokens {}  then rtok read 1  then native Read 1  then ToolSearch 1",
                3 * tokens
            )),
            "{table}"
        );
        fs::remove_dir_all(&dir).ok();
        let dir = tempfile_dir();
        let none = write_and_collect(&dir, "empty.jsonl", &[]);
        assert!(!none.to_table().contains("read deny"));
        assert!(!serde_json::to_string(&none).unwrap().contains("read_deny"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_delta_counts_reread_after_edit_not_unchanged_reread() {
        let dir = tempfile_dir();
        // Read /a.rs (10 B) → Edit /a.rs → Read /a.rs (20 B, counts) → Read /a.rs
        // again with no edit (does not count) → Read /b.rs only once (does not).
        let lines = [
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/a.rs"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"0123456789"}]}}),
            json!({"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"/proj/src/a.rs","old_string":"x","new_string":"y"}}]}}),
            json!({"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"t3","name":"Read","input":{"file_path":"src/a.rs"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3","content":"01234567890123456789"}]}}),
            json!({"type":"assistant","message":{"id":"m4","content":[{"type":"tool_use","id":"t4","name":"Read","input":{"file_path":"src/a.rs"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t4","content":"same"}]}}),
            json!({"type":"assistant","message":{"id":"m5","content":[{"type":"tool_use","id":"t5","name":"Read","input":{"file_path":"/b.rs"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t5","content":"bbbb"}]}}),
        ];
        let r = write_and_collect(&dir, "d.jsonl", &lines);
        let d = &r.read_delta;
        assert_eq!((d.calls, d.bytes, d.read_bytes), (1, 20, 10 + 20 + 4 + 4));
        let table = r.to_table();
        assert!(
            table.contains("read delta calls 1  bytes 20  of Read bytes 38  52.6%"),
            "{table}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn repeat_counts_identical_bodies_from_different_tools() {
        let dir = tempfile_dir();
        // Bash then Read, same 20-byte body; a third distinct body does not count.
        let lines = [
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cat a"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"01234567890123456789"}]}}),
            json!({"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"a"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"01234567890123456789"}]}}),
            json!({"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"t3","name":"Bash","input":{"command":"head -1000 a"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3","content":"different-output"}]}}),
        ];
        let r = write_and_collect(&dir, "r.jsonl", &lines);
        let d = &r.repeat;
        assert_eq!((d.calls, d.bytes, d.result_bytes), (1, 20, 20 + 20 + 16));
        let table = r.to_table();
        assert!(
            table.contains("repeat calls 1  bytes 20  of result bytes 56  35.7%"),
            "{table}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// T179: one repeat native `Read` per class. Session A carries `fired`, `changed`,
    /// `ranged`, `declined`; session B (no marker anywhere) gives `hook_absent`;
    /// session C + its sub-agent give `subagent` (T127).
    #[test]
    fn repeat_reads_classify_each_kind_once() {
        let dir = tempfile_dir();
        let use_ = |id: &str, name: &str, input: Value| {
            json!({"type":"assistant","message":{"id":id,"content":[
                {"type":"tool_use","id":id,"name":name,"input":input}]}})
        };
        let res = |id: &str, body: &str| {
            json!({"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":id,"content":body}]}})
        };
        let pair = |id: &str, name: &str, input: Value, body: &str| {
            vec![use_(id, name, input), res(id, body)]
        };
        let write = |path: std::path::PathBuf, lines: Vec<Value>| {
            let mut f = fs::File::create(path).unwrap();
            for l in lines {
                writeln!(f, "{l}").unwrap();
            }
        };

        // Bodies stay short so rustfmt keeps each `pair(...)` call on one line.
        let fired_body = "[archived aa11: 1 lines · 1 tokens · expand(aa11)]";
        let mut a = pair("t1", "Read", json!({"file_path":"fA.rs"}), "base");
        a.extend(pair("t2", "Read", json!({"file_path":"fA.rs"}), fired_body));
        a.extend(pair("t3", "Read", json!({"file_path":"gA.rs"}), "base-g"));
        a.push(use_("t4", "Edit", json!({"file_path":"gA.rs"})));
        a.extend(pair("t5", "Read", json!({"file_path":"gA.rs"}), "chg"));
        let in6 = json!({"file_path":"hA.rs","limit":100});
        let in7 = json!({"file_path":"hA.rs","limit":5});
        a.extend(pair("t6", "Read", in6, "bh"));
        a.extend(pair("t7", "Read", in7, "rng"));
        a.extend(pair("t8", "Read", json!({"file_path":"iA.rs"}), "base-i"));
        a.extend(pair("t9", "Read", json!({"file_path":"iA.rs"}), "dec"));
        write(dir.join("sessA.jsonl"), a);

        let mut b = pair("u1", "Read", json!({"file_path":"kB.rs"}), "base-k");
        b.extend(pair("u2", "Read", json!({"file_path":"kB.rs"}), "abs"));
        write(dir.join("sessB.jsonl"), b);

        let mut c = pair("v1", "Read", json!({"file_path":"pC.rs"}), "par");
        c.push(use_("v2", "Agent", json!({})));
        write(dir.join("sessC.jsonl"), c);
        let subs = dir.join("sessC").join("subagents");
        fs::create_dir_all(&subs).unwrap();
        let sub_body = "sub";
        let w1 = pair("w1", "Read", json!({"file_path":"pC.rs"}), sub_body);
        write(subs.join("agent-1.jsonl"), w1);
        fs::write(
            subs.join("agent-1.meta.json"),
            r#"{"agentType":"explore","model":"haiku","toolUseId":"v2"}"#,
        )
        .unwrap();

        let replay = Replay::from_cfg(&Config::default());
        let r = collect(&dir, Duration::from_secs(86400 * 60), "", replay).unwrap();
        let d = &r.repeat_reads;
        for (got, want_bytes) in [
            (d.fired, fired_body.len() as u64),
            (d.changed, "chg".len() as u64),
            (d.ranged, "rng".len() as u64),
            (d.declined, "dec".len() as u64),
            (d.hook_absent, "abs".len() as u64),
            (d.subagent, sub_body.len() as u64),
        ] {
            assert_eq!((got.calls, got.bytes), (1, want_bytes), "{d:?}");
        }
        assert!(r.to_table().contains("repeat reads  total 6"));
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(feature = "read")]
    #[test]
    fn bash_touches_needs_a_write_signal_not_just_a_named_read() {
        let cases = [
            ("cat a.rs", false),
            ("grep foo a.rs", false),
            ("warm-cache a.rs", false), // "rm" must not match inside "warm"
            ("sed -i '' 's/x/y/' a.rs", true),
            ("echo hi > a.rs", true),
            ("git checkout -- .", true), // blanket writer, path not even named
        ];
        for (cmd, want) in cases {
            assert_eq!(bash_touches(cmd, "a.rs"), want, "{cmd}");
        }
    }

    #[test]
    fn expand_right_after_counts_expands_of_shown_ids_only() {
        let dir = tempfile_dir();
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        let shown = format!("x\n… 23 lines omitted (expand {a})");
        let lines = [
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"grep -A3 x f"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":shown}]}}),
            json!({"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":format!("rtok expand {a} --lines 1-9")}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"0123456789"}]}}),
            json!({"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"t3","name":"mcp__rtok__expand","input":{"id":a}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3","content":"01234"}]}}),
            json!({"type":"assistant","message":{"id":"m4","content":[{"type":"tool_use","id":"t4","name":"mcp__rtok__expand","input":{"id":b}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t4","content":"never shown"}]}}),
        ];
        let r = write_and_collect(&dir, "e.jsonl", &lines);
        let d = &r.expand_after;
        let want = shown.len() as u64;
        assert_eq!((d.calls, d.bytes, d.shown_bytes), (2, 15, want));
        let table = r.to_table();
        assert!(
            table.contains(&format!(
                "expand right after  calls 2  bytes 15  shown bytes {want}"
            )),
            "{table}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn archive_replay_keeps_young_turns_and_shrinks_old_ones() {
        let rp = Replay {
            keep_turns: 2,
            min_tokens: 100,
            head_lines: 1,
            tail_lines: 1,
            ..Replay::default()
        };
        let content = "x".repeat(50) + "\n" + &"y".repeat(500) + "\n" + &"z".repeat(50);
        let tokens = est_tokens(content.len() as u64);
        assert_eq!(replay_ctt(&content, tokens, 2, rp), tokens * 2); // within keep_turns
        assert_eq!(replay_ctt(&content, 10, 5, rp), 50); // below min_tokens
        let after = replay_ctt(&content, tokens, 5, rp);
        let pointer = est_tokens(51 + 51 + 64); // head line, tail line, pointer line
        assert_eq!(after, tokens * 2 + pointer * 3);
        assert!(after < tokens * 5);
    }

    #[test]
    fn archive_replay_never_doubles_short_bodies() {
        let rp = Replay {
            keep_turns: 2,
            min_tokens: 100,
            head_lines: 1,
            tail_lines: 1,
            ..Replay::default()
        };
        // Single line, fewer than head_lines + tail_lines: `take(head)` and
        // `rev().take(tail)` used to grab the same line twice, so `kept` came out
        // ~2x too big and the estimate could land above not archiving at all.
        let single = "y".repeat(600);
        let tokens = est_tokens(single.len() as u64);
        let replay = replay_ctt(&single, tokens, 5, rp);
        assert!(
            replay <= tokens * 5,
            "replay {replay} > no-archive {}",
            tokens * 5
        );

        // Multi-line, no overlap between head and tail: unaffected by the fix.
        let content = "x".repeat(50) + "\n" + &"y".repeat(500) + "\n" + &"z".repeat(50);
        let tokens = est_tokens(content.len() as u64);
        let pointer = est_tokens(51 + 51 + 64); // head line, tail line, pointer line
        assert_eq!(
            replay_ctt(&content, tokens, 5, rp),
            tokens * 2 + pointer * 3
        );
    }

    #[test]
    fn compact_boundary_counts_once_per_event() {
        let dir = tempfile_dir();
        std::fs::write(
            dir.join("s.jsonl"),
            r#"{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}
{"type":"user","isCompactSummary":true,"message":{"content":"summary"}}
{"type":"assistant","message":{"content":"ok"}}
"#,
        )
        .unwrap();
        let r = collect(
            &dir,
            Duration::from_secs(86400 * 60),
            "",
            Replay::from_cfg(&Config::default()),
        )
        .unwrap();
        assert_eq!(r.sessions, 1);
        assert_eq!(r.compact, 1);
        assert!(
            r.to_table()
                .starts_with("sessions 1  compact 1  checkpoint 0  no_checkpoint 1  lines 3"),
            "{}",
            r.to_table()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoint_notes_split_sessions_with_and_without() {
        let dir = std::env::temp_dir().join(format!("rtok-stats-t712-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("has.jsonl"),
            r#"{"type":"user","message":{"content":"a"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("miss.jsonl"),
            r#"{"type":"user","message":{"content":"b"}}"#,
        )
        .unwrap();
        let mut r = collect(
            &dir,
            Duration::from_secs(86400 * 60),
            "",
            Replay::from_cfg(&Config::default()),
        )
        .unwrap();
        assert_eq!(r.sessions, 2);
        assert_eq!(r.checkpoint, 0);
        assert_eq!(r.no_checkpoint, 2);
        let store = Store::open_in_memory().unwrap();
        store
            .insert_note(Some("rtok"), "checkpoint:has", "compact", "checkpoint\n")
            .unwrap();
        attach_checkpoint_notes(&mut r, &store).unwrap();
        assert_eq!(r.checkpoint, 1);
        assert_eq!(r.no_checkpoint, 1);
        assert!(
            r.to_table()
                .starts_with("sessions 2  compact 0  checkpoint 1  no_checkpoint 1"),
            "{}",
            r.to_table()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T177: `attach_bash_cmd` splits unmarked `cmd` bytes into "below size gate"
    /// (`raw`) vs "no rule matched" (`unmatched`), independent of `rule`/`formatter`
    /// measurements that recorded a real shrink.
    #[test]
    fn bash_unmarked_splits_gate_from_unmatched() {
        let store = Store::open_in_memory().unwrap();
        let meas = |kind: &'static str, before: u64| Measurement {
            plugin: "cmd",
            kind,
            before_bytes: before,
            after_bytes: before,
            est_before: 10,
            est_after: 10,
            ref_id: None,
            call_id: None,
        };
        store.insert_measurement("s1", &meas("raw", 50)).unwrap();
        store.insert_measurement("s1", &meas("raw", 30)).unwrap();
        store
            .insert_measurement("s1", &meas("unmatched", 900))
            .unwrap();
        // A real shrink must not count as unmarked either way.
        store
            .insert_measurement(
                "s1",
                &Measurement {
                    after_bytes: 100,
                    ..meas("rule", 1000)
                },
            )
            .unwrap();
        let mut report = Report::default();
        attach_bash_cmd(&mut report, &store).unwrap();
        let d = &report.bash_unmarked;
        assert_eq!(d.gate_calls, 2, "{d:?}");
        assert_eq!(d.gate_bytes, 80, "{d:?}");
        assert_eq!(d.unmatched_calls, 1, "{d:?}");
        assert_eq!(d.unmatched_bytes, 900, "{d:?}");
        assert!(
            report
                .to_table()
                .contains("bash unmarked  below_gate calls 2 bytes 80"),
            "{}",
            report.to_table()
        );
    }

    /// Write `lines` to the dir's transcript and collect the report — the tail
    /// every collect-based test repeats.
    fn write_and_collect(dir: &std::path::Path, name: &str, lines: &[serde_json::Value]) -> Report {
        let mut f = fs::File::create(dir.join(name)).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
        collect(
            dir,
            Duration::from_secs(86400 * 60),
            "",
            Replay::from_cfg(&Config::default()),
        )
        .unwrap()
    }

    /// A fresh directory per call. The counter keeps two parallel tests apart: macOS clocks
    /// tick in microseconds, so pid + nanos alone collided and one test read the other's
    /// transcript.
    fn tempfile_dir() -> std::path::PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "rtok-stats-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn two_apis_print_as_two_table_rows() {
        let store = Store::open_in_memory().unwrap();
        crate::testutil::seed_two_apis(&store);
        let mut report = Report::default();
        attach_api(&mut report, &store).unwrap();
        let table = report.to_table();
        let names: Vec<&str> = table
            .lines()
            .filter(|l| l.starts_with("anthropic") || l.starts_with("openai_chat"))
            .collect();
        assert_eq!(names.len(), 2, "{table}");
        assert!(names.iter().any(|l| l.starts_with("anthropic")));
        assert!(names.iter().any(|l| l.starts_with("openai_chat")));
    }
}

// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Memo of read-only MCP tool results, so an identical repeated request does not redo the
//! store / FTS work.
//!
//! Safety first: a result is served only for a byte-identical canonical request. The key holds
//! the tool name, the caller scope (cwd and linked agent id) and every argument, `agent`
//! included. Keys are compared in full, never by hash, so two different requests cannot
//! collide. Only tools in [`MEMO_TOOLS`] are cached, errors never are, and any other successful
//! tool call (possibly a write) clears the memo.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Read-only tools whose answer depends only on their arguments, the caller scope and the
/// store. File-system tools (`read`, `search`, `tree`, graph) are left out: files change
/// under them without any MCP call.
pub(crate) const MEMO_TOOLS: &[&str] = &[
    "docs_resolve",
    "docs_query",
    "docs_get",
    "mem_search",
    "mem_get",
    "mem_pack",
];

/// Tools that change nothing, so a call to one does not clear the memo.
const PURE: &[&str] = &["ping", "expand"];

/// The memo itself: an LRU bounded by `max` entries, each fresh for `ttl`.
pub(crate) struct Memo {
    ttl: Duration,
    max: usize,
    map: HashMap<String, (Instant, String)>,
    order: VecDeque<String>,
}

impl Memo {
    pub(crate) fn new(ttl_secs: u64, max: u32) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_secs),
            max: max as usize,
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn enabled(&self) -> bool {
        !self.ttl.is_zero() && self.max > 0
    }

    /// The key of `tool` called with `args` by `scope`, or `None` when `tool` is never cached.
    pub(crate) fn key(&self, tool: &str, scope: &str, args: &Value) -> Option<String> {
        (self.enabled() && MEMO_TOOLS.contains(&tool))
            .then(|| Value::Array(vec![tool.into(), scope.into(), canonical(args)]).to_string())
    }

    pub(crate) fn get(&mut self, key: &str, now: Instant) -> Option<String> {
        let (at, text) = self.map.get(key)?;
        if now.duration_since(*at) >= self.ttl {
            self.remove(key);
            return None;
        }
        let text = text.clone();
        self.touch(key);
        Some(text)
    }

    pub(crate) fn put(&mut self, key: String, text: String, now: Instant) {
        if !self.enabled() {
            return;
        }
        if self.map.insert(key.clone(), (now, text)).is_some() {
            self.touch(&key);
        } else {
            self.order.push_back(key);
        }
        while self.map.len() > self.max {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            self.map.remove(&old);
        }
    }

    /// Called after every tool call that was not served from the memo.
    pub(crate) fn after_call(&mut self, tool: &str, ok: bool) {
        if ok && !MEMO_TOOLS.contains(&tool) && !PURE.contains(&tool) {
            self.clear();
        }
    }

    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    fn touch(&mut self, key: &str) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
    }

    fn remove(&mut self, key: &str) {
        self.map.remove(key);
        self.order.retain(|k| k != key);
    }
}

/// `v` with every object's keys sorted, whatever map type `serde_json` was built with. Array
/// order is kept: it can change the answer.
pub(crate) fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                out.insert(k.clone(), canonical(&m[k]));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    /// A memoized call: the memo, then `inner` on a miss. Mirrors `Server::call_tool`.
    fn call(
        m: &mut Memo,
        calls: &Cell<u32>,
        tool: &str,
        scope: &str,
        args: &Value,
        now: Instant,
    ) -> String {
        let key = m.key(tool, scope, args);
        if let Some(k) = &key
            && let Some(hit) = m.get(k, now)
        {
            return hit;
        }
        calls.set(calls.get() + 1);
        let text = format!("{tool}|{scope}|{}#{}", canonical(args), calls.get());
        if let Some(k) = key {
            m.put(k, text.clone(), now);
        }
        m.after_call(tool, true);
        text
    }

    fn memo() -> Memo {
        Memo::new(300, 64)
    }

    #[test]
    fn identical_request_hits_once_called() {
        let (mut m, n, t) = (memo(), Cell::new(0), Instant::now());
        let a = json!({"name":"serde","query":"derive"});
        let first = call(&mut m, &n, "docs_query", "s", &a, t);
        let second = call(&mut m, &n, "docs_query", "s", &a, t);
        assert_eq!(first, second);
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn any_arg_difference_is_a_fresh_call() {
        let (mut m, n, t) = (memo(), Cell::new(0), Instant::now());
        let variants = [
            json!({"name":"serde"}),
            json!({"name":"serde "}),
            json!({"name":"Serde"}),
            json!({"name":"serde","version":"1"}),
            json!({"name":"serde","version":1}),
            json!({"name":"serde","version":null}),
            json!({"name":"serde","agent":"a1"}),
            json!({"name":"serde","agent":"a2"}),
        ];
        let mut seen = std::collections::HashSet::new();
        for v in &variants {
            assert!(seen.insert(call(&mut m, &n, "docs_resolve", "s", v, t)));
        }
        assert_eq!(n.get(), variants.len() as u32);
    }

    #[test]
    fn different_scope_agent_or_cwd_is_a_fresh_call() {
        let (mut m, n, t) = (memo(), Cell::new(0), Instant::now());
        let a = json!({"query":"x"});
        let r1 = call(&mut m, &n, "mem_search", "/repo|agent-1", &a, t);
        let r2 = call(&mut m, &n, "mem_search", "/repo|agent-2", &a, t);
        let r3 = call(&mut m, &n, "mem_search", "/other|agent-1", &a, t);
        assert_ne!(r1, r2);
        assert_ne!(r1, r3);
        assert_eq!(n.get(), 3);
    }

    #[test]
    fn same_args_on_another_tool_never_hit() {
        let (mut m, n, t) = (memo(), Cell::new(0), Instant::now());
        let a = json!({"id":"42"});
        let r1 = call(&mut m, &n, "docs_get", "s", &a, t);
        let r2 = call(&mut m, &n, "mem_get", "s", &a, t);
        assert!(r1.starts_with("docs_get|") && r2.starts_with("mem_get|"));
        assert_eq!(n.get(), 2);
    }

    #[test]
    fn near_collision_keys_stay_apart() {
        let m = memo();
        let pairs = [
            (json!({"a":"b,c"}), json!({"a":"b","c":null})),
            (json!({"a":"[1]"}), json!({"a":[1]})),
            (json!({"a":"1"}), json!({"a":1})),
            (json!({}), json!(null)),
        ];
        for (x, y) in pairs {
            assert_ne!(m.key("docs_query", "s", &x), m.key("docs_query", "s", &y));
        }
        // A scope containing the separator of another field cannot forge a key.
        assert_ne!(
            m.key("docs_get", "s\",\"x", &json!({})),
            m.key("docs_get", "s", &json!({"x":""}))
        );
    }

    #[test]
    fn key_order_is_normalized_array_order_is_not() {
        let (mut m, n, t) = (memo(), Cell::new(0), Instant::now());
        let a: Value = serde_json::from_str(r#"{"name":"x","query":{"b":1,"a":2}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"query":{"a":2,"b":1},"name":"x"}"#).unwrap();
        call(&mut m, &n, "docs_query", "s", &a, t);
        call(&mut m, &n, "docs_query", "s", &b, t);
        assert_eq!(n.get(), 1);
        call(&mut m, &n, "docs_query", "s", &json!({"l":[1,2]}), t);
        call(&mut m, &n, "docs_query", "s", &json!({"l":[2,1]}), t);
        assert_eq!(n.get(), 3);
    }

    #[test]
    fn a_mutation_invalidates_everything() {
        let (mut m, n, t) = (memo(), Cell::new(0), Instant::now());
        let a = json!({"query":"x"});
        call(&mut m, &n, "mem_search", "s", &a, t);
        call(&mut m, &n, "mem_save", "s", &json!({"title":"x"}), t);
        assert_eq!(m.len(), 0);
        call(&mut m, &n, "mem_search", "s", &a, t);
        assert_eq!(n.get(), 3);
        // Read-only `ping` / `expand` keep the memo.
        call(&mut m, &n, "ping", "s", &json!({}), t);
        call(&mut m, &n, "mem_search", "s", &a, t);
        assert_eq!(n.get(), 4);
    }

    #[test]
    fn non_memo_tools_are_never_cached() {
        let (mut m, n, t) = (memo(), Cell::new(0), Instant::now());
        for tool in [
            "read",
            "search",
            "tree",
            "whoami",
            "agent_inbox",
            "mem_save",
            "task_list",
        ] {
            assert!(m.key(tool, "s", &json!({})).is_none(), "{tool}");
            call(&mut m, &n, tool, "s", &json!({}), t);
            call(&mut m, &n, tool, "s", &json!({}), t);
        }
        assert_eq!(n.get(), 14);
    }

    #[test]
    fn ttl_expiry_and_size_bound() {
        let (mut m, n, t) = (Memo::new(10, 2), Cell::new(0), Instant::now());
        let a = json!({"id":"1"});
        call(&mut m, &n, "docs_get", "s", &a, t);
        call(&mut m, &n, "docs_get", "s", &a, t + Duration::from_secs(9));
        assert_eq!(n.get(), 1);
        call(&mut m, &n, "docs_get", "s", &a, t + Duration::from_secs(10));
        assert_eq!(n.get(), 2);
        for id in ["2", "3", "4"] {
            call(&mut m, &n, "docs_get", "s", &json!({"id":id}), t);
        }
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn disabled_memo_caches_nothing() {
        for mut m in [Memo::new(0, 10), Memo::new(10, 0)] {
            let n = Cell::new(0);
            let t = Instant::now();
            call(&mut m, &n, "docs_get", "s", &json!({"id":"1"}), t);
            call(&mut m, &n, "docs_get", "s", &json!({"id":"1"}), t);
            assert_eq!(n.get(), 2);
        }
    }
}

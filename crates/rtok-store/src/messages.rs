// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T287: messages between agents and the user — `rtok agents send` / `inbox` (MCP in PR 2).
//! Local store only. `from_agent: None` is the user at a terminal. Rendering lives in one
//! place, the renderer's agent message frame, so every surface frames a body the same.

use crate::Result;
use diesel::prelude::*;
use serde::Serialize;

use super::schema::{agents, hosts, messages};
use super::{Store, TaskId, coalesce, unixepoch};

/// The cap on a cleaned body, in bytes of UTF-8.
pub const MAX_BODY: usize = 4096;

/// One message with its sender's host slug (`None` for the user, or a sender row since gone).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub id: i32,
    pub from_agent: Option<String>,
    pub from_host: Option<String>,
    pub to_agent: String,
    pub body: String,
    pub created_at: i64,
    pub delivered_at: Option<i64>,
    pub read_at: Option<i64>,
    /// The task the message is about (T441.13.1), canonical text such as `A12`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

type MessageTuple = (
    i32,
    Option<String>,
    Option<String>,
    String,
    String,
    i64,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

/// Control characters stripped (`\n` and `\t` kept); refused when empty or over [`MAX_BODY`]
/// — refused, not truncated, so nothing a sender wrote is silently lost.
pub fn clean_body(text: &str) -> Result<String> {
    let body: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    if body.trim().is_empty() {
        bail!("empty message");
    }
    if body.len() > MAX_BODY {
        bail!("message is {} bytes; the cap is {MAX_BODY}", body.len());
    }
    Ok(body)
}

impl Store {
    /// Queue `text` for agent `to` (a canonical id from [`Store::resolve_agent`]); returns the
    /// message id. Refused for an unknown or ended recipient and for a body [`clean_body`]
    /// rejects.
    pub fn send_message(&self, from: Option<&str>, to: &str, text: &str) -> Result<i32> {
        self.send_message_on(from, to, text, None)
    }

    /// [`Store::send_message`] tagged with the task it is about (T441.13.1). `task` must parse
    /// as a [`TaskId`] and is stored in its canonical upper-case form.
    pub fn send_message_on(
        &self,
        from: Option<&str>,
        to: &str,
        text: &str,
        task: Option<&str>,
    ) -> Result<i32> {
        let body = clean_body(text)?;
        let task = canonical_task(task)?;
        let mut conn = self.lock()?;
        let ended: Option<Option<i64>> = agents::table
            .filter(agents::id.eq(to))
            .select(agents::ended_at)
            .first(&mut *conn)
            .optional()?;
        match ended {
            None => bail!("unknown agent {to}"),
            Some(Some(_)) => bail!(
                "agent {} has ended; it can no longer receive messages",
                short_agent_id(to)
            ),
            Some(None) => {}
        }
        Ok(diesel::insert_into(messages::table)
            .values((
                messages::from_agent.eq(from),
                messages::to_agent.eq(to),
                messages::body.eq(&body),
                messages::task.eq(task),
            ))
            .returning(messages::id)
            .get_result(&mut *conn)?)
    }

    /// Agent `to`'s messages in send order. `mark_read` is the recipient reading its own
    /// inbox: every returned row gets `read_at` (and `delivered_at`, if unset) stamped. The
    /// rows come back as they were before that stamp, so `read_at: None` still tells which
    /// were new. The user peeking at an agent's queue passes `mark_read: false`.
    pub fn inbox(&self, to: &str, unread_only: bool, mark_read: bool) -> Result<Vec<Message>> {
        self.inbox_limited(to, unread_only, mark_read, None, None)
    }

    /// [`Store::inbox`] with at most `limit` messages, the oldest, and only those about `task`
    /// when one is given. The cut is made before the read mark, so a message that did not fit
    /// stays unread for the next call, and a message about another task is never marked.
    pub fn inbox_limited(
        &self,
        to: &str,
        unread_only: bool,
        mark_read: bool,
        limit: Option<usize>,
        task: Option<&str>,
    ) -> Result<Vec<Message>> {
        let task = canonical_task(task)?;
        let mut conn = self.lock()?;
        let only = if unread_only { Only::Unread } else { Only::All };
        let mut rows = load(&mut conn, to, only, task.as_deref())?;
        if let Some(limit) = limit {
            rows.truncate(limit);
        }
        if mark_read && !rows.is_empty() {
            let ids: Vec<i32> = rows.iter().map(|m| m.id).collect();
            diesel::update(messages::table.filter(messages::id.eq_any(ids)))
                .set((
                    messages::read_at.eq(coalesce(messages::read_at, unixepoch())),
                    messages::delivered_at.eq(coalesce(messages::delivered_at, unixepoch())),
                ))
                .execute(&mut *conn)?;
        }
        Ok(rows)
    }

    /// T284: how many unread messages each agent has, in one grouped query; an agent with none
    /// is absent.
    pub fn unread_counts(&self) -> Result<std::collections::HashMap<String, i64>> {
        let rows: Vec<(String, i64)> = messages::table
            .filter(messages::read_at.is_null())
            .group_by(messages::to_agent)
            .select((messages::to_agent, diesel::dsl::count_star()))
            .load(&mut *self.lock()?)?;
        Ok(rows.into_iter().collect())
    }

    /// T288: agent `to`'s messages no hook has pushed yet, in send order — one query on the
    /// `messages_to_agent` index. Stamps nothing: the hook marks only what it pushed.
    pub fn undelivered(&self, to: &str) -> Result<Vec<Message>> {
        load(&mut *self.lock()?, to, Only::Undelivered, None)
    }

    /// T288: stamp `delivered_at` on exactly `ids` where it is unset; `read_at` is left alone,
    /// so a pushed message still shows as unread in `rtok agents inbox`.
    pub fn mark_delivered(&self, ids: &[i32]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        diesel::update(
            messages::table
                .filter(messages::id.eq_any(ids))
                .filter(messages::delivered_at.is_null()),
        )
        .set(messages::delivered_at.eq(unixepoch()))
        .execute(&mut *self.lock()?)?;
        Ok(())
    }
}

/// `task` as the text the column holds, or an error when it is not a task id.
fn canonical_task(task: Option<&str>) -> Result<Option<String>> {
    Ok(task
        .map(|t| t.trim().parse::<TaskId>().map(|id| id.to_string()))
        .transpose()?)
}

/// Which of an agent's messages [`load`] returns.
enum Only {
    All,
    Unread,
    Undelivered,
}

fn load(
    conn: &mut SqliteConnection,
    to: &str,
    only: Only,
    task: Option<&str>,
) -> Result<Vec<Message>> {
    let mut query = messages::table
        .left_join(agents::table.on(agents::id.nullable().eq(messages::from_agent)))
        .left_join(hosts::table.on(hosts::id.nullable().eq(agents::host_id.nullable())))
        .filter(messages::to_agent.eq(to))
        .select((
            messages::id,
            messages::from_agent,
            hosts::slug.nullable(),
            messages::to_agent,
            messages::body,
            messages::created_at,
            messages::delivered_at,
            messages::read_at,
            messages::task,
        ))
        .order(messages::id.asc())
        .into_boxed();
    if let Some(task) = task {
        query = query.filter(messages::task.eq(task));
    }
    match only {
        Only::All => {}
        Only::Unread => query = query.filter(messages::read_at.is_null()),
        Only::Undelivered => query = query.filter(messages::delivered_at.is_null()),
    }
    Ok(query
        .load::<MessageTuple>(conn)?
        .into_iter()
        .map(|t| Message {
            id: t.0,
            from_agent: t.1,
            from_host: t.2,
            to_agent: t.3,
            body: t.4,
            created_at: t.5,
            delivered_at: t.6,
            read_at: t.7,
            task: t.8,
        })
        .collect())
}

/// The 8-char display form of an rtok agent id (D34).
pub fn short_agent_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_agents(store: &Store) -> (String, String) {
        let claude = store.host_id("claude").unwrap().unwrap();
        let a = store
            .register_agent(claude, "m-a", None, None, None)
            .unwrap();
        let b = store
            .register_agent(claude, "m-b", None, None, None)
            .unwrap();
        (a, b)
    }

    #[test]
    fn a_limited_read_marks_only_what_it_returned() {
        let store = Store::open_in_memory().unwrap();
        let (a, b) = two_agents(&store);
        let ids: Vec<i32> = ["one", "two", "three"]
            .iter()
            .map(|t| store.send_message(Some(&a), &b, t).unwrap())
            .collect();
        let page = store.inbox_limited(&b, true, true, Some(2), None).unwrap();
        assert_eq!(page.iter().map(|m| m.id).collect::<Vec<_>>(), ids[..2]);
        let rest = store.inbox(&b, true, false).unwrap();
        assert_eq!(rest.iter().map(|m| m.id).collect::<Vec<_>>(), ids[2..]);
    }

    #[test]
    fn send_then_inbox_in_order_and_read_marks() {
        let store = Store::open_in_memory().unwrap();
        let (a, b) = two_agents(&store);
        let first = store.send_message(Some(&a), &b, "one").unwrap();
        let second = store.send_message(None, &b, "two").unwrap();
        assert!(first < second);

        // The user peeking marks nothing.
        let peek = store.inbox(&b, true, false).unwrap();
        assert_eq!(
            peek.iter().map(|m| m.id).collect::<Vec<_>>(),
            [first, second]
        );
        assert_eq!(peek[0].from_host.as_deref(), Some("claude"));
        assert_eq!(peek[1].from_agent, None, "NULL sender = the user");
        assert_eq!(store.inbox(&b, true, false).unwrap().len(), 2);

        // The recipient reading marks both; the rows returned still show them as new.
        let read = store.inbox(&b, true, true).unwrap();
        assert!(read.iter().all(|m| m.read_at.is_none()));
        assert!(store.inbox(&b, true, false).unwrap().is_empty());
        let all = store.inbox(&b, false, false).unwrap();
        assert!(
            all.iter()
                .all(|m| m.read_at.is_some() && m.delivered_at.is_some())
        );
        assert!(
            store.inbox(&a, false, false).unwrap().is_empty(),
            "a's own queue"
        );
    }

    #[test]
    fn a_task_tag_is_canonical_filters_the_inbox_and_marks_only_its_thread() {
        let store = Store::open_in_memory().unwrap();
        let (a, b) = two_agents(&store);
        let on_a12 = store
            .send_message_on(Some(&a), &b, "one", Some(" a12 "))
            .unwrap();
        let on_a13 = store
            .send_message_on(Some(&a), &b, "two", Some("A13"))
            .unwrap();
        let plain = store.send_message(Some(&a), &b, "three").unwrap();

        let thread = store
            .inbox_limited(&b, true, true, None, Some("a12"))
            .unwrap();
        assert_eq!(thread.iter().map(|m| m.id).collect::<Vec<_>>(), [on_a12]);
        assert_eq!(thread[0].task.as_deref(), Some("A12"));

        let rest = store.inbox(&b, true, false).unwrap();
        assert_eq!(
            rest.iter().map(|m| m.id).collect::<Vec<_>>(),
            [on_a13, plain]
        );
        assert_eq!(rest[0].task.as_deref(), Some("A13"));
        assert_eq!(rest[1].task, None, "a message sent without a task has none");
    }

    #[test]
    fn a_task_that_is_not_an_id_is_refused_on_both_sides() {
        let store = Store::open_in_memory().unwrap();
        let (a, b) = two_agents(&store);
        for bad in ["", "12", "A0", "A1.2.3", "A12\n> forged"] {
            assert!(
                store.send_message_on(Some(&a), &b, "x", Some(bad)).is_err(),
                "{bad:?}"
            );
            assert!(
                store
                    .inbox_limited(&b, false, false, None, Some(bad))
                    .is_err(),
                "{bad:?}"
            );
        }
        assert!(
            store.inbox(&b, false, false).unwrap().is_empty(),
            "nothing was queued"
        );
    }

    #[test]
    fn mark_delivered_stamps_only_the_given_ids_and_never_read() {
        let store = Store::open_in_memory().unwrap();
        let (a, b) = two_agents(&store);
        let one = store.send_message(Some(&a), &b, "one").unwrap();
        let two = store.send_message(None, &b, "two").unwrap();
        store.mark_delivered(&[one]).unwrap();
        let left = store.undelivered(&b).unwrap();
        assert_eq!(left.iter().map(|m| m.id).collect::<Vec<_>>(), [two]);
        let all = store.inbox(&b, true, false).unwrap();
        assert_eq!(all.len(), 2, "delivered is not read");
        assert!(all[0].delivered_at.is_some() && all[0].read_at.is_none());
        assert!(all[1].delivered_at.is_none());
    }

    #[test]
    fn body_is_cleaned_and_capped() {
        let store = Store::open_in_memory().unwrap();
        let (a, b) = two_agents(&store);
        store
            .send_message(Some(&a), &b, "a\u{1b}[31mb\r\n\tc\u{7}\u{0}")
            .unwrap();
        assert_eq!(
            store.inbox(&b, false, false).unwrap()[0].body,
            "a[31mb\n\tc"
        );

        let at_cap = "é".repeat(MAX_BODY / 2);
        assert!(store.send_message(None, &b, &at_cap).is_ok());
        let over = store
            .send_message(None, &b, &format!("{at_cap}x"))
            .unwrap_err();
        assert!(over.to_string().contains("the cap is 4096"), "{over}");
        assert!(
            store.send_message(None, &b, "\u{1b}\u{7} \n").is_err(),
            "empty after clean"
        );
    }

    #[test]
    fn an_ended_or_unknown_recipient_is_refused() {
        let store = Store::open_in_memory().unwrap();
        let (a, b) = two_agents(&store);
        store.end_agent(&b, 1).unwrap();
        let err = store
            .send_message(Some(&a), &b, "hi")
            .unwrap_err()
            .to_string();
        assert!(err.contains("has ended"), "{err}");
        assert!(store.send_message(None, "nope", "hi").is_err());
        assert!(store.inbox(&b, false, false).unwrap().is_empty());
    }
}

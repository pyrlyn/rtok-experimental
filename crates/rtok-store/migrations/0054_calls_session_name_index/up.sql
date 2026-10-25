-- T525: `recent_hook_inputs_for_event` (the SubagentStart spawn brief reads the session's
-- newest PreToolUse rows) filters `session_id`, `kind = 'hook'` and `name`, newest first.
-- With only `calls_session (session_id, ts)` and `calls_kind (kind, ts)` SQLite walked every
-- hook row in the store and sorted them: 37 ms at 60,000 rows, 0.2 ms with this index.
CREATE INDEX IF NOT EXISTS calls_session_name ON calls (session_id, name, id);

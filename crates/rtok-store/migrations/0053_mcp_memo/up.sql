-- Persistent memo of read-only MCP tool results. `key` is the full canonical request and is
-- compared on every read, so a `key_hash` collision can never serve another request's value.
CREATE TABLE mcp_memo (
    key_hash TEXT PRIMARY KEY NOT NULL,
    key TEXT NOT NULL,
    tool TEXT NOT NULL,
    value TEXT NOT NULL,
    created BIGINT NOT NULL,
    last_used BIGINT NOT NULL,
    schema_hash TEXT NOT NULL,
    rtok_version TEXT NOT NULL
);
CREATE INDEX mcp_memo_last_used ON mcp_memo (last_used);

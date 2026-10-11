//! Diesel `table!` macros for the six 0001 tables (plan T13.1).
//! `notes_fts` and `symbols_fts` are VIRTUAL TABLEs — FTS5 `MATCH` / `bm25` have no Diesel DSL form (T163.3).

#![allow(unused)]

diesel::table! {
    events (id) {
        id -> Integer,
        ts -> BigInt,
        session -> Text,
        event -> Text,
        tool -> Nullable<Text>,
        plugin -> Nullable<Text>,
        ms -> Nullable<Double>,
    }
}

diesel::table! {
    measurements (id) {
        id -> Integer,
        ts -> BigInt,
        session -> Text,
        plugin -> Text,
        kind -> Text,
        before_bytes -> BigInt,
        after_bytes -> BigInt,
        est_before -> Integer,
        est_after -> Integer,
        ref_id -> Nullable<Text>,
        call_id -> Nullable<Integer>,
        once_key -> Nullable<Text>,
    }
}

diesel::table! {
    archive (id) {
        id -> Text,
        ts -> BigInt,
        session -> Text,
        tool -> Nullable<Text>,
        bytes -> BigInt,
        path -> Text,
        sha256 -> Text,
        // 0019 (T127): the context that wrote this row — a sub-agent's `agent_id`, or NULL
        // for the main window. Scopes `archive_in_session` so a pointer never names a body
        // a different context never saw.
        agent_id -> Nullable<Text>,
    }
}

// Composite PK (session, tool_use_id) since 0014: a repeated tool_use_id in a second
// session is a second decision, not an ignored insert.
diesel::table! {
    archive_decisions (session, tool_use_id) {
        session -> Text,
        tool_use_id -> Text,
        archive_id -> Text,
        pointer -> Text,
        expanded_ts -> Nullable<BigInt>,
        ts -> BigInt,
    }
}

diesel::table! {
    read_cache (session, path) {
        session -> Text,
        path -> Text,
        sha256 -> Text,
        ts -> BigInt,
        archive_id -> Nullable<Text>,
    }
}

diesel::table! {
    note_embeddings (note_id) {
        note_id -> Integer,
        model -> Text,
        dims -> Integer,
        text_hash -> Text,
        embedded_at -> BigInt,
        vector -> Binary,
    }
}

// 0033 (T374): the files a note is about, root-relative.
diesel::table! {
    note_files (note_id, path) {
        note_id -> Integer,
        path -> Text,
    }
}

// 0039 (T455): cached rustdoc JSON items. FTS5 is virtual (sql_ext), not table!.
diesel::table! {
    doc_crates (name, version) {
        name -> Text,
        version -> Text,
        sha256 -> Text,
        format_version -> Integer,
        fetched_unix -> BigInt,
    }
}

diesel::table! {
    doc_items (id) {
        id -> Integer,
        crate_name -> Text,
        version -> Text,
        path -> Text,
        kind -> Text,
        docs -> Text,
    }
}

// 0035 (T472): previous title and body when an upsert changes the body.
diesel::table! {
    note_versions (id) {
        id -> Integer,
        note_id -> Integer,
        title -> Text,
        body -> Text,
        version -> Integer,
        ts -> BigInt,
    }
}

// 0036 (T454): mechanical tool-call observations. `observations_fts` is virtual.
// 0044 (T501): `call_key`, the hook call it came from.
diesel::table! {
    observations (id) {
        id -> Integer,
        ts -> BigInt,
        session_id -> Text,
        project -> Nullable<Text>,
        obs_type -> Text,
        title -> Text,
        narrative -> Text,
        dedup -> Text,
        call_key -> Nullable<Text>,
    }
}

diesel::table! {
    observation_files (observation_id, path) {
        observation_id -> Integer,
        path -> Text,
    }
}

diesel::table! {
    notes (id) {
        id -> Integer,
        ts -> BigInt,
        project -> Nullable<Text>,
        kind -> Text,
        title -> Text,
        body -> Text,
        retired -> Nullable<BigInt>,
        superseded_by -> Nullable<Integer>,
        pinned -> Integer,
        // 0017 (T69.2); listed in table! so T104's drift guard holds (once outside the typed schema).
        uses -> Integer,
        last_used -> Nullable<BigInt>,
    }
}

diesel::table! {
    usage (id) {
        id -> Integer,
        ts -> BigInt,
        session -> Text,
        model -> Nullable<Text>,
        input -> BigInt,
        cache_create -> BigInt,
        cache_read -> BigInt,
        output -> BigInt,
        call_id -> Nullable<Integer>,
        api -> Text,
    }
}

diesel::table! {
    otel_export (stream) {
        stream -> Text,
        mark -> BigInt,
    }
}

diesel::table! {
    hosts (id) {
        id -> Integer,
        slug -> Text,
        kind -> Text,
        created_at -> BigInt,
    }
}

diesel::table! {
    providers (id) {
        id -> Integer,
        slug -> Text,
        name -> Text,
        created_at -> BigInt,
    }
}

diesel::table! {
    models (id) {
        id -> Integer,
        provider_id -> Integer,
        slug -> Text,
        created_at -> BigInt,
    }
}

diesel::table! {
    sessions (id) {
        id -> Text,
        host_id -> Nullable<Integer>,
        project -> Nullable<Text>,
        cwd -> Nullable<Text>,
        source -> Nullable<Text>,
        started_at -> BigInt,
        ended_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    calls (id) {
        id -> Integer,
        ts -> BigInt,
        session_id -> Text,
        host_id -> Nullable<Integer>,
        provider_id -> Nullable<Integer>,
        model_id -> Nullable<Integer>,
        plugin -> Nullable<Text>,
        surface -> Text,
        kind -> Text,
        parent_id -> Nullable<Integer>,
        name -> Nullable<Text>,
        ms -> Nullable<Double>,
        ok -> Integer,
        error -> Nullable<Text>,
        service_tier -> Nullable<Text>,
    }
}

diesel::table! {
    call_io (call_id) {
        call_id -> Integer,
        request_bytes -> BigInt,
        response_bytes -> BigInt,
        request_sha256 -> Nullable<Text>,
        response_sha256 -> Nullable<Text>,
        request_json -> Nullable<Text>,
        response_json -> Nullable<Text>,
        request_archive -> Nullable<Text>,
        response_archive -> Nullable<Text>,
        // T211: exact bytes, written only when the inline body is not valid UTF-8.
        request_raw -> Nullable<Binary>,
        response_raw -> Nullable<Binary>,
        // 0032 (T433): the session fields split out of a hook stdin; NULL = full body.
        hook_session_id -> Nullable<Integer>,
    }
}

// 0032 (T433): one row per distinct set of hook session fields, as compact JSON.
diesel::table! {
    hook_sessions (id) {
        id -> Integer,
        fields -> Text,
    }
}

diesel::table! {
    tokens (id) {
        id -> Integer,
        ts -> BigInt,
        call_id -> Integer,
        plugin -> Nullable<Text>,
        phase -> Text,
        source -> Text,
        #[sql_name = "tokens"]
        n_tokens -> BigInt,
        bytes -> Nullable<BigInt>,
        input -> Nullable<BigInt>,
        output -> Nullable<BigInt>,
        cache_create -> Nullable<BigInt>,
        cache_read -> Nullable<BigInt>,
    }
}

diesel::table! {
    logs (id) {
        id -> Integer,
        ts -> BigInt,
        level -> Text,
        source -> Text,
        name -> Text,
        session -> Nullable<Text>,
        call_id -> Nullable<Integer>,
        plugin -> Nullable<Text>,
        message -> Text,
        fields -> Nullable<Text>,
    }
}

// 0018 (T69.6): hand-edit guard digest, keyed by name.
diesel::table! {
    kv (key) {
        key -> Text,
        value -> Text,
    }
}

diesel::table! {
    symbols (id) {
        id -> Integer,
        path -> Text,
        name -> Text,
        kind -> Text,
        line -> Integer,
        is_def -> Integer,
        file_sha -> Text,
        root -> Text,
        mtime -> BigInt,
        size -> BigInt,
        end_line -> Integer,
        scope -> Text,
        start_byte -> BigInt,
        end_byte -> BigInt,
        content_hash -> Text,
        signature -> Text,
        doc -> Text,
    }
}

// 0011 + 0016 (T68.3): one fingerprint and last index time per root.
diesel::table! {
    extractor (root) {
        root -> Text,
        fingerprint -> Text,
        indexed_at -> Nullable<BigInt>,
    }
}

// 0016 (T68.3): hook-staled files, listed until the next index replaces their rows.
diesel::table! {
    symbol_stale (root, path) {
        root -> Text,
        path -> Text,
    }
}

// 0030 (T370): the file graph and its global PageRank, one JSON document per root.
diesel::table! {
    file_rank (root) {
        root -> Text,
        graph -> Text,
    }
}

// 0024 (T282, D34): the rtok agent id — one row per host session, one per sub-agent inside
// it. `parent_key` is '' for the main window or the host's own sub-agent `agent_id`;
// `parent_id` is the resolved rtok id of that sub-agent's parent row.
diesel::table! {
    agents (id) {
        id -> Text,
        host_id -> Integer,
        host_session_id -> Text,
        parent_key -> Text,
        parent_id -> Nullable<Text>,
        cwd -> Nullable<Text>,
        started_at -> BigInt,
        last_seen -> BigInt,
        ended_at -> Nullable<BigInt>,
        activity -> Nullable<Text>,
        status_text -> Nullable<Text>,
        ancestors -> Nullable<Text>,
    }
}

// 0025 (T285): the agent a worktree is bound to; the git lock reason is the source of truth.
diesel::table! {
    worktree_claims (path) {
        path -> Text,
        agent_id -> Text,
        task -> Text,
        claimed_at -> BigInt,
        released_at -> Nullable<BigInt>,
    }
}

// 0045 (T289.5): an adopted worktree no single agent could be named for yet.
diesel::table! {
    worktree_pending (path) {
        path -> Text,
        task -> Text,
        created_at -> BigInt,
    }
}

// 0026 (T287): messages between agents and the user. `from_agent` NULL = the user at a
// terminal; `body` is capped and cleaned by `Store::send_message`.
// 0046 (T441.13.1): `task`, the task the message is about.
diesel::table! {
    messages (id) {
        id -> Integer,
        from_agent -> Nullable<Text>,
        to_agent -> Text,
        body -> Text,
        created_at -> BigInt,
        delivered_at -> Nullable<BigInt>,
        read_at -> Nullable<BigInt>,
        task -> Nullable<Text>,
    }
}

// 0027 (T329.1): the graph project registry; `name` NULL = the directory name, at most one `selected`.
diesel::table! {
    projects (id) {
        id -> Integer,
        root -> Text,
        name -> Nullable<Text>,
        origin -> Text,
        created_at -> BigInt,
        last_used_at -> BigInt,
        selected -> Integer,
    }
}

// 0028 (T329.3): directed project links; `unlinked = 1` is a remembered removal of an auto link.
diesel::table! {
    project_links (from_id, to_id) {
        from_id -> Integer,
        to_id -> Integer,
        kind -> Text,
        reason -> Nullable<Text>,
        unlinked -> Integer,
        created_at -> BigInt,
    }
}

// 0031 (T441.3): task id counters, one per project and parent ('' = top level).
diesel::table! {
    task_counters (project, parent) {
        project -> Text,
        parent -> Text,
        last -> BigInt,
    }
}

// 0034 (T442): the claim SessionStart names. The file or the issue stays the source of truth.
diesel::table! {
    task_claims (project, task_id) {
        project -> Text,
        task_id -> Text,
        agent_id -> Text,
        title -> Text,
        since -> BigInt,
    }
}

// 0040 (T329.15): graph call events; `rows_json` is the call's `measurements` rows as JSON.
// 0041 (T329.33): `symbols`, how many symbols the call asked for.
// 0042 (T329.36): `symbols_returned`, `files_touched`, `projects_hit`: what the answer held.
diesel::table! {
    graph_events (id) {
        id -> BigInt,
        ts_ms -> BigInt,
        call -> Text,
        phase -> Text,
        session -> Text,
        tool -> Text,
        target -> Nullable<Text>,
        project -> Nullable<Text>,
        backend -> Nullable<Text>,
        ok -> Integer,
        error -> Nullable<Text>,
        ms -> Nullable<Double>,
        done -> Nullable<Integer>,
        total -> Nullable<Integer>,
        answer_tokens -> Nullable<Integer>,
        rows_json -> Nullable<Text>,
        symbols -> Nullable<Integer>,
        symbols_returned -> Nullable<Integer>,
        files_touched -> Nullable<Integer>,
        projects_hit -> Nullable<Integer>,
    }
}

diesel::joinable!(archive_decisions -> archive (archive_id));
diesel::joinable!(models -> providers (provider_id));
diesel::joinable!(sessions -> hosts (host_id));
diesel::joinable!(agents -> hosts (host_id));
diesel::joinable!(worktree_claims -> agents (agent_id));
diesel::joinable!(calls -> hosts (host_id));
diesel::joinable!(calls -> providers (provider_id));
diesel::joinable!(calls -> models (model_id));
diesel::joinable!(calls -> sessions (session_id));
diesel::joinable!(call_io -> calls (call_id));
diesel::joinable!(call_io -> hook_sessions (hook_session_id));
diesel::joinable!(tokens -> calls (call_id));
diesel::joinable!(logs -> calls (call_id));
diesel::joinable!(measurements -> calls (call_id));
diesel::joinable!(usage -> calls (call_id));
diesel::joinable!(note_embeddings -> notes (note_id));
diesel::joinable!(note_files -> notes (note_id));
diesel::joinable!(observation_files -> observations (observation_id));

diesel::allow_tables_to_appear_in_same_query!(
    events,
    measurements,
    archive,
    archive_decisions,
    read_cache,
    notes,
    note_embeddings,
    note_files,
    doc_crates,
    doc_items,
    observations,
    observation_files,
    usage,
    hosts,
    providers,
    models,
    sessions,
    calls,
    call_io,
    hook_sessions,
    tokens,
    logs,
    symbols,
    extractor,
    file_rank,
    symbol_stale,
    agents,
    worktree_claims,
    worktree_pending,
    messages,
    projects,
    project_links,
    task_counters,
    task_claims,
    graph_events,
);

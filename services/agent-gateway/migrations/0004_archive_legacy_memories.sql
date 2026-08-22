-- Session-scoped memories predate authenticated, user-owned memory_items.
-- Preserve rows whose deleted conversations make ownership unknowable, but
-- remove the legacy table from the runtime read/write path.
CREATE TABLE legacy_memory_archive (
    id BIGINT PRIMARY KEY,
    session_id TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at DOUBLE PRECISION NOT NULL
);

INSERT INTO legacy_memory_archive(id, session_id, content, created_at)
SELECT id, session_id, content, created_at
FROM memories
ON CONFLICT (id) DO NOTHING;

INSERT INTO memory_items(
    id,
    user_id,
    scope_type,
    project_id,
    conversation_id,
    source_turn_id,
    source_response_id,
    kind,
    user_note,
    visual_summary,
    cover_asset_id,
    captured_at,
    created_at,
    updated_at,
    is_pinned,
    archived_at
)
SELECT
    'legacy-session-' || m.id,
    c.user_id,
    'personal',
    NULL,
    c.id,
    NULL,
    'legacy-session-import',
    'text',
    m.content,
    '',
    NULL,
    NULL,
    m.created_at,
    m.created_at,
    0,
    NULL
FROM memories m
JOIN conversations c ON c.id = m.session_id
ON CONFLICT (id) DO NOTHING;

DROP TABLE memories;

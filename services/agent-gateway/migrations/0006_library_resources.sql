CREATE TABLE library_resources (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    scope_type TEXT NOT NULL DEFAULT 'personal'
        CHECK (scope_type IN ('personal', 'project')),
    project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    resource_type TEXT NOT NULL
        CHECK (resource_type IN ('note', 'link', 'file')),
    asset_id TEXT REFERENCES assets(id) ON DELETE SET NULL,
    source_url TEXT,
    mime_type TEXT,
    extracted_text TEXT NOT NULL DEFAULT '',
    metadata JSONB NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'ready'
        CHECK (status IN ('ready', 'stored', 'error')),
    created_at DOUBLE PRECISION NOT NULL,
    updated_at DOUBLE PRECISION NOT NULL,
    archived_at DOUBLE PRECISION,
    CHECK (
        (scope_type = 'personal' AND project_id IS NULL) OR
        (scope_type = 'project' AND project_id IS NOT NULL)
    )
);

CREATE INDEX idx_library_resources_user
ON library_resources(user_id, archived_at, updated_at DESC);

CREATE INDEX idx_library_resources_project
ON library_resources(project_id, archived_at, updated_at DESC);

CREATE INDEX idx_library_resources_search
ON library_resources
USING GIN ((title || ' ' || extracted_text || ' ' || COALESCE(source_url, '')) gin_trgm_ops);

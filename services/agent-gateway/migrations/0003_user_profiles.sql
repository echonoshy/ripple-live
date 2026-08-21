CREATE TABLE user_profiles (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    ai_identity TEXT NOT NULL DEFAULT '',
    user_identity TEXT NOT NULL DEFAULT '',
    preferred_name TEXT NOT NULL DEFAULT '',
    basic_memory TEXT NOT NULL DEFAULT '',
    updated_at DOUBLE PRECISION NOT NULL
);

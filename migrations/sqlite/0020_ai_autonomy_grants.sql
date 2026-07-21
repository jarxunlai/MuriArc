CREATE TABLE IF NOT EXISTS ai_autonomy_grants (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL UNIQUE REFERENCES ai_conversations(id),
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    session_id TEXT,
    mode TEXT NOT NULL CHECK (mode IN ('ask', 'auto', 'full')),
    allowed_categories_json TEXT NOT NULL,
    batch_limit INTEGER NOT NULL CHECK (batch_limit BETWEEN 1 AND 100),
    step_up_verified_at TEXT,
    last_used_at TEXT NOT NULL,
    expires_at TEXT,
    revoked_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);

CREATE INDEX IF NOT EXISTS idx_ai_autonomy_grants_owner
    ON ai_autonomy_grants(lab_id, user_id, conversation_id)
    WHERE deleted_at IS NULL;

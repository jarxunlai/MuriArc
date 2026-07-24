CREATE TABLE IF NOT EXISTS ai_conversation_sources (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    conversation_id TEXT REFERENCES ai_conversations(id),
    project_id TEXT REFERENCES projects(id),
    attachment_id TEXT NOT NULL UNIQUE REFERENCES attachments(id),
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    last_activity_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    archived_at TEXT,
    error_code TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);

CREATE INDEX IF NOT EXISTS idx_ai_conversation_sources_owner
    ON ai_conversation_sources(lab_id, user_id, created_at DESC)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_ai_conversation_sources_conversation
    ON ai_conversation_sources(conversation_id, created_at DESC)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_ai_conversation_sources_retention
    ON ai_conversation_sources(lab_id, expires_at)
    WHERE deleted_at IS NULL AND status NOT IN ('archived', 'expired');

CREATE TABLE IF NOT EXISTS ai_conversation_sources (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    user_id UUID NOT NULL REFERENCES users(id),
    conversation_id UUID REFERENCES ai_conversations(id),
    project_id UUID REFERENCES projects(id),
    attachment_id UUID NOT NULL UNIQUE REFERENCES attachments(id),
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    last_activity_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    archived_at TIMESTAMPTZ,
    error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
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

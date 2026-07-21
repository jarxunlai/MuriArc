CREATE TABLE IF NOT EXISTS ai_conversation_messages (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES ai_conversations(id),
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    user_id UUID NOT NULL REFERENCES users(id),
    sequence BIGINT NOT NULL CHECK (sequence > 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content TEXT NOT NULL CHECK (length(btrim(content)) > 0 AND octet_length(content) <= 262144),
    response_json JSONB,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (conversation_id, sequence),
    CHECK ((role = 'user' AND response_json IS NULL)
        OR (role = 'assistant' AND response_json IS NOT NULL)),
    CHECK (response_json IS NULL OR octet_length(response_json::text) <= 2097152)
);

CREATE INDEX IF NOT EXISTS idx_ai_conversation_messages_history
    ON ai_conversation_messages(conversation_id, sequence DESC)
    WHERE deleted_at IS NULL;

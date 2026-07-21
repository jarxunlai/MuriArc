CREATE TABLE IF NOT EXISTS ai_conversation_messages (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES ai_conversations(id),
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content TEXT NOT NULL CHECK (length(trim(content)) > 0 AND length(content) <= 262144),
    response_json TEXT CHECK (response_json IS NULL OR length(response_json) <= 2097152),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (conversation_id, sequence),
    CHECK ((role = 'user' AND response_json IS NULL)
        OR (role = 'assistant' AND response_json IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS idx_ai_conversation_messages_history
    ON ai_conversation_messages(conversation_id, sequence DESC)
    WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS ai_conversation_source_object_deletions (
    source_id UUID PRIMARY KEY REFERENCES ai_conversation_sources(id),
    attachment_id UUID NOT NULL UNIQUE REFERENCES attachments(id),
    lab_id UUID NOT NULL REFERENCES labs(id),
    enqueued_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ai_source_object_deletions_lab
    ON ai_conversation_source_object_deletions(lab_id, enqueued_at, source_id);

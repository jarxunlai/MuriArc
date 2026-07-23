CREATE TABLE IF NOT EXISTS ai_conversation_source_object_deletions (
    source_id TEXT PRIMARY KEY NOT NULL REFERENCES ai_conversation_sources(id),
    attachment_id TEXT NOT NULL UNIQUE REFERENCES attachments(id),
    lab_id TEXT NOT NULL REFERENCES labs(id),
    enqueued_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ai_source_object_deletions_lab
    ON ai_conversation_source_object_deletions(lab_id, enqueued_at, source_id);

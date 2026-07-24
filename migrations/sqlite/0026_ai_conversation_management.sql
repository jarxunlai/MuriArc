ALTER TABLE ai_conversations ADD COLUMN pinned_at TEXT;
ALTER TABLE ai_conversations ADD COLUMN archived_at TEXT;

CREATE INDEX IF NOT EXISTS idx_ai_conversations_listing
    ON ai_conversations(
        lab_id,
        user_id,
        archived_at,
        pinned_at DESC,
        updated_at DESC,
        id DESC
    )
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_ai_conversations_project_listing
    ON ai_conversations(
        lab_id,
        user_id,
        project_id,
        archived_at,
        pinned_at DESC,
        updated_at DESC,
        id DESC
    )
    WHERE deleted_at IS NULL;

ALTER TABLE ai_conversations
    ADD COLUMN pinned_at TIMESTAMPTZ,
    ADD COLUMN archived_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_ai_conversations_listing
    ON ai_conversations(
        lab_id,
        user_id,
        archived_at,
        pinned_at DESC NULLS LAST,
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
        pinned_at DESC NULLS LAST,
        updated_at DESC,
        id DESC
    )
    WHERE deleted_at IS NULL;

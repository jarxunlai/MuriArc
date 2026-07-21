CREATE TABLE IF NOT EXISTS ai_autonomy_grants (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL UNIQUE REFERENCES ai_conversations(id),
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    user_id UUID NOT NULL REFERENCES users(id),
    session_id UUID,
    mode TEXT NOT NULL CHECK (mode IN ('ask', 'auto', 'full')),
    allowed_categories_json JSONB NOT NULL,
    batch_limit INTEGER NOT NULL CHECK (batch_limit BETWEEN 1 AND 100),
    step_up_verified_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);

CREATE INDEX IF NOT EXISTS idx_ai_autonomy_grants_owner
    ON ai_autonomy_grants(lab_id, user_id, conversation_id)
    WHERE deleted_at IS NULL;

ALTER TABLE ai_lab_settings
    ADD COLUMN IF NOT EXISTS max_autonomy_mode TEXT NOT NULL DEFAULT 'full'
    CHECK (max_autonomy_mode IN ('ask', 'auto', 'full'));

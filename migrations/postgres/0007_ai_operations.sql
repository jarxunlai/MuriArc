CREATE TABLE IF NOT EXISTS ai_conversations (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    user_id UUID NOT NULL REFERENCES users(id),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 256),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_ai_conversations_user
    ON ai_conversations(lab_id, user_id, updated_at DESC) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS ai_tool_runs (
    id UUID PRIMARY KEY,
    conversation_id UUID REFERENCES ai_conversations(id),
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    user_id UUID NOT NULL REFERENCES users(id),
    tool_name TEXT NOT NULL CHECK (length(tool_name) BETWEEN 1 AND 64),
    input_json JSONB NOT NULL,
    output_json JSONB,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    error TEXT CHECK (error IS NULL OR length(error) <= 1024),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_ai_tool_runs_conversation
    ON ai_tool_runs(conversation_id, created_at, id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_ai_tool_runs_user
    ON ai_tool_runs(lab_id, user_id, created_at DESC) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS ai_approvals (
    id UUID PRIMARY KEY,
    tool_run_id UUID NOT NULL UNIQUE REFERENCES ai_tool_runs(id),
    requested_diff_json JSONB NOT NULL,
    decision TEXT NOT NULL,
    decided_by UUID REFERENCES users(id),
    decided_at TIMESTAMPTZ,
    reason TEXT CHECK (reason IS NULL OR length(reason) <= 1024),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    CHECK ((decision = 'pending' AND decided_by IS NULL AND decided_at IS NULL)
        OR (decision IN ('approved', 'rejected') AND decided_by IS NOT NULL AND decided_at IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS idx_ai_approvals_pending
    ON ai_approvals(decision, created_at) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS ai_provider_settings (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    provider_config JSONB NOT NULL,
    secret_key_version INTEGER CHECK (secret_key_version IS NULL OR secret_key_version > 0),
    secret_nonce BYTEA CHECK (secret_nonce IS NULL OR octet_length(secret_nonce) = 12),
    secret_ciphertext BYTEA CHECK (secret_ciphertext IS NULL OR octet_length(secret_ciphertext) > 16),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    CHECK ((secret_key_version IS NULL AND secret_nonce IS NULL AND secret_ciphertext IS NULL)
        OR (secret_key_version IS NOT NULL AND secret_nonce IS NOT NULL AND secret_ciphertext IS NOT NULL))
);

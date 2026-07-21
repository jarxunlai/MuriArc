CREATE TABLE IF NOT EXISTS ai_conversations (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 256),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_ai_conversations_user
    ON ai_conversations(lab_id, user_id, updated_at DESC) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS ai_tool_runs (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT REFERENCES ai_conversations(id),
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    tool_name TEXT NOT NULL CHECK (length(tool_name) BETWEEN 1 AND 64),
    input_json TEXT NOT NULL,
    output_json TEXT,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    error TEXT CHECK (error IS NULL OR length(error) <= 1024),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_ai_tool_runs_conversation
    ON ai_tool_runs(conversation_id, created_at, id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_ai_tool_runs_user
    ON ai_tool_runs(lab_id, user_id, created_at DESC) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS ai_approvals (
    id TEXT PRIMARY KEY NOT NULL,
    tool_run_id TEXT NOT NULL UNIQUE REFERENCES ai_tool_runs(id),
    requested_diff_json TEXT NOT NULL,
    decision TEXT NOT NULL,
    decided_by TEXT REFERENCES users(id),
    decided_at TEXT,
    reason TEXT CHECK (reason IS NULL OR length(reason) <= 1024),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    CHECK ((decision = 'pending' AND decided_by IS NULL AND decided_at IS NULL)
        OR (decision IN ('approved', 'rejected') AND decided_by IS NOT NULL AND decided_at IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS idx_ai_approvals_pending
    ON ai_approvals(decision, created_at) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS attachment_links (
    id TEXT PRIMARY KEY NOT NULL, lab_id TEXT NOT NULL REFERENCES labs(id), project_id TEXT NOT NULL REFERENCES projects(id),
    attachment_id TEXT NOT NULL REFERENCES attachments(id), target_type TEXT NOT NULL, target_id TEXT NOT NULL,
    created_by TEXT NOT NULL REFERENCES users(id), created_at TEXT NOT NULL, updated_at TEXT NOT NULL, deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0), UNIQUE (attachment_id, target_type, target_id)
);
CREATE INDEX IF NOT EXISTS idx_attachment_links_target ON attachment_links(project_id, target_type, target_id) WHERE deleted_at IS NULL;
CREATE TABLE IF NOT EXISTS attachment_derivatives (
    id TEXT PRIMARY KEY NOT NULL, lab_id TEXT NOT NULL REFERENCES labs(id), project_id TEXT REFERENCES projects(id),
    attachment_id TEXT NOT NULL REFERENCES attachments(id), kind TEXT NOT NULL, media_type TEXT, relative_path TEXT,
    size_bytes INTEGER CHECK (size_bytes IS NULL OR size_bytes >= 0), sha256 TEXT, status TEXT NOT NULL, error_code TEXT,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL, deleted_at TEXT, revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (attachment_id, kind)
);
CREATE TABLE IF NOT EXISTS ai_private_images (
    id TEXT PRIMARY KEY NOT NULL, lab_id TEXT NOT NULL REFERENCES labs(id), user_id TEXT NOT NULL REFERENCES users(id),
    conversation_id TEXT REFERENCES ai_conversations(id), attachment_id TEXT NOT NULL UNIQUE REFERENCES attachments(id),
    project_id TEXT REFERENCES projects(id), status TEXT NOT NULL, last_activity_at TEXT NOT NULL, expires_at TEXT NOT NULL,
    archived_at TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_ai_private_images_retention ON ai_private_images(lab_id, user_id, expires_at) WHERE deleted_at IS NULL;
CREATE TABLE IF NOT EXISTS ai_extraction_drafts (
    id TEXT PRIMARY KEY NOT NULL, lab_id TEXT NOT NULL REFERENCES labs(id), user_id TEXT NOT NULL REFERENCES users(id),
    project_id TEXT NOT NULL REFERENCES projects(id), experiment_id TEXT NOT NULL REFERENCES experiments(id),
    experiment_event_id TEXT NOT NULL REFERENCES experiment_events(id), private_image_id TEXT NOT NULL REFERENCES ai_private_images(id),
    attachment_id TEXT NOT NULL REFERENCES attachments(id), image_sha256 TEXT NOT NULL CHECK (length(image_sha256) = 64),
    provider TEXT NOT NULL, model TEXT NOT NULL, tool_run_id TEXT REFERENCES ai_tool_runs(id), status TEXT NOT NULL,
    items_json TEXT NOT NULL, error_code TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_ai_extraction_drafts_user ON ai_extraction_drafts(lab_id, user_id, created_at DESC) WHERE deleted_at IS NULL;

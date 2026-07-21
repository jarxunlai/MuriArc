CREATE TABLE IF NOT EXISTS ai_provider_endpoints (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    provider_kind TEXT NOT NULL CHECK (provider_kind IN ('open_ai_compatible', 'local_http')),
    label TEXT NOT NULL CHECK (length(label) > 0 AND length(label) <= 120),
    base_url TEXT NOT NULL CHECK (length(base_url) > 0 AND length(base_url) <= 2048),
    normalized_base_url TEXT NOT NULL CHECK (length(normalized_base_url) > 0 AND length(normalized_base_url) <= 2048),
    enabled INTEGER NOT NULL DEFAULT 1,
    builtin INTEGER NOT NULL DEFAULT 0,
    created_by TEXT REFERENCES users(id),
    updated_by TEXT REFERENCES users(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, provider_kind, normalized_base_url)
);

CREATE INDEX IF NOT EXISTS idx_ai_provider_endpoints_enabled
    ON ai_provider_endpoints(lab_id, provider_kind, enabled);

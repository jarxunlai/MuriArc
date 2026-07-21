CREATE TABLE IF NOT EXISTS ai_provider_endpoints (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    provider_kind TEXT NOT NULL CHECK (provider_kind IN ('open_ai_compatible', 'local_http')),
    label TEXT NOT NULL CHECK (length(label) > 0 AND length(label) <= 120),
    base_url TEXT NOT NULL CHECK (length(base_url) > 0 AND length(base_url) <= 2048),
    normalized_base_url TEXT NOT NULL CHECK (length(normalized_base_url) > 0 AND length(normalized_base_url) <= 2048),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    builtin BOOLEAN NOT NULL DEFAULT FALSE,
    created_by UUID REFERENCES users(id),
    updated_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, provider_kind, normalized_base_url)
);

CREATE INDEX IF NOT EXISTS idx_ai_provider_endpoints_enabled
    ON ai_provider_endpoints(lab_id, provider_kind, enabled);

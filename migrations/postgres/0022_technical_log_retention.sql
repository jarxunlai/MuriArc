CREATE TABLE IF NOT EXISTS technical_log_policies (
    lab_id UUID PRIMARY KEY REFERENCES labs(id),
    max_rows BIGINT NOT NULL DEFAULT 20000 CHECK (max_rows BETWEEN 1000 AND 1000000),
    min_retention_days INTEGER NOT NULL DEFAULT 30 CHECK (min_retention_days BETWEEN 1 AND 3650),
    updated_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0)
);

CREATE TABLE IF NOT EXISTS technical_log_events (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    user_id UUID REFERENCES users(id),
    request_id TEXT,
    method TEXT NOT NULL,
    path TEXT NOT NULL,
    status_code INTEGER NOT NULL CHECK (status_code BETWEEN 100 AND 599),
    occurred_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_technical_log_events_retention
    ON technical_log_events(lab_id, occurred_at DESC, id);

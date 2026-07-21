CREATE TABLE IF NOT EXISTS jobs (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    created_by UUID NOT NULL REFERENCES users(id),
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
    progress_current BIGINT NOT NULL DEFAULT 0 CHECK (progress_current >= 0),
    progress_total BIGINT CHECK (progress_total IS NULL OR progress_total >= 0),
    result_json JSONB,
    error_report_json JSONB,
    cancellation_requested BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, created_by, idempotency_key)
);
CREATE INDEX IF NOT EXISTS idx_jobs_lab_created
    ON jobs(lab_id, deleted_at, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_project_created
    ON jobs(project_id, deleted_at, created_at DESC);

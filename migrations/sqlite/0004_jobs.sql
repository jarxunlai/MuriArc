CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    created_by TEXT NOT NULL REFERENCES users(id),
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
    progress_current INTEGER NOT NULL DEFAULT 0 CHECK (progress_current >= 0),
    progress_total INTEGER CHECK (progress_total IS NULL OR progress_total >= 0),
    result_json TEXT,
    error_report_json TEXT,
    cancellation_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancellation_requested IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, created_by, idempotency_key)
);
CREATE INDEX IF NOT EXISTS idx_jobs_lab_created
    ON jobs(lab_id, deleted_at, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_project_created
    ON jobs(project_id, deleted_at, created_at DESC);

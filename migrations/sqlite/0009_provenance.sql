CREATE TABLE provenance (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('human', 'import', 'ai', 'migration')),
    actor_user_id TEXT REFERENCES users(id),
    import_job_id TEXT REFERENCES jobs(id),
    import_commit_id TEXT,
    tool_run_id TEXT REFERENCES ai_tool_runs(id),
    provider TEXT,
    model TEXT,
    confidence REAL CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    request_id TEXT,
    recorded_at TEXT NOT NULL
);

CREATE INDEX idx_provenance_entity ON provenance(lab_id, entity_type, entity_id, recorded_at);
CREATE INDEX idx_provenance_project ON provenance(project_id, recorded_at);
CREATE INDEX idx_provenance_import_job ON provenance(import_job_id) WHERE import_job_id IS NOT NULL;
CREATE INDEX idx_provenance_tool_run ON provenance(tool_run_id) WHERE tool_run_id IS NOT NULL;

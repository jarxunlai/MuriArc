CREATE TABLE provenance (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    entity_type TEXT NOT NULL,
    entity_id UUID NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('human', 'import', 'ai', 'migration')),
    actor_user_id UUID REFERENCES users(id),
    import_job_id UUID REFERENCES jobs(id),
    import_commit_id UUID,
    tool_run_id UUID REFERENCES ai_tool_runs(id),
    provider TEXT,
    model TEXT,
    confidence DOUBLE PRECISION CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    request_id TEXT,
    recorded_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_provenance_entity ON provenance(lab_id, entity_type, entity_id, recorded_at);
CREATE INDEX idx_provenance_project ON provenance(project_id, recorded_at);
CREATE INDEX idx_provenance_import_job ON provenance(import_job_id) WHERE import_job_id IS NOT NULL;
CREATE INDEX idx_provenance_tool_run ON provenance(tool_run_id) WHERE tool_run_id IS NOT NULL;

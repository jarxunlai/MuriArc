CREATE TABLE project_animal_assignments (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    animal_id TEXT NOT NULL REFERENCES animals(id),
    assigned_by TEXT REFERENCES users(id),
    reason TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);

CREATE UNIQUE INDEX idx_project_animal_assignments_active
    ON project_animal_assignments(project_id, animal_id)
    WHERE deleted_at IS NULL;
CREATE INDEX idx_project_animal_assignments_project
    ON project_animal_assignments(project_id, deleted_at, animal_id);
CREATE INDEX idx_project_animal_assignments_animal
    ON project_animal_assignments(animal_id, deleted_at, project_id);

-- Preserve project visibility for upgraded databases while separating future
-- authorization from experiment participation history.
INSERT INTO project_animal_assignments (
    id, lab_id, project_id, animal_id, assigned_by, reason,
    created_at, updated_at, deleted_at, revision
)
SELECT
    lower(substr(hex(randomblob(16)), 1, 8) || '-' ||
          substr(hex(randomblob(16)), 1, 4) || '-4' ||
          substr(hex(randomblob(16)), 1, 3) || '-a' ||
          substr(hex(randomblob(16)), 1, 3) || '-' ||
          substr(hex(randomblob(16)), 1, 12)),
    e.lab_id,
    e.project_id,
    ep.animal_id,
    NULL,
    'Backfilled from existing experiment participation',
    MIN(ep.created_at),
    MIN(ep.created_at),
    NULL,
    1
FROM experiment_participations ep
JOIN experiments e ON e.id = ep.experiment_id AND e.deleted_at IS NULL
JOIN animals a ON a.id = ep.animal_id AND a.deleted_at IS NULL AND a.lab_id = e.lab_id
WHERE ep.deleted_at IS NULL
GROUP BY e.lab_id, e.project_id, ep.animal_id;

INSERT INTO audit_entries (
    id, lab_id, project_id, entity_type, entity_id, action,
    actor_type, actor_user_id, actor_display_name, source, request_id, reason,
    before_json, after_json, operation_code, operation_version,
    operation_params_json, entity_name_snapshot, entity_revision, occurred_at
)
SELECT
    lower(substr(hex(randomblob(16)), 1, 8) || '-' ||
          substr(hex(randomblob(16)), 1, 4) || '-4' ||
          substr(hex(randomblob(16)), 1, 3) || '-a' ||
          substr(hex(randomblob(16)), 1, 3) || '-' ||
          substr(hex(randomblob(16)), 1, 12)),
    paa.lab_id,
    paa.project_id,
    'project_animal_assignment',
    paa.id,
    'create',
    'migration',
    NULL,
    'MuriArc migration',
    'migration',
    NULL,
    paa.reason,
    NULL,
    json_object(
        'id', paa.id,
        'lab_id', paa.lab_id,
        'project_id', paa.project_id,
        'animal_id', paa.animal_id,
        'assigned_by', paa.assigned_by,
        'reason', paa.reason,
        'meta', json_object(
            'created_at', paa.created_at,
            'updated_at', paa.updated_at,
            'deleted_at', paa.deleted_at,
            'revision', paa.revision
        )
    ),
    'project_animal_assignment.create',
    1,
    json_object('backfilled', 1),
    NULL,
    1,
    paa.created_at
FROM project_animal_assignments paa
WHERE paa.reason = 'Backfilled from existing experiment participation';

INSERT INTO provenance (
    id, lab_id, project_id, entity_type, entity_id, source,
    actor_user_id, import_job_id, import_commit_id, tool_run_id,
    provider, model, confidence, request_id, recorded_at
)
SELECT
    lower(substr(hex(randomblob(16)), 1, 8) || '-' ||
          substr(hex(randomblob(16)), 1, 4) || '-4' ||
          substr(hex(randomblob(16)), 1, 3) || '-a' ||
          substr(hex(randomblob(16)), 1, 3) || '-' ||
          substr(hex(randomblob(16)), 1, 12)),
    paa.lab_id,
    paa.project_id,
    'project_animal_assignment',
    paa.id,
    'migration',
    NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
    paa.created_at
FROM project_animal_assignments paa
WHERE paa.reason = 'Backfilled from existing experiment participation';

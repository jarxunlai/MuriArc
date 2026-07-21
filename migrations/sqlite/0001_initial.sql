PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS labs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    email TEXT NOT NULL,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, email)
);

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    name TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_projects_lab ON projects(lab_id, deleted_at);

CREATE TABLE IF NOT EXISTS memberships (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    lab_role TEXT,
    project_role TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    CHECK ((lab_role IS NOT NULL AND project_role IS NULL AND project_id IS NULL)
        OR (lab_role IS NULL AND project_role IS NOT NULL AND project_id IS NOT NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_memberships_lab_role
    ON memberships(lab_id, user_id, lab_role) WHERE project_id IS NULL AND deleted_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_memberships_project_role
    ON memberships(project_id, user_id) WHERE project_id IS NOT NULL AND deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS cages (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    section TEXT NOT NULL,
    display_id TEXT NOT NULL,
    location TEXT,
    kind TEXT NOT NULL,
    sort_order INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, section, display_id)
);
CREATE INDEX IF NOT EXISTS idx_cages_lab ON cages(lab_id, deleted_at, sort_order);

CREATE TABLE IF NOT EXISTS animals (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    identifier_scope TEXT NOT NULL,
    display_id TEXT NOT NULL,
    legacy_id TEXT,
    species TEXT NOT NULL,
    strain TEXT,
    sex TEXT NOT NULL,
    birth_date TEXT,
    death_date TEXT,
    current_cage_id TEXT REFERENCES cages(id),
    current_status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, identifier_scope, display_id)
);
CREATE INDEX IF NOT EXISTS idx_animals_lab ON animals(lab_id, deleted_at, current_status);
CREATE INDEX IF NOT EXISTS idx_animals_cage ON animals(current_cage_id, deleted_at);

CREATE TABLE IF NOT EXISTS animal_events (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    animal_id TEXT NOT NULL REFERENCES animals(id),
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    recorded_by TEXT,
    notes TEXT
);
CREATE INDEX IF NOT EXISTS idx_animal_events_timeline
    ON animal_events(animal_id, occurred_at, recorded_at);

CREATE TABLE IF NOT EXISTS experiments (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    template_version_id TEXT,
    name TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL,
    starts_at TEXT,
    ends_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_experiments_project ON experiments(project_id, deleted_at, status);

CREATE TABLE IF NOT EXISTS experiment_participations (
    id TEXT PRIMARY KEY NOT NULL,
    experiment_id TEXT NOT NULL REFERENCES experiments(id),
    animal_id TEXT NOT NULL REFERENCES animals(id),
    cohort_id TEXT,
    status TEXT NOT NULL,
    enrolled_at TEXT NOT NULL,
    exited_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (experiment_id, animal_id)
);
CREATE INDEX IF NOT EXISTS idx_participations_animal ON experiment_participations(animal_id, deleted_at);

CREATE TABLE IF NOT EXISTS measurements (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    experiment_id TEXT REFERENCES experiments(id),
    animal_id TEXT NOT NULL REFERENCES animals(id),
    procedure_id TEXT,
    measurement_key TEXT NOT NULL,
    label TEXT NOT NULL,
    value_type TEXT NOT NULL,
    value_number REAL,
    value_text TEXT,
    value_boolean INTEGER,
    value_date TEXT,
    unit TEXT,
    measured_at TEXT NOT NULL,
    status TEXT NOT NULL,
    signed_by TEXT,
    signed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    CHECK (value_boolean IS NULL OR value_boolean IN (0, 1))
);
CREATE INDEX IF NOT EXISTS idx_measurements_project ON measurements(project_id, deleted_at, measured_at);
CREATE INDEX IF NOT EXISTS idx_measurements_animal ON measurements(animal_id, deleted_at, measured_at);

CREATE TABLE IF NOT EXISTS samples (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    experiment_id TEXT REFERENCES experiments(id),
    animal_id TEXT NOT NULL REFERENCES animals(id),
    collection_event_id TEXT REFERENCES animal_events(id),
    sample_type TEXT NOT NULL,
    quantity REAL CHECK (quantity IS NULL OR quantity >= 0),
    unit TEXT,
    location TEXT,
    collected_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_samples_project ON samples(project_id, deleted_at, collected_at);
CREATE INDEX IF NOT EXISTS idx_samples_animal ON samples(animal_id, deleted_at, collected_at);

CREATE TABLE IF NOT EXISTS audit_entries (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL,
    actor_type TEXT NOT NULL,
    actor_user_id TEXT,
    actor_display_name TEXT NOT NULL,
    source TEXT NOT NULL,
    request_id TEXT,
    reason TEXT,
    before_json TEXT,
    after_json TEXT,
    occurred_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_lab ON audit_entries(lab_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_audit_entity ON audit_entries(entity_type, entity_id, occurred_at);
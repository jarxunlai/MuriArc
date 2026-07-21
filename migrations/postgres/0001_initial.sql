CREATE TABLE IF NOT EXISTS labs (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    email TEXT NOT NULL,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, email)
);

CREATE TABLE IF NOT EXISTS projects (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    name TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_projects_lab ON projects(lab_id, deleted_at);

CREATE TABLE IF NOT EXISTS memberships (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    user_id UUID NOT NULL REFERENCES users(id),
    lab_role TEXT,
    project_role TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    CHECK ((lab_role IS NOT NULL AND project_role IS NULL AND project_id IS NULL)
        OR (lab_role IS NULL AND project_role IS NOT NULL AND project_id IS NOT NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_memberships_lab_role
    ON memberships(lab_id, user_id, lab_role) WHERE project_id IS NULL AND deleted_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_memberships_project_role
    ON memberships(project_id, user_id) WHERE project_id IS NOT NULL AND deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS cages (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    section TEXT NOT NULL,
    display_id TEXT NOT NULL,
    location TEXT,
    kind TEXT NOT NULL,
    sort_order INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, section, display_id)
);
CREATE INDEX IF NOT EXISTS idx_cages_lab ON cages(lab_id, deleted_at, sort_order);

CREATE TABLE IF NOT EXISTS animals (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    identifier_scope TEXT NOT NULL,
    display_id TEXT NOT NULL,
    legacy_id TEXT,
    species TEXT NOT NULL,
    strain TEXT,
    sex TEXT NOT NULL,
    birth_date DATE,
    death_date DATE,
    current_cage_id UUID REFERENCES cages(id),
    current_status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, identifier_scope, display_id)
);
CREATE INDEX IF NOT EXISTS idx_animals_lab ON animals(lab_id, deleted_at, current_status);
CREATE INDEX IF NOT EXISTS idx_animals_cage ON animals(current_cage_id, deleted_at);

CREATE TABLE IF NOT EXISTS animal_events (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    animal_id UUID NOT NULL REFERENCES animals(id),
    event_type TEXT NOT NULL,
    payload_json JSONB NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL,
    recorded_by UUID,
    notes TEXT
);
CREATE INDEX IF NOT EXISTS idx_animal_events_timeline
    ON animal_events(animal_id, occurred_at, recorded_at);

CREATE TABLE IF NOT EXISTS experiments (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    template_version_id UUID,
    name TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL,
    starts_at TIMESTAMPTZ,
    ends_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_experiments_project ON experiments(project_id, deleted_at, status);

CREATE TABLE IF NOT EXISTS experiment_participations (
    id UUID PRIMARY KEY,
    experiment_id UUID NOT NULL REFERENCES experiments(id),
    animal_id UUID NOT NULL REFERENCES animals(id),
    cohort_id UUID,
    status TEXT NOT NULL,
    enrolled_at TIMESTAMPTZ NOT NULL,
    exited_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (experiment_id, animal_id)
);
CREATE INDEX IF NOT EXISTS idx_participations_animal ON experiment_participations(animal_id, deleted_at);

CREATE TABLE IF NOT EXISTS measurements (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    experiment_id UUID REFERENCES experiments(id),
    animal_id UUID NOT NULL REFERENCES animals(id),
    procedure_id UUID,
    measurement_key TEXT NOT NULL,
    label TEXT NOT NULL,
    value_type TEXT NOT NULL,
    value_number DOUBLE PRECISION,
    value_text TEXT,
    value_boolean BOOLEAN,
    value_date DATE,
    unit TEXT,
    measured_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL,
    signed_by UUID,
    signed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_measurements_project ON measurements(project_id, deleted_at, measured_at);
CREATE INDEX IF NOT EXISTS idx_measurements_animal ON measurements(animal_id, deleted_at, measured_at);

CREATE TABLE IF NOT EXISTS samples (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    experiment_id UUID REFERENCES experiments(id),
    animal_id UUID NOT NULL REFERENCES animals(id),
    collection_event_id UUID REFERENCES animal_events(id),
    sample_type TEXT NOT NULL,
    quantity DOUBLE PRECISION CHECK (quantity IS NULL OR quantity >= 0),
    unit TEXT,
    location TEXT,
    collected_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_samples_project ON samples(project_id, deleted_at, collected_at);
CREATE INDEX IF NOT EXISTS idx_samples_animal ON samples(animal_id, deleted_at, collected_at);

CREATE TABLE IF NOT EXISTS audit_entries (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    entity_type TEXT NOT NULL,
    entity_id UUID NOT NULL,
    action TEXT NOT NULL,
    actor_type TEXT NOT NULL,
    actor_user_id UUID,
    actor_display_name TEXT NOT NULL,
    source TEXT NOT NULL,
    request_id TEXT,
    reason TEXT,
    before_json JSONB,
    after_json JSONB,
    occurred_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_lab ON audit_entries(lab_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_audit_entity ON audit_entries(entity_type, entity_id, occurred_at);
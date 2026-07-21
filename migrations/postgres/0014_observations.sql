CREATE TABLE experiment_events (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    experiment_id UUID NOT NULL REFERENCES experiments(id),
    event_key TEXT NOT NULL,
    label TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    details_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (experiment_id, event_key, occurred_at)
);
CREATE INDEX idx_experiment_events_experiment
    ON experiment_events(experiment_id, deleted_at, occurred_at);

CREATE TABLE observation_definitions (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    experiment_id UUID NOT NULL REFERENCES experiments(id),
    observation_key TEXT NOT NULL,
    label TEXT NOT NULL,
    value_type TEXT NOT NULL,
    unit TEXT,
    categories_json JSONB NOT NULL,
    policy TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (experiment_id, observation_key)
);
CREATE INDEX idx_observation_definitions_experiment
    ON observation_definitions(experiment_id, deleted_at, observation_key);

CREATE TABLE observations (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    experiment_id UUID NOT NULL REFERENCES experiments(id),
    experiment_event_id UUID NOT NULL REFERENCES experiment_events(id),
    definition_id UUID NOT NULL REFERENCES observation_definitions(id),
    subject_type TEXT NOT NULL,
    subject_id UUID NOT NULL,
    context_json JSONB NOT NULL,
    current_value_version INTEGER NOT NULL CHECK (current_value_version > 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (experiment_event_id, definition_id, subject_type, subject_id)
);
CREATE INDEX idx_observations_experiment
    ON observations(experiment_id, deleted_at, experiment_event_id);
CREATE INDEX idx_observations_subject
    ON observations(subject_type, subject_id, deleted_at);

CREATE TABLE observation_values (
    id UUID PRIMARY KEY,
    observation_id UUID NOT NULL REFERENCES observations(id),
    version INTEGER NOT NULL CHECK (version > 0),
    value_json JSONB NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL,
    recorded_by UUID REFERENCES users(id),
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (observation_id, version)
);
CREATE INDEX idx_observation_values_observation
    ON observation_values(observation_id, deleted_at, version);

CREATE TABLE experiment_events (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    experiment_id TEXT NOT NULL REFERENCES experiments(id),
    event_key TEXT NOT NULL,
    label TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    details_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (experiment_id, event_key, occurred_at)
);
CREATE INDEX idx_experiment_events_experiment
    ON experiment_events(experiment_id, deleted_at, occurred_at);

CREATE TABLE observation_definitions (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    experiment_id TEXT NOT NULL REFERENCES experiments(id),
    observation_key TEXT NOT NULL,
    label TEXT NOT NULL,
    value_type TEXT NOT NULL,
    unit TEXT,
    categories_json TEXT NOT NULL,
    policy TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (experiment_id, observation_key)
);
CREATE INDEX idx_observation_definitions_experiment
    ON observation_definitions(experiment_id, deleted_at, observation_key);

CREATE TABLE observations (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    experiment_id TEXT NOT NULL REFERENCES experiments(id),
    experiment_event_id TEXT NOT NULL REFERENCES experiment_events(id),
    definition_id TEXT NOT NULL REFERENCES observation_definitions(id),
    subject_type TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    context_json TEXT NOT NULL,
    current_value_version INTEGER NOT NULL CHECK (current_value_version > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (experiment_event_id, definition_id, subject_type, subject_id)
);
CREATE INDEX idx_observations_experiment
    ON observations(experiment_id, deleted_at, experiment_event_id);
CREATE INDEX idx_observations_subject
    ON observations(subject_type, subject_id, deleted_at);

CREATE TABLE observation_values (
    id TEXT PRIMARY KEY NOT NULL,
    observation_id TEXT NOT NULL REFERENCES observations(id),
    version INTEGER NOT NULL CHECK (version > 0),
    value_json TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    recorded_by TEXT REFERENCES users(id),
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (observation_id, version)
);
CREATE INDEX idx_observation_values_observation
    ON observation_values(observation_id, deleted_at, version);

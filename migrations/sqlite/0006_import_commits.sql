CREATE TABLE IF NOT EXISTS import_commits (
    commit_id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
    preview_hash TEXT NOT NULL CHECK (length(preview_hash) = 64),
    animal_count INTEGER NOT NULL CHECK (animal_count >= 0),
    animal_event_count INTEGER NOT NULL CHECK (animal_event_count >= 0),
    genotype_count INTEGER NOT NULL CHECK (genotype_count >= 0),
    pedigree_count INTEGER NOT NULL CHECK (pedigree_count >= 0),
    measurement_count INTEGER NOT NULL CHECK (measurement_count >= 0),
    committed_at TEXT NOT NULL,
    UNIQUE (lab_id, idempotency_key),
    UNIQUE (lab_id, preview_hash)
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_measurements_animal_key_time_active
    ON measurements (animal_id, measurement_key, measured_at)
    WHERE deleted_at IS NULL;

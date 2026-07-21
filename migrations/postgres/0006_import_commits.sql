CREATE TABLE IF NOT EXISTS import_commits (
    commit_id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
    preview_hash TEXT NOT NULL CHECK (length(preview_hash) = 64),
    animal_count BIGINT NOT NULL CHECK (animal_count >= 0),
    animal_event_count BIGINT NOT NULL CHECK (animal_event_count >= 0),
    genotype_count BIGINT NOT NULL CHECK (genotype_count >= 0),
    pedigree_count BIGINT NOT NULL CHECK (pedigree_count >= 0),
    measurement_count BIGINT NOT NULL CHECK (measurement_count >= 0),
    committed_at TIMESTAMPTZ NOT NULL,
    UNIQUE (lab_id, idempotency_key),
    UNIQUE (lab_id, preview_hash)
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_measurements_animal_key_time_active
    ON measurements (animal_id, measurement_key, measured_at)
    WHERE deleted_at IS NULL;

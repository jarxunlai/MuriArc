ALTER TABLE experiment_participations
    ADD COLUMN genotype_snapshot_json JSONB NOT NULL DEFAULT '[]'::jsonb;

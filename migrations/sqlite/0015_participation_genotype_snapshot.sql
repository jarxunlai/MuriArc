ALTER TABLE experiment_participations
    ADD COLUMN genotype_snapshot_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE genotyping_records ADD COLUMN supersedes_record_id TEXT REFERENCES genotyping_records(id);
ALTER TABLE genotyping_records ADD COLUMN voided_at TEXT;
ALTER TABLE genotyping_records ADD COLUMN void_reason TEXT;

CREATE INDEX idx_genotyping_records_supersedes
    ON genotyping_records(supersedes_record_id);
CREATE INDEX idx_genotyping_records_current
    ON genotyping_records(animal_id, genotype_definition_id, voided_at, created_at, id);

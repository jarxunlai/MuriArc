CREATE TABLE genotype_definitions (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    name TEXT NOT NULL,
    description TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, name)
);
CREATE INDEX idx_genotype_definitions_lab
    ON genotype_definitions(lab_id, deleted_at, name);

CREATE TABLE genotype_components (
    id TEXT PRIMARY KEY NOT NULL,
    genotype_definition_id TEXT NOT NULL REFERENCES genotype_definitions(id),
    locus_id TEXT NOT NULL REFERENCES gene_loci(id),
    allele_1_id TEXT NOT NULL REFERENCES alleles(id),
    allele_2_id TEXT REFERENCES alleles(id),
    mode TEXT NOT NULL,
    display_order INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (genotype_definition_id, locus_id, display_order)
);
CREATE INDEX idx_genotype_components_definition
    ON genotype_components(genotype_definition_id, deleted_at, display_order);

CREATE TABLE genotyping_records (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    animal_id TEXT NOT NULL REFERENCES animals(id),
    genotype_definition_id TEXT NOT NULL REFERENCES genotype_definitions(id),
    state TEXT NOT NULL,
    assessed_at TEXT,
    method TEXT,
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX idx_genotyping_records_animal
    ON genotyping_records(animal_id, deleted_at, created_at);
CREATE INDEX idx_genotyping_records_definition
    ON genotyping_records(genotype_definition_id, deleted_at, created_at);

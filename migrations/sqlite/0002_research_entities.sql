CREATE TABLE IF NOT EXISTS gene_loci (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    symbol TEXT NOT NULL,
    description TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, symbol)
);

CREATE TABLE IF NOT EXISTS alleles (
    id TEXT PRIMARY KEY NOT NULL,
    locus_id TEXT NOT NULL REFERENCES gene_loci(id),
    symbol TEXT NOT NULL,
    description TEXT,
    is_wild_type INTEGER NOT NULL CHECK (is_wild_type IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (locus_id, symbol)
);

CREATE TABLE IF NOT EXISTS genotypes (
    id TEXT PRIMARY KEY NOT NULL,
    animal_id TEXT NOT NULL REFERENCES animals(id),
    locus_id TEXT NOT NULL REFERENCES gene_loci(id),
    allele_1_id TEXT REFERENCES alleles(id),
    allele_2_id TEXT REFERENCES alleles(id),
    assessed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (animal_id, locus_id)
);
CREATE INDEX IF NOT EXISTS idx_genotypes_animal ON genotypes(animal_id, deleted_at);

CREATE TABLE IF NOT EXISTS pedigrees (
    id TEXT PRIMARY KEY NOT NULL,
    animal_id TEXT NOT NULL REFERENCES animals(id),
    parent_id TEXT NOT NULL REFERENCES animals(id),
    parent_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    CHECK (animal_id <> parent_id),
    UNIQUE (animal_id, parent_id, parent_type)
);
CREATE INDEX IF NOT EXISTS idx_pedigrees_animal ON pedigrees(animal_id, deleted_at);

CREATE TABLE IF NOT EXISTS experiment_template_versions (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    template_key TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    name TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL,
    fields_json TEXT NOT NULL,
    published_at TEXT,
    published_by TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, template_key, version)
);
CREATE INDEX IF NOT EXISTS idx_templates_lab ON experiment_template_versions(lab_id, template_key, version);

CREATE TABLE IF NOT EXISTS cohorts (
    id TEXT PRIMARY KEY NOT NULL,
    experiment_id TEXT NOT NULL REFERENCES experiments(id),
    name TEXT NOT NULL,
    description TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (experiment_id, name)
);

CREATE TABLE IF NOT EXISTS procedures (
    id TEXT PRIMARY KEY NOT NULL,
    experiment_id TEXT NOT NULL REFERENCES experiments(id),
    animal_id TEXT REFERENCES animals(id),
    name TEXT NOT NULL,
    scheduled_at TEXT,
    performed_at TEXT,
    status TEXT NOT NULL,
    details_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_procedures_experiment ON procedures(experiment_id, animal_id, deleted_at);

CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    file_name TEXT NOT NULL,
    media_type TEXT,
    relative_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64),
    version INTEGER NOT NULL CHECK (version > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, entity_type, entity_id, file_name, version)
);
CREATE INDEX IF NOT EXISTS idx_attachments_entity ON attachments(entity_type, entity_id, deleted_at);
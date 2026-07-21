CREATE TABLE IF NOT EXISTS gene_loci (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    symbol TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, symbol)
);
CREATE TABLE IF NOT EXISTS alleles (
    id UUID PRIMARY KEY,
    locus_id UUID NOT NULL REFERENCES gene_loci(id),
    symbol TEXT NOT NULL,
    description TEXT,
    is_wild_type BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (locus_id, symbol)
);
CREATE TABLE IF NOT EXISTS genotypes (
    id UUID PRIMARY KEY,
    animal_id UUID NOT NULL REFERENCES animals(id),
    locus_id UUID NOT NULL REFERENCES gene_loci(id),
    allele_1_id UUID REFERENCES alleles(id),
    allele_2_id UUID REFERENCES alleles(id),
    assessed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (animal_id, locus_id)
);
CREATE INDEX IF NOT EXISTS idx_genotypes_animal ON genotypes(animal_id, deleted_at);
CREATE TABLE IF NOT EXISTS pedigrees (
    id UUID PRIMARY KEY,
    animal_id UUID NOT NULL REFERENCES animals(id),
    parent_id UUID NOT NULL REFERENCES animals(id),
    parent_type TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    CHECK (animal_id <> parent_id),
    UNIQUE (animal_id, parent_id, parent_type)
);
CREATE INDEX IF NOT EXISTS idx_pedigrees_animal ON pedigrees(animal_id, deleted_at);
CREATE TABLE IF NOT EXISTS experiment_template_versions (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    template_key TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    name TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL,
    fields_json JSONB NOT NULL,
    published_at TIMESTAMPTZ,
    published_by UUID,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, template_key, version)
);
CREATE INDEX IF NOT EXISTS idx_templates_lab ON experiment_template_versions(lab_id, template_key, version);
CREATE TABLE IF NOT EXISTS cohorts (
    id UUID PRIMARY KEY,
    experiment_id UUID NOT NULL REFERENCES experiments(id),
    name TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (experiment_id, name)
);
CREATE TABLE IF NOT EXISTS procedures (
    id UUID PRIMARY KEY,
    experiment_id UUID NOT NULL REFERENCES experiments(id),
    animal_id UUID REFERENCES animals(id),
    name TEXT NOT NULL,
    scheduled_at TIMESTAMPTZ,
    performed_at TIMESTAMPTZ,
    status TEXT NOT NULL,
    details_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);
CREATE INDEX IF NOT EXISTS idx_procedures_experiment ON procedures(experiment_id, animal_id, deleted_at);
CREATE TABLE IF NOT EXISTS attachments (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    entity_type TEXT NOT NULL,
    entity_id UUID NOT NULL,
    file_name TEXT NOT NULL,
    media_type TEXT,
    relative_path TEXT NOT NULL,
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    sha256 CHAR(64) NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, entity_type, entity_id, file_name, version)
);
CREATE INDEX IF NOT EXISTS idx_attachments_entity ON attachments(entity_type, entity_id, deleted_at);
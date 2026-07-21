CREATE TABLE genotype_definitions (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    name TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, name)
);
CREATE INDEX idx_genotype_definitions_lab
    ON genotype_definitions(lab_id, deleted_at, name);

CREATE TABLE genotype_components (
    id UUID PRIMARY KEY,
    genotype_definition_id UUID NOT NULL REFERENCES genotype_definitions(id),
    locus_id UUID NOT NULL REFERENCES gene_loci(id),
    allele_1_id UUID NOT NULL REFERENCES alleles(id),
    allele_2_id UUID REFERENCES alleles(id),
    mode TEXT NOT NULL,
    display_order INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (genotype_definition_id, locus_id, display_order)
);
CREATE INDEX idx_genotype_components_definition
    ON genotype_components(genotype_definition_id, deleted_at, display_order);

CREATE TABLE genotyping_records (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    animal_id UUID NOT NULL REFERENCES animals(id),
    genotype_definition_id UUID NOT NULL REFERENCES genotype_definitions(id),
    state TEXT NOT NULL,
    assessed_at TIMESTAMPTZ,
    method TEXT,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);
CREATE INDEX idx_genotyping_records_animal
    ON genotyping_records(animal_id, deleted_at, created_at);
CREATE INDEX idx_genotyping_records_definition
    ON genotyping_records(genotype_definition_id, deleted_at, created_at);

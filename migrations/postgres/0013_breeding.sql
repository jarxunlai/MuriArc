CREATE TABLE breeding_lines (
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
CREATE INDEX idx_breeding_lines_lab ON breeding_lines(lab_id, deleted_at, name);

CREATE TABLE breeding_line_genotype_definitions (
    breeding_line_id UUID NOT NULL REFERENCES breeding_lines(id),
    genotype_definition_id UUID NOT NULL REFERENCES genotype_definitions(id),
    display_order INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (breeding_line_id, genotype_definition_id),
    UNIQUE (breeding_line_id, display_order)
);

CREATE TABLE colonies (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    breeding_line_id UUID NOT NULL REFERENCES breeding_lines(id),
    name TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, name)
);
CREATE INDEX idx_colonies_line ON colonies(breeding_line_id, deleted_at, name);

CREATE TABLE breeding_pairs (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    colony_id UUID NOT NULL REFERENCES colonies(id),
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (colony_id, name)
);
CREATE INDEX idx_breeding_pairs_colony ON breeding_pairs(colony_id, deleted_at, status, name);

CREATE TABLE breeding_pair_members (
    id UUID PRIMARY KEY,
    breeding_pair_id UUID NOT NULL REFERENCES breeding_pairs(id),
    animal_id UUID NOT NULL REFERENCES animals(id),
    role TEXT NOT NULL,
    joined_at TIMESTAMPTZ NOT NULL,
    left_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (breeding_pair_id, animal_id)
);
CREATE INDEX idx_breeding_pair_members_pair ON breeding_pair_members(breeding_pair_id, deleted_at, role);
CREATE INDEX idx_breeding_pair_members_animal ON breeding_pair_members(animal_id, deleted_at, left_at);

CREATE TABLE mating_events (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    breeding_pair_id UUID NOT NULL REFERENCES breeding_pairs(id),
    male_animal_id UUID NOT NULL REFERENCES animals(id),
    female_animal_id UUID NOT NULL REFERENCES animals(id),
    occurred_at TIMESTAMPTZ NOT NULL,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);
CREATE INDEX idx_mating_events_pair ON mating_events(breeding_pair_id, deleted_at, occurred_at);

CREATE TABLE litters (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    mating_event_id UUID NOT NULL REFERENCES mating_events(id),
    born_on DATE NOT NULL,
    size_total INTEGER NOT NULL CHECK (size_total >= 0),
    size_alive INTEGER NOT NULL CHECK (size_alive >= 0 AND size_alive <= size_total),
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);
CREATE INDEX idx_litters_mating ON litters(mating_event_id, deleted_at, born_on);

CREATE TABLE animal_drafts (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    litter_id UUID NOT NULL REFERENCES litters(id),
    temporary_label TEXT NOT NULL,
    sex TEXT NOT NULL,
    birth_date DATE NOT NULL,
    status TEXT NOT NULL,
    registered_animal_id UUID REFERENCES animals(id),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (litter_id, temporary_label),
    UNIQUE (registered_animal_id)
);
CREATE INDEX idx_animal_drafts_litter ON animal_drafts(litter_id, deleted_at, status, temporary_label);

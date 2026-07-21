CREATE TABLE breeding_lines (
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
CREATE INDEX idx_breeding_lines_lab ON breeding_lines(lab_id, deleted_at, name);

CREATE TABLE breeding_line_genotype_definitions (
    breeding_line_id TEXT NOT NULL REFERENCES breeding_lines(id),
    genotype_definition_id TEXT NOT NULL REFERENCES genotype_definitions(id),
    display_order INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (breeding_line_id, genotype_definition_id),
    UNIQUE (breeding_line_id, display_order)
);

CREATE TABLE colonies (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    breeding_line_id TEXT NOT NULL REFERENCES breeding_lines(id),
    name TEXT NOT NULL,
    description TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, name)
);
CREATE INDEX idx_colonies_line ON colonies(breeding_line_id, deleted_at, name);

CREATE TABLE breeding_pairs (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    colony_id TEXT NOT NULL REFERENCES colonies(id),
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (colony_id, name)
);
CREATE INDEX idx_breeding_pairs_colony ON breeding_pairs(colony_id, deleted_at, status, name);

CREATE TABLE breeding_pair_members (
    id TEXT PRIMARY KEY NOT NULL,
    breeding_pair_id TEXT NOT NULL REFERENCES breeding_pairs(id),
    animal_id TEXT NOT NULL REFERENCES animals(id),
    role TEXT NOT NULL,
    joined_at TEXT NOT NULL,
    left_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (breeding_pair_id, animal_id)
);
CREATE INDEX idx_breeding_pair_members_pair ON breeding_pair_members(breeding_pair_id, deleted_at, role);
CREATE INDEX idx_breeding_pair_members_animal ON breeding_pair_members(animal_id, deleted_at, left_at);

CREATE TABLE mating_events (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    breeding_pair_id TEXT NOT NULL REFERENCES breeding_pairs(id),
    male_animal_id TEXT NOT NULL REFERENCES animals(id),
    female_animal_id TEXT NOT NULL REFERENCES animals(id),
    occurred_at TEXT NOT NULL,
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX idx_mating_events_pair ON mating_events(breeding_pair_id, deleted_at, occurred_at);

CREATE TABLE litters (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    mating_event_id TEXT NOT NULL REFERENCES mating_events(id),
    born_on TEXT NOT NULL,
    size_total INTEGER NOT NULL CHECK (size_total >= 0),
    size_alive INTEGER NOT NULL CHECK (size_alive >= 0 AND size_alive <= size_total),
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);
CREATE INDEX idx_litters_mating ON litters(mating_event_id, deleted_at, born_on);

CREATE TABLE animal_drafts (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    litter_id TEXT NOT NULL REFERENCES litters(id),
    temporary_label TEXT NOT NULL,
    sex TEXT NOT NULL,
    birth_date TEXT NOT NULL,
    status TEXT NOT NULL,
    registered_animal_id TEXT REFERENCES animals(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (litter_id, temporary_label),
    UNIQUE (registered_animal_id)
);
CREATE INDEX idx_animal_drafts_litter ON animal_drafts(litter_id, deleted_at, status, temporary_label);

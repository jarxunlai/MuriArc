PRAGMA foreign_keys = OFF;

CREATE TABLE location (
    id INTEGER PRIMARY KEY,
    identifier TEXT NOT NULL UNIQUE,
    description TEXT,
    "order" INTEGER NOT NULL
);

CREATE TABLE cage (
    id INTEGER PRIMARY KEY,
    section TEXT NOT NULL,
    cage_id TEXT NOT NULL,
    location TEXT,
    cage_type TEXT,
    "order" INTEGER NOT NULL,
    mice_birth_date TEXT,
    mice_count INTEGER,
    mice_sex TEXT,
    mice_genotype TEXT
);

CREATE TABLE mouse (
    tid INTEGER PRIMARY KEY,
    id TEXT NOT NULL,
    sex TEXT,
    live_status INTEGER,
    birth_date TEXT,
    death_date TEXT,
    cage_id INTEGER,
    strain TEXT,
    tests_planned TEXT
);

CREATE TABLE gene_locus (
    id INTEGER PRIMARY KEY,
    symbol TEXT NOT NULL UNIQUE,
    description TEXT
);

CREATE TABLE allele (
    id INTEGER PRIMARY KEY,
    symbol TEXT NOT NULL,
    locus_id INTEGER NOT NULL,
    description TEXT,
    is_wildtype INTEGER
);

CREATE TABLE genotype (
    id INTEGER PRIMARY KEY,
    mouse_id INTEGER NOT NULL,
    locus_id INTEGER NOT NULL,
    allele1_id INTEGER,
    allele2_id INTEGER
);

CREATE TABLE pedigree (
    id INTEGER PRIMARY KEY,
    mouse_id INTEGER,
    parent_id INTEGER,
    parent_type TEXT
);

INSERT INTO location VALUES (1, '默认区域', 'fixture', 0);
INSERT INTO cage VALUES
    (1, '默认区域', '01', 'A-01', 'breeding', 0, '2026-01-01', 9, 'Mixed', 'GeneA');

INSERT INTO mouse VALUES
    (1, 'M1', 'M', 1, '2026-01-01', NULL, 1, 'C57BL/6J', '[]'),
    (2, 'M1', 'F', 1, '2026-01-02', NULL, 1, 'C57BL/6J', '[]'),
    (3, 'M2', 'F', 0, '2026-01-03', '2026-02-01', 1, 'C57BL/6J', '[]');

INSERT INTO gene_locus VALUES (1, 'GeneA', 'fixture locus');
INSERT INTO allele VALUES
    (1, 'WT', 1, 'wild type', 1),
    (2, 'KO', 1, 'knockout', 0);
INSERT INTO genotype VALUES
    (1, 1, 1, 1, 2),
    (2, 2, 1, 2, 2),
    (3, 3, 1, 1, 1);

INSERT INTO pedigree VALUES
    (1, 2, 1, 'father'),
    (2, NULL, 1, 'father');

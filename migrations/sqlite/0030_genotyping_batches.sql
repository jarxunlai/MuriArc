CREATE TABLE genotyping_batches (
    id TEXT PRIMARY KEY,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    project_id TEXT REFERENCES projects(id),
    batch_number TEXT NOT NULL,
    genotype_definition_id TEXT NOT NULL REFERENCES genotype_definitions(id),
    assessed_at TEXT NOT NULL,
    method TEXT,
    notes TEXT,
    status TEXT NOT NULL CHECK (status IN ('draft', 'committed', 'cancelled')),
    created_by TEXT REFERENCES users(id),
    source_attachment_id TEXT REFERENCES attachments(id),
    preview_hash TEXT,
    preview_row_count INTEGER,
    committed_at TEXT,
    cancelled_at TEXT,
    cancel_reason TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, batch_number),
    CHECK (
        (source_attachment_id IS NULL AND preview_hash IS NULL AND preview_row_count IS NULL)
        OR
        (
            source_attachment_id IS NOT NULL
            AND length(preview_hash) = 64
            AND preview_hash NOT GLOB '*[^0-9A-Fa-f]*'
            AND preview_row_count > 0
            AND preview_row_count <= 5000
        )
    ),
    CHECK (
        (status = 'draft' AND committed_at IS NULL AND cancelled_at IS NULL AND cancel_reason IS NULL)
        OR
        (status = 'committed' AND committed_at IS NOT NULL AND cancelled_at IS NULL AND cancel_reason IS NULL AND source_attachment_id IS NOT NULL)
        OR
        (status = 'cancelled' AND committed_at IS NULL AND cancelled_at IS NOT NULL AND length(trim(cancel_reason)) > 0)
    )
);

CREATE INDEX idx_genotyping_batches_scope
    ON genotyping_batches(lab_id, project_id, status, assessed_at DESC, id);
CREATE INDEX idx_genotyping_batches_definition
    ON genotyping_batches(genotype_definition_id, assessed_at DESC, id);

CREATE TABLE genotyping_batch_records (
    batch_id TEXT NOT NULL REFERENCES genotyping_batches(id),
    record_id TEXT NOT NULL UNIQUE REFERENCES genotyping_records(id),
    display_order INTEGER NOT NULL CHECK (display_order >= 0),
    PRIMARY KEY (batch_id, record_id),
    UNIQUE (batch_id, display_order)
);

CREATE INDEX idx_genotyping_batch_records_record
    ON genotyping_batch_records(record_id, batch_id);

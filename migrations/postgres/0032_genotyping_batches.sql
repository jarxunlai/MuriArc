CREATE TABLE genotyping_batches (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    project_id UUID REFERENCES projects(id),
    batch_number TEXT NOT NULL,
    genotype_definition_id UUID NOT NULL REFERENCES genotype_definitions(id),
    assessed_at TIMESTAMPTZ NOT NULL,
    method TEXT,
    notes TEXT,
    status TEXT NOT NULL CHECK (status IN ('draft', 'committed', 'cancelled')),
    created_by UUID REFERENCES users(id),
    source_attachment_id UUID REFERENCES attachments(id),
    preview_hash TEXT,
    preview_row_count BIGINT,
    committed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    cancel_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, batch_number),
    CHECK (
        (source_attachment_id IS NULL AND preview_hash IS NULL AND preview_row_count IS NULL)
        OR
        (
            source_attachment_id IS NOT NULL
            AND length(preview_hash) = 64
            AND preview_hash ~ '^[0-9A-Fa-f]{64}$'
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
    batch_id UUID NOT NULL REFERENCES genotyping_batches(id),
    record_id UUID NOT NULL UNIQUE REFERENCES genotyping_records(id),
    display_order INTEGER NOT NULL CHECK (display_order >= 0),
    PRIMARY KEY (batch_id, record_id),
    UNIQUE (batch_id, display_order)
);

CREATE INDEX idx_genotyping_batch_records_record
    ON genotyping_batch_records(record_id, batch_id);

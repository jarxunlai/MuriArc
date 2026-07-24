ALTER TABLE ai_extraction_drafts ADD COLUMN data_cell_definition_id TEXT
    REFERENCES observation_definitions(id);
ALTER TABLE ai_extraction_drafts ADD COLUMN data_cell_subject_type TEXT;
ALTER TABLE ai_extraction_drafts ADD COLUMN data_cell_subject_id TEXT;
ALTER TABLE ai_extraction_drafts ADD COLUMN model_profile_id TEXT;
ALTER TABLE ai_extraction_drafts ADD COLUMN model_profile_version INTEGER;
ALTER TABLE ai_extraction_drafts ADD COLUMN model_purpose TEXT;
ALTER TABLE ai_extraction_drafts ADD COLUMN usage_input_tokens INTEGER;
ALTER TABLE ai_extraction_drafts ADD COLUMN usage_output_tokens INTEGER;
ALTER TABLE ai_extraction_drafts ADD COLUMN usage_total_tokens INTEGER;
ALTER TABLE ai_extraction_drafts ADD COLUMN provider_request_id TEXT;
ALTER TABLE ai_extraction_drafts ADD COLUMN trace_json TEXT;

UPDATE attachment_derivatives
SET project_id = NULL
WHERE kind = 'ai_input'
  AND deleted_at IS NULL
  AND project_id IS NOT NULL
  AND attachment_id IN (
      SELECT attachment_id
      FROM ai_private_images
      WHERE deleted_at IS NULL
  );

CREATE TRIGGER IF NOT EXISTS trg_ai_extraction_drafts_phase4_insert
BEFORE INSERT ON ai_extraction_drafts
WHEN
    (
        (NEW.data_cell_definition_id IS NULL)
        <> (NEW.data_cell_subject_type IS NULL)
    )
    OR (
        (NEW.data_cell_definition_id IS NULL)
        <> (NEW.data_cell_subject_id IS NULL)
    )
    OR (
        (NEW.data_cell_definition_id IS NULL)
        <> (NEW.model_profile_id IS NULL)
    )
    OR (
        NEW.model_profile_id IS NOT NULL
        AND (
            NEW.model_profile_version IS NULL
            OR NEW.model_profile_version <= 0
            OR NEW.model_purpose IS NULL
            OR NEW.model_purpose != 'vision'
            OR NEW.usage_input_tokens IS NULL
            OR NEW.usage_input_tokens < 0
            OR NEW.usage_output_tokens IS NULL
            OR NEW.usage_output_tokens < 0
            OR NEW.usage_total_tokens IS NULL
            OR NEW.usage_total_tokens < NEW.usage_input_tokens + NEW.usage_output_tokens
            OR NEW.trace_json IS NULL
            OR json_type(NEW.trace_json) != 'object'
            OR length(CAST(NEW.trace_json AS BLOB)) > 16384
            OR (
                NEW.provider_request_id IS NOT NULL
                AND length(NEW.provider_request_id) NOT BETWEEN 1 AND 256
            )
            OR NOT EXISTS (
                SELECT 1
                FROM ai_model_profile_versions
                WHERE profile_id = NEW.model_profile_id
                  AND version = NEW.model_profile_version
            )
        )
    )
    OR (
        NEW.model_profile_id IS NULL
        AND (
            NEW.model_profile_version IS NOT NULL
            OR NEW.model_purpose IS NOT NULL
            OR NEW.usage_input_tokens IS NOT NULL
            OR NEW.usage_output_tokens IS NOT NULL
            OR NEW.usage_total_tokens IS NOT NULL
            OR NEW.provider_request_id IS NOT NULL
            OR NEW.trace_json IS NOT NULL
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'invalid phase 4 AI extraction binding');
END;

CREATE UNIQUE INDEX IF NOT EXISTS uq_ai_extraction_drafts_unresolved_data_cell
    ON ai_extraction_drafts(
        lab_id,
        user_id,
        project_id,
        experiment_id,
        experiment_event_id,
        data_cell_definition_id,
        data_cell_subject_type,
        data_cell_subject_id
    )
    WHERE deleted_at IS NULL
      AND data_cell_definition_id IS NOT NULL
      AND status IN ('draft', 'pending_approval');

CREATE TABLE IF NOT EXISTS ai_extraction_evidence (
    draft_id TEXT NOT NULL REFERENCES ai_extraction_drafts(id) ON DELETE CASCADE,
    display_order INTEGER NOT NULL CHECK (display_order BETWEEN 0 AND 7),
    private_image_id TEXT NOT NULL REFERENCES ai_private_images(id),
    private_attachment_id TEXT NOT NULL REFERENCES attachments(id),
    promoted_attachment_id TEXT REFERENCES attachments(id),
    original_sha256 TEXT NOT NULL CHECK (length(original_sha256) = 64),
    sanitized_sha256 TEXT NOT NULL CHECK (length(sanitized_sha256) = 64),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    PRIMARY KEY (draft_id, display_order),
    UNIQUE (draft_id, private_image_id),
    UNIQUE (draft_id, private_attachment_id),
    CHECK (
        promoted_attachment_id IS NULL
        OR promoted_attachment_id = private_attachment_id
    )
);

CREATE INDEX IF NOT EXISTS idx_ai_extraction_evidence_private_image
    ON ai_extraction_evidence(private_image_id);
CREATE INDEX IF NOT EXISTS idx_ai_extraction_evidence_promoted_attachment
    ON ai_extraction_evidence(promoted_attachment_id)
    WHERE promoted_attachment_id IS NOT NULL;

CREATE TRIGGER IF NOT EXISTS trg_ai_extraction_evidence_immutable
BEFORE UPDATE ON ai_extraction_evidence
WHEN NEW.draft_id IS NOT OLD.draft_id
    OR NEW.display_order IS NOT OLD.display_order
    OR NEW.private_image_id IS NOT OLD.private_image_id
    OR NEW.private_attachment_id IS NOT OLD.private_attachment_id
    OR NEW.original_sha256 IS NOT OLD.original_sha256
    OR NEW.sanitized_sha256 IS NOT OLD.sanitized_sha256
    OR NEW.created_at IS NOT OLD.created_at
    OR OLD.promoted_attachment_id IS NOT NULL
    OR NEW.promoted_attachment_id IS NULL
    OR NEW.revision != OLD.revision + 1
    OR NEW.updated_at <= OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'AI extraction evidence is immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_ai_extraction_binding_immutable
BEFORE UPDATE ON ai_extraction_drafts
WHEN NEW.data_cell_definition_id IS NOT OLD.data_cell_definition_id
    OR NEW.data_cell_subject_type IS NOT OLD.data_cell_subject_type
    OR NEW.data_cell_subject_id IS NOT OLD.data_cell_subject_id
    OR NEW.model_profile_id IS NOT OLD.model_profile_id
    OR NEW.model_profile_version IS NOT OLD.model_profile_version
    OR NEW.model_purpose IS NOT OLD.model_purpose
    OR NEW.usage_input_tokens IS NOT OLD.usage_input_tokens
    OR NEW.usage_output_tokens IS NOT OLD.usage_output_tokens
    OR NEW.usage_total_tokens IS NOT OLD.usage_total_tokens
    OR NEW.provider_request_id IS NOT OLD.provider_request_id
    OR NEW.trace_json IS NOT OLD.trace_json
BEGIN
    SELECT RAISE(ABORT, 'AI extraction model and data-cell binding is immutable');
END;

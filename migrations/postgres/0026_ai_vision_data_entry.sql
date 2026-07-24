ALTER TABLE ai_extraction_drafts
    ADD COLUMN IF NOT EXISTS data_cell_definition_id UUID,
    ADD COLUMN IF NOT EXISTS data_cell_subject_type TEXT,
    ADD COLUMN IF NOT EXISTS data_cell_subject_id UUID,
    ADD COLUMN IF NOT EXISTS model_profile_id UUID,
    ADD COLUMN IF NOT EXISTS model_profile_version BIGINT,
    ADD COLUMN IF NOT EXISTS model_purpose TEXT,
    ADD COLUMN IF NOT EXISTS usage_input_tokens BIGINT,
    ADD COLUMN IF NOT EXISTS usage_output_tokens BIGINT,
    ADD COLUMN IF NOT EXISTS usage_total_tokens BIGINT,
    ADD COLUMN IF NOT EXISTS provider_request_id TEXT,
    ADD COLUMN IF NOT EXISTS trace_json JSONB;

UPDATE attachment_derivatives derivative
SET project_id = NULL
WHERE derivative.kind = 'ai_input'
  AND derivative.deleted_at IS NULL
  AND derivative.project_id IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM ai_private_images image
      WHERE image.attachment_id = derivative.attachment_id
        AND image.deleted_at IS NULL
  );

ALTER TABLE ai_extraction_drafts
    DROP CONSTRAINT IF EXISTS ai_extraction_drafts_data_cell_check;
ALTER TABLE ai_extraction_drafts
    ADD CONSTRAINT ai_extraction_drafts_data_cell_check CHECK (
        (
            data_cell_definition_id IS NULL
            AND data_cell_subject_type IS NULL
            AND data_cell_subject_id IS NULL
        )
        OR (
            data_cell_definition_id IS NOT NULL
            AND data_cell_subject_type IS NOT NULL
            AND data_cell_subject_id IS NOT NULL
        )
    );

ALTER TABLE ai_extraction_drafts
    DROP CONSTRAINT IF EXISTS ai_extraction_drafts_model_trace_check;
ALTER TABLE ai_extraction_drafts
    ADD CONSTRAINT ai_extraction_drafts_model_trace_check CHECK (
        (
            model_profile_id IS NULL
            AND model_profile_version IS NULL
            AND model_purpose IS NULL
            AND usage_input_tokens IS NULL
            AND usage_output_tokens IS NULL
            AND usage_total_tokens IS NULL
            AND provider_request_id IS NULL
            AND trace_json IS NULL
        )
        OR (
            model_profile_id IS NOT NULL
            AND model_profile_version IS NOT NULL
            AND model_profile_version > 0
            AND model_purpose IS NOT NULL
            AND model_purpose = 'vision'
            AND usage_input_tokens IS NOT NULL
            AND usage_input_tokens >= 0
            AND usage_output_tokens IS NOT NULL
            AND usage_output_tokens >= 0
            AND usage_total_tokens IS NOT NULL
            AND usage_total_tokens >= usage_input_tokens + usage_output_tokens
            AND trace_json IS NOT NULL
            AND jsonb_typeof(trace_json) = 'object'
            AND octet_length(trace_json::text) <= 16384
            AND (
                provider_request_id IS NULL
                OR length(provider_request_id) BETWEEN 1 AND 256
            )
        )
    );

ALTER TABLE ai_extraction_drafts
    DROP CONSTRAINT IF EXISTS ai_extraction_drafts_phase4_binding_check;
ALTER TABLE ai_extraction_drafts
    ADD CONSTRAINT ai_extraction_drafts_phase4_binding_check CHECK (
        (data_cell_definition_id IS NULL) = (model_profile_id IS NULL)
    );

ALTER TABLE ai_extraction_drafts
    DROP CONSTRAINT IF EXISTS ai_extraction_drafts_data_cell_definition_fkey;
ALTER TABLE ai_extraction_drafts
    ADD CONSTRAINT ai_extraction_drafts_data_cell_definition_fkey
    FOREIGN KEY (data_cell_definition_id) REFERENCES observation_definitions(id);

ALTER TABLE ai_extraction_drafts
    DROP CONSTRAINT IF EXISTS ai_extraction_drafts_model_profile_version_fkey;
ALTER TABLE ai_extraction_drafts
    ADD CONSTRAINT ai_extraction_drafts_model_profile_version_fkey
    FOREIGN KEY (model_profile_id, model_profile_version)
    REFERENCES ai_model_profile_versions(profile_id, version);

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
    draft_id UUID NOT NULL REFERENCES ai_extraction_drafts(id) ON DELETE CASCADE,
    display_order INTEGER NOT NULL CHECK (display_order BETWEEN 0 AND 7),
    private_image_id UUID NOT NULL REFERENCES ai_private_images(id),
    private_attachment_id UUID NOT NULL REFERENCES attachments(id),
    promoted_attachment_id UUID REFERENCES attachments(id),
    original_sha256 CHAR(64) NOT NULL,
    sanitized_sha256 CHAR(64) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
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

CREATE OR REPLACE FUNCTION muriarc_restrict_ai_extraction_evidence_update()
RETURNS trigger AS $$
BEGIN
    IF NEW.draft_id IS DISTINCT FROM OLD.draft_id
        OR NEW.display_order IS DISTINCT FROM OLD.display_order
        OR NEW.private_image_id IS DISTINCT FROM OLD.private_image_id
        OR NEW.private_attachment_id IS DISTINCT FROM OLD.private_attachment_id
        OR NEW.original_sha256 IS DISTINCT FROM OLD.original_sha256
        OR NEW.sanitized_sha256 IS DISTINCT FROM OLD.sanitized_sha256
        OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'AI extraction evidence is immutable'
            USING ERRCODE = '23514';
    END IF;
    IF OLD.promoted_attachment_id IS NOT NULL THEN
        RAISE EXCEPTION 'promoted AI extraction evidence is immutable'
            USING ERRCODE = '23514';
    END IF;
    IF NEW.promoted_attachment_id IS NULL THEN
        RAISE EXCEPTION 'AI extraction evidence promotion is required'
            USING ERRCODE = '23514';
    END IF;
    IF NEW.revision <> OLD.revision + 1
        OR NEW.updated_at <= OLD.updated_at THEN
        RAISE EXCEPTION 'AI extraction evidence revision is invalid'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_ai_extraction_evidence_immutable
    ON ai_extraction_evidence;
CREATE TRIGGER trg_ai_extraction_evidence_immutable
BEFORE UPDATE ON ai_extraction_evidence
FOR EACH ROW EXECUTE FUNCTION muriarc_restrict_ai_extraction_evidence_update();

CREATE OR REPLACE FUNCTION muriarc_reject_ai_extraction_binding_update()
RETURNS trigger AS $$
BEGIN
    IF NEW.data_cell_definition_id IS DISTINCT FROM OLD.data_cell_definition_id
        OR NEW.data_cell_subject_type IS DISTINCT FROM OLD.data_cell_subject_type
        OR NEW.data_cell_subject_id IS DISTINCT FROM OLD.data_cell_subject_id
        OR NEW.model_profile_id IS DISTINCT FROM OLD.model_profile_id
        OR NEW.model_profile_version IS DISTINCT FROM OLD.model_profile_version
        OR NEW.model_purpose IS DISTINCT FROM OLD.model_purpose
        OR NEW.usage_input_tokens IS DISTINCT FROM OLD.usage_input_tokens
        OR NEW.usage_output_tokens IS DISTINCT FROM OLD.usage_output_tokens
        OR NEW.usage_total_tokens IS DISTINCT FROM OLD.usage_total_tokens
        OR NEW.provider_request_id IS DISTINCT FROM OLD.provider_request_id
        OR NEW.trace_json IS DISTINCT FROM OLD.trace_json THEN
        RAISE EXCEPTION 'AI extraction model and data-cell binding is immutable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_ai_extraction_binding_immutable
    ON ai_extraction_drafts;
CREATE TRIGGER trg_ai_extraction_binding_immutable
BEFORE UPDATE ON ai_extraction_drafts
FOR EACH ROW EXECUTE FUNCTION muriarc_reject_ai_extraction_binding_update();

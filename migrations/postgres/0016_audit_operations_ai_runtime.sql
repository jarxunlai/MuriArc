ALTER TABLE audit_entries
    ADD COLUMN IF NOT EXISTS operation_code TEXT NOT NULL DEFAULT 'legacy.generic',
    ADD COLUMN IF NOT EXISTS operation_version INTEGER NOT NULL DEFAULT 1 CHECK (operation_version > 0),
    ADD COLUMN IF NOT EXISTS operation_params_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS entity_name_snapshot TEXT,
    ADD COLUMN IF NOT EXISTS entity_revision BIGINT;

UPDATE audit_entries SET operation_code = entity_type || '.' || action,
    operation_version = 1,
    operation_params_json = jsonb_build_object('entityType', entity_type, 'action', action)
WHERE operation_code = 'legacy.generic';

CREATE OR REPLACE FUNCTION muriarc_fill_audit_operation() RETURNS trigger AS $$
BEGIN
    IF NEW.operation_code = 'legacy.generic' THEN
        NEW.operation_code := NEW.entity_type || '.' || NEW.action;
        NEW.operation_params_json := jsonb_build_object('entityType', NEW.entity_type, 'action', NEW.action);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS trg_audit_operation_defaults ON audit_entries;
CREATE TRIGGER trg_audit_operation_defaults BEFORE INSERT ON audit_entries
FOR EACH ROW EXECUTE FUNCTION muriarc_fill_audit_operation();

ALTER TABLE ai_provider_settings
    ADD COLUMN IF NOT EXISTS supports_vision BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS vision_model TEXT;

CREATE TABLE IF NOT EXISTS ai_lab_settings (
    lab_id UUID PRIMARY KEY REFERENCES labs(id),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    custom_url_approval_required BOOLEAN NOT NULL DEFAULT TRUE,
    updated_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0)
);

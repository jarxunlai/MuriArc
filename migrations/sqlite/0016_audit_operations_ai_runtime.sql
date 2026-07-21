ALTER TABLE audit_entries ADD COLUMN operation_code TEXT NOT NULL DEFAULT 'legacy.generic';
ALTER TABLE audit_entries ADD COLUMN operation_version INTEGER NOT NULL DEFAULT 1 CHECK (operation_version > 0);
ALTER TABLE audit_entries ADD COLUMN operation_params_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE audit_entries ADD COLUMN entity_name_snapshot TEXT;
ALTER TABLE audit_entries ADD COLUMN entity_revision INTEGER;
UPDATE audit_entries SET operation_code = entity_type || '.' || action,
    operation_version = 1,
    operation_params_json = json_object('entityType', entity_type, 'action', action)
WHERE operation_code = 'legacy.generic';
CREATE TRIGGER IF NOT EXISTS trg_audit_operation_defaults AFTER INSERT ON audit_entries
WHEN NEW.operation_code = 'legacy.generic'
BEGIN
    UPDATE audit_entries SET operation_code = NEW.entity_type || '.' || NEW.action,
        operation_params_json = json_object('entityType', NEW.entity_type, 'action', NEW.action)
    WHERE id = NEW.id;
END;
CREATE TABLE IF NOT EXISTS ai_lab_settings (
    lab_id TEXT PRIMARY KEY NOT NULL REFERENCES labs(id),
    enabled INTEGER NOT NULL DEFAULT 1,
    custom_url_approval_required INTEGER NOT NULL DEFAULT 1,
    updated_by TEXT REFERENCES users(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0)
);

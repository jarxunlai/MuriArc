CREATE TABLE IF NOT EXISTS ai_model_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 120),
    current_version INTEGER NOT NULL CHECK (current_version > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    archived_at TEXT,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);

CREATE TABLE IF NOT EXISTS ai_model_profile_versions (
    profile_id TEXT NOT NULL REFERENCES ai_model_profiles(id),
    version INTEGER NOT NULL CHECK (version > 0),
    protocol TEXT NOT NULL CHECK (protocol IN (
        'openai_chat_completions', 'openai_responses', 'anthropic_messages'
    )),
    transport TEXT NOT NULL CHECK (transport IN (
        'open_ai_compatible', 'local_http'
    )),
    base_url TEXT NOT NULL CHECK (length(base_url) BETWEEN 1 AND 2048),
    normalized_base_url TEXT NOT NULL CHECK (length(normalized_base_url) BETWEEN 1 AND 2048),
    model_id TEXT NOT NULL CHECK (length(trim(model_id)) BETWEEN 1 AND 256),
    supports_vision INTEGER NOT NULL DEFAULT 0 CHECK (supports_vision IN (0, 1)),
    context_window_tokens INTEGER NOT NULL CHECK (context_window_tokens BETWEEN 4096 AND 2000000),
    max_input_tokens INTEGER NOT NULL CHECK (max_input_tokens BETWEEN 1024 AND 1900000),
    max_output_tokens INTEGER NOT NULL CHECK (max_output_tokens BETWEEN 1 AND 131072),
    history_token_budget INTEGER NOT NULL CHECK (history_token_budget BETWEEN 0 AND 1000000),
    history_turns INTEGER NOT NULL CHECK (history_turns BETWEEN 0 AND 100),
    temperature REAL NOT NULL CHECK (temperature BETWEEN 0 AND 2),
    timeout_ms INTEGER NOT NULL CHECK (timeout_ms BETWEEN 100 AND 600000),
    created_at TEXT NOT NULL,
    PRIMARY KEY (profile_id, version),
    CHECK (max_input_tokens + max_output_tokens <= context_window_tokens),
    CHECK (history_token_budget <= max_input_tokens)
);

-- Desktop secrets remain in OS keyring. This table records only the redacted
-- exact-version account binding and present/revoked state, never the key.
CREATE TABLE IF NOT EXISTS ai_model_profile_secret_refs (
    profile_id TEXT NOT NULL,
    profile_version INTEGER NOT NULL CHECK (profile_version > 0),
    keyring_account TEXT NOT NULL,
    credential_state TEXT NOT NULL CHECK (credential_state IN ('present', 'revoked')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    PRIMARY KEY (profile_id, profile_version),
    UNIQUE (keyring_account),
    FOREIGN KEY (profile_id, profile_version)
        REFERENCES ai_model_profile_versions(profile_id, version)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS ai_user_model_defaults (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    default_conversation_profile_id TEXT REFERENCES ai_model_profiles(id),
    default_vision_profile_id TEXT REFERENCES ai_model_profiles(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    revision INTEGER NOT NULL CHECK (revision > 0)
);

ALTER TABLE ai_conversations ADD COLUMN model_profile_id TEXT;
ALTER TABLE ai_conversations ADD COLUMN model_profile_version INTEGER;
ALTER TABLE ai_conversations ADD COLUMN legacy_read_only INTEGER NOT NULL DEFAULT 0
    CHECK (legacy_read_only IN (0, 1));

UPDATE ai_conversations
SET legacy_read_only = 1
WHERE model_profile_id IS NULL;

CREATE TRIGGER IF NOT EXISTS trg_ai_conversations_model_binding_insert
BEFORE INSERT ON ai_conversations
WHEN (NEW.model_profile_id IS NULL) <> (NEW.model_profile_version IS NULL)
    OR NEW.model_profile_version <= 0
    OR (NEW.legacy_read_only = 0 AND NEW.model_profile_id IS NULL)
    OR (NEW.legacy_read_only = 1 AND NEW.model_profile_id IS NOT NULL)
    OR (
        NEW.model_profile_id IS NOT NULL
        AND NOT EXISTS (
            SELECT 1
            FROM ai_model_profile_versions
            WHERE profile_id = NEW.model_profile_id
              AND version = NEW.model_profile_version
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'invalid AI conversation model binding');
END;

CREATE TRIGGER IF NOT EXISTS trg_ai_conversations_model_binding_update
BEFORE UPDATE OF model_profile_id, model_profile_version ON ai_conversations
WHEN OLD.model_profile_id IS NOT NEW.model_profile_id
    OR OLD.model_profile_version IS NOT NEW.model_profile_version
BEGIN
    SELECT RAISE(ABORT, 'AI conversation model binding is immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_ai_conversations_model_binding_validate_update
BEFORE UPDATE OF model_profile_id, model_profile_version, legacy_read_only ON ai_conversations
WHEN (NEW.model_profile_id IS NULL) <> (NEW.model_profile_version IS NULL)
    OR NEW.model_profile_version <= 0
    OR (NEW.legacy_read_only = 0 AND NEW.model_profile_id IS NULL)
    OR (NEW.legacy_read_only = 1 AND NEW.model_profile_id IS NOT NULL)
    OR (
        NEW.model_profile_id IS NOT NULL
        AND NOT EXISTS (
            SELECT 1
            FROM ai_model_profile_versions
            WHERE profile_id = NEW.model_profile_id
              AND version = NEW.model_profile_version
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'invalid AI conversation model binding');
END;

INSERT INTO ai_model_profiles (
    id, lab_id, user_id, name, current_version, created_at, updated_at, revision
)
SELECT s.user_id,
    u.lab_id, s.user_id, 'Migrated default model', 1, s.created_at, s.updated_at, 1
FROM ai_provider_settings s
JOIN users u ON u.id = s.user_id
WHERE NOT EXISTS (
    SELECT 1 FROM ai_model_profiles p
    WHERE p.user_id = s.user_id AND p.name = 'Migrated default model'
)
ON CONFLICT(id) DO NOTHING;

INSERT INTO ai_model_profiles (
    id, lab_id, user_id, name, current_version, created_at, updated_at, revision
)
SELECT
    -- User identifiers are UUIDv4. Reserve UUID version nibble `f` for the
    -- deterministic compatibility vision profile so it cannot equal any
    -- migrated text profile identifier while preserving one-to-one uniqueness.
    substr(s.user_id, 1, 14) || 'f' || substr(s.user_id, 16),
    u.lab_id, s.user_id, 'Migrated vision model', 1, s.created_at, s.updated_at, 1
FROM ai_provider_settings s
JOIN users u ON u.id = s.user_id
WHERE s.supports_vision = 1
  AND NULLIF(trim(s.vision_model), '') IS NOT NULL
  AND NOT EXISTS (
    SELECT 1 FROM ai_model_profiles p
    WHERE p.user_id = s.user_id AND p.name = 'Migrated vision model'
  )
;

INSERT INTO ai_model_profile_versions (
    profile_id, version, protocol, transport, base_url, normalized_base_url, model_id,
    supports_vision, context_window_tokens, max_input_tokens, max_output_tokens,
    history_token_budget, history_turns, temperature, timeout_ms, created_at
)
SELECT p.id, 1, 'openai_chat_completions',
    json_extract(s.provider_config, '$.kind'),
    json_extract(s.provider_config, '$.base_url'),
    rtrim(json_extract(s.provider_config, '$.base_url'), '/'),
    json_extract(s.provider_config, '$.model'),
    0, s.context_window_tokens, s.max_input_tokens,
    s.max_output_tokens, s.history_token_budget, s.history_turns,
    s.temperature, s.timeout_ms, s.created_at
FROM ai_provider_settings s
JOIN ai_model_profiles p
  ON p.user_id = s.user_id AND p.name = 'Migrated default model'
WHERE true
ON CONFLICT(profile_id, version) DO NOTHING;

INSERT INTO ai_model_profile_versions (
    profile_id, version, protocol, transport, base_url, normalized_base_url, model_id,
    supports_vision, context_window_tokens, max_input_tokens, max_output_tokens,
    history_token_budget, history_turns, temperature, timeout_ms, created_at
)
SELECT p.id, 1, 'openai_chat_completions',
    json_extract(s.provider_config, '$.kind'),
    json_extract(s.provider_config, '$.base_url'),
    rtrim(json_extract(s.provider_config, '$.base_url'), '/'),
    s.vision_model,
    1, s.context_window_tokens, s.max_input_tokens,
    s.max_output_tokens, s.history_token_budget, s.history_turns,
    s.temperature, s.timeout_ms, s.created_at
FROM ai_provider_settings s
JOIN ai_model_profiles p
  ON p.user_id = s.user_id AND p.name = 'Migrated vision model'
WHERE s.supports_vision = 1
  AND NULLIF(trim(s.vision_model), '') IS NOT NULL
ON CONFLICT(profile_id, version) DO NOTHING;

INSERT INTO ai_user_model_defaults (
    user_id, default_conversation_profile_id, default_vision_profile_id,
    created_at, updated_at, revision
)
SELECT s.user_id, p.id, vision.id,
    s.created_at, s.updated_at, 1
FROM ai_provider_settings s
JOIN ai_model_profiles p
  ON p.user_id = s.user_id AND p.name = 'Migrated default model'
LEFT JOIN ai_model_profiles vision
  ON vision.user_id = s.user_id AND vision.name = 'Migrated vision model'
WHERE true
ON CONFLICT(user_id) DO NOTHING;

CREATE TRIGGER IF NOT EXISTS trg_ai_model_profile_versions_update_immutable
BEFORE UPDATE ON ai_model_profile_versions
BEGIN
    SELECT RAISE(ABORT, 'AI model profile versions are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_ai_model_profile_versions_delete_immutable
BEFORE DELETE ON ai_model_profile_versions
BEGIN
    SELECT RAISE(ABORT, 'AI model profile versions are immutable');
END;

-- The legacy identity allowed the same normalized URL once per transport.
-- Every legacy row maps to the Chat Completions protocol, so such pairs would
-- collide under the new protocol identity. Abort before rebuilding the table;
-- never silently choose or delete a security-boundary endpoint.
CREATE TABLE ai_provider_endpoint_protocol_conflict_guard (
    conflict_count INTEGER NOT NULL CHECK (conflict_count = 0)
);
INSERT INTO ai_provider_endpoint_protocol_conflict_guard (conflict_count)
SELECT count(*)
FROM (
    SELECT lab_id, normalized_base_url
    FROM ai_provider_endpoints
    GROUP BY lab_id, normalized_base_url
    HAVING count(*) > 1
);
DROP TABLE ai_provider_endpoint_protocol_conflict_guard;

ALTER TABLE ai_provider_endpoints RENAME TO ai_provider_endpoints_legacy;
DROP INDEX IF EXISTS idx_ai_provider_endpoints_enabled;

CREATE TABLE ai_provider_endpoints (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id),
    provider_kind TEXT NOT NULL CHECK (provider_kind IN ('open_ai_compatible', 'local_http')),
    protocol TEXT NOT NULL DEFAULT 'openai_chat_completions' CHECK (protocol IN (
        'openai_chat_completions', 'openai_responses', 'anthropic_messages'
    )),
    label TEXT NOT NULL CHECK (length(label) > 0 AND length(label) <= 120),
    base_url TEXT NOT NULL CHECK (length(base_url) > 0 AND length(base_url) <= 2048),
    normalized_base_url TEXT NOT NULL CHECK (length(normalized_base_url) > 0 AND length(normalized_base_url) <= 2048),
    enabled INTEGER NOT NULL DEFAULT 1,
    builtin INTEGER NOT NULL DEFAULT 0,
    created_by TEXT REFERENCES users(id),
    updated_by TEXT REFERENCES users(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    UNIQUE (lab_id, protocol, normalized_base_url)
);

INSERT INTO ai_provider_endpoints (
    id, lab_id, provider_kind, protocol, label, base_url, normalized_base_url,
    enabled, builtin, created_by, updated_by, created_at, updated_at, revision
)
SELECT id, lab_id, provider_kind, 'openai_chat_completions', label, base_url,
    normalized_base_url, enabled, builtin, created_by, updated_by, created_at,
    updated_at, revision
FROM ai_provider_endpoints_legacy;

DROP TABLE ai_provider_endpoints_legacy;

CREATE INDEX IF NOT EXISTS idx_ai_model_profiles_user
    ON ai_model_profiles(user_id, archived_at, updated_at DESC)
    WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_ai_model_profiles_active_name
    ON ai_model_profiles(user_id, name)
    WHERE archived_at IS NULL AND deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_ai_conversations_model_profile
    ON ai_conversations(model_profile_id, model_profile_version)
    WHERE model_profile_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_ai_provider_endpoints_enabled
    ON ai_provider_endpoints(lab_id, protocol, enabled);
CREATE UNIQUE INDEX IF NOT EXISTS idx_ai_provider_endpoints_protocol_url
    ON ai_provider_endpoints(lab_id, protocol, normalized_base_url);

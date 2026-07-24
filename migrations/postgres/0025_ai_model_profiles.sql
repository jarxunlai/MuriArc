CREATE TABLE IF NOT EXISTS ai_model_profiles (
    id UUID PRIMARY KEY,
    lab_id UUID NOT NULL REFERENCES labs(id),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(btrim(name)) BETWEEN 1 AND 120),
    current_version BIGINT NOT NULL CHECK (current_version > 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);

CREATE TABLE IF NOT EXISTS ai_model_profile_versions (
    profile_id UUID NOT NULL REFERENCES ai_model_profiles(id),
    version BIGINT NOT NULL CHECK (version > 0),
    protocol TEXT NOT NULL CHECK (protocol IN (
        'openai_chat_completions', 'openai_responses', 'anthropic_messages'
    )),
    transport TEXT NOT NULL CHECK (transport IN (
        'open_ai_compatible', 'local_http'
    )),
    base_url TEXT NOT NULL CHECK (length(base_url) BETWEEN 1 AND 2048),
    normalized_base_url TEXT NOT NULL CHECK (length(normalized_base_url) BETWEEN 1 AND 2048),
    model_id TEXT NOT NULL CHECK (length(btrim(model_id)) BETWEEN 1 AND 256),
    supports_vision BOOLEAN NOT NULL DEFAULT FALSE,
    context_window_tokens BIGINT NOT NULL CHECK (context_window_tokens BETWEEN 4096 AND 2000000),
    max_input_tokens BIGINT NOT NULL CHECK (max_input_tokens BETWEEN 1024 AND 1900000),
    max_output_tokens BIGINT NOT NULL CHECK (max_output_tokens BETWEEN 1 AND 131072),
    history_token_budget BIGINT NOT NULL CHECK (history_token_budget BETWEEN 0 AND 1000000),
    history_turns INTEGER NOT NULL CHECK (history_turns BETWEEN 0 AND 100),
    temperature DOUBLE PRECISION NOT NULL CHECK (temperature BETWEEN 0 AND 2),
    timeout_ms BIGINT NOT NULL CHECK (timeout_ms BETWEEN 100 AND 600000),
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (profile_id, version),
    CHECK (max_input_tokens + max_output_tokens <= context_window_tokens),
    CHECK (history_token_budget <= max_input_tokens)
);

CREATE TABLE IF NOT EXISTS ai_model_profile_secrets (
    profile_id UUID NOT NULL,
    profile_version BIGINT NOT NULL CHECK (profile_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BYTEA NOT NULL CHECK (octet_length(nonce) = 12),
    ciphertext BYTEA NOT NULL CHECK (octet_length(ciphertext) > 16),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (profile_id, profile_version),
    FOREIGN KEY (profile_id, profile_version)
        REFERENCES ai_model_profile_versions(profile_id, version)
);

CREATE TABLE IF NOT EXISTS ai_user_model_defaults (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    default_conversation_profile_id UUID REFERENCES ai_model_profiles(id),
    default_vision_profile_id UUID REFERENCES ai_model_profiles(id),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    revision BIGINT NOT NULL CHECK (revision > 0)
);

ALTER TABLE ai_conversations
    ADD COLUMN IF NOT EXISTS model_profile_id UUID,
    ADD COLUMN IF NOT EXISTS model_profile_version BIGINT,
    ADD COLUMN IF NOT EXISTS legacy_read_only BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE ai_conversations
SET legacy_read_only = TRUE
WHERE model_profile_id IS NULL;

ALTER TABLE ai_conversations
    DROP CONSTRAINT IF EXISTS ai_conversations_model_profile_binding_check;
ALTER TABLE ai_conversations
    ADD CONSTRAINT ai_conversations_model_profile_binding_check
    CHECK (
        (
            legacy_read_only
            AND model_profile_id IS NULL
            AND model_profile_version IS NULL
        )
        OR (
            NOT legacy_read_only
            AND model_profile_id IS NOT NULL
            AND model_profile_version IS NOT NULL
            AND model_profile_version > 0
        )
    );
ALTER TABLE ai_conversations
    DROP CONSTRAINT IF EXISTS ai_conversations_model_profile_version_fkey;
ALTER TABLE ai_conversations
    ADD CONSTRAINT ai_conversations_model_profile_version_fkey
    FOREIGN KEY (model_profile_id, model_profile_version)
    REFERENCES ai_model_profile_versions(profile_id, version);

CREATE OR REPLACE FUNCTION muriarc_reject_ai_conversation_model_rebind()
RETURNS trigger AS $$
BEGIN
    IF NEW.model_profile_id IS DISTINCT FROM OLD.model_profile_id
        OR NEW.model_profile_version IS DISTINCT FROM OLD.model_profile_version THEN
        RAISE EXCEPTION 'AI conversation model binding is immutable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_ai_conversations_model_binding_immutable ON ai_conversations;
CREATE TRIGGER trg_ai_conversations_model_binding_immutable
BEFORE UPDATE OF model_profile_id, model_profile_version ON ai_conversations
FOR EACH ROW EXECUTE FUNCTION muriarc_reject_ai_conversation_model_rebind();

-- Deterministic identifiers make the compatibility projection idempotent and
-- keep credentials in their legacy row until the application re-encrypts them
-- with profile-version-bound AAD.
INSERT INTO ai_model_profiles (
    id, lab_id, user_id, name, current_version, created_at, updated_at, revision
)
SELECT (
        substr(md5('muriarc-ai-text-profile:' || s.user_id::text), 1, 8) || '-' ||
        substr(md5('muriarc-ai-text-profile:' || s.user_id::text), 9, 4) || '-' ||
        substr(md5('muriarc-ai-text-profile:' || s.user_id::text), 13, 4) || '-' ||
        substr(md5('muriarc-ai-text-profile:' || s.user_id::text), 17, 4) || '-' ||
        substr(md5('muriarc-ai-text-profile:' || s.user_id::text), 21, 12)
    )::uuid,
    u.lab_id, s.user_id, 'Migrated default model', 1, s.created_at, s.updated_at, 1
FROM ai_provider_settings s
JOIN users u ON u.id = s.user_id
ON CONFLICT (id) DO NOTHING;

INSERT INTO ai_model_profiles (
    id, lab_id, user_id, name, current_version, created_at, updated_at, revision
)
SELECT (
        substr(md5('muriarc-ai-vision-profile:' || s.user_id::text), 1, 8) || '-' ||
        substr(md5('muriarc-ai-vision-profile:' || s.user_id::text), 9, 4) || '-' ||
        substr(md5('muriarc-ai-vision-profile:' || s.user_id::text), 13, 4) || '-' ||
        substr(md5('muriarc-ai-vision-profile:' || s.user_id::text), 17, 4) || '-' ||
        substr(md5('muriarc-ai-vision-profile:' || s.user_id::text), 21, 12)
    )::uuid,
    u.lab_id, s.user_id, 'Migrated vision model', 1, s.created_at, s.updated_at, 1
FROM ai_provider_settings s
JOIN users u ON u.id = s.user_id
WHERE COALESCE(s.supports_vision, FALSE)
  AND NULLIF(btrim(s.vision_model), '') IS NOT NULL
ON CONFLICT (id) DO NOTHING;

INSERT INTO ai_model_profile_versions (
    profile_id, version, protocol, transport, base_url,
    normalized_base_url, model_id,
    supports_vision, context_window_tokens, max_input_tokens, max_output_tokens,
    history_token_budget, history_turns, temperature, timeout_ms, created_at
)
SELECT p.id, 1, 'openai_chat_completions',
    s.provider_config->>'kind',
    s.provider_config->>'base_url',
    rtrim(s.provider_config->>'base_url', '/'),
    s.provider_config->>'model',
    FALSE,
    s.context_window_tokens, s.max_input_tokens, s.max_output_tokens,
    s.history_token_budget, s.history_turns, s.temperature, s.timeout_ms, s.created_at
FROM ai_provider_settings s
JOIN ai_model_profiles p
  ON p.user_id = s.user_id AND p.name = 'Migrated default model'
ON CONFLICT (profile_id, version) DO NOTHING;

INSERT INTO ai_model_profile_versions (
    profile_id, version, protocol, transport, base_url,
    normalized_base_url, model_id,
    supports_vision, context_window_tokens, max_input_tokens, max_output_tokens,
    history_token_budget, history_turns, temperature, timeout_ms, created_at
)
SELECT p.id, 1, 'openai_chat_completions',
    s.provider_config->>'kind',
    s.provider_config->>'base_url',
    rtrim(s.provider_config->>'base_url', '/'),
    s.vision_model,
    TRUE,
    s.context_window_tokens, s.max_input_tokens, s.max_output_tokens,
    s.history_token_budget, s.history_turns, s.temperature, s.timeout_ms, s.created_at
FROM ai_provider_settings s
JOIN ai_model_profiles p
  ON p.user_id = s.user_id AND p.name = 'Migrated vision model'
WHERE COALESCE(s.supports_vision, FALSE)
  AND NULLIF(btrim(s.vision_model), '') IS NOT NULL
ON CONFLICT (profile_id, version) DO NOTHING;

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
ON CONFLICT (user_id) DO NOTHING;

CREATE OR REPLACE FUNCTION muriarc_reject_ai_model_profile_version_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'AI model profile versions are immutable'
        USING ERRCODE = '23514';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_ai_model_profile_versions_immutable
    ON ai_model_profile_versions;
CREATE TRIGGER trg_ai_model_profile_versions_immutable
BEFORE UPDATE OR DELETE ON ai_model_profile_versions
FOR EACH ROW EXECUTE FUNCTION muriarc_reject_ai_model_profile_version_mutation();

-- Every legacy endpoint becomes a Chat Completions endpoint. The old unique
-- key included provider_kind, so the same normalized URL could legally exist
-- once for each transport. Refuse that ambiguous upgrade before changing the
-- table instead of silently choosing or deleting an endpoint.
DO $$
DECLARE
    conflicting_lab_id UUID;
    conflicting_normalized_base_url TEXT;
    conflicting_endpoint_count BIGINT;
BEGIN
    SELECT lab_id, normalized_base_url, count(*)
    INTO conflicting_lab_id, conflicting_normalized_base_url,
         conflicting_endpoint_count
    FROM ai_provider_endpoints
    GROUP BY lab_id, normalized_base_url
    HAVING count(*) > 1
    ORDER BY lab_id, normalized_base_url
    LIMIT 1;

    IF conflicting_endpoint_count IS NOT NULL THEN
        RAISE EXCEPTION
            'cannot migrate AI provider endpoints to protocol uniqueness: lab % has % legacy endpoints for normalized URL %',
            conflicting_lab_id,
            conflicting_endpoint_count,
            conflicting_normalized_base_url
            USING ERRCODE = '23505',
                  HINT = 'Resolve the legacy endpoint collision without deleting audit history, then retry migration 0025.';
    END IF;
END
$$;

ALTER TABLE ai_provider_endpoints ADD COLUMN IF NOT EXISTS protocol TEXT;
UPDATE ai_provider_endpoints
SET protocol = 'openai_chat_completions'
WHERE protocol IS NULL;
ALTER TABLE ai_provider_endpoints
    ALTER COLUMN protocol SET DEFAULT 'openai_chat_completions',
    ALTER COLUMN protocol SET NOT NULL;
ALTER TABLE ai_provider_endpoints
    DROP CONSTRAINT IF EXISTS ai_provider_endpoints_protocol_check;
ALTER TABLE ai_provider_endpoints
    ADD CONSTRAINT ai_provider_endpoints_protocol_check CHECK (protocol IN (
        'openai_chat_completions', 'openai_responses', 'anthropic_messages'
    ));
ALTER TABLE ai_provider_endpoints
    DROP CONSTRAINT IF EXISTS ai_provider_endpoints_lab_id_provider_kind_normalized_base_url_key;
DROP INDEX IF EXISTS idx_ai_provider_endpoints_enabled;
DO $$
DECLARE
    legacy_constraint_name TEXT;
BEGIN
    FOR legacy_constraint_name IN
        SELECT constraint_record.conname
        FROM pg_constraint constraint_record
        JOIN pg_class relation ON relation.oid = constraint_record.conrelid
        JOIN pg_namespace schema_record ON schema_record.oid = relation.relnamespace
        WHERE schema_record.nspname = current_schema()
          AND relation.relname = 'ai_provider_endpoints'
          AND constraint_record.contype = 'u'
          AND pg_get_constraintdef(constraint_record.oid)
              = 'UNIQUE (lab_id, provider_kind, normalized_base_url)'
    LOOP
        EXECUTE format(
            'ALTER TABLE ai_provider_endpoints DROP CONSTRAINT %I',
            legacy_constraint_name
        );
    END LOOP;
END
$$;

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

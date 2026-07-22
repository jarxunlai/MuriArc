ALTER TABLE ai_provider_settings
    ALTER COLUMN enabled SET DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS provider_preset_id TEXT,
    ADD COLUMN IF NOT EXISTS context_window_tokens BIGINT NOT NULL DEFAULT 131072
        CHECK (context_window_tokens BETWEEN 4096 AND 2000000),
    ADD COLUMN IF NOT EXISTS max_input_tokens BIGINT NOT NULL DEFAULT 65536
        CHECK (max_input_tokens BETWEEN 1024 AND 1900000),
    ADD COLUMN IF NOT EXISTS max_output_tokens BIGINT NOT NULL DEFAULT 4096
        CHECK (max_output_tokens BETWEEN 1 AND 131072),
    ADD COLUMN IF NOT EXISTS history_token_budget BIGINT NOT NULL DEFAULT 32768
        CHECK (history_token_budget BETWEEN 0 AND 1000000),
    ADD COLUMN IF NOT EXISTS history_turns INTEGER NOT NULL DEFAULT 20
        CHECK (history_turns BETWEEN 0 AND 100),
    ADD COLUMN IF NOT EXISTS temperature DOUBLE PRECISION NOT NULL DEFAULT 0
        CHECK (temperature >= 0 AND temperature <= 2),
    ADD COLUMN IF NOT EXISTS timeout_ms BIGINT NOT NULL DEFAULT 120000
        CHECK (timeout_ms BETWEEN 100 AND 600000);

-- Preserve the owner and infer a non-sensitive preset label from the existing
-- user's own Provider URL. Credentials remain in the same user-owned row.
UPDATE ai_provider_settings
SET provider_preset_id = CASE
    WHEN provider_config->>'base_url' LIKE 'https://api.deepseek.com%' THEN 'deepseek'
    WHEN provider_config->>'base_url' LIKE 'https://open.bigmodel.cn/%' THEN 'zhipu-glm'
    WHEN provider_config->>'base_url' LIKE 'https://api.moonshot.cn/%' THEN 'moonshot-kimi'
    WHEN provider_config->>'base_url' LIKE 'https://api.openai.com/%' THEN 'openai'
    ELSE 'custom-openai-compatible'
END
WHERE provider_preset_id IS NULL OR btrim(provider_preset_id) = '';

ALTER TABLE ai_provider_settings
    ALTER COLUMN provider_preset_id SET DEFAULT 'deepseek',
    ALTER COLUMN provider_preset_id SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ai_provider_settings_token_budget_check'
    ) THEN
        ALTER TABLE ai_provider_settings
            ADD CONSTRAINT ai_provider_settings_token_budget_check
            CHECK (max_input_tokens + max_output_tokens <= context_window_tokens
                AND history_token_budget <= max_input_tokens);
    END IF;
END
$$;

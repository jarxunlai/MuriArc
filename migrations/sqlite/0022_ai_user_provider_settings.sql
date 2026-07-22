-- Desktop keeps the API key in the OS keyring, but the portable SQLite schema
-- mirrors the non-sensitive per-user settings contract used by Server.
CREATE TABLE IF NOT EXISTS ai_provider_settings (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 1,
    provider_config TEXT NOT NULL,
    provider_preset_id TEXT NOT NULL DEFAULT 'deepseek',
    secret_key_version INTEGER CHECK (secret_key_version IS NULL OR secret_key_version > 0),
    secret_nonce BLOB CHECK (secret_nonce IS NULL OR length(secret_nonce) = 12),
    secret_ciphertext BLOB CHECK (secret_ciphertext IS NULL OR length(secret_ciphertext) > 16),
    supports_vision INTEGER NOT NULL DEFAULT 0,
    vision_model TEXT,
    context_window_tokens INTEGER NOT NULL DEFAULT 131072 CHECK (context_window_tokens BETWEEN 4096 AND 2000000),
    max_input_tokens INTEGER NOT NULL DEFAULT 65536 CHECK (max_input_tokens BETWEEN 1024 AND 1900000),
    max_output_tokens INTEGER NOT NULL DEFAULT 4096 CHECK (max_output_tokens BETWEEN 1 AND 131072),
    history_token_budget INTEGER NOT NULL DEFAULT 32768 CHECK (history_token_budget BETWEEN 0 AND 1000000),
    history_turns INTEGER NOT NULL DEFAULT 20 CHECK (history_turns BETWEEN 0 AND 100),
    temperature REAL NOT NULL DEFAULT 0 CHECK (temperature >= 0 AND temperature <= 2),
    timeout_ms INTEGER NOT NULL DEFAULT 120000 CHECK (timeout_ms BETWEEN 100 AND 600000),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    CHECK ((secret_key_version IS NULL AND secret_nonce IS NULL AND secret_ciphertext IS NULL)
        OR (secret_key_version IS NOT NULL AND secret_nonce IS NOT NULL AND secret_ciphertext IS NOT NULL)),
    CHECK (max_input_tokens + max_output_tokens <= context_window_tokens),
    CHECK (history_token_budget <= max_input_tokens)
);

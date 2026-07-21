ALTER TABLE user_credentials
    ADD COLUMN IF NOT EXISTS must_change_password BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0);

CREATE INDEX IF NOT EXISTS idx_user_credentials_password_change_required
    ON user_credentials(user_id) WHERE must_change_password = TRUE;

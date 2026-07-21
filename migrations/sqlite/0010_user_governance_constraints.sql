-- Keep local/server Store behavior aligned even though credentials and shared
-- account administration are PostgreSQL-only concerns.
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_lab_email_normalized
    ON users(lab_id, lower(email)) WHERE deleted_at IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_memberships_lab_user
    ON memberships(lab_id, user_id) WHERE project_id IS NULL AND deleted_at IS NULL;
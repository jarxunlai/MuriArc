ALTER TABLE user_credentials
    ADD COLUMN credential_policy_revision INTEGER NOT NULL DEFAULT 1
        CHECK (credential_policy_revision > 0);

CREATE TABLE auth_login_backoff (
    identity_digest BYTEA PRIMARY KEY CHECK (octet_length(identity_digest) = 32),
    failure_count INTEGER NOT NULL CHECK (failure_count > 0),
    blocked_until TIMESTAMPTZ,
    first_failed_at TIMESTAMPTZ NOT NULL,
    last_failed_at TIMESTAMPTZ NOT NULL,
    CHECK (last_failed_at >= first_failed_at)
);

CREATE INDEX idx_auth_login_backoff_cleanup
    ON auth_login_backoff(last_failed_at);

-- Migration 0033 installed database write fences before this table existed.
-- Re-run the shared installer so login security state is covered by the same
-- active generation Write Lease as every other persistent auth record.
SELECT muriarc_install_write_fences();

CREATE TABLE muriarc_generation_sets (
    generation_id TEXT PRIMARY KEY NOT NULL,
    data_epoch TEXT NOT NULL,
    backend_state_digest TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('candidate', 'active', 'retired', 'recovery')),
    manifest_digest TEXT,
    created_at TEXT NOT NULL,
    activated_at TEXT,
    first_write_at TEXT,
    CHECK (status <> 'active' OR activated_at IS NOT NULL)
);

CREATE UNIQUE INDEX muriarc_one_active_generation
    ON muriarc_generation_sets(status) WHERE status = 'active';

CREATE TABLE muriarc_upgrade_operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    source_generation_id TEXT NOT NULL REFERENCES muriarc_generation_sets(generation_id),
    candidate_generation_id TEXT REFERENCES muriarc_generation_sets(generation_id),
    target_application_version TEXT NOT NULL,
    target_data_epoch TEXT NOT NULL,
    target_backend_state_digest TEXT NOT NULL,
    target_gateway_contract_revision TEXT NOT NULL,
    maintenance_class TEXT NOT NULL CHECK (maintenance_class IN ('M0', 'M1', 'M2', 'M3')),
    phase TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'recovery_required')),
    journal_version INTEGER NOT NULL CHECK (journal_version > 0),
    journal_json TEXT NOT NULL,
    started_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT
);

CREATE UNIQUE INDEX muriarc_one_running_upgrade
    ON muriarc_upgrade_operations(status) WHERE status = 'running';

CREATE TABLE muriarc_write_leases (
    lease_id TEXT PRIMARY KEY NOT NULL,
    generation_id TEXT NOT NULL REFERENCES muriarc_generation_sets(generation_id),
    holder TEXT NOT NULL,
    fencing_token INTEGER NOT NULL CHECK (fencing_token > 0),
    status TEXT NOT NULL CHECK (status IN ('active', 'draining', 'revoked')),
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT,
    CHECK ((status = 'revoked') = (revoked_at IS NOT NULL))
);

CREATE UNIQUE INDEX muriarc_one_active_write_lease
    ON muriarc_write_leases(status) WHERE status IN ('active', 'draining');

CREATE TABLE muriarc_deployment_state (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    application_version TEXT NOT NULL,
    data_epoch TEXT NOT NULL,
    backend_state_digest TEXT NOT NULL,
    gateway_contract_revision TEXT NOT NULL,
    generation_id TEXT NOT NULL REFERENCES muriarc_generation_sets(generation_id),
    write_lease_id TEXT REFERENCES muriarc_write_leases(lease_id),
    first_write_at TEXT,
    updated_at TEXT NOT NULL
);

-- BEGIN GENERATED BUSINESS WRITE FENCES
-- Generated from the final preview_epoch_0 SQLite schema. Every future
-- migration that adds a table must add its three fences before release.

CREATE TRIGGER muriarc_write_fence_ai_approvals_insert
BEFORE INSERT ON "ai_approvals"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_approvals_update
BEFORE UPDATE ON "ai_approvals"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_approvals_delete
BEFORE DELETE ON "ai_approvals"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_autonomy_grants_insert
BEFORE INSERT ON "ai_autonomy_grants"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_autonomy_grants_update
BEFORE UPDATE ON "ai_autonomy_grants"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_autonomy_grants_delete
BEFORE DELETE ON "ai_autonomy_grants"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversation_messages_insert
BEFORE INSERT ON "ai_conversation_messages"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversation_messages_update
BEFORE UPDATE ON "ai_conversation_messages"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversation_messages_delete
BEFORE DELETE ON "ai_conversation_messages"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversation_source_object_deletions_insert
BEFORE INSERT ON "ai_conversation_source_object_deletions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversation_source_object_deletions_update
BEFORE UPDATE ON "ai_conversation_source_object_deletions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversation_source_object_deletions_delete
BEFORE DELETE ON "ai_conversation_source_object_deletions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversation_sources_insert
BEFORE INSERT ON "ai_conversation_sources"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversation_sources_update
BEFORE UPDATE ON "ai_conversation_sources"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversation_sources_delete
BEFORE DELETE ON "ai_conversation_sources"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversations_insert
BEFORE INSERT ON "ai_conversations"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversations_update
BEFORE UPDATE ON "ai_conversations"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_conversations_delete
BEFORE DELETE ON "ai_conversations"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_extraction_drafts_insert
BEFORE INSERT ON "ai_extraction_drafts"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_extraction_drafts_update
BEFORE UPDATE ON "ai_extraction_drafts"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_extraction_drafts_delete
BEFORE DELETE ON "ai_extraction_drafts"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_extraction_evidence_insert
BEFORE INSERT ON "ai_extraction_evidence"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_extraction_evidence_update
BEFORE UPDATE ON "ai_extraction_evidence"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_extraction_evidence_delete
BEFORE DELETE ON "ai_extraction_evidence"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_lab_settings_insert
BEFORE INSERT ON "ai_lab_settings"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_lab_settings_update
BEFORE UPDATE ON "ai_lab_settings"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_lab_settings_delete
BEFORE DELETE ON "ai_lab_settings"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_model_profile_secret_refs_insert
BEFORE INSERT ON "ai_model_profile_secret_refs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_model_profile_secret_refs_update
BEFORE UPDATE ON "ai_model_profile_secret_refs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_model_profile_secret_refs_delete
BEFORE DELETE ON "ai_model_profile_secret_refs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_model_profile_versions_insert
BEFORE INSERT ON "ai_model_profile_versions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_model_profile_versions_update
BEFORE UPDATE ON "ai_model_profile_versions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_model_profile_versions_delete
BEFORE DELETE ON "ai_model_profile_versions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_model_profiles_insert
BEFORE INSERT ON "ai_model_profiles"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_model_profiles_update
BEFORE UPDATE ON "ai_model_profiles"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_model_profiles_delete
BEFORE DELETE ON "ai_model_profiles"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_private_images_insert
BEFORE INSERT ON "ai_private_images"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_private_images_update
BEFORE UPDATE ON "ai_private_images"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_private_images_delete
BEFORE DELETE ON "ai_private_images"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_provider_endpoints_insert
BEFORE INSERT ON "ai_provider_endpoints"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_provider_endpoints_update
BEFORE UPDATE ON "ai_provider_endpoints"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_provider_endpoints_delete
BEFORE DELETE ON "ai_provider_endpoints"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_provider_settings_insert
BEFORE INSERT ON "ai_provider_settings"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_provider_settings_update
BEFORE UPDATE ON "ai_provider_settings"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_provider_settings_delete
BEFORE DELETE ON "ai_provider_settings"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_tool_runs_insert
BEFORE INSERT ON "ai_tool_runs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_tool_runs_update
BEFORE UPDATE ON "ai_tool_runs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_tool_runs_delete
BEFORE DELETE ON "ai_tool_runs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_user_model_defaults_insert
BEFORE INSERT ON "ai_user_model_defaults"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_user_model_defaults_update
BEFORE UPDATE ON "ai_user_model_defaults"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_ai_user_model_defaults_delete
BEFORE DELETE ON "ai_user_model_defaults"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_alleles_insert
BEFORE INSERT ON "alleles"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_alleles_update
BEFORE UPDATE ON "alleles"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_alleles_delete
BEFORE DELETE ON "alleles"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_animal_drafts_insert
BEFORE INSERT ON "animal_drafts"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_animal_drafts_update
BEFORE UPDATE ON "animal_drafts"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_animal_drafts_delete
BEFORE DELETE ON "animal_drafts"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_animal_events_insert
BEFORE INSERT ON "animal_events"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_animal_events_update
BEFORE UPDATE ON "animal_events"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_animal_events_delete
BEFORE DELETE ON "animal_events"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_animals_insert
BEFORE INSERT ON "animals"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_animals_update
BEFORE UPDATE ON "animals"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_animals_delete
BEFORE DELETE ON "animals"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_attachment_derivatives_insert
BEFORE INSERT ON "attachment_derivatives"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_attachment_derivatives_update
BEFORE UPDATE ON "attachment_derivatives"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_attachment_derivatives_delete
BEFORE DELETE ON "attachment_derivatives"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_attachment_links_insert
BEFORE INSERT ON "attachment_links"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_attachment_links_update
BEFORE UPDATE ON "attachment_links"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_attachment_links_delete
BEFORE DELETE ON "attachment_links"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_attachments_insert
BEFORE INSERT ON "attachments"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_attachments_update
BEFORE UPDATE ON "attachments"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_attachments_delete
BEFORE DELETE ON "attachments"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_audit_entries_insert
BEFORE INSERT ON "audit_entries"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_audit_entries_update
BEFORE UPDATE ON "audit_entries"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_audit_entries_delete
BEFORE DELETE ON "audit_entries"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_line_genotype_definitions_insert
BEFORE INSERT ON "breeding_line_genotype_definitions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_line_genotype_definitions_update
BEFORE UPDATE ON "breeding_line_genotype_definitions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_line_genotype_definitions_delete
BEFORE DELETE ON "breeding_line_genotype_definitions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_lines_insert
BEFORE INSERT ON "breeding_lines"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_lines_update
BEFORE UPDATE ON "breeding_lines"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_lines_delete
BEFORE DELETE ON "breeding_lines"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_pair_members_insert
BEFORE INSERT ON "breeding_pair_members"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_pair_members_update
BEFORE UPDATE ON "breeding_pair_members"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_pair_members_delete
BEFORE DELETE ON "breeding_pair_members"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_pairs_insert
BEFORE INSERT ON "breeding_pairs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_pairs_update
BEFORE UPDATE ON "breeding_pairs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_breeding_pairs_delete
BEFORE DELETE ON "breeding_pairs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_cages_insert
BEFORE INSERT ON "cages"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_cages_update
BEFORE UPDATE ON "cages"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_cages_delete
BEFORE DELETE ON "cages"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_cohorts_insert
BEFORE INSERT ON "cohorts"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_cohorts_update
BEFORE UPDATE ON "cohorts"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_cohorts_delete
BEFORE DELETE ON "cohorts"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_colonies_insert
BEFORE INSERT ON "colonies"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_colonies_update
BEFORE UPDATE ON "colonies"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_colonies_delete
BEFORE DELETE ON "colonies"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiment_events_insert
BEFORE INSERT ON "experiment_events"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiment_events_update
BEFORE UPDATE ON "experiment_events"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiment_events_delete
BEFORE DELETE ON "experiment_events"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiment_participations_insert
BEFORE INSERT ON "experiment_participations"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiment_participations_update
BEFORE UPDATE ON "experiment_participations"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiment_participations_delete
BEFORE DELETE ON "experiment_participations"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiment_template_versions_insert
BEFORE INSERT ON "experiment_template_versions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiment_template_versions_update
BEFORE UPDATE ON "experiment_template_versions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiment_template_versions_delete
BEFORE DELETE ON "experiment_template_versions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiments_insert
BEFORE INSERT ON "experiments"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiments_update
BEFORE UPDATE ON "experiments"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_experiments_delete
BEFORE DELETE ON "experiments"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_gene_loci_insert
BEFORE INSERT ON "gene_loci"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_gene_loci_update
BEFORE UPDATE ON "gene_loci"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_gene_loci_delete
BEFORE DELETE ON "gene_loci"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotype_components_insert
BEFORE INSERT ON "genotype_components"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotype_components_update
BEFORE UPDATE ON "genotype_components"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotype_components_delete
BEFORE DELETE ON "genotype_components"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotype_definitions_insert
BEFORE INSERT ON "genotype_definitions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotype_definitions_update
BEFORE UPDATE ON "genotype_definitions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotype_definitions_delete
BEFORE DELETE ON "genotype_definitions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotypes_insert
BEFORE INSERT ON "genotypes"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotypes_update
BEFORE UPDATE ON "genotypes"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotypes_delete
BEFORE DELETE ON "genotypes"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotyping_batch_records_insert
BEFORE INSERT ON "genotyping_batch_records"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotyping_batch_records_update
BEFORE UPDATE ON "genotyping_batch_records"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotyping_batch_records_delete
BEFORE DELETE ON "genotyping_batch_records"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotyping_batches_insert
BEFORE INSERT ON "genotyping_batches"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotyping_batches_update
BEFORE UPDATE ON "genotyping_batches"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotyping_batches_delete
BEFORE DELETE ON "genotyping_batches"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotyping_records_insert
BEFORE INSERT ON "genotyping_records"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotyping_records_update
BEFORE UPDATE ON "genotyping_records"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_genotyping_records_delete
BEFORE DELETE ON "genotyping_records"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_import_commits_insert
BEFORE INSERT ON "import_commits"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_import_commits_update
BEFORE UPDATE ON "import_commits"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_import_commits_delete
BEFORE DELETE ON "import_commits"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_jobs_insert
BEFORE INSERT ON "jobs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_jobs_update
BEFORE UPDATE ON "jobs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_jobs_delete
BEFORE DELETE ON "jobs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_labs_insert
BEFORE INSERT ON "labs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_labs_update
BEFORE UPDATE ON "labs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_labs_delete
BEFORE DELETE ON "labs"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_litters_insert
BEFORE INSERT ON "litters"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_litters_update
BEFORE UPDATE ON "litters"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_litters_delete
BEFORE DELETE ON "litters"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_mating_events_insert
BEFORE INSERT ON "mating_events"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_mating_events_update
BEFORE UPDATE ON "mating_events"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_mating_events_delete
BEFORE DELETE ON "mating_events"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_measurements_insert
BEFORE INSERT ON "measurements"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_measurements_update
BEFORE UPDATE ON "measurements"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_measurements_delete
BEFORE DELETE ON "measurements"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_memberships_insert
BEFORE INSERT ON "memberships"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_memberships_update
BEFORE UPDATE ON "memberships"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_memberships_delete
BEFORE DELETE ON "memberships"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_observation_definitions_insert
BEFORE INSERT ON "observation_definitions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_observation_definitions_update
BEFORE UPDATE ON "observation_definitions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_observation_definitions_delete
BEFORE DELETE ON "observation_definitions"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_observation_values_insert
BEFORE INSERT ON "observation_values"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_observation_values_update
BEFORE UPDATE ON "observation_values"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_observation_values_delete
BEFORE DELETE ON "observation_values"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_observations_insert
BEFORE INSERT ON "observations"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_observations_update
BEFORE UPDATE ON "observations"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_observations_delete
BEFORE DELETE ON "observations"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_pedigrees_insert
BEFORE INSERT ON "pedigrees"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_pedigrees_update
BEFORE UPDATE ON "pedigrees"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_pedigrees_delete
BEFORE DELETE ON "pedigrees"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_procedures_insert
BEFORE INSERT ON "procedures"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_procedures_update
BEFORE UPDATE ON "procedures"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_procedures_delete
BEFORE DELETE ON "procedures"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_project_animal_assignments_insert
BEFORE INSERT ON "project_animal_assignments"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_project_animal_assignments_update
BEFORE UPDATE ON "project_animal_assignments"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_project_animal_assignments_delete
BEFORE DELETE ON "project_animal_assignments"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_projects_insert
BEFORE INSERT ON "projects"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_projects_update
BEFORE UPDATE ON "projects"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_projects_delete
BEFORE DELETE ON "projects"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_provenance_insert
BEFORE INSERT ON "provenance"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_provenance_update
BEFORE UPDATE ON "provenance"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_provenance_delete
BEFORE DELETE ON "provenance"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_samples_insert
BEFORE INSERT ON "samples"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_samples_update
BEFORE UPDATE ON "samples"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_samples_delete
BEFORE DELETE ON "samples"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_users_insert
BEFORE INSERT ON "users"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_users_update
BEFORE UPDATE ON "users"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

CREATE TRIGGER muriarc_write_fence_users_delete
BEFORE DELETE ON "users"
BEGIN
    SELECT CASE WHEN EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = 1) AND NOT EXISTS (
            SELECT 1
              FROM muriarc_deployment_state AS state
              JOIN muriarc_generation_sets AS generation
                ON generation.generation_id = state.generation_id
               AND generation.status = 'active'
              JOIN muriarc_write_leases AS lease
                ON lease.lease_id = state.write_lease_id
               AND lease.generation_id = state.generation_id
               AND lease.status = 'active'
               AND julianday(lease.expires_at) > julianday('now')
             WHERE state.singleton = 1
        ) THEN RAISE(ABORT, 'muriarc_write_lease_required') END;
        UPDATE muriarc_deployment_state
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE singleton = 1;
        UPDATE muriarc_generation_sets
           SET first_write_at = COALESCE(first_write_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         WHERE generation_id = (
             SELECT generation_id FROM muriarc_deployment_state WHERE singleton = 1
         );
END;

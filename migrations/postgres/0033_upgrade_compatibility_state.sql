CREATE TABLE muriarc_generation_sets (
    generation_id UUID PRIMARY KEY,
    data_epoch TEXT NOT NULL,
    backend_state_digest TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('candidate', 'active', 'retired', 'recovery')),
    manifest_digest TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    activated_at TIMESTAMPTZ,
    first_write_at TIMESTAMPTZ,
    CHECK (status <> 'active' OR activated_at IS NOT NULL)
);

CREATE UNIQUE INDEX muriarc_one_active_generation
    ON muriarc_generation_sets ((status)) WHERE status = 'active';

CREATE TABLE muriarc_upgrade_operations (
    operation_id UUID PRIMARY KEY,
    source_generation_id UUID NOT NULL REFERENCES muriarc_generation_sets(generation_id),
    candidate_generation_id UUID REFERENCES muriarc_generation_sets(generation_id),
    target_application_version TEXT NOT NULL,
    target_data_epoch TEXT NOT NULL,
    target_backend_state_digest TEXT NOT NULL,
    target_gateway_contract_revision TEXT NOT NULL,
    maintenance_class TEXT NOT NULL CHECK (maintenance_class IN ('M0', 'M1', 'M2', 'M3')),
    phase TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'recovery_required')),
    journal_version INTEGER NOT NULL CHECK (journal_version > 0),
    journal_json JSONB NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX muriarc_one_running_upgrade
    ON muriarc_upgrade_operations ((status)) WHERE status = 'running';

CREATE TABLE muriarc_write_leases (
    lease_id UUID PRIMARY KEY,
    generation_id UUID NOT NULL REFERENCES muriarc_generation_sets(generation_id),
    holder TEXT NOT NULL,
    fencing_token BIGINT NOT NULL CHECK (fencing_token > 0),
    status TEXT NOT NULL CHECK (status IN ('active', 'draining', 'revoked')),
    issued_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    CHECK ((status = 'revoked') = (revoked_at IS NOT NULL))
);

CREATE UNIQUE INDEX muriarc_one_active_write_lease
    ON muriarc_write_leases ((status)) WHERE status IN ('active', 'draining');

CREATE TABLE muriarc_deployment_state (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    application_version TEXT NOT NULL,
    data_epoch TEXT NOT NULL,
    backend_state_digest TEXT NOT NULL,
    gateway_contract_revision TEXT NOT NULL,
    generation_id UUID NOT NULL REFERENCES muriarc_generation_sets(generation_id),
    write_lease_id UUID REFERENCES muriarc_write_leases(lease_id),
    first_write_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE OR REPLACE FUNCTION muriarc_require_active_write_lease()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    active_generation UUID;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM muriarc_deployment_state WHERE singleton = TRUE) THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;

    SELECT state.generation_id
      INTO active_generation
      FROM muriarc_deployment_state AS state
      JOIN muriarc_generation_sets AS generation
        ON generation.generation_id = state.generation_id
       AND generation.status = 'active'
      JOIN muriarc_write_leases AS lease
        ON lease.lease_id = state.write_lease_id
       AND lease.generation_id = state.generation_id
       AND lease.status = 'active'
       AND lease.expires_at > CURRENT_TIMESTAMP
     WHERE state.singleton = TRUE
     FOR UPDATE OF state;

    IF active_generation IS NULL THEN
        RAISE EXCEPTION 'muriarc_write_lease_required'
            USING ERRCODE = '55000';
    END IF;

    UPDATE muriarc_deployment_state
       SET first_write_at = COALESCE(first_write_at, CURRENT_TIMESTAMP),
           updated_at = CURRENT_TIMESTAMP
     WHERE singleton = TRUE;
    UPDATE muriarc_generation_sets
       SET first_write_at = COALESCE(first_write_at, CURRENT_TIMESTAMP)
     WHERE generation_id = active_generation;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END
$$;

CREATE OR REPLACE FUNCTION muriarc_install_write_fences()
RETURNS VOID
LANGUAGE plpgsql
AS $$
DECLARE
    business_table RECORD;
BEGIN
    FOR business_table IN
        SELECT table_schema, table_name
          FROM information_schema.tables
         WHERE table_schema = 'public'
           AND table_type = 'BASE TABLE'
           AND table_name <> '_sqlx_migrations'
           AND table_name !~ '^muriarc_'
    LOOP
        EXECUTE format(
            'DROP TRIGGER IF EXISTS muriarc_write_fence ON %I.%I',
            business_table.table_schema,
            business_table.table_name
        );
        EXECUTE format(
            'CREATE TRIGGER muriarc_write_fence BEFORE INSERT OR UPDATE OR DELETE ON %I.%I FOR EACH ROW EXECUTE FUNCTION muriarc_require_active_write_lease()',
            business_table.table_schema,
            business_table.table_name
        );
    END LOOP;
END
$$;

SELECT muriarc_install_write_fences();

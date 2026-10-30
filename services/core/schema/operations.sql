-- Core-owned, non-financial operational state. Explicit installation only.
CREATE SCHEMA core_operations;

CREATE TABLE core_operations.instances (
    boot_id TEXT PRIMARY KEY CHECK (boot_id ~ '^[a-f0-9]{32}$'),
    instance_name TEXT NOT NULL CHECK (octet_length(instance_name) BETWEEN 1 AND 128),
    version TEXT NOT NULL CHECK (octet_length(version) BETWEEN 1 AND 128),
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    phase TEXT NOT NULL CHECK (phase IN ('preparing','ready','draining','stopped')),
    in_flight BIGINT NOT NULL CHECK (in_flight >= 0),
    models_ready BOOLEAN NOT NULL,
    rpc_ready BOOLEAN NOT NULL
);
CREATE INDEX operations_instance_heartbeat ON core_operations.instances(heartbeat_at DESC,boot_id);

CREATE TABLE core_operations.log_cleanup_jobs (
    id BIGINT GENERATED ALWAYS AS IDENTITY UNIQUE,
    task_id TEXT PRIMARY KEY CHECK (task_id ~ '^[a-f0-9]{64}$'),
    created_by BIGINT NOT NULL REFERENCES core_identity.users(id),
    target_timestamp BIGINT NOT NULL CHECK (target_timestamp BETWEEN 1 AND 253402300799),
    max_source_id BIGINT NOT NULL CHECK (max_source_id >= 0),
    batch_size INTEGER NOT NULL CHECK (batch_size BETWEEN 1 AND 1000),
    status TEXT NOT NULL CHECK (status IN ('pending','running','succeeded')),
    total BIGINT NOT NULL CHECK (total >= 0),
    processed BIGINT NOT NULL DEFAULT 0 CHECK (processed BETWEEN 0 AND total),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE UNIQUE INDEX operations_one_log_cleanup ON core_operations.log_cleanup_jobs((true))
    WHERE status IN ('pending','running');

-- A whole batch and its progress commit together. A lost process before commit
-- leaves the same batch available; a lost acknowledgement never repeats a row.
-- Runtime has no DELETE authority on imported logs or any financial relation.
CREATE FUNCTION core_operations.run_log_cleanup_batch() RETURNS TEXT
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE job core_operations.log_cleanup_jobs%ROWTYPE; removed BIGINT;
BEGIN
    SELECT * INTO job FROM core_operations.log_cleanup_jobs
        WHERE status IN ('pending','running') ORDER BY id
        LIMIT 1 FOR UPDATE SKIP LOCKED;
    IF NOT FOUND THEN RETURN NULL; END IF;
    WITH candidates AS (
        SELECT l.source_id,l.source_log_id FROM core_history.log_records l
        WHERE l.created_at<job.target_timestamp AND l.source_id<=job.max_source_id
        ORDER BY l.created_at,l.source_id,l.source_log_id LIMIT job.batch_size
    ), deleted AS (
        DELETE FROM core_history.log_records l USING candidates c
        WHERE l.source_id=c.source_id AND l.source_log_id=c.source_log_id RETURNING 1
    ) SELECT count(*) INTO removed FROM deleted;
    UPDATE core_operations.log_cleanup_jobs SET processed=processed+removed,
        status=CASE WHEN processed+removed=total THEN 'succeeded' ELSE 'running' END,
        updated_at=clock_timestamp() WHERE task_id=job.task_id;
    RETURN job.task_id;
END $$;

CREATE TABLE core_operations.upgrade_receipts (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK (octet_length(request_sha256)=32),
    request JSONB NOT NULL,
    preserved_data_sha256 BYTEA NOT NULL CHECK (octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK (octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK (octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER operations_upgrades_immutable BEFORE UPDATE OR DELETE
    ON core_operations.upgrade_receipts FOR EACH ROW EXECUTE FUNCTION core_charging.no_delete();
CREATE TRIGGER operations_upgrades_no_truncate BEFORE TRUNCATE
    ON core_operations.upgrade_receipts FOR EACH STATEMENT EXECUTE FUNCTION core_charging.no_delete();
REVOKE ALL ON SCHEMA core_operations FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_operations FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_operations FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_operations FROM PUBLIC;

-- Native network-access settings have one owner-controlled, fixed-key writer.
CREATE FUNCTION core_console.write_ip_access_rules(p_value TEXT) RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE result BIGINT;
BEGIN
    -- Rust validates the shared bounded policy grammar before entering this writer.
    IF p_value IS NULL OR octet_length(p_value) > 16384 OR btrim(p_value) = '' THEN
        RAISE EXCEPTION 'invalid IP access rules' USING ERRCODE = '22023';
    END IF;
    PERFORM core_console.lock_public_options();
    INSERT INTO core_console.options AS o(key, value)
        VALUES ('IPAccessRoutingRules', p_value)
    ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value,
        revision = o.revision + 1, updated_at = statement_timestamp()
    RETURNING revision INTO result;
    RETURN result;
END
$$;
REVOKE ALL ON FUNCTION core_console.write_ip_access_rules(TEXT) FROM PUBLIC;

-- Exact offline 75 -> 76 receipt; runtime receives no receipt privileges.
CREATE TABLE core_meta.network_access_upgrades (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
    request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
    preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER network_access_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.network_access_upgrades
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER network_access_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.network_access_upgrades
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.network_access_upgrades FROM PUBLIC;

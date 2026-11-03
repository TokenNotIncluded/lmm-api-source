-- Explicit 46 -> 47 extension. Existing options, revisions and grants are retained.
CREATE FUNCTION core_console.write_navigation_option(p_key TEXT, p_value TEXT) RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE result BIGINT;
BEGIN
    IF p_key IS NULL OR p_key <> 'HeaderNavModules' THEN
        RAISE EXCEPTION 'unsupported public option' USING ERRCODE = '22023';
    END IF;
    PERFORM core_console.lock_public_options();
    INSERT INTO core_console.options AS o(key, value) VALUES (p_key, p_value)
    ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value,
        revision = o.revision + 1, updated_at = statement_timestamp()
    RETURNING revision INTO result;
    RETURN result;
END
$$;
REVOKE ALL ON FUNCTION core_console.write_navigation_option(TEXT, TEXT) FROM PUBLIC;
-- Exact offline 46 -> 47 receipt; no runtime privileges.
CREATE TABLE core_meta.navigation_options_upgrades (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
    request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
    preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER navigation_options_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.navigation_options_upgrades
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER navigation_options_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.navigation_options_upgrades
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.navigation_options_upgrades FROM PUBLIC;

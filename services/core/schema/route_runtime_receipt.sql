REVOKE ALL ON core_channels.route_health FROM PUBLIC;

CREATE TABLE core_meta.route_runtime_upgrades (
 upgrade_id TEXT PRIMARY KEY CHECK(octet_length(upgrade_id) BETWEEN 16 AND 96),
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL,
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER route_runtime_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.route_runtime_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER route_runtime_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.route_runtime_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.route_runtime_upgrades FROM PUBLIC;

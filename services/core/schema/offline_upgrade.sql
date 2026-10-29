-- Native v3 receipts. Historical v1 receipts keep their exact old contracts.
CREATE TABLE core_meta.offline_upgrades (
 operation_id TEXT PRIMARY KEY,
 procedure TEXT NOT NULL CHECK(procedure='core-offline-upgrade-v3'),
 binding JSONB NOT NULL CHECK(jsonb_typeof(binding)='object'),
 predecessor_catalog_sha256 TEXT NOT NULL CHECK(predecessor_catalog_sha256 ~ '^[0-9a-f]{64}$'),
 successor_catalog_sha256 TEXT NOT NULL CHECK(successor_catalog_sha256 ~ '^[0-9a-f]{64}$'),
 preserved_data_sha256 TEXT NOT NULL CHECK(preserved_data_sha256 ~ '^[0-9a-f]{64}$'),
 preserved_permissions_sha256 TEXT NOT NULL CHECK(preserved_permissions_sha256 ~ '^[0-9a-f]{64}$'),
 runtime_permissions_sha256 TEXT NOT NULL CHECK(runtime_permissions_sha256 ~ '^[0-9a-f]{64}$'),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER offline_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.offline_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER offline_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.offline_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.offline_upgrades FROM PUBLIC;

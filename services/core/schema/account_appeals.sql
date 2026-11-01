-- Independent account-appeal component; applied only by init-db or reviewed offline upgrade.
-- Writers lock core_identity.users before locking a flow or request.
CREATE SCHEMA core_account_appeals;
CREATE TABLE core_account_appeals.requests (
    id BIGINT GENERATED ALWAYS AS IDENTITY (MAXVALUE 9007199254740991) PRIMARY KEY
        CHECK(id BETWEEN 1 AND 9007199254740991),
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    user_version BIGINT NOT NULL CHECK(user_version>0),
    reason TEXT NOT NULL CHECK(length(reason) BETWEEN 5 AND 2000 AND octet_length(reason)<=8192),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','approved','rejected')),
    admin_note TEXT NOT NULL DEFAULT '' CHECK(length(admin_note)<=2000 AND octet_length(admin_note)<=8192),
    reviewed_by BIGINT REFERENCES core_identity.users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    reviewed_at TIMESTAMPTZ,
    revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
    CHECK((status='pending' AND reviewed_by IS NULL AND reviewed_at IS NULL)
        OR (status IN ('approved','rejected') AND reviewed_by IS NOT NULL AND reviewed_at IS NOT NULL))
);
CREATE UNIQUE INDEX account_appeals_one_pending ON core_account_appeals.requests(user_id,user_version)
    WHERE status='pending';
CREATE TABLE core_account_appeals.flows (
    digest BYTEA PRIMARY KEY CHECK(octet_length(digest)=32),
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    user_version BIGINT NOT NULL CHECK(user_version>0),
    proof_kind TEXT NOT NULL CHECK(length(proof_kind) BETWEEN 1 AND 64),
    expires_at TIMESTAMPTZ NOT NULL,
    request_id BIGINT REFERENCES core_account_appeals.requests(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- Owner-only receipt for the explicit, exact-target account appeals upgrade.
-- Runtime receives no SELECT or write authority on this record.
CREATE TABLE core_meta.account_appeal_upgrades (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
    request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
    preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER account_appeal_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.account_appeal_upgrades
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER account_appeal_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.account_appeal_upgrades
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.account_appeal_upgrades FROM PUBLIC;
REVOKE ALL ON SCHEMA core_account_appeals FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_account_appeals FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_account_appeals FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_account_appeals FROM PUBLIC;

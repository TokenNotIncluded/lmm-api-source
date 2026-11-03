-- Imported legacy usage aggregates and passkey handles, installed by fresh init-db.
-- Offline owner import only; runtime receives SELECT and never writes here.
CREATE TABLE core_history.quota_aggregates (
  source_id bigint NOT NULL REFERENCES core_history.sources(source_id),
  source_row_id bigint NOT NULL CHECK(source_row_id BETWEEN 1 AND 9007199254740991),
  source_row_sha256 text NOT NULL CHECK(source_row_sha256 ~ '^[0-9a-f]{64}$'),
  user_id bigint NOT NULL REFERENCES core_identity.users(id),
  owner_account_id bigint NOT NULL REFERENCES core_identity.accounts(id),
  token_id bigint REFERENCES core_identity.credentials(id),
  username text NOT NULL DEFAULT '' CHECK(octet_length(username)<=256),
  model_name text NOT NULL DEFAULT '' CHECK(octet_length(model_name)<=256),
  group_name text NOT NULL DEFAULT '' CHECK(octet_length(group_name)<=256),
  channel_id bigint NOT NULL DEFAULT 0 CHECK(channel_id>=0),
  node_name text NOT NULL DEFAULT '' CHECK(octet_length(node_name)<=256),
  -- Start of the legacy hourly bucket, Unix seconds.
  created_at bigint NOT NULL CHECK(created_at BETWEEN 0 AND 253402300799),
  request_count bigint NOT NULL CHECK(request_count BETWEEN 0 AND 9007199254740991),
  token_used bigint NOT NULL CHECK(token_used BETWEEN 0 AND 9007199254740991),
  -- Exact stored legacy counter and its reviewed conversion to current credits.
  raw_quota bigint NOT NULL,
  quota_credits bigint NOT NULL CHECK(quota_credits BETWEEN 0 AND 9007199254740991),
  credits_basis text NOT NULL CHECK(credits_basis IN ('current_units','rebased_legacy_units')),
  PRIMARY KEY(source_id,source_row_id)
);
CREATE INDEX history_quota_time ON core_history.quota_aggregates(created_at,user_id);
CREATE INDEX history_quota_user_time ON core_history.quota_aggregates(user_id,owner_account_id,created_at);
-- Legacy WebAuthn user handles were the decimal user ID, not a UUID. Only an
-- imported credential may present its exact recorded legacy handle.
CREATE TABLE core_history.legacy_passkey_handles (
  credential_id bytea PRIMARY KEY REFERENCES core_passkeys.credentials(credential_id),
  user_id bigint NOT NULL REFERENCES core_identity.users(id),
  user_handle bytea NOT NULL CHECK(octet_length(user_handle) BETWEEN 1 AND 64)
);
CREATE TRIGGER quota_aggregates_immutable BEFORE UPDATE OR DELETE ON core_history.quota_aggregates
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER quota_aggregates_no_truncate BEFORE TRUNCATE ON core_history.quota_aggregates
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_passkey_handles_immutable BEFORE UPDATE OR DELETE ON core_history.legacy_passkey_handles
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_passkey_handles_no_truncate BEFORE TRUNCATE ON core_history.legacy_passkey_handles
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_history.quota_aggregates, core_history.legacy_passkey_handles FROM PUBLIC;

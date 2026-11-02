-- Component 46: display-only model-catalog attempts. Owner-installed, no startup DDL.
-- Deliberately empty on installation: old tests have no proven validation scope.
CREATE TABLE core_channels.catalog_observations (
    channel_id BIGINT PRIMARY KEY REFERENCES core_channels.identities(id),
    credential_id BIGINT NOT NULL CHECK(credential_id>0),
    credential_revision BIGINT NOT NULL CHECK(credential_revision>0),
    binding_sha256 BYTEA NOT NULL CHECK(octet_length(binding_sha256)=32),
    scope TEXT NOT NULL DEFAULT 'model_catalog' CHECK(scope='model_catalog'),
    result TEXT NOT NULL CHECK(result IN ('success','failed')),
    started_at TIMESTAMPTZ NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    duration_ms BIGINT NOT NULL CHECK(duration_ms BETWEEN 0 AND 9007199254740991),
    CHECK(observed_at>=started_at)
);
REVOKE ALL ON core_channels.catalog_observations FROM PUBLIC;
-- Runtime grant/revoke integration is separate. No DELETE, TRUNCATE, schema
-- CREATE or table-wide grants. channel_id is never updated by runtime.
-- SELECT(channel_id,credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms)
-- INSERT(channel_id,credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms)
-- UPDATE(credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms)

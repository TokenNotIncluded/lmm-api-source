-- Shared expiry and capacity apply to all serving nodes. No input value is stored.
CREATE TABLE core_channels.affinity_cache (
    key_sha256 BYTEA PRIMARY KEY CHECK (octet_length(key_sha256) = 32),
    rule_name TEXT NOT NULL CHECK (octet_length(rule_name) BETWEEN 1 AND 128),
    channel_id BIGINT NOT NULL CHECK (channel_id > 0),
    binding_sha256 BYTEA NOT NULL CHECK (octet_length(binding_sha256) = 32),
    expires_at TIMESTAMPTZ NOT NULL,
    touched_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX affinity_cache_expiry ON core_channels.affinity_cache(expires_at);
CREATE INDEX affinity_cache_rule ON core_channels.affinity_cache(rule_name);
CREATE INDEX affinity_cache_recency ON core_channels.affinity_cache(touched_at DESC, key_sha256);
REVOKE ALL ON core_channels.affinity_cache FROM PUBLIC;

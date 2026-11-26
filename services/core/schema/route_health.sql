-- Shared across Core instances. Each generation identifies one immutable routing
-- release and credential revision; an old probe cannot modify a replacement.
CREATE TABLE core_channels.route_health (
    routing_version bigint NOT NULL CHECK (routing_version > 0),
    channel_id bigint NOT NULL CHECK (channel_id > 0),
    credential_id bigint NOT NULL CHECK (credential_id > 0),
    credential_revision bigint NOT NULL CHECK (credential_revision > 0),
    disabled_scope text CHECK (disabled_scope IN ('channel','credential')),
    reason text CHECK (reason IN ('http_status','error_keyword','slow_response','probe_failure')),
    status_code smallint CHECK (status_code BETWEEN 100 AND 599),
    last_probe_at timestamptz,
    latency_ms bigint CHECK (latency_ms >= 0),
    next_probe_at timestamptz NOT NULL DEFAULT now(),
    lease_until timestamptz,
    generation bigint NOT NULL DEFAULT 0 CHECK (generation >= 0),
    PRIMARY KEY (routing_version,channel_id,credential_id,credential_revision)
);

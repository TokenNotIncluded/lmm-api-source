GRANT SELECT, INSERT, DELETE ON core_channels.affinity_cache TO :"runtime_role";
GRANT UPDATE(rule_name,channel_id,binding_sha256,expires_at,touched_at)
    ON core_channels.affinity_cache TO :"runtime_role";
GRANT SELECT, INSERT ON core_channels.route_health TO :"runtime_role";
GRANT UPDATE(disabled_scope,reason,status_code,last_probe_at,latency_ms,next_probe_at,lease_until,generation)
    ON core_channels.route_health TO :"runtime_role";

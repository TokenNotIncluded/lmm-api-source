-- A soft hold uses an otherwise eligible key's next eligibility time. A hard
-- quarantine keeps its own scope, reason and status, and cannot be probed early.
INSERT INTO core_channels.route_health AS h
    (routing_version, channel_id, credential_id, credential_revision, reason, status_code, next_probe_at)
SELECT $1, $2, k.id, k.revision, 'http_status', 429, $5
FROM unnest($3::bigint[], $4::bigint[]) AS k(id, revision)
WHERE $5 > statement_timestamp()
  AND coalesce((public.routing_control_read() #>> '{routing,version}')::bigint, $1) = $1
ORDER BY k.id, k.revision
ON CONFLICT (routing_version, channel_id, credential_id, credential_revision) DO UPDATE SET
    reason = CASE WHEN h.disabled_scope IS NULL THEN EXCLUDED.reason ELSE h.reason END,
    status_code = CASE WHEN h.disabled_scope IS NULL THEN EXCLUDED.status_code ELSE h.status_code END,
    next_probe_at = CASE WHEN h.disabled_scope IS NOT NULL OR (h.reason = 'http_status' AND h.status_code = 429)
                         THEN greatest(h.next_probe_at, EXCLUDED.next_probe_at) ELSE EXCLUDED.next_probe_at END,
    lease_until = NULL, generation = h.generation + 1

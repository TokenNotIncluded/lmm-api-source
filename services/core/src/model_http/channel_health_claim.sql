WITH due AS (
 SELECT c.*, h.next_probe_at
 FROM unnest($2::bigint[], $3::bigint[], $4::bigint[]) AS c(channel_id,credential_id,credential_revision)
 LEFT JOIN core_channels.route_health h ON h.routing_version=$1 AND
   (h.channel_id,h.credential_id,h.credential_revision)=(c.channel_id,c.credential_id,c.credential_revision)
 WHERE ($5 OR h.disabled_scope IS NOT NULL) AND coalesce(h.next_probe_at,'-infinity')<=now()
   AND coalesce(h.lease_until,'-infinity')<=now()
   AND coalesce((public.routing_control_read() #>> '{routing,version}')::bigint,$1)=$1
 ORDER BY h.next_probe_at NULLS FIRST,c.channel_id,c.credential_id,c.credential_revision LIMIT 1
)
INSERT INTO core_channels.route_health AS h (routing_version,channel_id,credential_id,credential_revision,lease_until,generation)
 SELECT $1,channel_id,credential_id,credential_revision,now()+make_interval(secs=>$6),1 FROM due
 ON CONFLICT (routing_version,channel_id,credential_id,credential_revision) DO UPDATE SET
   lease_until=EXCLUDED.lease_until,generation=h.generation+1
 WHERE coalesce(h.lease_until,'-infinity')<=now() AND h.next_probe_at<=now()
   AND ($5 OR h.disabled_scope IS NOT NULL)
 RETURNING channel_id,credential_id,credential_revision,generation

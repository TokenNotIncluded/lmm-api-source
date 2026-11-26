UPDATE core_channels.route_health SET last_probe_at=clock_timestamp(),latency_ms=$6,
 next_probe_at=now()+make_interval(secs=>$7),lease_until=NULL,generation=generation+1,
 disabled_scope=CASE WHEN $8 THEN NULL WHEN disabled_scope='channel' THEN 'channel' ELSE coalesce($9,disabled_scope) END,
 reason=CASE WHEN $8 THEN NULL ELSE coalesce($10,reason) END,
 status_code=CASE WHEN $8 THEN NULL WHEN $10 IS NOT NULL THEN $11 ELSE status_code END
 WHERE routing_version=$1 AND channel_id=$2 AND credential_id=$3 AND credential_revision=$4
   AND generation=$5 AND lease_until>clock_timestamp()
   AND EXISTS (SELECT FROM public.routing_active_release WHERE coalesce(config_version,$1)=$1)

WITH requested AS (
  SELECT * FROM unnest($1::bigint[],$2::bigint[],$3::bigint[],$4::bytea[])
    AS q(id,credential_id,credential_revision,binding_sha256)
), native AS (
  SELECT channel_id,sum(gross_credits-refunded_credits) AS net,
    sum(unknown_requests) AS unknown
  FROM core_channels.usage_rollups
  WHERE day=-1 AND (channel_id=0 OR channel_id=ANY($1))
  GROUP BY channel_id
), facts AS (
  SELECT q.*,i.id IS NOT NULL AND NOT i.history_ambiguous AND i.retired_at IS NULL AS identified,
    coalesce(n.net,0)+coalesce(g.net,0)+coalesce(c.gross_credits-c.refunded_credits,0) AS net,
    coalesce(n.unknown,0)+coalesce(g.unknown,0) AS unknown,
    coalesce(c.historical_coverage,false) AS native_covered,
    c.source_channel_id IS NULL AS legacy_gap_only,
    o.channel_id IS NOT NULL AS has_opening,o.historical_coverage AS opening_covered,
    o.kind AS opening_kind,coalesce(o.offset_numerator,0) AS offset_n,
    coalesce(o.offset_denominator,1) AS offset_d
  FROM requested q
  LEFT JOIN core_channels.identities i ON i.id=q.id
  LEFT JOIN native n ON n.channel_id=q.id
  LEFT JOIN native g ON g.channel_id=0
  LEFT JOIN core_channels.counter_baselines c ON c.channel_id=q.id
  LEFT JOIN core_channels.opening_baselines o ON o.channel_id=q.id
)
SELECT f.id,
  jsonb_build_array(f.net,f.unknown,f.native_covered,f.has_opening,f.opening_covered,f.opening_kind,f.offset_n,f.offset_d,b.observed_at,b.recorded_at,p.started_at,p.observed_at)::text AS generation,
  source.observed_at AS historical_observed_at,
  (f.identified AND f.unknown=0 AND
    CASE WHEN f.has_opening THEN f.opening_covered AND
      (f.native_covered OR f.legacy_gap_only OR f.opening_kind='copied')
    ELSE f.native_covered END) AS complete,
  (f.net*f.offset_d+f.offset_n*500000)::text AS numerator,
  (f.offset_d*500000)::text AS denominator,
  b.balance,b.unit_label,b.kind AS balance_kind,
  floor(extract(epoch FROM b.observed_at))::bigint AS balance_updated_time,
  floor(extract(epoch FROM b.recorded_at))::bigint AS balance_recorded_at,
  b.source_channel_id AS balance_source_channel_id,
  p.result AS probe_result,p.duration_ms,
  floor(extract(epoch FROM p.observed_at))::bigint AS test_time
FROM facts f
LEFT JOIN core_channels.opening_baselines opening ON opening.channel_id=f.id
LEFT JOIN core_channels.opening_sources source ON source.source_key=opening.source_key
LEFT JOIN core_channels.balance_observations b ON b.channel_id=f.id AND f.identified
LEFT JOIN core_channels.catalog_observations p ON p.channel_id=f.id AND f.identified
  AND p.credential_id=f.credential_id AND p.credential_revision=f.credential_revision
  AND p.binding_sha256=f.binding_sha256 AND p.scope='model_catalog'

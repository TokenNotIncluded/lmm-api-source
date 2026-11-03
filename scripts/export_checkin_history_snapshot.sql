-- Offline restored source only; observed_at is the reviewed archive observation,
-- not the wall-clock time at which this export happens to run.
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT jsonb_build_object(
 'source_namespace','lmm_api.lmm_prod_20260802.checkins',
 'observed_at',:'observed_at'::bigint,
 'source_row_count',(SELECT count(*) FROM lmm_prod_20260802.checkins),
 'source_user_ids',(SELECT jsonb_agg(id ORDER BY id) FROM lmm_prod_20260802.users),
 'settings',(SELECT jsonb_object_agg(key,value) FROM lmm_prod_20260802.options
  WHERE key IN('checkin_setting.enabled','checkin_setting.min_quota',
   'checkin_setting.max_quota','checkin_setting.level_multipliers')),
 'rows',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY id),'[]'::jsonb)
  FROM lmm_prod_20260802.checkins c)
);
ROLLBACK;

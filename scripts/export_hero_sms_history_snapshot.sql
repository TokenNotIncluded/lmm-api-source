-- Offline restored source only. Preserve original encrypted payloads and price
-- strings verbatim; never call a supplier or replay old reserve/refund entries.
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT jsonb_build_object(
 'source_namespace','lmm_api.lmm_prod_20260802.hero_sms_sms',
 'observed_at',:'observed_at'::bigint,
 'source_user_ids',(SELECT jsonb_agg(id ORDER BY id) FROM lmm_prod_20260802.users),
 'tables',jsonb_build_object(
  'hero_sms_sms_orders',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY id),'[]'::jsonb)
   FROM lmm_prod_20260802.hero_sms_sms_orders o),
  'hero_sms_sms_quota_ledgers',(SELECT coalesce(jsonb_agg(to_jsonb(l) ORDER BY id),'[]'::jsonb)
   FROM lmm_prod_20260802.hero_sms_sms_quota_ledgers l))
);
ROLLBACK;

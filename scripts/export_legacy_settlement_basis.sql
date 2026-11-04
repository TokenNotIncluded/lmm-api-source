-- Frozen local restored source only. Pair with the pinned original-Go oracle.
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT jsonb_build_object(
 'source_namespace','lmm_api.lmm_prod_20260802.unsettled_topup_basis',
 'observed_at',:'observed_at'::bigint,
 'source_user_ids',(SELECT jsonb_agg(id ORDER BY id) FROM lmm_prod_20260802.users),
 'tables',jsonb_build_object(
  'top_ups',(SELECT coalesce(jsonb_agg(to_jsonb(t)||jsonb_build_object('money',t.money::text) ORDER BY id),'[]'::jsonb)
   FROM lmm_prod_20260802.top_ups t WHERE status='pending' OR(status='failed' AND payment_provider='waffo_pancake' AND failure_reason_code='checkout_timeout')),
  'wallet_credit_rebases',(SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY migration_id),'[]'::jsonb) FROM lmm_prod_20260802.wallet_credit_rebases p),
  'credit_options',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY key),'[]'::jsonb) FROM lmm_prod_20260802.options o
   WHERE key IN('CreditsPerUSD','LegacyPricingQuotaPerUnit','QuotaPerUnit','USDExchangeRate','TopUpPlatformUnitsPerCNY','PublicCreditsPerUSD'))));
ROLLBACK;

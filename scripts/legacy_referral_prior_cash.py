#!/usr/bin/env python3
"""Read-only supplement to frozen contracts export: exact prior-payment eligibility.

Matches old successfulExternalPaidTopUpQuery + positiveNormalizedCreditedQuotaSQL
and referral_excluded=false. Positive scale is explicitly source QuotaPerUnit.
Outputs one row for EVERY pending invitee, including zero prior qualifying orders.
"""
print('''BEGIN READ ONLY ISOLATION LEVEL REPEATABLE READ;
SELECT json_build_object('format_version',1,'pending_prior_cash',
 (SELECT json_agg(t ORDER BY invitee_id) FROM (
 SELECT r.invitee_id,COUNT(p.id) AS qualifying_prior_count
 FROM referral_rewards r LEFT JOIN top_ups p ON p.user_id=r.invitee_id
 AND p.status='success' AND p.referral_excluded=false
 AND (p.settled_amount_micros>0 OR (p.settled_amount_micros=0 AND p.money>0))
 AND COALESCE(p.payment_method,'')<>'balance' AND COALESCE(p.payment_provider,'')<>'balance'
 AND NOT(lower(COALESCE(p.payment_provider,''))='epay' AND lower(COALESCE(p.payment_method,'')) IN ('ldc','linuxdo','linux_do','linuxdo_credit'))
 AND NOT(lower(COALESCE(p.payment_provider,''))='epay' AND lower(COALESCE(p.payment_method,''))='epay'
   AND (upper(COALESCE(p.settlement_currency,''))<>'CNY' OR (p.expected_amount_micros<=0 AND p.settled_amount_micros<=0)))
 AND CASE WHEN NOT(COALESCE(p.payment_provider,'') IN ('epay','stripe','creem','waffo','waffo_pancake')
   OR (COALESCE(p.payment_provider,'')='' AND COALESCE(p.payment_method,'') IN ('stripe','creem','waffo','waffo_pancake','alipay','wxpay'))) THEN false
 WHEN p.credited_quota>0 THEN true
 WHEN p.payment_provider='creem' OR p.payment_method='creem' THEN p.amount>0
 WHEN p.payment_provider IN ('epay','stripe','waffo','waffo_pancake') OR p.payment_method IN ('stripe','waffo','waffo_pancake')
   OR (COALESCE(p.payment_provider,'')='' AND p.payment_method IN ('alipay','wxpay')) THEN p.amount>0
 ELSE false END
 WHERE r.status='earned' AND r.stage='awaiting_first_top_up'
 GROUP BY r.invitee_id) t));
COMMIT;''')

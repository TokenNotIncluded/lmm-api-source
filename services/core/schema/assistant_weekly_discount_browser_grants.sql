-- Browser claim can refresh a nonce and invoke bounded coupon issuance.
GRANT USAGE ON SCHEMA core_weekly,core_payments TO :"runtime_role";
GRANT SELECT ON core_weekly.weekly_discounts,core_weekly.weekly_discount_policy TO :"runtime_role";
GRANT UPDATE(claim_nonce_sha256,claim_nonce_expires_at)
    ON core_weekly.weekly_discounts TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_weekly.lock_weekly_discount_policy(),
    core_weekly.claim_weekly_discount_code(TEXT,TEXT),
    core_weekly.claimed_weekly_discount_code(TEXT,BIGINT,BIGINT) TO :"runtime_role";

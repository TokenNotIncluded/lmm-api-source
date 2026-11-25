-- Read only the owner restriction needed by coupon consumption.
GRANT USAGE ON SCHEMA core_weekly,core_payments TO :"runtime_role";
GRANT SELECT(code_id,user_id) ON core_weekly.weekly_discounts TO :"runtime_role";
GRANT SELECT ON core_payments.weekly_discount_owners TO :"runtime_role";

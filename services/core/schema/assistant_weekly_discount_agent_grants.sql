-- The worker may insert native decisions; origin and claim columns are excluded.
GRANT USAGE ON SCHEMA core_weekly TO :"runtime_role";
GRANT SELECT ON core_weekly.weekly_discounts,core_weekly.weekly_discount_policy TO :"runtime_role";
GRANT INSERT(id,user_id,account_id,week_start,status,discount_percent,reason,conversation_id,operation_id,credential_id,auth_version,web_session_sid,created_at)
    ON core_weekly.weekly_discounts TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_weekly.lock_weekly_discount_policy() TO :"runtime_role";

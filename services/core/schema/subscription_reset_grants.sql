-- Only the explicit owner installer/upgrader invokes this file. Reset codes and
-- imported grant/plan assertions remain owner-written migration facts.
GRANT SELECT ON core_subscriptions.reset_codes,core_subscriptions.reset_imported_grant_plans TO :"runtime_role";
GRANT SELECT,INSERT ON core_subscriptions.reset_vouchers,core_subscriptions.reset_voucher_redemptions,
    core_subscriptions.reset_previews,core_subscriptions.reset_operations,core_subscriptions.reset_events TO :"runtime_role";
GRANT USAGE ON SEQUENCE core_subscriptions.reset_vouchers_id_seq,core_subscriptions.reset_events_id_seq TO :"runtime_role";
GRANT SELECT,INSERT ON core_charging.subscription_request_resets TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_charging.subscription_plan(bigint),
    core_charging.subscription_base_window(bigint,bigint),
    core_charging.subscription_consumption(bigint,bigint,text) TO :"runtime_role";

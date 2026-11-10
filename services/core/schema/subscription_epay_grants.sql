GRANT USAGE ON SCHEMA core_subscriptions,core_payments,core_charging TO :"runtime_role";
GRANT SELECT,INSERT ON core_subscriptions.external_prices,core_subscriptions.external_orders,
 core_subscriptions.external_proofs,core_subscriptions.external_grants TO :"runtime_role";
GRANT UPDATE(confirmed_at,canceled_at) ON core_subscriptions.external_orders TO :"runtime_role";
GRANT SELECT,INSERT ON core_payments.provider_identifiers TO :"runtime_role";
GRANT SELECT,INSERT ON core_charging.subscriptions TO :"runtime_role";
GRANT USAGE ON SEQUENCE core_charging.subscriptions_id_seq TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_subscriptions.external_ends_at(JSONB,BIGINT),core_subscriptions.apply_epay_grant(TEXT,TEXT,TEXT) TO :"runtime_role";

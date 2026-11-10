-- :runtime_role is supplied only by the explicit owner installer/upgrader.
GRANT SELECT,INSERT ON core_subscriptions.stripe_prices,core_subscriptions.stripe_state,
 core_subscriptions.stripe_events,core_subscriptions.stripe_cycles TO :"runtime_role";
GRANT UPDATE(provider_subscription_id,provider_customer_id,status,event_created,event_id,period_start,period_end)
 ON core_subscriptions.stripe_state TO :"runtime_role";
-- Existing shared orders/namespace/charging grants remain necessary.
-- No EXECUTE grant, wallet journal authority, upgrade receipt access or DDL.

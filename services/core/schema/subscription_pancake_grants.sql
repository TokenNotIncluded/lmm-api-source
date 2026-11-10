-- Runtime never installs this component and never writes wallet journals here.
GRANT SELECT,INSERT ON core_subscriptions.pancake_prices,core_subscriptions.pancake_orders,core_subscriptions.pancake_events,core_subscriptions.pancake_cycles TO :"runtime_role";
GRANT UPDATE(provider_order_id,status,event_created,event_id,paid_period_start,paid_period_end) ON core_subscriptions.pancake_orders TO :"runtime_role";

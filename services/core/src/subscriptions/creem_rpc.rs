use super::*;
use crate::internal_rpc::pb;

pub(super) fn order(o: CreemOrder) -> Result<pb::CreemSubscriptionOrder> {
    let p = o.binding.price;
    Ok(pb::CreemSubscriptionOrder {
        order: Some(super::epay_rpc::order_wire(o.order)?),
        price: Some(pb::CreemSubscriptionPrice {
            plan_id: p.plan_id,
            plan_revision: p.plan_revision,
            channel_id: p.channel_id,
            currency: p.currency,
            amount_minor: p.amount_minor,
            provider_price_id: p.provider_price_id,
            interval: p.interval,
            interval_count: p.interval_count,
            success_url: p.success_url,
            cancel_url: p.cancel_url,
        }),
        merchant: o.binding.merchant,
        environment: o.binding.environment,
        provider_subscription_id: o.provider_subscription_id.unwrap_or_default(),
        provider_customer_id: o.provider_customer_id.unwrap_or_default(),
        state: o.state,
        state_created: o.state_created,
        current_period_start: o.current_period_start,
        current_period_end: o.current_period_end,
        latest_grant_id: o.latest_grant_id,
    })
}
pub(super) fn event(e: CreemEventReceipt) -> pb::CreemSubscriptionEvent {
    pb::CreemSubscriptionEvent {
        order_id: e.order_id,
        event_id: e.event_id,
        payload_sha256: e.payload_sha256,
        kind: e.kind,
        state: e.state,
        transaction_id: e.transaction_id,
        subscription_id: e.subscription_id,
        period_start: e.period_start,
        period_end: e.period_end,
    }
}

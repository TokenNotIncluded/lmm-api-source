use super::*;
use crate::internal_rpc::pb;
pub(super) fn order(v: PancakeOrder) -> Result<pb::PancakeSubscriptionOrder> {
    let p = v.binding.price;
    Ok(pb::PancakeSubscriptionOrder {
        order: Some(super::epay_rpc::order_wire(v.order)?),
        price: Some(pb::PancakeSubscriptionPrice {
            plan_id: p.plan_id,
            plan_revision: p.plan_revision,
            channel_id: p.channel_id,
            currency: p.currency,
            amount_minor: p.amount_minor,
            store_id: p.store_id,
            product_id: p.product_id,
            product_type: p.product_type,
            billing_period: p.billing_period,
            provider_plan_reference: p.provider_plan_reference,
            return_url: p.return_url,
            checkout_region: p.checkout_region,
            checkout_language: p.checkout_language,
            tax_category: p.tax_category,
        }),
        merchant: v.binding.merchant,
        environment: v.binding.environment,
        provider_reference: v.provider_reference,
        provider_order_id: v.provider_order_id.unwrap_or_default(),
        state: v.state,
        state_created: v.state_created,
        paid_period_start: v.paid_period_start,
        paid_period_end: v.paid_period_end,
        latest_grant_id: v.latest_grant_id,
        legacy_source_json: v
            .legacy_source
            .map(|v| serde_json::to_vec(&v))
            .transpose()
            .map_err(|_| Error::Unavailable)?
            .unwrap_or_default(),
    })
}
pub(super) fn event(v: PancakeEventReceipt) -> pb::PancakeSubscriptionEvent {
    pb::PancakeSubscriptionEvent {
        order_id: v.order_id,
        event_id: v.event_id,
        payload_sha256: v.payload_sha256,
        kind: v.kind,
        state: v.state,
        provider_order_id: v.provider_order_id,
        payment_id: v.payment_id,
        cycle_key: v.cycle_key,
        subscription_id: v.subscription_id,
        period_start: v.period_start,
        period_end: v.period_end,
    }
}

//! Current published subscription terms and prices. This reader reuses the
//! purchase counter and provider binding validators; it never prepares an order.
use super::*;
use crate::payments::offers::{OfferSource, binding};

#[derive(Serialize)]
pub(crate) struct PlanOffer {
    pub id: i64,
    pub revision: i64,
    pub terms: PlanTerms,
    pub purchases: i64,
    pub external_prices: Vec<PriceOffer>,
}
#[derive(Serialize)]
pub(crate) struct PriceOffer {
    pub channel_id: i64,
    pub provider: String,
    pub environment: String,
    pub currency: String,
    pub amount_minor: String,
    pub binding_sha256: String,
    pub configured: bool,
}
pub(crate) async fn read(
    c: &mut PgConnection,
    user: i64,
    source: Option<&OfferSource>,
) -> Result<Vec<PlanOffer>> {
    let rows = sqlx::query(
        "SELECT p.id AS plan_id,p.revision,p.archived,r.terms FROM core_subscriptions.plans p
         JOIN core_subscriptions.plan_revisions r ON r.plan_id=p.id AND r.revision=p.revision
         WHERE NOT p.archived AND (r.terms->>'enabled')::boolean ORDER BY p.id LIMIT 1001",
    )
    .fetch_all(&mut *c)
    .await?;
    if rows.len() > 1000 {
        return Err(Error::Unavailable);
    }
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let plan = plan_row(&row)?;
        let purchases = purchase_count(c, user, plan.id).await?;
        let mut prices = Vec::new();
        if let Some(source) = source {
            let published: Vec<(i64, String, i64)> = sqlx::query_as(
                "SELECT channel_id,currency,amount_minor FROM core_subscriptions.external_prices
                 WHERE plan_id=$1 AND plan_revision=$2 ORDER BY channel_id",
            )
            .bind(plan.id)
            .bind(plan.revision)
            .fetch_all(&mut *c)
            .await?;
            for (id, currency, amount) in published {
                let Some(channel) = source.channel(id) else {
                    continue;
                };
                let valid = match channel.provider.as_str() {
                    "epay" => {
                        channel.enabled()
                            && channel.currency == currency
                            && amount <= channel.max_amount_minor
                            && source.store.subscription_binding(id).is_ok()
                    }
                    "stripe" => match external_stripe::price(c, plan.id, id).await {
                        Ok(p) => source.store.stripe_subscription_binding(p).is_ok(),
                        Err(Error::NotFound) => false,
                        Err(e) => return Err(e),
                    },
                    "creem" => match external_creem::price(c, plan.id, id).await {
                        Ok(p) => source.store.creem_subscription_binding(p).is_ok(),
                        Err(Error::NotFound) => false,
                        Err(e) => return Err(e),
                    },
                    "waffo_pancake" => match external_pancake::price(c, plan.id, id).await {
                        Ok(p) => source.store.pancake_subscription_binding(p).is_ok(),
                        Err(Error::NotFound) => false,
                        Err(e) => return Err(e),
                    },
                    _ => false,
                };
                prices.push(PriceOffer {
                    channel_id: id,
                    provider: channel.provider.clone(),
                    environment: channel.environment.clone(),
                    currency,
                    amount_minor: amount.to_string(),
                    binding_sha256: binding(channel, true),
                    configured: source.subscription_orders && valid,
                });
            }
        }
        out.push(PlanOffer {
            id: plan.id,
            revision: plan.revision,
            terms: plan.terms,
            purchases,
            external_prices: prices,
        });
    }
    Ok(out)
}

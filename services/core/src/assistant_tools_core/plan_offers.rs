//! A closed original-Session read. Host checkout configuration is injected from
//! the same FinancialRpcs instances that serve requests, never from tool JSON.
use super::{Error, context};
use crate::payments::offers::OfferSource;
use serde::Serialize;
use sqlx::PgPool;

#[derive(Clone)]
pub(crate) struct PlanOffersStore {
    pool: PgPool,
    source: Option<OfferSource>,
}
#[derive(Serialize)]
pub(crate) struct PlanOffers {
    pub contract: &'static str,
    pub actor_user_id: i64,
    pub checked_at_unix: i64,
    pub topup_restricted: bool,
    pub plans: Vec<crate::subscriptions::assistant_offers::PlanOffer>,
    pub topup_channels: Vec<crate::payments::offers::ChannelOffer>,
}
impl PlanOffersStore {
    pub(crate) async fn new(pool: PgPool, source: Option<OfferSource>) -> Result<Self, Error> {
        let mut tx = pool.begin().await?;
        for sql in [
            "SELECT id,revision,archived FROM core_subscriptions.plans WHERE FALSE",
            "SELECT plan_id,revision,terms FROM core_subscriptions.plan_revisions WHERE FALSE",
            "SELECT actor_user_id,quote_id,refund_journal_id FROM core_subscriptions.purchases WHERE FALSE",
            "SELECT id,plan_id FROM core_subscriptions.quotes WHERE FALSE",
            "SELECT actor_user_id,plan_id,confirmed_at FROM core_subscriptions.external_orders WHERE FALSE",
            "SELECT actor_user_id,request,result FROM core_subscriptions.commands WHERE FALSE",
            "SELECT plan_id,plan_revision,channel_id,currency,amount_minor FROM core_subscriptions.external_prices WHERE FALSE",
            "SELECT * FROM core_subscriptions.stripe_prices WHERE FALSE",
            "SELECT * FROM core_subscriptions.creem_prices WHERE FALSE",
            "SELECT * FROM core_subscriptions.pancake_prices WHERE FALSE",
        ] {
            sqlx::query(sql).fetch_all(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(Self { pool, source })
    }
    pub(crate) async fn read(&self, session: &str) -> Result<PlanOffers, Error> {
        let source = self
            .source
            .as_ref()
            .map(OfferSource::freeze)
            .transpose()
            .map_err(|_| Error::Unavailable)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='2000ms'")
            .execute(&mut *tx)
            .await?;
        let context = context::from_session(&mut tx, session).await?;
        let marked = crate::abuse::is_marked_in_transaction(&mut tx, context.user_id())
            .await
            .map_err(|_| Error::Unavailable)?;
        let plans = crate::subscriptions::assistant_offers::read(
            &mut tx,
            context.user_id(),
            source.as_ref(),
        )
        .await
        .map_err(|_| Error::Unavailable)?;
        let checked: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        context.revalidate(&mut tx).await?;
        tx.commit().await?;
        Ok(PlanOffers {
            contract: "assistant.plan-offers.v1",
            actor_user_id: context.user_id(),
            checked_at_unix: checked,
            topup_restricted: marked,
            plans,
            topup_channels: source.map(|s| s.topups()).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests;

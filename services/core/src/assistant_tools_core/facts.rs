//! Private host projections use the original live Session. Host configuration
//! and the actual serving gateway are injected at construction, never by Go.
use super::{
    Error, context,
    models::{AvailableModels, ModelsProjection},
    support,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

#[derive(Clone)]
pub struct BusinessFactsStore {
    pool: PgPool,
    models: ModelsProjection,
    plan_offers: Option<super::plan_offers::PlanOffersStore>,
    invitation_facts: bool,
}
impl BusinessFactsStore {
    /// Startup observes actual required columns with the runtime connection;
    /// it creates no data and cannot be satisfied by owner-only privileges.
    pub async fn check_readonly(&self) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await?;
        let gate: Option<i32> =
            sqlx::query_scalar("SELECT id FROM core_humangrant.human_grant_gate WHERE id=1")
                .fetch_optional(&mut *tx)
                .await?;
        if gate != Some(1) {
            return Err(Error::Unavailable);
        }
        for sql in [
            "SELECT user_id,group_name,enabled FROM core_identity.user_routing_groups WHERE FALSE",
            "SELECT revision,document FROM core_abuse.settings WHERE FALSE",
            "SELECT user_id,marked,policy_id,revision FROM core_abuse.user_marks WHERE FALSE",
            "SELECT account_id,created_by_user_id,provider,environment,amount_minor,credit_journal_id,provider_transaction,refunded_minor,refund_fenced,id FROM core_payments.intents WHERE FALSE",
            "SELECT intent_id FROM core_payments.accepted_proofs WHERE FALSE",
            "SELECT intent_id,state FROM core_payments.refund_holds WHERE FALSE",
            "SELECT intent_id FROM core_payments.blocked_refunds WHERE FALSE",
        ] {
            sqlx::query(sql).fetch_all(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub fn new(pool: PgPool, gateway: &crate::model_http::ModelGateway) -> Self {
        Self {
            pool,
            models: ModelsProjection::from_gateway(gateway),
            plan_offers: None,
            invitation_facts: false,
        }
    }
    pub(crate) async fn enable_invitation_facts(&mut self) -> Result<(), Error> {
        super::invitations::check_ready(&self.pool).await?;
        self.invitation_facts = true;
        Ok(())
    }
    pub(crate) fn invitation_facts_available(&self) -> bool {
        self.invitation_facts
    }
    pub(crate) async fn invitation_facts(
        &self,
        session: &str,
    ) -> Result<super::invitations::InvitationFacts, Error> {
        if !self.invitation_facts {
            return Err(Error::Unavailable);
        }
        super::invitations::read(&self.pool, session).await
    }
    pub(crate) async fn enable_plan_offers(
        &mut self,
        payments: Option<crate::payments::rpc::PaymentsRpc>,
        subscription_orders: bool,
    ) -> Result<(), Error> {
        self.plan_offers = Some(
            super::plan_offers::PlanOffersStore::new(
                self.pool.clone(),
                payments.map(|p| p.offer_source(subscription_orders)),
            )
            .await?,
        );
        Ok(())
    }
    pub(crate) fn plan_offers_available(&self) -> bool {
        self.plan_offers.is_some()
    }
    pub(crate) async fn plan_offers(
        &self,
        session: &str,
    ) -> Result<super::plan_offers::PlanOffers, Error> {
        self.plan_offers
            .as_ref()
            .ok_or(Error::Unavailable)?
            .read(session)
            .await
    }
    pub async fn available_models(&self, original_session: &str) -> Result<AvailableModels, Error> {
        let mut tx = self.pool.begin().await?;
        let context = context::from_session(&mut tx, original_session).await?;
        let result = self.models.read_in_transaction(&mut tx, &context).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn support_eligibility(&self, original_session: &str) -> Result<Value, Error> {
        let mut tx = self.pool.begin().await?;
        let context = context::from_session(&mut tx, original_session).await?;
        let mut result = support::eligibility_in_transaction(&mut tx, &context).await?;
        result["actor_user_id"] = serde_json::json!(context.user_id());
        let facts = context.revalidate(&mut tx).await?;
        let checked: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        result["checked_at_unix"] = serde_json::json!(checked);
        // An authenticated observation digest, never a grant or ledger receipt.
        let binding = serde_json::json!({"result":result,"account_id":context.account_id(),"credential_id":context.credential_id(),"auth_version":facts.auth_version});
        let digest = Sha256::digest(serde_json::to_vec(&binding).map_err(|_| Error::Unavailable)?);
        result["projection_id"] = serde_json::json!(format!(
            "support-observation-v1:{}",
            crate::hex_codec::encode(digest)
        ));
        result["projection_kind"] =
            serde_json::json!("authenticated_read_observation_not_financial_receipt");
        tx.commit().await?;
        Ok(result)
    }
}

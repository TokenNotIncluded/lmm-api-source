//! Assistant market authority is constructed from the already validated live
//! Agent configuration. A host token, a service ID or model output is not consent.
use super::{
    Error, Owner, Result,
    credit_access::{ConsentReceipt, CreditStore},
    credit_execution::{Receipt, ReserveRequest},
    credits::Quote,
};
use crate::{
    assistant_model::agent::{
        market_policy::{
            AssistantMarketPolicy, MarketIntent, authorize_market_policy_context_in_transaction,
            authorize_market_tool_operation_in_transaction,
        },
        store::AgentPolicy,
    },
    assistant_tools_core::context::VerifiedToolContext,
};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct AssistantMarketRuntime {
    policy: Arc<AgentPolicy>,
}
impl AssistantMarketRuntime {
    pub(crate) fn from_policy(policy: Arc<AgentPolicy>) -> Option<Self> {
        policy.market.as_ref()?;
        Some(Self { policy })
    }
    pub(crate) fn require(&self, tool: &str) -> Result<&AssistantMarketPolicy> {
        if !matches!(
            tool,
            "get_connected_market_tools" | "connect_market_tool" | "call_market_tool"
        ) || !self.policy.allowed_tools.contains(tool)
        {
            return Err(Error::Forbidden);
        }
        self.policy.market.as_ref().ok_or(Error::Forbidden)
    }
}
pub(super) fn assistant_error(error: crate::assistant_model::Error) -> Error {
    match error {
        crate::assistant_model::Error::Invalid => Error::Invalid,
        crate::assistant_model::Error::Unauthorized => Error::Unauthorized,
        crate::assistant_model::Error::Conflict => Error::Conflict,
        crate::assistant_model::Error::Unavailable => Error::Storage,
        _ => Error::Forbidden,
    }
}
fn context_error(error: crate::assistant_tools_core::Error) -> Error {
    match error {
        crate::assistant_tools_core::Error::Invalid => Error::Invalid,
        crate::assistant_tools_core::Error::Unauthorized => Error::Unauthorized,
        crate::assistant_tools_core::Error::Conflict => Error::Conflict,
        crate::assistant_tools_core::Error::Unavailable => Error::Storage,
        _ => Error::Forbidden,
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantReserveRequest {
    pub request: ReserveRequest,
    pub assistant_tool_operation_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCatalogueRequest {
    pub owner: Owner,
    #[serde(default)]
    pub service_id: String,
    #[serde(default)]
    pub version_id: String,
    #[serde(default)]
    pub tool_ids: Vec<String>,
    #[serde(default)]
    pub grant_id: String,
}
#[derive(Serialize)]
pub struct AssistantCatalogueReceipt {
    pub owner: Owner,
    pub policy_revision: String,
    pub allowed_service_ids: Vec<String>,
    pub consents: Vec<ConsentReceipt>,
    pub checked_at_unix: i64,
}
impl CreditStore {
    pub(crate) fn with_assistant_runtime(mut self, runtime: AssistantMarketRuntime) -> Self {
        self.assistant_runtime = Some(runtime);
        self
    }
    pub async fn reserve_assistant(
        &self,
        secret: &str,
        input: &AssistantReserveRequest,
    ) -> Result<Receipt> {
        if input.request.client_id != "builtin-assistant"
            || !super::name(&input.assistant_tool_operation_id)
            || input.request.request_key != input.assistant_tool_operation_id
        {
            return Err(Error::Invalid);
        }
        self.reserve_from_source(
            secret,
            &input.request,
            Some(input.assistant_tool_operation_id.as_str()),
        )
        .await
    }
    pub(super) async fn authorize_assistant_call(
        &self,
        c: &mut PgConnection,
        secret: &str,
        operation_id: &str,
        request: &ReserveRequest,
        quote: &Quote,
    ) -> Result<()> {
        let policy = self
            .assistant_runtime
            .as_ref()
            .ok_or(Error::Forbidden)?
            .require("call_market_tool")?;
        let actor = crate::identity::authorize_in_transaction(&mut *c, secret).await?;
        if actor.credential_kind != crate::identity::CredentialKind::Session
            || actor.owner.kind != crate::accounts::AccountKind::Personal
            || actor.owner.id != actor.user_id
            || request.client_id != "builtin-assistant"
            || request.request_key != operation_id
        {
            return Err(Error::Forbidden);
        }
        let authority = authorize_market_tool_operation_in_transaction(
            &mut *c,
            &actor,
            operation_id,
            MarketIntent {
                service_id: &quote.service_id,
                tool_id: &request.tool_id,
                version_id: &request.version_id,
                grant_id: &request.grant_id,
                input_digest: &request.input_digest,
                quote_id: &request.quote_id,
            },
            policy,
        )
        .await
        .map_err(assistant_error)?;
        if authority.operation_id() != operation_id
            || authority.quote_id() != request.quote_id
            || authority.consent_id() != request.grant_id
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    /// Read only current host scope and this Session's exact-version consents.
    /// Go separately joins the real published schemas, never an empty fallback.
    pub async fn assistant_catalogue(
        &self,
        secret: &str,
        input: &AssistantCatalogueRequest,
    ) -> Result<AssistantCatalogueReceipt> {
        let policy = self
            .assistant_runtime
            .as_ref()
            .ok_or(Error::Forbidden)?
            .require(if input.grant_id.is_empty() {
                "get_connected_market_tools"
            } else {
                "call_market_tool"
            })?;
        if (!input.grant_id.is_empty()
            && (!super::name(&input.grant_id) || input.tool_ids.len() != 1))
            || input.tool_ids.len() > 2
            || input.tool_ids.iter().any(|id| !super::name(id))
            || input
                .tool_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != input.tool_ids.len()
            || (input.service_id.is_empty()
                && (!input.version_id.is_empty()
                    || !input.tool_ids.is_empty()
                    || !input.grant_id.is_empty()))
            || (!input.service_id.is_empty()
                && (!super::name(&input.service_id) || !super::name(&input.version_id)))
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.base.begin().await?;
        let actor = Self::owned(&mut tx, secret, &input.owner).await?;
        let context = VerifiedToolContext::verify(&mut tx, &actor)
            .await
            .map_err(context_error)?;
        let facts = context.revalidate(&mut tx).await.map_err(context_error)?;
        // Even an empty page must verify any imported administrator's full policy.
        if matches!(facts.role.as_str(), "admin" | "superadmin")
            && crate::identity::has_privileged_import_facts(&mut tx, actor.user_id)
                .await
                .map_err(|_| Error::Storage)?
        {
            crate::identity::load_complete_policy(&mut tx, actor.user_id)
                .await
                .map_err(|_| Error::Forbidden)?;
        }
        let checked_at_unix: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        let mut consents = Vec::new();
        if !input.service_id.is_empty() {
            authorize_market_policy_context_in_transaction(
                &mut tx,
                &context,
                &input.service_id,
                policy,
            )
            .await
            .map_err(assistant_error)?;
            let ids: Vec<String> = if input.grant_id.is_empty() {
                sqlx::query_scalar(
                "SELECT g.id FROM unnest($4::text[]) AS requested(tool_id) CROSS JOIN LATERAL (SELECT c.id,c.created_at FROM core_tool_market.credit_consents c JOIN core_tool_market.credit_quotes q ON q.id=c.quote_id WHERE c.actor_user_id=$1 AND c.payer_account_id=$2 AND c.request->>'client_id'='builtin-assistant' AND c.request->>'version_id'=$3 AND c.request->>'tool_id'=requested.tool_id AND q.snapshot->>'service_id'=$5 AND c.revoked_at=0 AND (c.request->>'expires_at')::bigint>$6 AND q.approved AND NOT q.suspended AND (q.snapshot->>'expires')::bigint>$6 ORDER BY c.created_at DESC,c.id DESC LIMIT 10) g ORDER BY requested.tool_id,g.created_at DESC,g.id DESC",
            ).bind(actor.user_id).bind(actor.owner_account_id).bind(&input.version_id)
                .bind(&input.tool_ids).bind(&input.service_id).bind(checked_at_unix)
                .fetch_all(&mut *tx).await?
            } else {
                sqlx::query_scalar("SELECT c.id FROM core_tool_market.credit_consents c JOIN core_tool_market.credit_quotes q ON q.id=c.quote_id WHERE c.id=$7 AND c.actor_user_id=$1 AND c.payer_account_id=$2 AND c.request->>'client_id'='builtin-assistant' AND c.request->>'version_id'=$3 AND c.request->>'tool_id'=ANY($4::text[]) AND q.snapshot->>'service_id'=$5 AND c.revoked_at=0 AND (c.request->>'expires_at')::bigint>$6 AND q.approved AND NOT q.suspended AND (q.snapshot->>'expires')::bigint>$6")
                    .bind(actor.user_id).bind(actor.owner_account_id).bind(&input.version_id)
                    .bind(&input.tool_ids).bind(&input.service_id).bind(checked_at_unix).bind(&input.grant_id)
                    .fetch_all(&mut *tx).await?
            };
            for id in ids {
                consents.push(Self::consent(&mut tx, &id, actor.user_id).await?);
            }
        }
        context.revalidate(&mut tx).await.map_err(context_error)?;
        let result = AssistantCatalogueReceipt {
            owner: input.owner.clone(),
            policy_revision: policy.revision.clone(),
            allowed_service_ids: policy.allowed_service_ids.iter().cloned().collect(),
            consents,
            checked_at_unix,
        };
        tx.commit().await?;
        Ok(result)
    }
}

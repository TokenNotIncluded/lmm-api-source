//! Dedicated service capability plus current user session for user operations.
use super::*;
use crate::{
    internal_rpc::pb,
    payments::{
        PaymentsStore,
        rpc::{PaymentsRpc, Permissions as PaymentPermissions, single},
    },
};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tonic::{Request, Response, Status};

#[derive(Clone, Copy, Default)]
pub struct Permissions {
    pub user_orders: bool,
    pub proof_ingress: bool,
    pub proof_read: bool,
    pub grant: bool,
}
#[derive(Clone)]
pub struct EPaySubscriptionsRpc {
    store: SubscriptionStore,
    payments: PaymentsStore,
    auth: PaymentsRpc,
    permissions: Permissions,
    slots: Arc<Semaphore>,
}
impl EPaySubscriptionsRpc {
    pub fn new(
        store: SubscriptionStore,
        payments: PaymentsStore,
        token: &[u8],
        permissions: Permissions,
    ) -> Result<Self> {
        let auth = PaymentsRpc::new(payments.clone(), token, PaymentPermissions::default())
            .map_err(|_| Error::Invalid)?;
        Ok(Self {
            store,
            payments,
            auth,
            permissions,
            slots: Arc::new(Semaphore::new(8)),
        })
    }
    pub fn price_router(&self) -> axum::Router {
        super::epay_price_router(self.store.clone(), self.payments.clone())
            .merge(super::pancake_http::router(
                self.store.clone(),
                self.payments.clone(),
            ))
            .merge(super::stripe_http::router(
                self.store.clone(),
                self.payments.clone(),
            ))
    }
    pub fn features(&self) -> Vec<String> {
        [
            (self.permissions.user_orders, "subscriptions.epay.orders"),
            (
                self.permissions.proof_ingress,
                "subscriptions.epay.proof.accept",
            ),
            (self.permissions.proof_read, "subscriptions.epay.proof.read"),
            (self.permissions.grant, "subscriptions.epay.grant"),
        ]
        .into_iter()
        .filter(|(enabled, _)| *enabled)
        .map(|(_, s)| s.into())
        .collect()
    }
    async fn run<T>(
        &self,
        f: impl Future<Output = Result<T>>,
    ) -> std::result::Result<Response<T>, Status> {
        let _slot = self
            .slots
            .try_acquire()
            .map_err(|_| Status::resource_exhausted("subscription service busy"))?;
        tokio::time::timeout(Duration::from_secs(3), f)
            .await
            .map_err(|_| Status::deadline_exceeded("reconcile original subscription operation"))?
            .map(Response::new)
            .map_err(|e| match e {
                Error::Invalid => Status::invalid_argument("invalid subscription request"),
                Error::Unauthorized => Status::unauthenticated("session required"),
                Error::Forbidden => Status::permission_denied("subscription authority denied"),
                Error::NotFound => Status::not_found("subscription record not found"),
                Error::Conflict | Error::Used => {
                    Status::aborted("reconcile original subscription operation")
                }
                Error::Unavailable => Status::unavailable("subscription dependency unavailable"),
            })
    }
}
pub(super) fn order_wire(o: EPayPurchase) -> Result<pb::EPaySubscriptionOrder> {
    Ok(pb::EPaySubscriptionOrder {
        order_id: o.order_id,
        account_id: o.account_id,
        plan_id: o.plan.id,
        plan_revision: o.plan.revision,
        payment_channel_id: o.channel_id,
        currency: o.currency,
        amount_minor: o.amount_minor,
        expires_at_unix: o.expires_at,
        confirmed_at_unix: o.confirmed_at,
        canceled_at_unix: o.canceled_at,
        proof_id: o.proof_id.unwrap_or_default(),
        subscription_id: o.subscription_id,
        granted_at_unix: o.granted_at,
        ends_at_unix: o.ends_at,
        plan_json: serde_json::to_vec(&o.plan).map_err(|_| Error::Unavailable)?,
        checkout: None,
    })
}
fn proof_wire(p: EPayProof) -> pb::EPaySubscriptionProof {
    pb::EPaySubscriptionProof {
        proof_id: p.proof_id,
        order_id: p.order_id,
        provider_event_id: p.event_id,
        provider_transaction: p.transaction_id,
        payload_sha256: p.payload_sha256,
        accepted_at_unix: p.accepted_at,
    }
}
#[tonic::async_trait]
impl pb::core_subscription_payments_server::CoreSubscriptionPayments for EPaySubscriptionsRpc {
    async fn prepare_pancake_purchase(
        &self,
        r: Request<pb::PrepareEPayPurchaseRequest>,
    ) -> std::result::Result<Response<pb::PancakeSubscriptionOrder>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        let r = r.into_inner();
        self.run(async {
            let key = r.mutation.ok_or(Error::Invalid)?.idempotency_key;
            super::pancake_rpc::order(
                self.store
                    .prepare_pancake_purchase(
                        &self.payments,
                        &user,
                        &EPayPurchaseRequest {
                            operation_key: key,
                            plan_id: r.plan_id,
                            channel_id: r.payment_channel_id,
                        },
                    )
                    .await?,
            )
        })
        .await
    }
    async fn confirm_pancake_purchase(
        &self,
        r: Request<pb::EPaySubscriptionOrderRequest>,
    ) -> std::result::Result<Response<pb::PancakeSubscriptionOrder>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        let r = r.into_inner();
        self.run(async {
            super::pancake_rpc::order(
                self.store
                    .confirm_pancake_purchase(&user, &r.order_id)
                    .await?,
            )
        })
        .await
    }
    async fn read_pancake_purchase(
        &self,
        r: Request<pb::EPaySubscriptionOrderRequest>,
    ) -> std::result::Result<Response<pb::PancakeSubscriptionOrder>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        let r = r.into_inner();
        self.run(async {
            super::pancake_rpc::order(self.store.read_pancake_purchase(&user, &r.order_id).await?)
        })
        .await
    }
    async fn list_pancake_purchases(
        &self,
        r: Request<pb::ListPancakePurchasesRequest>,
    ) -> std::result::Result<Response<pb::PancakeSubscriptionOrders>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        self.run(async {
            let orders = self
                .store
                .pancake_purchases(&user)
                .await?
                .into_iter()
                .map(super::pancake_rpc::order)
                .collect::<Result<Vec<_>>>()?;
            Ok(pb::PancakeSubscriptionOrders { orders })
        })
        .await
    }
    async fn accept_pancake_subscription_event(
        &self,
        r: Request<pb::AcceptPancakeSubscriptionEventRequest>,
    ) -> std::result::Result<Response<pb::PancakeSubscriptionEvent>, Status> {
        self.auth.check(
            r.metadata(),
            self.permissions.proof_ingress && self.permissions.grant,
        )?;
        let r = r.into_inner();
        self.run(async {
            Ok(super::pancake_rpc::event(
                self.store
                    .accept_pancake_event(
                        &self.payments,
                        r.payment_channel_id,
                        &r.provider_reference,
                        &r.signed_payload,
                        &r.signature,
                    )
                    .await?,
            ))
        })
        .await
    }
    async fn get_pancake_subscription_event(
        &self,
        r: Request<pb::GetPancakeSubscriptionEventRequest>,
    ) -> std::result::Result<Response<pb::PancakeSubscriptionEvent>, Status> {
        self.auth.check(r.metadata(), self.permissions.proof_read)?;
        let r = r.into_inner();
        self.run(async {
            Ok(super::pancake_rpc::event(
                self.store
                    .get_pancake_event(
                        r.payment_channel_id,
                        &r.provider_reference,
                        &r.provider_event_id,
                        &r.payload_sha256,
                    )
                    .await?,
            ))
        })
        .await
    }

    async fn prepare_stripe_purchase(
        &self,
        r: Request<pb::PrepareEPayPurchaseRequest>,
    ) -> std::result::Result<Response<pb::StripeSubscriptionOrder>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        let r = r.into_inner();
        self.run(async {
            let key = r.mutation.ok_or(Error::Invalid)?.idempotency_key;
            super::stripe_rpc::order(
                self.store
                    .prepare_stripe_purchase(
                        &self.payments,
                        &user,
                        &EPayPurchaseRequest {
                            operation_key: key,
                            plan_id: r.plan_id,
                            channel_id: r.payment_channel_id,
                        },
                    )
                    .await?,
            )
        })
        .await
    }
    async fn confirm_stripe_purchase(
        &self,
        r: Request<pb::EPaySubscriptionOrderRequest>,
    ) -> std::result::Result<Response<pb::StripeSubscriptionOrder>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        let r = r.into_inner();
        self.run(async {
            super::stripe_rpc::order(
                self.store
                    .confirm_stripe_purchase(&user, &r.order_id)
                    .await?,
            )
        })
        .await
    }
    async fn read_stripe_purchase(
        &self,
        r: Request<pb::EPaySubscriptionOrderRequest>,
    ) -> std::result::Result<Response<pb::StripeSubscriptionOrder>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        let r = r.into_inner();
        self.run(async {
            super::stripe_rpc::order(self.store.read_stripe_purchase(&user, &r.order_id).await?)
        })
        .await
    }
    async fn accept_stripe_subscription_event(
        &self,
        r: Request<pb::AcceptEPaySubscriptionProofRequest>,
    ) -> std::result::Result<Response<pb::StripeSubscriptionEvent>, Status> {
        self.auth.check(
            r.metadata(),
            self.permissions.proof_ingress && self.permissions.grant,
        )?;
        let r = r.into_inner();
        self.run(async {
            Ok(super::stripe_rpc::event(
                self.store
                    .accept_stripe_event(
                        &self.payments,
                        &r.order_id,
                        &r.signed_payload,
                        &r.signature,
                    )
                    .await?,
            ))
        })
        .await
    }
    async fn get_stripe_subscription_event(
        &self,
        r: Request<pb::GetEPaySubscriptionProofRequest>,
    ) -> std::result::Result<Response<pb::StripeSubscriptionEvent>, Status> {
        self.auth.check(r.metadata(), self.permissions.proof_read)?;
        let r = r.into_inner();
        self.run(async {
            Ok(super::stripe_rpc::event(
                self.store
                    .stripe_event(&r.order_id, &r.provider_event_id, &r.payload_sha256)
                    .await?,
            ))
        })
        .await
    }
    async fn prepare_e_pay_purchase(
        &self,
        r: Request<pb::PrepareEPayPurchaseRequest>,
    ) -> std::result::Result<Response<pb::EPaySubscriptionOrder>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        let r = r.into_inner();
        self.run(async {
            let key = r.mutation.ok_or(Error::Invalid)?.idempotency_key;
            order_wire(
                self.store
                    .prepare_epay_purchase(
                        &self.payments,
                        &user,
                        &EPayPurchaseRequest {
                            operation_key: key,
                            plan_id: r.plan_id,
                            channel_id: r.payment_channel_id,
                        },
                    )
                    .await?,
            )
        })
        .await
    }
    async fn confirm_e_pay_purchase(
        &self,
        r: Request<pb::EPaySubscriptionOrderRequest>,
    ) -> std::result::Result<Response<pb::EPayCheckout>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        let r = r.into_inner();
        self.run(
            self.store
                .confirm_epay_purchase(&self.payments, &user, &r.order_id),
        )
        .await
    }
    async fn read_e_pay_purchase(
        &self,
        r: Request<pb::EPaySubscriptionOrderRequest>,
    ) -> std::result::Result<Response<pb::EPaySubscriptionOrder>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        let r = r.into_inner();
        self.run(async {
            let (o, checkout) = self
                .store
                .read_epay_purchase_with_checkout(&self.payments, &user, &r.order_id)
                .await?;
            let mut wire = order_wire(o)?;
            wire.checkout = checkout;
            Ok(wire)
        })
        .await
    }
    async fn cancel_e_pay_purchase(
        &self,
        r: Request<pb::EPaySubscriptionOrderRequest>,
    ) -> std::result::Result<Response<pb::EPaySubscriptionOrder>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.user_orders)?;
        let user = single(r.metadata(), "x-lmm-user-credential")?.to_owned();
        let r = r.into_inner();
        self.run(async { order_wire(self.store.cancel_epay_purchase(&user, &r.order_id).await?) })
            .await
    }
    async fn accept_e_pay_subscription_proof(
        &self,
        r: Request<pb::AcceptEPaySubscriptionProofRequest>,
    ) -> std::result::Result<Response<pb::EPaySubscriptionProof>, Status> {
        self.auth
            .check(r.metadata(), self.permissions.proof_ingress)?;
        let r = r.into_inner();
        self.run(async {
            Ok(proof_wire(
                self.store
                    .accept_epay_proof(&self.payments, &r.order_id, &r.signed_payload, &r.signature)
                    .await?,
            ))
        })
        .await
    }
    async fn get_e_pay_subscription_proof(
        &self,
        r: Request<pb::GetEPaySubscriptionProofRequest>,
    ) -> std::result::Result<Response<pb::EPaySubscriptionProof>, Status> {
        self.auth.check(r.metadata(), self.permissions.proof_read)?;
        let r = r.into_inner();
        self.run(async {
            Ok(proof_wire(
                self.store
                    .get_epay_proof(&r.order_id, &r.provider_event_id, &r.payload_sha256)
                    .await?,
            ))
        })
        .await
    }
    async fn apply_e_pay_subscription_grant(
        &self,
        r: Request<pb::ApplyEPaySubscriptionGrantRequest>,
    ) -> std::result::Result<Response<pb::EPaySubscriptionGrant>, Status> {
        self.auth.check(r.metadata(), self.permissions.grant)?;
        let r = r.into_inner();
        self.run(async {
            let g = self
                .store
                .apply_epay_grant(&r.order_id, &r.proof_id, &r.operation_key)
                .await?;
            Ok(pb::EPaySubscriptionGrant {
                order_id: g.order_id,
                proof_id: g.proof_id,
                operation_key: g.operation_key,
                subscription_id: g.subscription_id,
                granted_at_unix: g.granted_at,
                ends_at_unix: g.ends_at,
            })
        })
        .await
    }
}

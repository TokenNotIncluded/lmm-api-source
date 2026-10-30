//! Shared disposable PostgreSQL identity and wallet fixture for native APIs.
use crate::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        ledger::{Action, Ledger, Request as LedgerRequest},
    },
    funding::FundingPolicy,
    identity::IdentityStore,
};
use sqlx::PgPool;

pub(crate) struct Fixture {
    pub(crate) pool: PgPool,
    pub(crate) key: String,
    pub(crate) key_id: i64,
    pub(crate) session: String,
    pub(crate) account: i64,
    pub(crate) ledger: Ledger,
}
pub(crate) async fn fixture(pool: PgPool, funds: i64, models: &[&str]) -> Fixture {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    let actor = identity.authorize(&session.secret).await.unwrap();
    let key = identity
        .issue_key(
            &session.secret,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'synthetic-input-api-pg')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,model_limits_enabled,model_limits) VALUES($1,'input-api-key','allowed',TRUE,$2)")
        .bind(key.id).bind(models).execute(&pool).await.unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(actor.owner_account_id).await.unwrap();
    ledger
        .execute(&LedgerRequest {
            scope: "input-api-pg".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(7),
            reason: "Synthetic funds in disposable PostgreSQL".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(funds).unwrap(),
                payment_reference: "synthetic:input-api".into(),
            },
        })
        .await
        .unwrap();
    Fixture {
        pool,
        key: key.secret,
        key_id: key.id,
        session: session.secret,
        account: actor.owner_account_id,
        ledger,
    }
}

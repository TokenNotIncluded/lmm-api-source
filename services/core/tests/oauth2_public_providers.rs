//! Real disposable PG, native browser consent and Core TCP model regressions.
//! The synthetic loopback listener is not an external browser/TLS acceptance.
use axum::{body::Body, http::Request};
use lmm_core::{
    billing::charging::{Error, Outcome, Usage},
    identity::{
        IdentityError,
        grants::{LmmIssuer, LmmProfile, TokenForm, TokenWire},
    },
    oauth2_http,
};
use reqwest::{Response, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::BTreeSet, sync::atomic::Ordering};
use tower::ServiceExt;

#[allow(dead_code)]
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[expect(
    dead_code,
    reason = "This public-provider binary uses a subset of the shared OAuth fixture."
)]
#[path = "common/pi_oauth_fixture.rs"]
mod pi;
#[allow(dead_code)]
#[path = "common/runtime.rs"]
mod runtime;

const CLIENTS: [&str; 6] = [
    "lmm-pi",
    "lmm-dsh",
    "lmm-codewhale",
    "lmm-opencode",
    "lmm-vscode",
    "lmm-zed",
];
const SCOPES: [&str; 5] = [
    "catalog:read",
    "balance:read",
    "usage:read",
    "models:invoke",
    "group:YWxsb3dlZA",
];

fn profile() -> LmmProfile {
    LmmProfile::new("https://pi-issuer.example.test", vec!["allowed".into()]).unwrap()
}
fn ceiling(index: usize) -> i64 {
    400 + i64::try_from(index).unwrap() * 10
}
fn all_enabled() -> LmmProfile {
    CLIENTS
        .into_iter()
        .enumerate()
        .fold(profile(), |p, (index, client)| {
            p.with_public_model_budget_limit(client, ceiling(index))
                .unwrap()
        })
}
fn denied<T>(result: Result<T, Error>) {
    assert!(matches!(
        result,
        Err(Error::Unauthorized | Error::Forbidden)
    ));
}
fn assert_scopes(scope: &str) {
    assert_eq!(
        scope.split_ascii_whitespace().collect::<BTreeSet<_>>(),
        BTreeSet::from(SCOPES)
    );
}
async fn metadata(issuer: LmmIssuer) -> Value {
    let response = oauth2_http::router(issuer)
        .oneshot(
            Request::builder()
                .uri("/.well-known/oauth-authorization-server")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
async fn invalid_grant(response: Response) {
    let status = response.status();
    let value: Value = response.json().await.unwrap();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(value["error"], "invalid_grant");
}
async fn token_value(response: Response) -> Value {
    let status = response.status();
    let value: Value = response.json().await.unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["token_type"], "Bearer");
    assert!((1..=900).contains(&value["expires_in"].as_i64().unwrap()));
    assert_scopes(value["scope"].as_str().unwrap());
    assert!(
        value["access_token"]
            .as_str()
            .unwrap()
            .starts_with("lmm_at_")
    );
    assert!(
        value["refresh_token"]
            .as_str()
            .unwrap()
            .starts_with("lmm_rt_")
    );
    value
}
async fn refresh_http(f: &pi::Fixture, client: &str, refresh: &str) -> Response {
    f.client
        .post(format!("{}/api/oauth2/token", f.base))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client),
            ("resource", f.issuer.profile().resource()),
            ("refresh_token", refresh),
        ])
        .send()
        .await
        .unwrap()
}
async fn refresh_issuer(
    f: &pi::Fixture,
    client: &str,
    refresh: &str,
) -> Result<TokenWire, IdentityError> {
    let form = TokenForm {
        grant_type: "refresh_token".into(),
        client_id: client.into(),
        resource: f.issuer.profile().resource().into(),
        code: None,
        redirect_uri: None,
        code_verifier: None,
        refresh_token: Some(refresh.into()),
        scope: None,
    };
    f.issuer.token(form.validate(f.issuer.profile())?).await
}
async fn revoke_http(f: &pi::Fixture, client: &str, token: &str, hint: &str) -> Response {
    f.client
        .post(format!("{}/api/oauth2/revoke", f.base))
        .form(&[
            ("client_id", client),
            ("token", token),
            ("token_type_hint", hint),
        ])
        .send()
        .await
        .unwrap()
}

// These tests create exactly one valid family per client. Compare actual durable
// state around rejected requests, including the code and refresh consumption.
async fn family_state(f: &pi::Fixture, client: &str) -> (String, i64, bool, i64, i64, i64) {
    sqlx::query_as(
        "SELECT f.id,f.budget_credits,f.revoked_at IS NOT NULL,
         (SELECT count(*) FROM core_oauth.authorization_codes c
          WHERE c.family_id=f.id AND c.consumed_at IS NOT NULL),
         (SELECT count(*) FROM core_oauth.access_tokens a WHERE a.family_id=f.id),
         (SELECT count(*) FROM core_oauth.refresh_tokens r
          WHERE r.family_id=f.id AND r.consumed_at IS NOT NULL)
         FROM core_oauth.families f WHERE f.client_id=$1",
    )
    .bind(client)
    .fetch_one(&f.owner)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn public_provider_metadata_requires_each_explicit_finite_configuration(pool: PgPool) {
    pi::with_public_fixture(pool, true, profile(), |f| async move {
        let response = f
            .client
            .get(format!("{}/.well-known/oauth-authorization-server", f.base))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let default: Value = response.json().await.unwrap();
        assert_eq!(default["lmm_client_ids_supported"], json!(["lmm"]));
        assert_eq!(
            default["scopes_supported"],
            json!(["catalog:read", "balance:read"])
        );
        assert!(default.get("lmm_managed_clients").is_none());
        for client in CLIENTS {
            assert!(matches!(
                f.issuer.begin(f.request_for(client)).await,
                Err(IdentityError::Unauthorized)
            ));
        }
        for (index, client) in CLIENTS.into_iter().enumerate() {
            let configured = profile()
                .with_public_model_budget_limit(client, ceiling(index))
                .unwrap();
            let issuer = f
                .issuer_for_profile(configured, pi::ManagedKeyring::Missing)
                .await;
            for other in CLIENTS {
                assert_eq!(
                    issuer.profile().model_budget_limit(other),
                    (other == client).then_some(ceiling(index))
                );
                if other != client {
                    assert!(matches!(
                        issuer.begin(f.request_for(other)).await,
                        Err(IdentityError::Unauthorized)
                    ));
                }
            }
            let document = metadata(issuer).await;
            assert_eq!(document["lmm_client_ids_supported"], json!(["lmm", client]));
            assert_eq!(
                document["scopes_supported"],
                json!([
                    "catalog:read",
                    "balance:read",
                    "usage:read",
                    "models:invoke"
                ])
            );
            assert!(document.get("lmm_managed_clients").is_none());
        }
        let issuer = f
            .issuer_for_profile(all_enabled(), pi::ManagedKeyring::Missing)
            .await;
        let mut expected = vec!["lmm"];
        expected.extend(CLIENTS);
        assert_eq!(
            metadata(issuer).await["lmm_client_ids_supported"],
            json!(expected)
        );
        let compatibility = profile().with_pi_budget_limit(77).unwrap();
        assert_eq!(compatibility.pi_budget_limit(), Some(77));
        let issuer = f
            .issuer_for_profile(compatibility, pi::ManagedKeyring::Missing)
            .await;
        assert_eq!(
            metadata(issuer).await["lmm_client_ids_supported"],
            json!(["lmm", "lmm-pi"])
        );
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.ledger.balance(f.account).await.unwrap().available_units,
            1000
        );
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn public_provider_browser_code_refresh_and_revoke_keep_client_families_separate(
    pool: PgPool,
) {
    // Ordinary accounts with an allowed routing group may explicitly consent.
    // Consent neither activates developer access nor manufactures a native key.
    pi::with_public_fixture(pool, false, all_enabled(), |f| async move {
        let native_before: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_identity.credentials WHERE kind='api_key'",
        )
        .fetch_one(&f.owner)
        .await
        .unwrap();
        for (index, client) in CLIENTS.into_iter().enumerate() {
            for cap in [None, Some(0), Some(ceiling(index) + 1)] {
                let pending = f.issuer.begin(f.request_for(client)).await.unwrap();
                let page = f
                    .issuer
                    .continue_authorization(&pending.binding, &pending.csrf, Some(&f.refresh))
                    .await
                    .unwrap();
                assert_eq!(page.client_id, client);
                assert_eq!(page.maximum_budget_credits, Some(ceiling(index)));
                assert!(matches!(
                    f.issuer
                        .consent_with_budget(&page.binding, &page.csrf, Some(&f.refresh), true, cap)
                        .await,
                    Err(IdentityError::Invalid)
                ));
            }
            let unconsented: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM core_oauth.families WHERE client_id=$1",
            )
            .bind(client)
            .fetch_one(&f.owner)
            .await
            .unwrap();
            assert_eq!(unconsented, 0, "invalid budgets must not create a family");
            let code = f.code_for(client, ceiling(index)).await;
            let before = family_state(&f, client).await;
            assert_eq!(
                (before.1, before.2, before.3, before.4, before.5),
                (ceiling(index), false, 0, 0, 0)
            );
            // A cyclic foreign client covers every client as both owner and
            // attacker while retaining the real per-peer request limit.
            let foreign = CLIENTS[(index + 1) % CLIENTS.len()];
            invalid_grant(f.exchange_code_for(foreign, &code).await).await;
            assert_eq!(family_state(&f, client).await, before);
            let first = token_value(f.exchange_code_for(client, &code).await).await;
            let issued = family_state(&f, client).await;
            assert_eq!((issued.3, issued.4, issued.5), (1, 1, 0));

            let old_refresh = first["refresh_token"].as_str().unwrap();
            invalid_grant(refresh_http(&f, foreign, old_refresh).await).await;
            assert_eq!(family_state(&f, client).await, issued);
            let rotated = token_value(refresh_http(&f, client, old_refresh).await).await;
            assert!(rotated["refresh_token"] != first["refresh_token"]);
            let active = family_state(&f, client).await;
            assert_eq!(active.0, before.0);
            assert_eq!(
                (active.1, active.2, active.3, active.4, active.5),
                (ceiling(index), false, 1, 2, 1)
            );

            // A foreign client replaying an already consumed refresh must not
            // reach the legitimate family's replay-revocation mutation.
            assert!(matches!(
                refresh_issuer(&f, foreign, old_refresh).await,
                Err(IdentityError::Unauthorized)
            ));
            assert_eq!(family_state(&f, client).await, active);
            let (revoke_token, hint) = if index % 2 == 0 {
                (rotated["access_token"].as_str().unwrap(), "access_token")
            } else {
                (rotated["refresh_token"].as_str().unwrap(), "refresh_token")
            };
            invalid_grant(revoke_http(&f, foreign, revoke_token, hint).await).await;
            assert_eq!(family_state(&f, client).await, active);
            let ordinary = f
                .reserve(
                    rotated["access_token"].as_str().unwrap(),
                    &f.spec(&format!("{client}-ordinary-reserve"), 1),
                    &[1; 32],
                )
                .await
                .unwrap();
            assert!(ordinary.key_id.is_none());
            assert_eq!(ordinary.oauth_source.as_ref().unwrap().client_id(), client);
            assert_eq!(ordinary.payer_account_id, f.account);
            f.charging.release(&ordinary.id, &[1; 32]).await.unwrap();
            let response = revoke_http(&f, client, revoke_token, hint).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert!(response.bytes().await.unwrap().is_empty());
            let revoked = family_state(&f, client).await;
            assert_eq!(revoked.0, before.0);
            assert!(revoked.2);
            assert!(matches!(
                refresh_issuer(&f, client, rotated["refresh_token"].as_str().unwrap()).await,
                Err(IdentityError::Unauthorized)
            ));
        }
        let families: i64 = sqlx::query_scalar("SELECT count(*) FROM core_oauth.families")
            .fetch_one(&f.owner)
            .await
            .unwrap();
        assert_eq!(families, 6);
        let native_after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_identity.credentials WHERE kind='api_key'",
        )
        .fetch_one(&f.owner)
        .await
        .unwrap();
        assert_eq!(native_after, native_before);
        let developer: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core_identity.developer_access WHERE user_id=$1 AND granted)",
        )
        .bind(f.user)
        .fetch_one(&f.owner)
        .await
        .unwrap();
        assert!(!developer);
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
        assert_eq!(f.ledger.balance(f.account).await.unwrap().available_units, 1000);
        assert_eq!(f.ledger.balance(f.account).await.unwrap().reserved_units, 0);
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn public_provider_real_model_usage_and_finite_budgets_do_not_cross_families(pool: PgPool) {
    pi::with_public_fixture(pool, true, all_enabled(), |f| async move {
        let mut tokens = Vec::new();
        for (index, client) in CLIENTS.into_iter().enumerate() {
            let token = f.token_for(client, ceiling(index)).await;
            assert_scopes(&token.scope);
            let response = f
                .client
                .post(format!("{}/v1/chat/completions", f.base))
                .bearer_auth(&token.access_token)
                .header("x-lmm-group", "YWxsb3dlZA")
                .header("idempotency-key", format!("{client}-real-model"))
                .json(&json!({"model":"provider-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20}))
                .send()
                .await
                .unwrap();
            let status = response.status();
            let text = response.text().await.unwrap();
            assert_eq!(status, StatusCode::OK);
            assert!(text.contains("local synthetic answer"));
            tokens.push(token);
        }
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 6);
        let rows: Vec<(String, String, i64, Option<i64>, i64)> = sqlx::query_as(
            "SELECT oauth_client_id,oauth_family_id,settled,key_id,payer_account_id
             FROM core_charging.requests WHERE state='settled' ORDER BY oauth_client_id",
        )
        .fetch_all(&f.owner)
        .await
        .unwrap();
        assert_eq!(rows.len(), 6);
        assert_eq!(
            rows.iter().map(|r| r.0.as_str()).collect::<BTreeSet<_>>(),
            BTreeSet::from(CLIENTS)
        );
        assert_eq!(
            rows.iter().map(|r| r.1.as_str()).collect::<BTreeSet<_>>().len(),
            6
        );
        for (_, _, settled, key, payer) in rows {
            assert_eq!(settled, 49);
            assert!(key.is_none());
            assert_eq!(payer, f.account);
        }
        for (index, token) in tokens.iter().enumerate() {
            let usage = f.issuer.usage(&token.access_token).await.unwrap();
            assert_eq!(usage["usage_scope"], "oauth_family");
            assert_eq!(usage["items"].as_array().unwrap().len(), 1);
            assert_eq!(usage["summary"]["net_consumption_credits"], 49);
            assert!(matches!(
                f.reserve(
                    &token.access_token,
                    &f.spec(&format!("public-over-cap-{index}"), ceiling(index) - 48),
                    &[2; 32],
                )
                .await,
                Err(Error::BudgetExceeded)
            ));
            let exact = f
                .reserve(
                    &token.access_token,
                    &f.spec(&format!("public-exact-cap-{index}"), ceiling(index) - 49),
                    &[3; 32],
                )
                .await
                .unwrap();
            let source = exact.oauth_source.as_ref().unwrap();
            assert_eq!(source.client_id(), CLIENTS[index]);
            f.charging.release(&exact.id, &[3; 32]).await.unwrap();
        }
        for token in &tokens {
            let usage = f.issuer.usage(&token.access_token).await.unwrap();
            assert_eq!(
                usage["summary"]["net_consumption_credits"],
                49
            );
        }
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 6);
        assert_eq!(f.ledger.balance(f.account).await.unwrap().available_units, 706);
        assert_eq!(f.ledger.balance(f.account).await.unwrap().reserved_units, 0);
    })
    .await;
}

#[derive(Clone, Copy)]
enum Withdrawal {
    Developer,
    Group,
    Configuration,
}

#[sqlx::test(migrations = false)]
async fn public_provider_developer_epoch_revocation_preserves_started_settlement(pool: PgPool) {
    withdrawal_preserves_settlement(pool, Withdrawal::Developer).await;
}
#[sqlx::test(migrations = false)]
async fn public_provider_group_withdrawal_preserves_started_settlement(pool: PgPool) {
    withdrawal_preserves_settlement(pool, Withdrawal::Group).await;
}
#[sqlx::test(migrations = false)]
async fn public_provider_disabled_configuration_preserves_started_settlement(pool: PgPool) {
    withdrawal_preserves_settlement(pool, Withdrawal::Configuration).await;
}

async fn withdrawal_preserves_settlement(pool: PgPool, withdrawal: Withdrawal) {
    pi::with_public_fixture(pool, true, all_enabled(), move |f| async move {
        let mut admitted = Vec::new();
        for client in CLIENTS {
            let token = f.token_for(client, 200).await;
            let started = f
                .reserve(&token.access_token, &f.spec(&format!("{client}-started"), 60), &[4; 32])
                .await
                .unwrap();
            let pending = f
                .reserve(&token.access_token, &f.spec(&format!("{client}-pending"), 30), &[5; 32])
                .await
                .unwrap();
            assert!(f.charging.start_oauth(&f.issuer, &started.id, &[4; 32]).await.unwrap());
            assert_eq!(started.oauth_source.as_ref().unwrap().client_id(), client);
            assert_eq!(started.oauth_source, pending.oauth_source);
            admitted.push((client, token, started, pending));
        }
        assert_eq!(f.ledger.balance(f.account).await.unwrap().reserved_units, 540);
        let checking_issuer = match withdrawal {
            Withdrawal::Developer => {
                // The existing policy trigger advances auth_version. This tests
                // old-grant revocation, not a developer-only consumption tier.
                assert_eq!(sqlx::query("UPDATE core_identity.developer_access SET granted=false WHERE user_id=$1")
                    .bind(f.user).execute(&f.owner).await.unwrap().rows_affected(), 1);
                f.issuer.clone()
            }
            Withdrawal::Group => {
                assert_eq!(sqlx::query("UPDATE core_identity.user_routing_groups SET enabled=false WHERE user_id=$1 AND group_name='allowed'")
                    .bind(f.user).execute(&f.owner).await.unwrap().rows_affected(), 1);
                f.issuer.clone()
            }
            Withdrawal::Configuration => {
                // A newly configured issuer reads the same durable source. This
                // is not a claim that the running HTTP server hot-reloads config.
                f.issuer_for_profile(profile(), pi::ManagedKeyring::Missing).await
            }
        };
        for (client, token, started, pending) in admitted {
            denied(f.charging.reserve_oauth_with_context(
                &checking_issuer,
                &token.access_token,
                &f.spec(&format!("{client}-new-denied"), 1),
                &[6; 32],
                None,
            ).await);
            denied(f.charging.start_oauth(&checking_issuer, &pending.id, &[5; 32]).await);
            denied(f.charging.increase_oauth_with_context(
                &checking_issuer,
                &token.access_token,
                &started.id,
                &[4; 32],
                70,
                None,
            ).await);
            let settled = f.charging.settle(
                &started.id,
                &[4; 32],
                Usage { input: 40, ..Default::default() },
                Outcome::Completed,
            ).await.unwrap();
            assert_eq!(settled.settled, 40);
            assert_eq!(settled.oauth_source, started.oauth_source);
            let refund_key = format!("{client}-original-refund");
            f.charging.refund(&started.id, &refund_key, 5, "synthetic original provider refund")
                .await.unwrap();
            f.charging.refund(&started.id, &refund_key, 5, "synthetic original provider refund")
                .await.unwrap();
            f.charging.release(&pending.id, &[5; 32]).await.unwrap();
            let preserved = f.charging.resolve(&started.id).await.unwrap().unwrap();
            assert_eq!(preserved.oauth_source, started.oauth_source);
            assert_eq!(preserved.settled, 40);
            assert_eq!(preserved.refunded, 5);
            assert_eq!(preserved.payer_account_id, f.account);
            assert!(preserved.key_id.is_none());
            if matches!(withdrawal, Withdrawal::Configuration) {
                // Disabling issuance must not make the original grant
                // impossible to revoke using its own registered client.
                checking_issuer.revoke_for_client(client, &token.refresh_token).await.unwrap();
                assert!(family_state(&f, client).await.2);
            }
        }
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
        assert_eq!(f.ledger.balance(f.account).await.unwrap().available_units, 790);
        assert_eq!(f.ledger.balance(f.account).await.unwrap().reserved_units, 0);
    })
    .await;
}

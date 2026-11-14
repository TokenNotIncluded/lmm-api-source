//! Actual Core/PG budget and installation isolation; not public-TLS acceptance.
use lmm_core::billing::charging::{Error, Outcome, Usage};
use serde_json::json;
use sqlx::PgPool;
use std::sync::atomic::Ordering;

// Shared real-PG/provider fixture also serves protocol/process test binaries.
#[allow(dead_code)]
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[expect(
    dead_code,
    reason = "This test binary uses the financial subset of the shared delegated OAuth fixture."
)]
#[path = "common/pi_oauth_fixture.rs"]
mod pi;
#[allow(dead_code)]
#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test(migrations = false)]
async fn managed_http_model_dispatch_keeps_its_client_and_personal_payer(pool: PgPool) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let token = f.managed_token(900, &"e".repeat(64)).await;
        let response = f
            .client
            .post(format!("{}/v1/chat/completions", f.base))
            .bearer_auth(&token.access_token)
            .header("x-lmm-group", "YWxsb3dlZA")
            .json(&json!({
                "model": "provider-model",
                "messages": [{"role": "user", "content": "hello"}],
                "max_tokens": 20
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "local synthetic answer");
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);
        let requests: (i64, i64) = sqlx::query_as(
            "SELECT count(*),count(*) FILTER(WHERE oauth_source->>'client_id'='lmm-cc-switch' AND oauth_source->>'grant_id'=$1 AND payer_account_id=$2 AND actor_user_id=$3 AND key_id IS NULL) FROM core_charging.requests",
        )
        .bind(&token.grant_id)
        .bind(f.account)
        .bind(f.user)
        .fetch_one(&f.owner)
        .await
        .unwrap();
        assert_eq!(requests, (1, 1));
        let balance = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(balance.available_units, 951);
        assert_eq!(balance.reserved_units, 0);
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn managed_generations_share_pending_spent_and_refunded_family_budget(pool: PgPool) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let installation = "a".repeat(64);
        let rotation = "b".repeat(64);
        let first = f.managed_token(70, &installation).await;
        let next = f
            .managed_refresh(&first.refresh_token, &installation, &rotation)
            .await
            .unwrap();
        let replay = f
            .managed_refresh(&first.refresh_token, &installation, &rotation)
            .await
            .unwrap();
        assert_eq!(next.grant_id, first.grant_id);
        assert_eq!(replay.access_token, next.access_token);
        assert_eq!(replay.refresh_token, next.refresh_token);
        assert_eq!(replay.budget_credits, 70);
        assert!(replay.expires_in > 0 && replay.expires_in <= next.expires_in);

        let a = f.spec("managed-cap-a", 50);
        let b = f.spec("managed-cap-b", 50);
        let (a, b) = tokio::join!(
            f.reserve(&first.access_token, &a, &[1; 32]),
            f.reserve(&next.access_token, &b, &[2; 32])
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        let (charge, worker) = if let Ok(a) = a {
            assert!(matches!(b, Err(Error::BudgetExceeded)));
            (a, [1; 32])
        } else {
            assert!(matches!(a, Err(Error::BudgetExceeded)));
            (b.unwrap(), [2; 32])
        };
        let source = charge.oauth_source.as_ref().unwrap();
        assert_eq!(source.client_id(), "lmm-cc-switch");
        assert_eq!(source.grant_id(), first.grant_id);
        assert_eq!(source.personal_account_id(), f.account);
        assert_eq!(charge.actor_user_id, f.user);
        assert_eq!(charge.payer_account_id, f.account);
        assert!(charge.key_id.is_none());
        let stored_client: String = sqlx::query_scalar(
            "SELECT oauth_source->>'client_id' FROM core_charging.requests WHERE id=$1",
        )
        .bind(&charge.id)
        .fetch_one(&f.owner)
        .await
        .unwrap();
        assert_eq!(stored_client, "lmm-cc-switch");
        assert_eq!(
            f.ledger.balance(f.account).await.unwrap().reserved_units,
            50
        );
        assert!(
            f.charging
                .start_oauth(&f.issuer, &charge.id, &worker)
                .await
                .unwrap()
        );
        f.charging
            .settle(
                &charge.id,
                &worker,
                Usage {
                    input: 40,
                    ..Default::default()
                },
                Outcome::Completed,
            )
            .await
            .unwrap();
        assert!(matches!(
            f.reserve(&next.access_token, &f.spec("managed-over", 31), &[3; 32])
                .await,
            Err(Error::BudgetExceeded)
        ));
        f.charging
            .refund(
                &charge.id,
                "managed-refund",
                10,
                "synthetic verified provider refund",
            )
            .await
            .unwrap();
        let exact = f
            .reserve(
                &next.access_token,
                &f.spec("managed-after-refund", 40),
                &[4; 32],
            )
            .await
            .unwrap();
        f.charging.release(&exact.id, &[4; 32]).await.unwrap();
        let balance = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(balance.available_units, 970);
        assert_eq!(balance.reserved_units, 0);
        let usage = f.issuer.usage(&next.access_token).await.unwrap();
        assert_eq!(usage["usage_scope"], "oauth_family");
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn wrong_installation_cannot_consume_or_revoke_and_client_budgets_stay_separate(
    pool: PgPool,
) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let installation_a = "a".repeat(64);
        let installation_b = "c".repeat(64);
        let a = f.managed_token(60, &installation_a).await;
        let b = f.managed_token(60, &installation_b).await;
        let pi = f.token(100).await;
        assert_ne!(a.grant_id, b.grant_id);
        assert!(f.managed_refresh(&a.refresh_token, &installation_b, &"d".repeat(64)).await.is_err());
        let state: (bool, i64) = sqlx::query_as(
            "SELECT f.revoked_at IS NULL,(SELECT count(*) FROM core_oauth.refresh_tokens r WHERE r.family_id=f.id AND r.consumed_at IS NOT NULL) FROM core_oauth.families f WHERE f.id=$1",
        )
        .bind(&a.grant_id)
        .fetch_one(&f.owner)
        .await
        .unwrap();
        assert_eq!(state, (true, 0));
        let (a_charge, b_charge, pi_charge) = (
            f.reserve(&a.access_token, &f.spec("managed-install-a", 50), &[1; 32]).await.unwrap(),
            f.reserve(&b.access_token, &f.spec("managed-install-b", 50), &[2; 32]).await.unwrap(),
            f.reserve(&pi.access_token, &f.spec("pi-separate", 50), &[3; 32]).await.unwrap(),
        );
        for (charge, expected_client) in [
            (&a_charge, "lmm-cc-switch"),
            (&b_charge, "lmm-cc-switch"),
            (&pi_charge, "lmm-pi"),
        ] {
            assert_eq!(charge.oauth_source.as_ref().unwrap().client_id(), expected_client);
            assert_eq!(charge.payer_account_id, f.account);
        }
        assert_eq!(f.ledger.balance(f.account).await.unwrap().reserved_units, 150);
        for (token, amount, id) in [
            (&a.access_token, 11, "managed-a-over"),
            (&b.access_token, 11, "managed-b-over"),
            (&pi.access_token, 51, "pi-over"),
        ] {
            assert!(matches!(
                f.reserve(token, &f.spec(id, amount), &[4; 32]).await,
                Err(Error::BudgetExceeded)
            ));
        }
        for (charge, worker) in [(&a_charge, [1; 32]), (&b_charge, [2; 32]), (&pi_charge, [3; 32])] {
            f.charging.release(&charge.id, &worker).await.unwrap();
        }
        let balance = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(balance.available_units, 1000);
        assert_eq!(balance.reserved_units, 0);
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
    })
    .await;
}

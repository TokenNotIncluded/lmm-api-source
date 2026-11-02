//! Author PG/TCP cases, UNRUN until host compiles and independent tester runs.
use lmm_core::{
    billing::charging::{Budget, Error, Outcome, Usage},
    identity::{IdentityError, grants::ReadError},
};
use serde_json::json;
use sqlx::PgPool;
use std::sync::atomic::Ordering;
// This shared fixture also serves protocol/process tests with additional fields.
#[allow(dead_code)]
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[path = "common/pi_oauth_fixture.rs"]
mod pi;
// This shared fixture also serves protocol/process tests with additional fields.
#[allow(dead_code)]
#[path = "common/runtime.rs"]
mod runtime;
use pi::with_fixture;

#[sqlx::test(migrations = false)]
async fn real_pi_browser_consent_is_bounded_without_issuing_native_keys(pool: PgPool) {
    with_fixture(pool, true, |f| async move {
        let before: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_identity.credentials WHERE kind='api_key'",
        )
        .fetch_one(&f.owner)
        .await
        .unwrap();
        for cap in [None, Some(0), Some(1001)] {
            let first = f.issuer.begin(f.request()).await.unwrap();
            let page = f
                .issuer
                .continue_authorization(&first.binding, &first.csrf, Some(&f.refresh))
                .await
                .unwrap();
            assert!(matches!(
                f.issuer
                    .consent_with_budget(&page.binding, &page.csrf, Some(&f.refresh), true, cap)
                    .await,
                Err(IdentityError::Invalid)
            ));
        }
        let token = f.token(100).await;
        for sql in [
            "UPDATE core_oauth.access_tokens SET expires_at=expires_at WHERE false",
            "DELETE FROM core_oauth.access_tokens WHERE false",
        ] {
            let error = sqlx::query(sql).execute(&f.role.pool).await.unwrap_err();
            assert_eq!(
                error.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("42501")
            );
        }

        for required in [
            "catalog:read",
            "balance:read",
            "usage:read",
            "models:invoke",
            "group:YWxsb3dlZA",
        ] {
            assert!(token.scope.split(' ').any(|v| v == required));
        }
        assert_eq!(token.scope.split(' ').count(), 5);
        assert!(!token.scope.contains("mcp:"));
        assert!(!token.scope.contains("remote:"));
        let after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_identity.credentials WHERE kind='api_key'",
        )
        .fetch_one(&f.owner)
        .await
        .unwrap();
        assert_eq!(after, before);
        assert_eq!(
            f.issuer
                .catalog(&token.access_token)
                .await
                .unwrap()
                .models
                .len(),
            1
        );
        assert!(matches!(
            f.refresh(
                &token.refresh_token,
                Some(format!("{} remote:control", token.scope))
            )
            .await,
            Err(IdentityError::Invalid) | Err(IdentityError::Forbidden)
        ));
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
    })
    .await;
}
#[sqlx::test(migrations = false)]
async fn l0_unmarked_cannot_self_grant_pi_developer_or_spending(pool: PgPool) {
    with_fixture(pool,false,|f|async move{
        let first=f.issuer.begin(f.request()).await.unwrap();
        assert!(matches!(f.issuer.continue_authorization(&first.binding,&first.csrf,Some(&f.refresh)).await,Err(IdentityError::Forbidden)));
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM core_oauth.families WHERE client_id='lmm-pi'").fetch_one(&f.owner).await.unwrap();assert_eq!(count,0);
        let granted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_identity.developer_access WHERE user_id=$1 AND granted)").bind(f.user).fetch_one(&f.owner).await.unwrap();assert!(!granted);
        sqlx::query("UPDATE core_identity.users SET platform_role='admin' WHERE id=$1").bind(f.user).execute(&f.owner).await.unwrap();
        let browser=lmm_core::identity::BrowserAuth::new(lmm_core::identity::IdentityStore::from_pool(f.role.pool.clone()),&[43;32]).unwrap();
        let admin=browser.login("pi-alice","test passphrase",&Default::default()).await.unwrap();let first=f.issuer.begin(f.request()).await.unwrap();
        assert!(matches!(f.issuer.continue_authorization(&first.binding,&first.csrf,Some(&admin.refresh_token)).await,Err(IdentityError::Forbidden)));
        assert_eq!(f.provider.hits.load(Ordering::SeqCst),0);assert_eq!(f.ledger.balance(f.account).await.unwrap().available_units,1000);
    }).await;
}
#[sqlx::test(migrations = false)]
async fn family_cap_serializes_pending_and_committed_spending_across_refresh(pool: PgPool) {
    with_fixture(pool, true, |f| async move {
        let first = f.token(70).await;
        let next = f.refresh(&first.refresh_token, None).await.unwrap();
        let a = f.spec("pi-cap-a", 50);
        let b = f.spec("pi-cap-b", 50);
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
        assert!(charge.key_id.is_none());
        assert!(charge.oauth_source.is_some());
        assert_eq!(charge.payer_account_id, f.account);
        assert_eq!(charge.actor_user_id, f.user);
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
            f.reserve(&next.access_token, &f.spec("pi-over-cap", 31), &[3; 32])
                .await,
            Err(Error::BudgetExceeded)
        ));
        f.charging
            .refund(
                &charge.id,
                "pi-refund",
                10,
                "synthetic verified provider refund",
            )
            .await
            .unwrap();
        let exact = f
            .reserve(&next.access_token, &f.spec("pi-after-refund", 40), &[4; 32])
            .await
            .unwrap();
        f.charging.release(&exact.id, &[4; 32]).await.unwrap();
        assert_eq!(
            f.ledger.balance(f.account).await.unwrap().available_units,
            970
        );
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
        let usage = f.issuer.usage(&next.access_token).await.unwrap();
        assert_eq!(usage["usage_scope"], "oauth_family");
        assert_eq!(usage["summary"]["net_consumption_credits"], 30);
    })
    .await;
}
#[sqlx::test(migrations = false)]
async fn oauth_usage_streams_over_1000_without_cross_family_leak(pool: PgPool) {
    with_fixture(pool, true, |f| async move {
        let token = f.token(100).await;
        let seed = f.reserve(&token.access_token, &f.spec("pi-usage-seed", 1), &[11; 32]).await.unwrap();
        let other = f.token(100).await;
        let other_seed = f.reserve(&other.access_token, &f.spec("pi-usage-other", 1), &[12; 32]).await.unwrap();
        let mut tx = f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
        // Synthetic reservation rows exercise the read model only; they are not
        // 1000 real reserves or evidence of any financial transactions.
        let clone_sql = r#"INSERT INTO core_charging.requests(id,actor_user_id,key_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved,oauth_source)
            SELECT 'pi-many-'||lpad(g::text,6,'0'),actor_user_id,key_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,'reserved',reserved,oauth_source
            FROM core_charging.requests CROSS JOIN generate_series($2::bigint,$3::bigint) g WHERE id=$1"#;
        assert_eq!(sqlx::query(clone_sql).bind(&seed.id).bind(1_i64).bind(999_i64).execute(&mut *tx).await.unwrap().rows_affected(), 999);
        tx.commit().await.unwrap();
        let at_limit = f.issuer.usage(&token.access_token).await.unwrap();
        assert_eq!(at_limit["items"].as_array().unwrap().len(), 1000);
        let mut tx = f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
        assert_eq!(sqlx::query(clone_sql).bind(&seed.id).bind(1000_i64).bind(1000_i64).execute(&mut *tx).await.unwrap().rows_affected(), 1);
        tx.commit().await.unwrap();
        let usage = f.issuer.usage(&token.access_token).await.unwrap();
        assert_eq!(usage["usage_scope"], "oauth_family");
        assert_eq!(usage["items"].as_array().unwrap().len(), 1001);
        assert_eq!(usage["summary"]["incomplete_count"], 1001);
        assert!(!usage["items"].as_array().unwrap().iter().any(|item| item["request_id"] == other_seed.id));
        let other_usage = f.issuer.usage(&other.access_token).await.unwrap();
        assert_eq!(other_usage["items"].as_array().unwrap().len(), 1);
        let mut tx = f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
        let inserted = sqlx::query("INSERT INTO core_charging.ledger_commands(operation_id,request_id,command) VALUES(repeat('f',64),'pi-many-000001','{}'::jsonb)").execute(&mut *tx).await.unwrap();
        assert_eq!(inserted.rows_affected(), 1);
        tx.commit().await.unwrap();
        assert!(f.issuer.usage(&token.access_token).await.is_err());
        assert!(f.issuer.usage(&other.access_token).await.is_ok());
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
    }).await;
}

#[sqlx::test(migrations = false)]
async fn oauth_usage_validates_receipts_beyond_2048(pool: PgPool) {
    with_fixture(pool, true, |f| async move {
        let token = f.token(100).await;
        let seed = f.reserve(&token.access_token, &f.spec("pi-many-receipts", 1), &[15; 32]).await.unwrap();
        // Owner-only synthetic audit receipts; no additional money is posted.
        let mut tx = f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
        let inserted = sqlx::query(r#"INSERT INTO core_charging.ledger_commands(operation_id,request_id,command)
            SELECT op,$1,jsonb_set(source.command,'{operation_id}',to_jsonb(op))
            FROM (SELECT command FROM core_charging.ledger_commands WHERE request_id=$1 LIMIT 1) source
            CROSS JOIN (SELECT md5('oauth-extra-'||g::text)||md5('oauth-extra-2-'||g::text) AS op FROM generate_series(1,2049) g) extra"#)
            .bind(&seed.id).execute(&mut *tx).await.unwrap().rows_affected();
        assert_eq!(inserted, 2049);
        tx.commit().await.unwrap();
        let usage = f.issuer.usage(&token.access_token).await.unwrap();
        assert_eq!(usage["items"].as_array().unwrap().len(), 1);
        let mut tx = f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
        let updated = sqlx::query("UPDATE core_charging.ledger_commands SET command=jsonb_set(command,'{actor_user_id}','999999') WHERE request_id=$1 AND operation_id=(SELECT operation_id FROM core_charging.ledger_commands WHERE request_id=$1 ORDER BY operation_id COLLATE \"C\" DESC LIMIT 1)")
            .bind(&seed.id).execute(&mut *tx).await.unwrap().rows_affected();
        assert_eq!(updated, 1);
        tx.commit().await.unwrap();
        assert!(f.issuer.usage(&token.access_token).await.is_err());
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
    }).await;
}

#[sqlx::test(migrations = false)]
async fn oauth_usage_batches_share_authority_snapshot(pool: PgPool) {
    with_fixture(pool, true, |f| async move {
        let token = f.token(100).await;
        let seed = f.reserve(&token.access_token, &f.spec("pi-snapshot-seed", 1), &[14; 32]).await.unwrap();
        let mut initial = f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *initial).await.unwrap();
        sqlx::query(r#"INSERT INTO core_charging.requests(id,actor_user_id,key_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved,oauth_source)
            SELECT 'pi-snapshot-initial-'||g::text,actor_user_id,key_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,'reserved',reserved,oauth_source
            FROM core_charging.requests CROSS JOIN generate_series(1,16) g WHERE id=$1"#).bind(&seed.id).execute(&mut *initial).await.unwrap();
        initial.commit().await.unwrap();
        let mut lock = f.owner.begin().await.unwrap();
        sqlx::query("LOCK TABLE core_charging.ledger_commands IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *lock).await.unwrap();
        let issuer = f.issuer.clone();
        let access = token.access_token.clone();
        let read = tokio::spawn(async move { issuer.usage(&access).await });
        // Wait for the actual family query to be blocked after authentication
        // established the snapshot, rather than guessing based on a sleep.
        tokio::time::timeout(std::time::Duration::from_millis(700), async {
            loop {
                let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%FROM core_charging.ledger_commands WHERE request_id=ANY%')")
                    .fetch_one(&f.owner).await.unwrap();
                if waiting { break; }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }).await.expect("OAuth read did not reach the locked receipt query");
        let mut writer = f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *writer).await.unwrap();
        sqlx::query(r#"INSERT INTO core_charging.requests(id,actor_user_id,key_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved,oauth_source)
            SELECT 'pi-snapshot-later',actor_user_id,key_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,'reserved',reserved,oauth_source
            FROM core_charging.requests WHERE id=$1"#).bind(&seed.id).execute(&mut *writer).await.unwrap();
        writer.commit().await.unwrap();
        lock.commit().await.unwrap();
        let first = read.await.unwrap().unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 17);
        let next = f.issuer.usage(&token.access_token).await.unwrap();
        assert_eq!(next["items"].as_array().unwrap().len(), 18);
    }).await;
}

#[sqlx::test(migrations = false)]
async fn oauth_usage_overdense_read_fails_without_partial_data(pool: PgPool) {
    with_fixture(pool, true, |f| async move {
        let token = f.token(100).await;
        let seed = f.reserve(&token.access_token, &f.spec("pi-overflow-seed", 1), &[13; 32]).await.unwrap();
        let initial = f.issuer.usage(&token.access_token).await.unwrap();
        let item = &initial["items"].as_array().unwrap()[0];
        // IDs in the bulk data are no more than ten bytes shorter than this seed.
        // The normal 1500ms SQL statement timeout may reject this dense query
        // before it reaches the byte guard; either way no partial result escapes.
        let minimum = serde_json::to_vec(item).unwrap().len().checked_sub(10).unwrap();
        let clones = (16 * 1024 * 1024 / minimum + 2) as i64;
        assert!(clones < 30000);
        let mut tx = f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
        let inserted = sqlx::query(r#"INSERT INTO core_charging.requests(id,actor_user_id,key_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved,oauth_source)
            SELECT 'pi-over-'||lpad(g::text,6,'0'),actor_user_id,key_id,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,'reserved',reserved,oauth_source
            FROM core_charging.requests CROSS JOIN generate_series(1,$2::bigint) g WHERE id=$1"#)
            .bind(&seed.id).bind(clones).execute(&mut *tx).await.unwrap().rows_affected();
        assert_eq!(inserted, clones as u64);
        tx.commit().await.unwrap();
        assert!(matches!(f.issuer.usage(&token.access_token).await, Err(ReadError::Unavailable)));
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60)).build().unwrap()
            .get(format!("{}/api/oauth2/usage", f.base))
            .bearer_auth(&token.access_token).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body, json!({"error":"temporarily_unavailable"}));
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
    }).await;
}

#[sqlx::test(migrations = false)]
async fn revoke_blocks_new_start_increase_but_preserves_old_hold_settle_refund(pool: PgPool) {
    with_fixture(pool, true, |f| async move {
        let token = f.token(200).await;
        let started = f
            .reserve(&token.access_token, &f.spec("pi-started", 60), &[1; 32])
            .await
            .unwrap();
        let pending = f
            .reserve(&token.access_token, &f.spec("pi-pending", 50), &[2; 32])
            .await
            .unwrap();
        assert!(
            f.charging
                .start_oauth(&f.issuer, &started.id, &[1; 32])
                .await
                .unwrap()
        );
        f.issuer
            .revoke_for_client("lmm-pi", &token.access_token)
            .await
            .unwrap();
        assert!(matches!(
            f.charging
                .start_oauth(&f.issuer, &pending.id, &[2; 32])
                .await,
            Err(Error::Unauthorized) | Err(Error::Forbidden)
        ));
        assert!(matches!(
            f.charging
                .increase_oauth_with_context(
                    &f.issuer,
                    &token.access_token,
                    &started.id,
                    &[1; 32],
                    70,
                    None
                )
                .await,
            Err(Error::Unauthorized) | Err(Error::Forbidden)
        ));
        let settled = f
            .charging
            .settle(
                &started.id,
                &[1; 32],
                Usage {
                    input: 40,
                    ..Default::default()
                },
                Outcome::Completed,
            )
            .await
            .unwrap();
        assert_eq!(settled.settled, 40);
        f.charging
            .refund(
                &started.id,
                "pi-old-refund",
                5,
                "synthetic original provider refund",
            )
            .await
            .unwrap();
        f.charging
            .refund(
                &started.id,
                "pi-old-refund",
                5,
                "synthetic original provider refund",
            )
            .await
            .unwrap();
        f.charging.release(&pending.id, &[2; 32]).await.unwrap();
        assert_eq!(
            f.ledger.balance(f.account).await.unwrap().available_units,
            965
        );
        assert_eq!(f.ledger.balance(f.account).await.unwrap().reserved_units, 0);
        assert!(
            f.charging
                .resolve(&started.id)
                .await
                .unwrap()
                .unwrap()
                .oauth_source
                .is_some()
        );
        assert!(matches!(
            f.issuer.usage(&token.access_token).await,
            Err(ReadError::Identity(IdentityError::Unauthorized))
        ));
    })
    .await;
}
#[sqlx::test(migrations = false)]
async fn global_self_budget_still_constrains_oauth_and_native_history(pool: PgPool) {
    with_fixture(pool, true, |f| async move {
        let browser = lmm_core::identity::BrowserAuth::new(
            lmm_core::identity::IdentityStore::from_pool(f.role.pool.clone()),
            &[43; 32],
        )
        .unwrap();
        let session = browser
            .login("pi-alice", "test passphrase", &Default::default())
            .await
            .unwrap();
        f.charging
            .set_budget(
                session.data["access_token"].as_str().unwrap(),
                &Budget {
                    scope: "self".into(),
                    account_id: None,
                    user_id: Some(f.user),
                    key_id: None,
                    period: "day".into(),
                    anchor: 0,
                    seconds: 0,
                    limit: 60,
                },
            )
            .await
            .unwrap();
        let token = f.token(200).await;
        let first = f
            .reserve(&token.access_token, &f.spec("pi-self-first", 50), &[1; 32])
            .await
            .unwrap();
        assert!(matches!(
            f.reserve(&token.access_token, &f.spec("pi-self-over", 11), &[2; 32])
                .await,
            Err(Error::BudgetExceeded)
        ));
        f.charging.release(&first.id, &[1; 32]).await.unwrap();
        assert_eq!(
            f.ledger.balance(f.account).await.unwrap().available_units,
            1000
        );
    })
    .await;
}
#[sqlx::test(migrations = false)]
async fn actual_core_tcp_pi_chat_responses_json_stream_and_native_control(pool: PgPool) {
    with_fixture(pool,true,|f|async move{
        let token=f.token(1000).await;
        for (index,(path,stream)) in [("/v1/chat/completions",false),("/v1/responses",false),("/v1/chat/completions",true),("/v1/responses",true)].into_iter().enumerate(){
            let body=if path.ends_with("responses"){json!({"model":"provider-model","input":"hello","max_output_tokens":20,"stream":stream})}else{json!({"model":"provider-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20,"stream":stream})};
            let response=f.client.post(format!("{}{path}",f.base)).bearer_auth(&token.access_token).header("x-lmm-group","YWxsb3dlZA").header("idempotency-key",format!("pi-http-{index}")).json(&body).send().await.unwrap();assert_eq!(response.status(),reqwest::StatusCode::OK);let body=response.text().await.unwrap();assert!(body.contains(if stream{"local stream answer"}else{"local synthetic answer"}));
        }
        assert_eq!(f.provider.hits.load(Ordering::SeqCst),4);
        let rows:Vec<(Option<i64>,Option<serde_json::Value>,i64,i64)>=sqlx::query_as("SELECT key_id,oauth_source,settled,payer_account_id FROM core_charging.requests ORDER BY id").fetch_all(&f.owner).await.unwrap();assert_eq!(rows.len(),4);for(key,source,settled,payer)in rows{assert!(key.is_none());assert!(source.is_some());assert_eq!(settled,49);assert_eq!(payer,f.account);}
        assert_eq!(f.ledger.balance(f.account).await.unwrap().available_units,804);
        let response=f.client.post(format!("{}/v1/chat/completions",f.base)).bearer_auth(&token.access_token).header("x-lmm-group","cHJpdmF0ZQ").json(&json!({"model":"provider-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20})).send().await.unwrap();assert_eq!(response.status(),reqwest::StatusCode::FORBIDDEN);assert_eq!(f.provider.hits.load(Ordering::SeqCst),4);
        let response=f.client.post(format!("{}/v1/chat/completions",f.base)).bearer_auth(&f.native_key).json(&json!({"model":"provider-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20})).send().await.unwrap();assert_eq!(response.status(),reqwest::StatusCode::OK);let _=response.bytes().await.unwrap();assert_eq!(f.provider.hits.load(Ordering::SeqCst),5);assert_eq!(f.ledger.balance(f.account).await.unwrap().available_units,804);
    }).await;
}

#[sqlx::test(migrations = false)]
async fn inventory_http_requires_live_catalog_authority_and_rejects_queries(pool: PgPool) {
    with_fixture(pool, true, |f| async move {
        let token = f.token(100).await;
        let response = f
            .client
            .get(format!("{}/api/oauth2/inventory", f.base))
            .bearer_auth(&token.access_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let inventory: serde_json::Value = response.json().await.unwrap();
        assert_eq!(inventory["inventory_schema_version"], 1);
        assert_eq!(inventory["scope"], "router_routes_and_aliases");
        let encoded = inventory.to_string().to_ascii_lowercase();
        for secret in [
            "access_token",
            "refresh_token",
            "api_key",
            "secret",
            "password",
        ] {
            assert!(!encoded.contains(secret), "inventory exposed {secret}");
        }

        // The existing v1 model catalog remains reachable with the same grant.
        let response = f
            .client
            .get(format!("{}/v1/models", f.base))
            .bearer_auth(&token.access_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let v1: serde_json::Value = response.json().await.unwrap();
        assert_eq!(v1["data"][0]["id"], "provider-model");

        // The separate grant family avoids narrowing the catalog token used above.
        let balance_family = f.token(100).await;
        let balance_only = f
            .refresh(
                &balance_family.refresh_token,
                Some("balance:read group:YWxsb3dlZA".into()),
            )
            .await
            .unwrap();
        assert_eq!(balance_only.scope, "balance:read group:YWxsb3dlZA");
        let response = f
            .client
            .get(format!("{}/api/oauth2/balance", f.base))
            .bearer_auth(&balance_only.access_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let response = f
            .client
            .get(format!("{}/api/oauth2/inventory", f.base))
            .bearer_auth(&balance_only.access_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

        let response = f
            .client
            .get(format!("{}/api/oauth2/inventory?unexpected=1", f.base))
            .bearer_auth(&token.access_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

        let response = f
            .client
            .get(format!("{}/api/oauth2/inventory", f.base))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

        f.issuer
            .revoke_for_client("lmm-pi", &token.access_token)
            .await
            .unwrap();
        let response = f
            .client
            .get(format!("{}/api/oauth2/inventory", f.base))
            .bearer_auth(&token.access_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn malformed_double_or_foreign_oauth_source_cannot_enter_financial_records(pool: PgPool) {
    with_fixture(pool,true,|f|async move{
        let token=f.token(200).await;let charge=f.reserve(&token.access_token,&f.spec("pi-source-original",50),&[1;32]).await.unwrap();
        let proof=serde_json::to_value(charge.oauth_source.as_ref().unwrap()).unwrap();
        let other=f.token(200).await;let other_charge=f.reserve(&other.access_token,&f.spec("pi-source-other",50),&[2;32]).await.unwrap();let other_proof=serde_json::to_value(other_charge.oauth_source.as_ref().unwrap()).unwrap();
        let mut wrong_cap=proof.clone();wrong_cap["budget_credits"]=json!(199);
        let mut wrong_access=proof.clone();wrong_access["access_id"]=other_proof["access_id"].clone();
        let mut wrong_issuer=proof.clone();wrong_issuer["issuer"]=json!("https://other.example.test");wrong_issuer["resource"]=json!("https://other.example.test/api/oauth2");
        let mut null_issuer=proof.clone();null_issuer["issuer"]=serde_json::Value::Null;
        let mut no_scope=proof.clone();no_scope["scopes"]=json!(["balance:read","group:YWxsb3dlZA"]);
        let native_id:i64=sqlx::query_scalar("SELECT id FROM core_identity.credentials WHERE kind='api_key' ORDER BY id LIMIT 1").fetch_one(&f.owner).await.unwrap();
        const COPY_SOURCE_ROW:&str="INSERT INTO core_charging.requests(id,actor_user_id,key_id,oauth_source,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved) SELECT $2,actor_user_id,$3,$4,owner_account_id,payer_account_id,fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,'reserved',1 FROM core_charging.requests WHERE id=$1";
        let mut control=f.owner.begin().await.unwrap();sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *control).await.unwrap();
        let valid=sqlx::query(COPY_SOURCE_ROW).bind(&charge.id).bind("pi-valid-control").bind(None::<i64>).bind(Some(proof.clone())).execute(&mut *control).await.unwrap_or_else(|_|panic!("valid source control failed"));assert_eq!(valid.rows_affected(),1);control.rollback().await.unwrap();
        for (index,(key,source)) in [(None,None),(Some(native_id),Some(proof.clone())),(None,Some(json!({}))),(None,Some(wrong_cap)),(None,Some(wrong_access)),(None,Some(wrong_issuer)),(None,Some(null_issuer)),(None,Some(no_scope))].into_iter().enumerate(){
            let mut tx=f.owner.begin().await.unwrap();sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
            let result=sqlx::query(COPY_SOURCE_ROW)
                .bind(&charge.id).bind(format!("pi-invalid-source-{index}")).bind(key).bind(source).execute(&mut *tx).await;
            let error=result.expect_err("invalid financial source was accepted");let code=error.as_database_error().and_then(|e|e.code());assert!(matches!(code.as_deref(),Some("23503"|"23514")),"negative oracle failed for unrelated SQL error");tx.rollback().await.unwrap();
        }
        f.charging.release(&charge.id,&[1;32]).await.unwrap();f.charging.release(&other_charge.id,&[2;32]).await.unwrap();assert_eq!(f.ledger.balance(f.account).await.unwrap().available_units,1000);
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE id LIKE 'pi-invalid-source-%'").fetch_one(&f.owner).await.unwrap();assert_eq!(count,0);assert_eq!(f.provider.hits.load(Ordering::SeqCst),0);
    }).await;
}

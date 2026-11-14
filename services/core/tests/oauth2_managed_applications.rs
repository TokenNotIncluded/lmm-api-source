//! Actual owned PG/TCP managed grants; not public-TLS/native-vault acceptance.
use lmm_core::identity::{
    IdentityError, IdentityStore,
    grants::{LmmIssuer, LmmProfile, ManagedRecoveryKeys},
};
use sqlx::PgPool;
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[expect(
    dead_code,
    reason = "This test binary uses the protocol subset of the shared delegated OAuth fixture."
)]
#[path = "common/pi_oauth_fixture.rs"]
mod pi;
#[path = "common/runtime.rs"]
mod runtime;

fn form(values: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse("http://127.0.0.1").unwrap();
    url.query_pairs_mut().extend_pairs(values.iter().copied());
    url.query().unwrap().into()
}
fn cookie(response: &reqwest::Response) -> String {
    let values: Vec<_> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap())
        .filter(|v| v.contains("Max-Age=300"))
        .collect();
    assert_eq!(values.len(), 1);
    values[0].split(';').next().unwrap().into()
}
fn csrf(html: &str) -> String {
    html.split_once("name=csrf value=\"")
        .unwrap()
        .1
        .split_once('"')
        .unwrap()
        .0
        .into()
}

#[sqlx::test(migrations = false)]
async fn tcp_managed_browser_consent_exchanges_strict_wire_and_live_status(pool: PgPool) {
    pi::with_managed_fixture(pool,true,|f|async move {
        let installation="a".repeat(64);
        let mut request=f.managed_request(&installation);
        request.display_label="<script>not executable</script>".into();
        let body=form(&[("response_type",&request.response_type),("client_id",&request.client_id),("redirect_uri",&request.redirect_uri),
            ("scope",&request.scope),("resource",&request.resource),("code_challenge",&request.code_challenge),
            ("code_challenge_method",&request.code_challenge_method),("state",&request.state),
            ("installation_id",&request.installation_id),("application",&request.application),("display_label",&request.display_label)]);
        let response=f.client.get(format!("{}/api/oauth2/authorize?{body}",f.base)).send().await.unwrap();
        assert_eq!(response.status(),200);
        let binding=cookie(&response);let html=response.text().await.unwrap();
        assert!(html.contains("&lt;script&gt;not executable&lt;/script&gt;"));
        assert!(!html.contains("<script>not executable"));
        let origin="https://pi-issuer.example.test";
        let browser=format!("new_api_refresh={}",f.refresh);
        let response=f.client.post(format!("{}/api/user/auth/oauth2/continue",f.base)).header("origin",origin)
            .header("cookie",format!("{binding}; {browser}")).header("content-type","application/x-www-form-urlencoded")
            .body(form(&[("csrf",&csrf(&html))])).send().await.unwrap();
        assert_eq!(response.status(),200);
        let binding=cookie(&response);let html=response.text().await.unwrap();
        assert!(html.contains("name=budget_credits"));assert!(html.contains(&installation));
        let response=f.client.post(format!("{}/api/user/auth/oauth2/consent",f.base)).header("origin",origin)
            .header("cookie",format!("{binding}; {browser}")).header("content-type","application/x-www-form-urlencoded")
            .body(form(&[("csrf",&csrf(&html)),("decision","allow"),("budget_credits","100")])).send().await.unwrap();
        assert_eq!(response.status(),303);
        let location=reqwest::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
        let code=location.query_pairs().find(|(k,_)|k=="code").unwrap().1.into_owned();
        let token_body=form(&[("grant_type","authorization_code"),("client_id","lmm-cc-switch"),("resource",&format!("{origin}/api/oauth2")),
            ("installation_id",&installation),("code",&code),("redirect_uri",&request.redirect_uri),
            ("code_verifier","abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-._~")]);
        let response=f.client.post(format!("{}/api/oauth2/token",f.base)).header("content-type","application/x-www-form-urlencoded").body(token_body.clone()).send().await.unwrap();
        assert_eq!(response.status(),200);
        let token:serde_json::Value=response.json().await.unwrap();assert_eq!(token.as_object().unwrap().len(),9);
        assert_eq!(token["installation_id"],installation);assert_eq!(token["client_id"],"lmm-cc-switch");assert_eq!(token["budget_credits"],100);
        let response=f.client.get(format!("{}/api/oauth2/application",f.base)).bearer_auth(token["access_token"].as_str().unwrap()).send().await.unwrap();
        assert_eq!(response.status(),200);assert_eq!(response.headers()["cache-control"],"no-store");
        let status:serde_json::Value=response.json().await.unwrap();assert_eq!(status["grant_id"],token["grant_id"]);assert_eq!(status["installation_id"],installation);
        let response=f.client.post(format!("{}/api/oauth2/token",f.base)).header("content-type","application/x-www-form-urlencoded").body(token_body).send().await.unwrap();
        assert_eq!(response.status(),400);
        let response=f.client.post(format!("{}/api/oauth2/token",f.base)).header("content-type","application/x-www-form-urlencoded")
            .body("grant_type=refresh_token&client_id=lmm-cc-switch&installation_id=bad&rotation_id=bad&scope=models%3Ainvoke").send().await.unwrap();
        assert_eq!(response.status(),400);
    }).await;
}

#[sqlx::test(migrations = false)]
async fn recovery_reuses_committed_successor_with_remaining_ttl_and_no_plaintext(pool: PgPool) {
    pi::with_managed_fixture(pool,true,|f|async move {
        let install="a".repeat(64);let rotation="b".repeat(64);
        let old=f.managed_token(100,&install).await;
        let (a,b)=tokio::join!(f.managed_refresh(&old.refresh_token,&install,&rotation),f.managed_refresh(&old.refresh_token,&install,&rotation));
        let first=a.unwrap();let replay=b.unwrap();
        assert_eq!(first.access_token,replay.access_token);assert_eq!(first.refresh_token,replay.refresh_token);
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        let later=f.managed_refresh(&old.refresh_token,&install,&rotation).await.unwrap();
        assert_eq!(later.access_token,first.access_token);assert!(later.expires_in<first.expires_in);
        let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_oauth.managed_refresh_lineage WHERE family_id=$1),(SELECT count(*) FROM core_oauth.access_tokens WHERE family_id=$1),(SELECT count(*) FROM core_oauth.refresh_tokens WHERE family_id=$1)")
            .bind(&old.grant_id).fetch_one(&f.owner).await.unwrap();assert_eq!(counts,(1,2,2));
        let encrypted:Vec<u8>=sqlx::query_scalar("SELECT encrypted_response FROM core_oauth.managed_refresh_responses").fetch_one(&f.owner).await.unwrap();
        assert!(!encrypted.windows(first.access_token.len()).any(|w|w==first.access_token.as_bytes()));
        assert!(!encrypted.windows(first.refresh_token.len()).any(|w|w==first.refresh_token.as_bytes()));
        assert!(f.issuer.managed_application_status(&later.access_token).await.is_ok());
    }).await;
}

#[sqlx::test(migrations = false)]
async fn stale_same_operation_and_foreign_installation_do_not_revoke_successor(pool: PgPool) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let install = "a".repeat(64);
        let old = f.managed_token(100, &install).await;
        let next = f
            .managed_refresh(&old.refresh_token, &install, &"b".repeat(64))
            .await
            .unwrap();
        let newest = f
            .managed_refresh(&next.refresh_token, &install, &"c".repeat(64))
            .await
            .unwrap();
        assert!(matches!(
            f.managed_refresh(&old.refresh_token, &install, &"b".repeat(64))
                .await,
            Err(IdentityError::Unauthorized)
        ));
        assert!(matches!(
            f.managed_refresh(&old.refresh_token, &"d".repeat(64), &"e".repeat(64))
                .await,
            Err(IdentityError::Unauthorized)
        ));
        assert!(
            f.issuer
                .managed_application_status(&newest.access_token)
                .await
                .is_ok()
        );
        assert!(matches!(
            f.managed_refresh(&old.refresh_token, &install, &"e".repeat(64))
                .await,
            Err(IdentityError::Unauthorized)
        ));
        assert!(matches!(
            f.issuer
                .managed_application_status(&newest.access_token)
                .await,
            Err(IdentityError::Unauthorized)
        ));
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn recovery_key_rotation_is_real_and_disabled_issuance_still_revokes(pool: PgPool) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let install = "a".repeat(64);
        let rotation = "b".repeat(64);
        let old = f.managed_token(100, &install).await;
        let next = f
            .managed_refresh(&old.refresh_token, &install, &rotation)
            .await
            .unwrap();
        let missing = f
            .issuer
            .clone()
            .with_managed_recovery_keys(
                ManagedRecoveryKeys::new("new", vec![("new".into(), vec![91; 32])]).unwrap(),
            )
            .await
            .unwrap();
        let request = || {
            lmm_core::identity::grants::ManagedTokenForm {
                grant_type: "refresh_token".into(),
                client_id: "lmm-cc-switch".into(),
                resource: f.issuer.profile().resource().into(),
                installation_id: install.clone(),
                code: None,
                redirect_uri: None,
                code_verifier: None,
                refresh_token: Some(old.refresh_token.clone()),
                scope: None,
                rotation_id: Some(rotation.clone()),
            }
            .validate(f.issuer.profile())
            .unwrap()
        };
        assert!(matches!(
            missing.token_managed(request()).await,
            Err(IdentityError::Storage)
        ));
        let rotated = f
            .issuer
            .clone()
            .with_managed_recovery_keys(
                ManagedRecoveryKeys::new(
                    "new",
                    vec![
                        ("new".into(), vec![91; 32]),
                        ("fixture-managed-v1".into(), vec![83; 32]),
                    ],
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            rotated
                .token_managed(request())
                .await
                .unwrap()
                .refresh_token,
            next.refresh_token
        );
        let disabled = LmmIssuer::open(
            IdentityStore::from_pool(f.role.pool.clone()),
            LmmProfile::new("https://pi-issuer.example.test", vec!["allowed".into()]).unwrap(),
            900,
            86400,
        )
        .await
        .unwrap();
        disabled
            .revoke_for_client("lmm-cc-switch", &next.refresh_token)
            .await
            .unwrap();
        assert!(matches!(
            f.issuer
                .managed_application_status(&next.access_token)
                .await,
            Err(IdentityError::Unauthorized)
        ));
    })
    .await;
}

async fn token_counts(pool: &PgPool) -> (i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM core_oauth.access_tokens), (SELECT count(*) FROM core_oauth.refresh_tokens), (SELECT count(*) FROM core_oauth.managed_refresh_lineage)")
        .fetch_one(pool).await.unwrap()
}

async fn lineage_snapshot(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(l) ORDER BY old_refresh_digest) FROM core_oauth.managed_refresh_lineage l")
        .fetch_one(pool).await.unwrap()
}

async fn response_ciphertext(pool: &PgPool, grant: &str) -> Vec<u8> {
    sqlx::query_scalar("SELECT r.encrypted_response FROM core_oauth.managed_refresh_responses r JOIN core_oauth.managed_refresh_lineage l USING(old_refresh_digest) WHERE l.family_id=$1")
        .bind(grant).fetch_one(pool).await.unwrap()
}

/// Explicit owner-only storage-fault injection, not a runtime maintenance path.
/// Replace only the ephemeral cache using its existing DELETE/INSERT rights.
/// All triggers remain enabled; immutable lineage and its timestamps stay intact.
async fn replace_response_ciphertext(pool: &PgPool, grant: &str, ciphertext: &[u8]) {
    let mut tx = pool.begin().await.unwrap();
    let (digest, key): (Vec<u8>, String) = sqlx::query_as("DELETE FROM core_oauth.managed_refresh_responses r USING core_oauth.managed_refresh_lineage l WHERE r.old_refresh_digest=l.old_refresh_digest AND l.family_id=$1 RETURNING r.old_refresh_digest,r.key_id")
        .bind(grant).fetch_one(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO core_oauth.managed_refresh_responses(old_refresh_digest,key_id,encrypted_response) VALUES($1,$2,$3)")
        .bind(digest).bind(key).bind(ciphertext).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn corrupt_persisted_response_refuses_without_mint_or_family_revocation(pool: PgPool) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let install = "a".repeat(64);
        let rotation = "b".repeat(64);
        let old = f.managed_token(100, &install).await;
        let next = f
            .managed_refresh(&old.refresh_token, &install, &rotation)
            .await
            .unwrap();
        let counts = token_counts(&f.owner).await;
        let lineage = lineage_snapshot(&f.owner).await;
        let mut ciphertext = response_ciphertext(&f.owner, &old.grant_id).await;
        *ciphertext.last_mut().unwrap() ^= 1;
        replace_response_ciphertext(&f.owner, &old.grant_id, &ciphertext).await;
        assert!(matches!(
            f.managed_refresh(&old.refresh_token, &install, &rotation)
                .await,
            Err(IdentityError::Storage)
        ));
        assert_eq!(token_counts(&f.owner).await, counts);
        assert_eq!(lineage_snapshot(&f.owner).await, lineage);
        assert!(
            f.issuer
                .managed_application_status(&next.access_token)
                .await
                .is_ok()
        );
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn another_familys_valid_ciphertext_fails_database_path_aad_binding(pool: PgPool) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let install_a = "a".repeat(64);
        let install_b = "b".repeat(64);
        let rotation_a = "c".repeat(64);
        let rotation_b = "d".repeat(64);
        let old_a = f.managed_token(100, &install_a).await;
        let old_b = f.managed_token(100, &install_b).await;
        let next_a = f
            .managed_refresh(&old_a.refresh_token, &install_a, &rotation_a)
            .await
            .unwrap();
        let next_b = f
            .managed_refresh(&old_b.refresh_token, &install_b, &rotation_b)
            .await
            .unwrap();
        let counts = token_counts(&f.owner).await;
        let lineage = lineage_snapshot(&f.owner).await;
        let ciphertext_b = response_ciphertext(&f.owner, &old_b.grant_id).await;
        replace_response_ciphertext(&f.owner, &old_a.grant_id, &ciphertext_b).await;
        assert!(matches!(
            f.managed_refresh(&old_a.refresh_token, &install_a, &rotation_a)
                .await,
            Err(IdentityError::Storage)
        ));
        let recovered_b = f
            .managed_refresh(&old_b.refresh_token, &install_b, &rotation_b)
            .await
            .unwrap();
        assert_eq!(recovered_b.access_token, next_b.access_token);
        assert_eq!(recovered_b.refresh_token, next_b.refresh_token);
        assert!(
            f.issuer
                .managed_application_status(&next_a.access_token)
                .await
                .is_ok()
        );
        assert!(
            f.issuer
                .managed_application_status(&next_b.access_token)
                .await
                .is_ok()
        );
        assert_eq!(token_counts(&f.owner).await, counts);
        assert_eq!(lineage_snapshot(&f.owner).await, lineage);
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn recovery_rechecks_real_developer_revocation_without_replacement_tokens(pool: PgPool) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let install = "a".repeat(64);
        let rotation = "b".repeat(64);
        let old = f.managed_token(100, &install).await;
        let next = f
            .managed_refresh(&old.refresh_token, &install, &rotation)
            .await
            .unwrap();
        let counts = token_counts(&f.owner).await;
        let lineage = lineage_snapshot(&f.owner).await;
        // Same legitimate owner policy update used by the existing Pi tests;
        // normal auth-version/revocation triggers execute in this transaction.
        let changed =
            sqlx::query("UPDATE core_identity.developer_access SET granted=false WHERE user_id=$1")
                .bind(f.user)
                .execute(&f.owner)
                .await
                .unwrap();
        assert_eq!(changed.rows_affected(), 1);
        assert!(matches!(
            f.managed_refresh(&old.refresh_token, &install, &rotation)
                .await,
            Err(IdentityError::Unauthorized | IdentityError::Forbidden)
        ));
        assert!(matches!(
            f.issuer
                .managed_application_status(&next.access_token)
                .await,
            Err(IdentityError::Unauthorized | IdentityError::Forbidden)
        ));
        assert_eq!(token_counts(&f.owner).await, counts);
        assert_eq!(lineage_snapshot(&f.owner).await, lineage);
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn lowered_cap_rejects_recovery_status_and_model_but_preserves_revoke_and_other_family(
    pool: PgPool,
) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let install_a = "a".repeat(64);
        let install_b = "b".repeat(64);
        let rotation_a = "c".repeat(64);
        let rotation_b = "d".repeat(64);
        let old_a = f.managed_token(1000, &install_a).await;
        let old_b = f.managed_token(50, &install_b).await;
        let next_a = f
            .managed_refresh(&old_a.refresh_token, &install_a, &rotation_a)
            .await
            .unwrap();
        let next_b = f
            .managed_refresh(&old_b.refresh_token, &install_b, &rotation_b)
            .await
            .unwrap();
        let counts = token_counts(&f.owner).await;
        let lineage = lineage_snapshot(&f.owner).await;
        let profile = LmmProfile::new("https://pi-issuer.example.test", vec!["allowed".into()])
            .unwrap()
            .with_pi_budget_limit(1000)
            .unwrap()
            .with_managed_budget_limit(100)
            .unwrap();
        let reduced = f
            .issuer_for_profile(profile, pi::ManagedKeyring::Configured)
            .await;
        let request = |old: &str, install: &str, rotation: &str| {
            lmm_core::identity::grants::ManagedTokenForm {
                grant_type: "refresh_token".into(),
                client_id: "lmm-cc-switch".into(),
                resource: reduced.profile().resource().into(),
                installation_id: install.into(),
                code: None,
                redirect_uri: None,
                code_verifier: None,
                refresh_token: Some(old.into()),
                scope: None,
                rotation_id: Some(rotation.into()),
            }
            .validate(reduced.profile())
            .unwrap()
        };
        assert!(matches!(
            reduced
                .token_managed(request(&old_a.refresh_token, &install_a, &rotation_a))
                .await,
            Err(IdentityError::Unauthorized)
        ));
        assert!(matches!(
            reduced
                .managed_application_status(&next_a.access_token)
                .await,
            Err(IdentityError::Unauthorized)
        ));
        let mut tx = f.role.pool.begin().await.unwrap();
        assert!(matches!(
            reduced
                .authorize_model_in_transaction(&mut tx, &next_a.access_token, None, None)
                .await,
            Err(IdentityError::Unauthorized)
        ));
        tx.rollback().await.unwrap();
        let recovered_b = reduced
            .token_managed(request(&old_b.refresh_token, &install_b, &rotation_b))
            .await
            .unwrap();
        assert_eq!(recovered_b.access_token, next_b.access_token);
        assert_eq!(recovered_b.refresh_token, next_b.refresh_token);
        let status_b = reduced
            .managed_application_status(&next_b.access_token)
            .await
            .unwrap();
        assert_eq!(status_b["budget_credits"], 50);
        let mut tx = f.role.pool.begin().await.unwrap();
        let source_b = reduced
            .authorize_model_in_transaction(&mut tx, &next_b.access_token, None, None)
            .await
            .unwrap();
        assert_eq!(source_b.grant_id(), old_b.grant_id);
        assert_eq!(
            serde_json::to_value(source_b).unwrap()["budget_credits"],
            50
        );
        tx.rollback().await.unwrap();
        let revoked = |grant: &str| {
            sqlx::query_scalar::<_, bool>(
                "SELECT revoked_at IS NOT NULL FROM core_oauth.families WHERE id=$1",
            )
            .bind(grant.to_owned())
            .fetch_one(&f.owner)
        };
        // Rejection is current qualification, not a hidden revocation workaround.
        assert!(!revoked(&old_a.grant_id).await.unwrap());
        assert!(!revoked(&old_b.grant_id).await.unwrap());
        assert_eq!(token_counts(&f.owner).await, counts);
        assert_eq!(lineage_snapshot(&f.owner).await, lineage);
        reduced
            .revoke_for_client("lmm-cc-switch", &next_a.refresh_token)
            .await
            .unwrap();
        assert!(revoked(&old_a.grant_id).await.unwrap());
        assert!(!revoked(&old_b.grant_id).await.unwrap());
        assert!(
            reduced
                .managed_application_status(&next_b.access_token)
                .await
                .is_ok()
        );
        let disabled = f
            .issuer_for_profile(
                LmmProfile::new("https://pi-issuer.example.test", vec!["allowed".into()])
                    .unwrap()
                    .with_pi_budget_limit(1000)
                    .unwrap(),
                pi::ManagedKeyring::Missing,
            )
            .await;
        disabled
            .revoke_for_client("lmm-cc-switch", &next_b.refresh_token)
            .await
            .unwrap();
        assert!(revoked(&old_b.grant_id).await.unwrap());
        assert_eq!(token_counts(&f.owner).await, counts);
        assert_eq!(lineage_snapshot(&f.owner).await, lineage);
    })
    .await;
}

/// A thin TCP adapter around the real router, sharing the existing fixture's
/// PG/provider/runtime. No extra Core process, database or fabricated response.
async fn discovery_over_tcp(issuer: LmmIssuer) -> (reqwest::StatusCode, serde_json::Value) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            lmm_core::oauth2_http::router(issuer)
                .into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = stopped.await;
        })
        .await
        .unwrap();
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let response = client
        .get(format!("{base}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let document: serde_json::Value = response.json().await.unwrap();
    if status.is_success() {
        // Exact common validation performed by qualified CLI96b16b protocol.rs.
        let issuer = document["issuer"].as_str().unwrap();
        assert_eq!(
            document["authorization_endpoint"],
            format!("{issuer}/api/oauth2/authorize")
        );
        assert_eq!(
            document["token_endpoint"],
            format!("{issuer}/api/oauth2/token")
        );
        assert_eq!(
            document["revocation_endpoint"],
            format!("{issuer}/api/oauth2/revoke")
        );
        assert_eq!(
            document["authorization_response_iss_parameter_supported"],
            true
        );
        for (field, value) in [
            ("code_challenge_methods_supported", "S256"),
            ("response_types_supported", "code"),
            ("token_endpoint_auth_methods_supported", "none"),
        ] {
            assert!(
                document[field]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item == value)
            );
        }
        let resource: serde_json::Value = client
            .get(format!(
                "{base}/.well-known/oauth-protected-resource/api/oauth2"
            ))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resource["resource"], format!("{issuer}/api/oauth2"));
        assert_eq!(
            resource["authorization_servers"],
            serde_json::json!([issuer])
        );
    }
    let _ = stop.send(());
    server.await.unwrap();
    (status, document)
}

#[sqlx::test(migrations = false)]
async fn real_http_discovery_preserves_pi_and_advertises_only_configured_managed_profile(
    pool: PgPool,
) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let (status, ready) = discovery_over_tcp(f.issuer.clone()).await;
        assert_eq!(status, 200);
        assert_eq!(
            ready["lmm_client_ids_supported"],
            serde_json::json!(["lmm", "lmm-pi", "lmm-cc-switch"])
        );
        assert_eq!(
            ready["lmm_managed_clients"],
            serde_json::json!([{
                "client_id":"lmm-cc-switch", "application":"cc-switch",
                "status_endpoint":"https://pi-issuer.example.test/api/oauth2/application",
                "refresh_recovery":{"contract":"lmm.refresh-recovery.v1", "window_seconds":600}
            }])
        );
        assert_eq!(
            ready["lmm_managed_clients"][0]["refresh_recovery"]["window_seconds"].as_u64(),
            Some(600)
        );
        let base =
            || LmmProfile::new("https://pi-issuer.example.test", vec!["allowed".into()]).unwrap();
        let pi_only = base().with_pi_budget_limit(1000).unwrap();
        let (status, disabled) = discovery_over_tcp(
            f.issuer_for_profile(pi_only.clone(), pi::ManagedKeyring::Configured)
                .await,
        )
        .await;
        assert_eq!(status, 200);
        assert!(disabled.get("lmm_managed_clients").is_none());
        assert_eq!(
            disabled["lmm_client_ids_supported"],
            serde_json::json!(["lmm", "lmm-pi"])
        );
        assert_eq!(disabled["scopes_supported"], ready["scopes_supported"]);
        let enabled = pi_only.clone().with_managed_budget_limit(1000).unwrap();
        let (status, missing) = discovery_over_tcp(
            f.issuer_for_profile(enabled.clone(), pi::ManagedKeyring::Missing)
                .await,
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(missing, disabled);
        let (status, managed_only) = discovery_over_tcp(
            f.issuer_for_profile(
                base().with_managed_budget_limit(1000).unwrap(),
                pi::ManagedKeyring::Configured,
            )
            .await,
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(
            managed_only["lmm_client_ids_supported"],
            serde_json::json!(["lmm", "lmm-cc-switch"])
        );
        assert_eq!(
            managed_only["lmm_managed_clients"],
            ready["lmm_managed_clients"]
        );
        assert_eq!(managed_only["scopes_supported"], ready["scopes_supported"]);
        let no_surface = LmmIssuer::open(
            IdentityStore::from_pool(f.role.pool.clone()),
            enabled,
            900,
            86400,
        )
        .await
        .unwrap()
        .with_managed_recovery_keys(
            ManagedRecoveryKeys::new(
                "fixture-managed-v1",
                vec![("fixture-managed-v1".into(), vec![83; 32])],
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let (status, absent_catalog) = discovery_over_tcp(no_surface).await;
        assert_eq!(status, 200);
        assert_eq!(absent_catalog, disabled);
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn real_http_discovery_refuses_live_catalog_drift_without_repair(pool: PgPool) {
    pi::with_managed_fixture(pool, true, |f| async move {
        assert_eq!(discovery_over_tcp(f.issuer.clone()).await.0, 200);
        let receipt: Vec<u8> =
            sqlx::query_scalar("SELECT catalog_fingerprint FROM core_meta.runtime_contract")
                .fetch_one(&f.owner)
                .await
                .unwrap();
        // Declared owner-only catalog fault: no existing DDL, fingerprint or
        // immutability trigger is rewritten or disabled.
        sqlx::query("CREATE TABLE public.qa_managed_discovery_drift(marker boolean)")
            .execute(&f.owner)
            .await
            .unwrap();
        let (status, document) = discovery_over_tcp(f.issuer.clone()).await;
        assert_eq!(status, 503);
        assert_eq!(
            document,
            serde_json::json!({"error":"temporarily_unavailable"})
        );
        let retained: Vec<u8> =
            sqlx::query_scalar("SELECT catalog_fingerprint FROM core_meta.runtime_contract")
                .fetch_one(&f.owner)
                .await
                .unwrap();
        assert_eq!(retained, receipt);
        assert!(lmm_core::core_schema::check(&f.owner).await.is_err());
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn real_http_discovery_refuses_missing_managed_runtime_privileges(pool: PgPool) {
    pi::with_managed_fixture(pool, true, |f| async move {
        assert_eq!(discovery_over_tcp(f.issuer.clone()).await.0, 200);
        assert!(
            f.role
                .name
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || value == b'_')
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "REVOKE INSERT ON core_oauth.managed_refresh_responses FROM {}",
            f.role.name
        )))
        .execute(&f.owner)
        .await
        .unwrap();
        // ACL-only fault leaves the actual catalog intact; readiness must also
        // check the restricted runtime's ability to persist encrypted recovery.
        lmm_core::core_schema::check(&f.owner).await.unwrap();
        let (status, document) = discovery_over_tcp(f.issuer.clone()).await;
        assert_eq!(status, 503);
        assert_eq!(
            document,
            serde_json::json!({"error":"temporarily_unavailable"})
        );
    })
    .await;
}

/// Run explicitly once against owned PG with --ignored --exact. This measures
/// real DB wall time; no clock injection, timestamp rewrite or trigger bypass.
#[sqlx::test(migrations = false)]
#[ignore = "explicit owned-PG gate: waits a real 600-second recovery window"]
async fn recovery_real_599_600_half_open_window_preserves_live_successor(pool: PgPool) {
    pi::with_managed_fixture(pool, true, |f| async move {
        let install = "a".repeat(64);
        let rotation = "b".repeat(64);
        let old = f.managed_token(100, &install).await;
        let next = f.managed_refresh(&old.refresh_token, &install, &rotation).await.unwrap();
        let counts = token_counts(&f.owner).await;
        let lineage = lineage_snapshot(&f.owner).await;
        let remaining = || sqlx::query_scalar::<_, f64>("SELECT extract(epoch FROM recover_until-clock_timestamp())::double precision FROM core_oauth.managed_refresh_lineage WHERE family_id=$1")
            .bind(&old.grant_id).fetch_one(&f.owner);
        tokio::time::sleep(std::time::Duration::from_secs_f64((remaining().await.unwrap()-0.8).max(0.0))).await;
        let before = remaining().await.unwrap();
        eprintln!("recovery boundary before: DB seconds remaining={before:.6}");
        assert!(before > 0.0 && before <= 1.0, "must actually exercise elapsed >=599 and <600");
        let recovered = f.managed_refresh(&old.refresh_token, &install, &rotation).await.unwrap();
        assert_eq!(recovered.access_token, next.access_token);
        assert_eq!(recovered.refresh_token, next.refresh_token);
        assert_eq!(recovered.grant_id, next.grant_id);
        assert_eq!(recovered.scope, next.scope);
        assert_eq!(recovered.budget_credits, next.budget_credits);
        assert!(recovered.expires_in > 0 && recovered.expires_in < next.expires_in);
        tokio::time::sleep(std::time::Duration::from_secs_f64(remaining().await.unwrap().max(0.0))+std::time::Duration::from_millis(100)).await;
        let after = remaining().await.unwrap();
        eprintln!("recovery boundary after: DB seconds remaining={after:.6}");
        assert!(after <= 0.0, "must actually exercise elapsed >=600");
        assert!(matches!(f.managed_refresh(&old.refresh_token, &install, &rotation).await, Err(IdentityError::Unauthorized)));
        assert!(f.issuer.managed_application_status(&next.access_token).await.is_ok());
        assert_eq!(token_counts(&f.owner).await, counts);
        assert_eq!(lineage_snapshot(&f.owner).await, lineage);
        let removed: i32 = sqlx::query_scalar("SELECT core_oauth.cleanup_managed_refresh_responses(1)").fetch_one(&f.owner).await.unwrap();
        assert_eq!(removed, 1);
        assert_eq!(lineage_snapshot(&f.owner).await, lineage);
        assert!(matches!(f.managed_refresh(&old.refresh_token, &install, &rotation).await, Err(IdentityError::Unauthorized)));
        assert!(f.issuer.managed_application_status(&next.access_token).await.is_ok());
    }).await;
}

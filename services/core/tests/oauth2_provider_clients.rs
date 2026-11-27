//! Real PostgreSQL, browser consent, Core HTTP and a bounded synthetic upstream.
//! Each provider keeps its own client binding and user-confirmed lifetime cap.
use lmm_core::{
    billing::charging::{Error, Outcome, Usage},
    identity::{
        IdentityError,
        grants::{LmmProfile, PROVIDER_CLIENTS, TokenForm},
    },
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::atomic::Ordering};

#[allow(dead_code)]
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[expect(
    dead_code,
    reason = "This provider target uses the shared OAuth fixture's public-client path."
)]
#[path = "common/pi_oauth_fixture.rs"]
mod pi;
#[allow(dead_code)]
#[path = "common/runtime.rs"]
mod runtime;

#[path = "oauth2_provider_clients/codewhale.rs"]
mod codewhale;
#[path = "oauth2_provider_clients/dsh_sdk.rs"]
mod dsh_sdk;
#[path = "oauth2_provider_clients/opencode_v2.rs"]
mod opencode_v2;
#[path = "oauth2_provider_clients/provider_host.rs"]
mod provider_host;
#[path = "oauth2_provider_clients/vscode_rejection.rs"]
mod vscode_rejection;

const ISSUER: &str = "https://pi-issuer.example.test";

async fn refresh_http(f: &pi::Fixture, client: &str, refresh: &str) -> reqwest::Response {
    f.client
        .post(format!("{}/api/oauth2/token", f.base))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client),
            ("resource", &format!("{ISSUER}/api/oauth2")),
            ("refresh_token", refresh),
        ])
        .send()
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn public_provider_http_lifecycle_preserves_client_and_financial_binding(pool: PgPool) {
    pi::with_provider_fixture(pool, |f| async move {
        let metadata: Value = f.client
            .get(format!("{}/.well-known/oauth-authorization-server", f.base))
            .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        let supported = metadata["lmm_client_ids_supported"].as_array().unwrap();
        assert_eq!(supported.len(), PROVIDER_CLIENTS.len() + 2);
        assert!(metadata.get("lmm_managed_clients").is_none());
        let before_keys: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_identity.credentials WHERE kind='api_key'",
        ).fetch_one(&f.owner).await.unwrap();

        for (index, (client, _)) in PROVIDER_CLIENTS.iter().enumerate() {
            assert!(supported.contains(&json!(client)));
            let mut request = f.request();
            request.client_id = (*client).into();
            let first = f.issuer.begin(request).await.unwrap();
            let consent = f.issuer.continue_authorization(
                &first.binding, &first.csrf, Some(&f.refresh),
            ).await.unwrap();
            assert_eq!(consent.maximum_budget_credits, Some(1000));
            assert!(matches!(
                f.issuer.consent_with_budget(
                    &consent.binding, &consent.csrf, Some(&f.refresh), true, Some(1001),
                ).await,
                Err(IdentityError::Invalid),
            ));

            let token = f.token_for_client(client, 500).await;
            assert_eq!(token.scope.split(' ').count(), 5);
            assert!(token.scope.contains("models:invoke"));
            assert!(!token.scope.contains("mcp:"));
            assert!(!token.scope.contains("remote:"));
            let catalog = f.client.get(format!("{}/api/oauth2/catalog", f.base))
                .bearer_auth(&token.access_token).send().await.unwrap();
            assert_eq!(catalog.status(), reqwest::StatusCode::OK);

            let other = PROVIDER_CLIENTS[(index + 1) % PROVIDER_CLIENTS.len()].0;
            let wrong = refresh_http(&f, other, &token.refresh_token).await;
            assert_eq!(wrong.status(), reqwest::StatusCode::BAD_REQUEST);
            let refreshed = refresh_http(&f, client, &token.refresh_token).await;
            assert_eq!(refreshed.status(), reqwest::StatusCode::OK);
            let refreshed: Value = refreshed.json().await.unwrap();
            let access = refreshed["access_token"].as_str().unwrap();
            let refresh = refreshed["refresh_token"].as_str().unwrap();
            assert_ne!(access, token.access_token);

            let response = f.client.post(format!("{}/v1/chat/completions", f.base))
                .bearer_auth(access)
                .header("x-lmm-group", "YWxsb3dlZA")
                .header("idempotency-key", format!("provider-http-{index}"))
                .json(&json!({
                    "model":"provider-model",
                    "messages":[{"role":"user","content":"hello"}],
                    "max_tokens":20
                }))
                .send().await.unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            assert!(response.text().await.unwrap().contains("local synthetic answer"));

            let wrong = f.client.post(format!("{}/api/oauth2/revoke", f.base))
                .form(&[("client_id", other), ("token", access)])
                .send().await.unwrap();
            assert_eq!(wrong.status(), reqwest::StatusCode::BAD_REQUEST);
            assert!(f.issuer.catalog(access).await.is_ok());
            let revoked = f.client.post(format!("{}/api/oauth2/revoke", f.base))
                .form(&[("client_id", *client), ("token", access)])
                .send().await.unwrap();
            assert_eq!(revoked.status(), reqwest::StatusCode::OK);
            assert!(f.issuer.catalog(access).await.is_err());
            assert_eq!(
                refresh_http(&f, client, refresh).await.status(),
                reqwest::StatusCode::BAD_REQUEST,
            );
        }
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 5);
        let charges: Vec<(Option<i64>, String, i64, i64)> = sqlx::query_as(
            "SELECT key_id,oauth_source->>'client_id',settled,payer_account_id FROM core_charging.requests",
        ).fetch_all(&f.owner).await.unwrap();
        assert_eq!(charges.len(), 5);
        let charges: BTreeMap<_, _> = charges.into_iter()
            .map(|(key, client, settled, payer)| (client, (key, settled, payer)))
            .collect();
        assert_eq!(charges.len(), PROVIDER_CLIENTS.len());
        for (client, _) in PROVIDER_CLIENTS {
            let (key, settled, payer) = &charges[*client];
            assert!(key.is_none());
            assert_eq!(*settled, 49);
            assert_eq!(*payer, f.account);
        }
        let balance = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(balance.available_units, 755);
        assert_eq!(balance.reserved_units, 0);
        let after_keys: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_identity.credentials WHERE kind='api_key'",
        ).fetch_one(&f.owner).await.unwrap();
        assert_eq!(after_keys, before_keys);
    }).await;
}

#[sqlx::test(migrations = false)]
async fn provider_cap_is_shared_across_rotation_and_revocation_survives_disable(pool: PgPool) {
    pi::with_provider_fixture(pool, |f| async move {
        let client = "lmm-dsh";
        let token = f.token_for_client(client, 70).await;
        let refreshed = refresh_http(&f, client, &token.refresh_token).await;
        assert_eq!(refreshed.status(), reqwest::StatusCode::OK);
        let refreshed: Value = refreshed.json().await.unwrap();
        let access = refreshed["access_token"].as_str().unwrap();
        let a = f.spec("provider-cap-a", 50);
        let b = f.spec("provider-cap-b", 50);
        let (a, b) = tokio::join!(
            f.reserve(&token.access_token, &a, &[1; 32]),
            f.reserve(access, &b, &[2; 32]),
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        let (charge, worker) = if let Ok(a) = a {
            assert!(matches!(b, Err(Error::BudgetExceeded)));
            (a, [1; 32])
        } else {
            assert!(matches!(a, Err(Error::BudgetExceeded)));
            (b.unwrap(), [2; 32])
        };
        assert_eq!(charge.oauth_source.as_ref().unwrap().client_id(), client);
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
            f.reserve(access, &f.spec("provider-cap-exhausted", 31), &[3; 32])
                .await,
            Err(Error::BudgetExceeded),
        ));

        let disabled = f
            .issuer_for_profile(
                LmmProfile::new(ISSUER, vec!["allowed".into()]).unwrap(),
                pi::ManagedKeyring::Missing,
            )
            .await;
        let mut request = f.request();
        request.client_id = client.into();
        assert!(disabled.begin(request).await.is_err());
        let form = TokenForm {
            grant_type: "refresh_token".into(),
            client_id: client.into(),
            resource: format!("{ISSUER}/api/oauth2"),
            code: None,
            redirect_uri: None,
            code_verifier: None,
            refresh_token: Some(refreshed["refresh_token"].as_str().unwrap().into()),
            scope: None,
        };
        assert!(form.validate(disabled.profile()).is_err());
        disabled.revoke_for_client(client, access).await.unwrap();
        assert!(f.issuer.catalog(access).await.is_err());
        let balance = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(balance.available_units, 960);
        assert_eq!(balance.reserved_units, 0);
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn codewhale_standard_chat_preserves_exact_group_and_oauth_settlement(pool: PgPool) {
    pi::with_provider_fixture(pool, |f| async move {
        let client = "lmm-codewhale";
        let token = f.token_for_client(client, 500).await;
        let other = f.token_for_client("lmm-dsh", 500).await;
        let models = format!("{}/api/oauth2/openai/v1/models", f.base);
        let chat = format!("{}/api/oauth2/openai/v1/chat/completions", f.base);

        for (access, status) in [
            (f.native_key.as_str(), 401),
            (other.access_token.as_str(), 403),
        ] {
            let list = f.client.get(&models).bearer_auth(access).send().await.unwrap();
            assert_eq!(list.status().as_u16(), status);
            let result = f.client.post(&chat).bearer_auth(access)
                .json(&json!({"model":"lmm:YWxsb3dlZA:cHJvdmlkZXItbW9kZWw",
                    "messages":[{"role":"user","content":"hello"}]}))
                .send().await.unwrap();
            assert_eq!(result.status().as_u16(), status);
        }

        let list = f.client.get(&models).bearer_auth(&token.access_token)
            .send().await.unwrap().error_for_status().unwrap();
        assert_eq!(list.headers()["cache-control"], "no-store");
        let list: Value = list.json().await.unwrap();
        assert_eq!(list["object"], "list");
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
        let model_id = "lmm:YWxsb3dlZA:cHJvdmlkZXItbW9kZWw";
        assert_eq!(list["data"][0]["id"], model_id);
        assert_eq!(list["data"][0]["name"], "allowed / provider-model");
        assert_eq!(
            list["data"][0]["limit"]["output"],
            lmm_core::relay::Limits::default().max_output_tokens,
        );

        let invoke = |id: &str| {
            f.client.post(&chat).bearer_auth(&token.access_token)
                .json(&json!({"model":id,"max_tokens":20,
                    "messages":[{"role":"user","content":"hello"}]}))
        };
        for (id, status) in [
            ("provider-model", 400),
            ("lmm:Zm9yYmlkZGVu:cHJvdmlkZXItbW9kZWw", 403),
            ("lmm:YWxsb3dlZA:cHJvdmlkZXItbW9kZWw=", 403),
        ] {
            assert_eq!(invoke(id).send().await.unwrap().status().as_u16(), status);
        }
        assert_eq!(invoke(model_id).header("x-lmm-group", "YWxsb3dlZA")
            .send().await.unwrap().status().as_u16(), 400);
        let ambiguous = format!(r#"{{"model":"{model_id}","MODEL":"{model_id}","messages":[]}}"#);
        let failed = f.client.post(&chat).bearer_auth(&token.access_token)
            .header("content-type", "application/json").body(ambiguous)
            .send().await.unwrap();
        assert_eq!(failed.status().as_u16(), 400);
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);

        let response = invoke(model_id).header("idempotency-key", "codewhale-json")
            .send().await.unwrap().error_for_status().unwrap();
        assert!(response.text().await.unwrap().contains("local synthetic answer"));
        let response = f.client.post(&chat).bearer_auth(&token.access_token)
            .header("idempotency-key", "codewhale-sse").json(&json!({
                "model":model_id, "max_tokens":20, "stream":true,
                "stream_options":{"include_usage":true},
                "messages":[{"role":"user","content":"hello"}]
            })).send().await.unwrap().error_for_status().unwrap();
        let content_type = response.headers()["content-type"].to_str().unwrap();
        assert_eq!(content_type.split(';').next(), Some("text/event-stream"));
        let stream = response.text().await.unwrap();
        assert!(stream.contains("local stream answer"));
        assert!(stream.contains("[DONE]"));
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 2);

        let receipts: Vec<(Option<i64>, String, i64, i64)> = sqlx::query_as(
            "SELECT key_id,oauth_source->>'client_id',settled,payer_account_id FROM core_charging.requests",
        ).fetch_all(&f.owner).await.unwrap();
        assert_eq!(receipts, vec![(None, client.to_owned(), 49, f.account); 2]);
        assert_eq!(f.ledger.balance(f.account).await.unwrap().reserved_units, 0);

        f.issuer.revoke_for_client(client, &token.access_token).await.unwrap();
        assert_eq!(f.client.get(&models).bearer_auth(&token.access_token)
            .send().await.unwrap().status().as_u16(), 401);
        assert_eq!(invoke(model_id).send().await.unwrap().status().as_u16(), 401);
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 2);
    }).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires the packaged VSIX and the official Electron host under Xvfb"]
async fn vscode_extension_host_streams_real_core_and_settles_once(pool: PgPool) {
    use std::{
        env,
        io::Write,
        path::Path,
        process::{Command, Stdio},
        time::Duration,
    };

    let origin = env::var("LMM_VSCODE_TEST_ISSUER").unwrap();
    let client = env::var("LMM_VSCODE_TEST_CLIENT").unwrap();
    pi::with_vscode_fixture(pool, &origin, move |f| async move {
        // Three genuine client requests share this bounded, disposable wallet.
        let token = f.token_for_client(&client, pi::WALLET_CREDITS).await;
        vscode_rejection::interrupt_zero_settlement(&f.owner).await;
        let receipt_path = std::path::PathBuf::from(env::var("LMM_VSCODE_TEST_RECEIPT").unwrap());
        let wire_path = receipt_path.with_extension("wire.jsonl");
        let child_wire_path = wire_path.clone();
        let catalog: Value = f
            .client
            .get(format!("{}/api/oauth2/catalog", f.base))
            .bearer_auth(&token.access_token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let routes = catalog["models"].as_array().unwrap();
        assert_eq!(
            routes.len(),
            1,
            "fixture must publish a single authorized route"
        );

        // The supplier withholds finish/usage until VS Code has yielded text.
        let progress = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let profile = json!({
            "base": f.base,
            "route": routes[0],
            "progressPort": progress.local_addr().unwrap().port(),
            "token": {
                "access_token": token.access_token, "refresh_token": token.refresh_token,
                "expires_in": token.expires_in, "scope": token.scope, "token_type": token.token_type
            }
        });
        let gate = f.provider.stream_gate.clone();
        let release = tokio::spawn(async move {
            let (_socket, peer) = progress.accept().await.unwrap();
            assert!(peer.ip().is_loopback());
            gate.add_permits(1);
        });
        let status = tokio::task::spawn_blocking(move || {
            let script =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/vscode-core-host/run.sh");
            let mut child = Command::new("bash")
                .arg(script)
                .env("LMM_VSCODE_WIRE_RECEIPT", child_wire_path)
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&serde_json::to_vec(&profile).unwrap())
                .unwrap();
            child.wait().unwrap()
        })
        .await
        .unwrap();
        assert!(
            status.success(),
            "official VS Code extension host exited with {status}"
        );
        tokio::time::timeout(Duration::from_secs(2), release)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 3);

        let proof = vscode_rejection::recover(&f, &client, &wire_path).await;
        let receipt = json!({
            "commit": env::var("GITHUB_SHA").unwrap(),
            "public_client": client,
            "upstream_calls": f.provider.hits.load(Ordering::SeqCst),
            "text_observed_before_finish": true,
            "stream_completed": true,
            "supplier_refusal_http": 502,
            "requests": proof["requests"],
            "recovery": proof["recovery"],
        });
        std::fs::write(receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    })
    .await;
}

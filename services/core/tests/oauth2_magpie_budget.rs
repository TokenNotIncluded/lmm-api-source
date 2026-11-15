//! Real isolated PG, restricted runtime and Core HTTP; synthetic upstream only.
use lmm_core::billing::charging::{Error, Outcome, Usage};
use lmm_core::identity::grants::{LmmProfile, ManagedTokenForm};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::atomic::Ordering;
#[allow(dead_code)]
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[allow(dead_code)]
#[path = "common/pi_oauth_fixture.rs"]
mod pi;
#[allow(dead_code)]
#[path = "common/runtime.rs"]
mod runtime;

async fn budget(f: &pi::Fixture, access: &str) -> Value {
    let response = f
        .client
        .get(format!("{}/api/oauth2/application/budget", f.base))
        .bearer_auth(access)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    response.json().await.unwrap()
}
fn amounts(v: &Value, consumed: i64, reserved: i64, remaining: i64) {
    assert_eq!(v["consumed_credits"], consumed);
    assert_eq!(v["reserved_credits"], reserved);
    assert_eq!(v["remaining_credits"], remaining);
}

#[sqlx::test(migrations = false)]
async fn magpie_http_budget_tracks_family_pending_settled_refund_cancel_and_unknown(pool: PgPool) {
    pi::with_magpie_fixture(pool, true, |f| async move {
        let installation = "a".repeat(64);
        let token = f.application_token("lmm-magpie", "magpie", 80, &installation).await;
        let initial = budget(&f, &token.access_token).await;
        assert_eq!(initial, json!({"contract":"lmm.application-budget.v1","unit":"credit_500k_usd",
            "client_id":"lmm-magpie","application":"magpie","installation_id":installation,
            "grant_id":token.grant_id,"budget_credits":80,"consumed_credits":0,
            "reserved_credits":0,"remaining_credits":80}));
        let refresh = ManagedTokenForm {grant_type:"refresh_token".into(),client_id:"lmm-magpie".into(),
            resource:f.issuer.profile().resource().into(),installation_id:installation.clone(),
            code:None,redirect_uri:None,code_verifier:None,refresh_token:Some(token.refresh_token.clone()),
            scope:None,rotation_id:Some("b".repeat(64))};
        let next = f.issuer.token_managed(refresh.validate(f.issuer.profile()).unwrap()).await.unwrap();
        assert_eq!(next.client_id,"lmm-magpie"); assert_eq!(next.grant_id,token.grant_id);
        let a = f.spec("magpie-concurrent-a", 50); let b = f.spec("magpie-concurrent-b", 50);
        let (a,b) = tokio::join!(f.reserve(&token.access_token,&a,&[1;32]),f.reserve(&next.access_token,&b,&[2;32]));
        let (charge,worker) = match (a,b) {
            (Ok(a),Err(Error::BudgetExceeded)) => (a,[1;32]),
            (Err(Error::BudgetExceeded),Ok(b)) => (b,[2;32]),
            _ => panic!("family cap did not serialize concurrent requests"),
        };
        amounts(&budget(&f,&next.access_token).await,0,50,30);
        assert_eq!(budget(&f,&token.access_token).await,budget(&f,&next.access_token).await);
        assert!(f.charging.start_oauth(&f.issuer,&charge.id,&worker).await.unwrap());
        f.charging.settle(&charge.id,&worker,Usage{input:40,..Default::default()},Outcome::Completed).await.unwrap();
        amounts(&budget(&f,&next.access_token).await,40,0,40);
        f.charging.refund(&charge.id,"magpie-refund",10,"synthetic verified refund").await.unwrap();
        amounts(&budget(&f,&next.access_token).await,30,0,50);
        // Owner-only historical fixture for the lifetime projection; this is
        // not elapsed-time acceptance. Restore the immutable guard atomically.
        let mut historical=f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *historical).await.unwrap();
        sqlx::query("ALTER TABLE core_charging.requests DISABLE TRIGGER request_identity")
            .execute(&mut *historical).await.unwrap();
        sqlx::query("UPDATE core_charging.requests SET authorized_at=authorized_at-40*86400 WHERE id=$1")
            .bind(&charge.id).execute(&mut *historical).await.unwrap();
        // Validate queued deferred constraints before the table ALTER.
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *historical).await.unwrap();
        sqlx::query("ALTER TABLE core_charging.requests ENABLE TRIGGER request_identity")
            .execute(&mut *historical).await.unwrap();
        historical.commit().await.unwrap();
        let enabled:String=sqlx::query_scalar("SELECT tgenabled::text FROM pg_trigger WHERE tgrelid='core_charging.requests'::regclass AND tgname='request_identity'")
            .fetch_one(&f.owner).await.unwrap();
        assert_eq!(enabled,"O");
        amounts(&budget(&f,&next.access_token).await,30,0,50);
        let canceled=f.reserve(&next.access_token,&f.spec("magpie-cancel",50),&[3;32]).await.unwrap();
        amounts(&budget(&f,&next.access_token).await,30,50,0);
        assert!(matches!(f.reserve(&next.access_token,&f.spec("magpie-over",1),&[4;32]).await,Err(Error::BudgetExceeded)));
        f.charging.release(&canceled.id,&[3;32]).await.unwrap();
        amounts(&budget(&f,&next.access_token).await,30,0,50);
        let unknown=f.reserve(&next.access_token,&f.spec("magpie-unknown",50),&[5;32]).await.unwrap();
        f.charging.start_oauth(&f.issuer,&unknown.id,&[5;32]).await.unwrap();
        let mut expired=f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *expired).await.unwrap();
        sqlx::query("UPDATE core_charging.requests SET lease_until=floor(extract(epoch FROM clock_timestamp()))::bigint-1,revision=revision+1 WHERE id=$1")
            .bind(&unknown.id).execute(&mut *expired).await.unwrap();
        expired.commit().await.unwrap();
        let recovered=f.charging.recover_expired(10).await.unwrap();
        assert_eq!(recovered[0].state,"reconcile");
        amounts(&budget(&f,&next.access_token).await,30,50,0);
        let before=f.ledger.balance(f.account).await.unwrap();
        budget(&f,&next.access_token).await;
        let after=f.ledger.balance(f.account).await.unwrap();
        assert_eq!(before.available_units,after.available_units);assert_eq!(before.reserved_units,after.reserved_units);
        assert_eq!(f.provider.hits.load(Ordering::SeqCst),0);
    }).await;
}

#[sqlx::test(migrations = false)]
async fn magpie_budget_rejects_wrong_authority_and_keeps_applications_installations_separate(
    pool: PgPool,
) {
    pi::with_magpie_fixture(pool, true, |f| async move {
        let install = "a".repeat(64);
        let a = f
            .application_token("lmm-magpie", "magpie", 60, &install)
            .await;
        let b = f
            .application_token("lmm-magpie", "magpie", 60, &"b".repeat(64))
            .await;
        let cc = f.managed_token(80, &install).await;
        let pi = f.token(100).await;
        let mut readonly_request = f.request();
        readonly_request.client_id = "lmm".into();
        readonly_request.scope = "catalog:read balance:read".into();
        let pending = f.issuer.begin(readonly_request).await.unwrap();
        let page = f
            .issuer
            .continue_authorization(&pending.binding, &pending.csrf, Some(&f.refresh))
            .await
            .unwrap();
        let redirect = f
            .issuer
            .consent_with_budget(&page.binding, &page.csrf, Some(&f.refresh), true, None)
            .await
            .unwrap();
        let callback = reqwest::Url::parse(&redirect.uri).unwrap();
        let code = callback
            .query_pairs()
            .find(|(key, _)| key == "code")
            .unwrap()
            .1
            .into_owned();
        let readonly_form = lmm_core::identity::grants::TokenForm {
            grant_type: "authorization_code".into(),
            client_id: "lmm".into(),
            resource: f.issuer.profile().resource().into(),
            code: Some(code),
            redirect_uri: Some("http://127.0.0.1:38423/oauth/lmm/callback".into()),
            code_verifier: Some(
                "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-._~".into(),
            ),
            refresh_token: None,
            scope: None,
        };
        let readonly = f
            .issuer
            .token(readonly_form.validate(f.issuer.profile()).unwrap())
            .await
            .unwrap();
        assert!(
            f.issuer
                .managed_application_budget(&readonly.access_token)
                .await
                .is_err()
        );

        let mut wrong = f.managed_request(&install);
        wrong.client_id = "lmm-magpie".into();
        assert!(f.issuer.begin_managed(wrong).await.is_err());
        assert!(
            f.managed_refresh(&a.refresh_token, &install, &"c".repeat(64))
                .await
                .is_err()
        );
        let charge = f
            .reserve(&a.access_token, &f.spec("magpie-private", 50), &[1; 32])
            .await
            .unwrap();
        amounts(&budget(&f, &a.access_token).await, 0, 50, 10);
        amounts(&budget(&f, &b.access_token).await, 0, 0, 60);
        amounts(&budget(&f, &cc.access_token).await, 0, 0, 80);
        for (suffix, bearer) in [
            ("", None),
            ("?grant_id=foreign", Some(a.access_token.as_str())),
            ("", Some(pi.access_token.as_str())),
        ] {
            let mut req = f
                .client
                .get(format!("{}/api/oauth2/application/budget{suffix}", f.base));
            if let Some(bearer) = bearer {
                req = req.bearer_auth(bearer);
            }
            assert!(!req.send().await.unwrap().status().is_success());
        }
        sqlx::query(
            "UPDATE core_oauth.families SET scopes=array_remove(scopes,'usage:read') WHERE id=$1",
        )
        .bind(&b.grant_id)
        .execute(&f.owner)
        .await
        .unwrap();
        assert!(
            f.issuer
                .managed_application_budget(&b.access_token)
                .await
                .is_err()
        );
        f.issuer
            .revoke_for_client("lmm-magpie", &a.refresh_token)
            .await
            .unwrap();
        assert!(
            f.issuer
                .managed_application_budget(&a.access_token)
                .await
                .is_err()
        );
        f.charging.release(&charge.id, &[1; 32]).await.unwrap();
        assert!(
            f.issuer
                .managed_application_budget(&cc.access_token)
                .await
                .is_ok()
        );
        // Isolate storage unavailability while this family's authority is valid.
        let before_fault = budget(&f, &cc.access_token).await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "REVOKE SELECT ON core_charging.requests FROM \"{}\"",
            f.role.name
        )))
        .execute(&f.owner)
        .await
        .unwrap();
        let failed = f
            .client
            .get(format!("{}/api/oauth2/application/budget", f.base))
            .bearer_auth(&cc.access_token)
            .send()
            .await
            .unwrap();
        assert_eq!(failed.status(), 503);
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT SELECT ON core_charging.requests TO \"{}\"",
            f.role.name
        )))
        .execute(&f.owner)
        .await
        .unwrap();
        assert_eq!(budget(&f, &cc.access_token).await, before_fault);

        let current = f
            .application_token("lmm-magpie", "magpie", 30, &"d".repeat(64))
            .await;
        sqlx::query("UPDATE core_identity.developer_access SET granted=false WHERE user_id=$1")
            .bind(f.user)
            .execute(&f.owner)
            .await
            .unwrap();
        assert!(
            f.issuer
                .managed_application_budget(&current.access_token)
                .await
                .is_err()
        );
        sqlx::query("UPDATE core_identity.developer_access SET granted=true WHERE user_id=$1")
            .bind(f.user)
            .execute(&f.owner)
            .await
            .unwrap();
        // Re-granting developer access never revives an old auth generation.
        let revoked = f
            .client
            .get(format!("{}/api/oauth2/application/budget", f.base))
            .bearer_auth(&cc.access_token)
            .send()
            .await
            .unwrap();
        assert_eq!(revoked.status(), 401);

        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 0);
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn magpie_configuration_and_discovery_are_independent(pool: PgPool) {
    pi::with_magpie_fixture(pool, true, |f| async move {
        for (client, application) in [("lmm-magpie", "magpie"), ("lmm-cc-switch", "cc-switch")] {
            let profile =
                LmmProfile::new(f.issuer.profile().issuer(), vec!["allowed".into()]).unwrap();
            let profile = if client == "lmm-magpie" {
                profile.with_magpie_budget_limit(35).unwrap()
            } else {
                profile.with_managed_budget_limit(35).unwrap()
            };
            let issuer = f
                .issuer_for_profile(profile, pi::ManagedKeyring::Configured)
                .await;
            let mut request = f.managed_request(&"a".repeat(64));
            request.client_id = client.into();
            request.application = application.into();
            assert_eq!(
                issuer
                    .begin_managed(request)
                    .await
                    .unwrap()
                    .maximum_budget_credits,
                Some(35)
            );
            let mut wrong = f.managed_request(&"a".repeat(64));
            if client == "lmm-cc-switch" {
                wrong.client_id = "lmm-magpie".into();
                wrong.application = "magpie".into();
            }
            assert!(issuer.begin_managed(wrong).await.is_err());
            // Exercise real Core discovery with this configured issuer.
            let app = lmm_core::oauth2_http::router(issuer);
            use tower::ServiceExt;
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/.well-known/oauth-authorization-server")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let bytes = axum::body::to_bytes(response.into_body(), 16384)
                .await
                .unwrap();
            let meta: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(meta["lmm_client_ids_supported"], json!(["lmm", client]));
            assert_eq!(meta["lmm_managed_clients"][0]["application"], application);
            assert_eq!(meta["lmm_managed_clients"].as_array().unwrap().len(), 1);
        }
    })
    .await;
}

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
async fn magpie_tcp_browser_consent_exchanges_strict_wire_and_live_budget(pool: PgPool) {
    pi::with_magpie_fixture(pool,true,|f|async move {
        let installation="a".repeat(64);
        let mut request=f.managed_request(&installation);
        request.client_id="lmm-magpie".into();request.application="magpie".into();
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
        assert!(html.contains("LMM Magpie"));assert!(html.contains("name=budget_credits"));assert!(html.contains(&installation));
        let response=f.client.post(format!("{}/api/user/auth/oauth2/consent",f.base)).header("origin",origin)
            .header("cookie",format!("{binding}; {browser}")).header("content-type","application/x-www-form-urlencoded")
            .body(form(&[("csrf",&csrf(&html)),("decision","allow"),("budget_credits","100")])).send().await.unwrap();
        assert_eq!(response.status(),303);
        let location=reqwest::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
        let code=location.query_pairs().find(|(k,_)|k=="code").unwrap().1.into_owned();
        let token_body=form(&[("grant_type","authorization_code"),("client_id","lmm-magpie"),("resource",&format!("{origin}/api/oauth2")),
            ("installation_id",&installation),("code",&code),("redirect_uri",&request.redirect_uri),
            ("code_verifier","abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-._~")]);
        let response=f.client.post(format!("{}/api/oauth2/token",f.base)).header("content-type","application/x-www-form-urlencoded").body(token_body.clone()).send().await.unwrap();
        assert_eq!(response.status(),200);
        let token:serde_json::Value=response.json().await.unwrap();assert_eq!(token.as_object().unwrap().len(),9);
        assert_eq!(token["installation_id"],installation);assert_eq!(token["client_id"],"lmm-magpie");assert_eq!(token["budget_credits"],100);
        let response=f.client.get(format!("{}/api/oauth2/application",f.base)).bearer_auth(token["access_token"].as_str().unwrap()).send().await.unwrap();
        assert_eq!(response.status(),200);assert_eq!(response.headers()["cache-control"],"no-store");
        let status:serde_json::Value=response.json().await.unwrap();assert_eq!(status["application"],"magpie");assert_eq!(status["grant_id"],token["grant_id"]);assert_eq!(status["installation_id"],installation);
        let response=f.client.post(format!("{}/api/oauth2/token",f.base)).header("content-type","application/x-www-form-urlencoded").body(token_body).send().await.unwrap();
        assert_eq!(response.status(),400);
        let response=f.client.post(format!("{}/api/oauth2/token",f.base)).header("content-type","application/x-www-form-urlencoded")
            .body("grant_type=refresh_token&client_id=lmm-magpie&installation_id=bad&rotation_id=bad&scope=models%3Ainvoke").send().await.unwrap();
        assert_eq!(response.status(),400);
    }).await;
}

#[sqlx::test(migrations = false)]
async fn magpie_http_model_dispatch_is_metered_by_its_own_family(pool: PgPool) {
    pi::with_magpie_fixture(pool, true, |f| async move {
        let token = f.application_token("lmm-magpie", "magpie", 900, &"e".repeat(64)).await;
        let response = f.client.post(format!("{}/v1/chat/completions", f.base))
            .bearer_auth(&token.access_token).header("x-lmm-group", "YWxsb3dlZA")
            .json(&json!({"model":"provider-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20}))
            .send().await.unwrap();
        assert_eq!(response.status(),200);
        let body:Value=response.json().await.unwrap();
        assert_eq!(body["choices"][0]["message"]["content"],"local synthetic answer");
        assert_eq!(f.provider.hits.load(Ordering::SeqCst),1);
        let matched:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE oauth_source->>'client_id'='lmm-magpie' AND oauth_family_id=$1 AND actor_user_id=$2 AND payer_account_id=$3 AND key_id IS NULL")
            .bind(&token.grant_id).bind(f.user).bind(f.account).fetch_one(&f.owner).await.unwrap();
        assert_eq!(matched,1);
        amounts(&budget(&f,&token.access_token).await,49,0,851);
    }).await;
}

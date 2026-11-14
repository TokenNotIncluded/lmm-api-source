//! Disposable setup for the independent regressions; assertions live in the review suite.
use super::native_http::test_support::Fixture;
use super::responses_plan_observer::PlanTrace;
use super::*;
use crate::{
    billing::charging,
    identity::{
        BrowserAuth, IdentityStore,
        grants::{AuthorizationRequest, LmmIssuer, LmmProfile, TokenRequest},
    },
    relay::{Billing, OutputCheckpoint, RouteProvider},
    routing::{CredentialRef, PriceSnapshot, Protocol, TokenRounding},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;

pub(super) const PUBLIC: &str = "independent-continuity-review";
pub(super) const MODEL: &str = "gpt-4.1-2025-04-14";
pub(super) const SECRET: &str = "independent-review-provider";
pub(super) type GatewayFactory =
    fn(&PgPool, &[String], [Option<bool>; 2], [usize; 2]) -> ModelGateway;
pub(super) struct OAuthToken {
    pub(super) token: String,
    pub(super) client_id: String,
    pub(super) grant_id: String,
}
pub(super) struct OAuthFixture {
    pub(super) gateway: ModelGateway,
    pub(super) first: OAuthToken,
    pub(super) second: OAuthToken,
    pub(super) denied_scope_token: String,
}
#[derive(Clone, Copy)]
pub(super) enum RetryLayout {
    SameGroupNoContinuity,
    SameGroupNonNative,
    LaterGroupNoContinuity,
    LaterGroupNonNative,
}
pub(super) struct RetryFixture {
    pub(super) gateway: ModelGateway,
    pub(super) traces: Arc<Mutex<Vec<PlanTrace>>>,
}
impl RetryLayout {
    pub(super) fn non_native(self) -> bool {
        matches!(self, Self::SameGroupNonNative | Self::LaterGroupNonNative)
    }
    pub(super) fn later_group(self) -> bool {
        matches!(
            self,
            Self::LaterGroupNoContinuity | Self::LaterGroupNonNative
        )
    }
}
fn rebuild(
    original: &ModelGateway,
    document: crate::routing::store::ReleaseDocument,
    contexts: Vec<ExternalModelContext>,
    runtime: &PgPool,
) -> ModelGateway {
    ModelGateway::new_with_provider_policy(
        runtime.clone(),
        Arc::new(
            crate::routing::Router::new(
                document.routing,
                Arc::new(PriceSnapshot::compile(document.pricing).unwrap()),
            )
            .unwrap(),
        ),
        (0..2)
            .map(|index| UpstreamCredential {
                reference: CredentialRef {
                    id: index + 7,
                    revision: 1,
                },
                secret: SECRET.into(),
            })
            .collect(),
        BTreeMap::new(),
        original.limits.clone(),
        original.bridge.policy,
        ProviderPolicy::new(contexts, 800).unwrap(),
    )
    .unwrap()
}

pub(super) async fn seed_legacy_checkpoint(
    factory: GatewayFactory,
    fixture: &Fixture,
    runtime: &PgPool,
    request_id: &str,
    supplier_base: &str,
    corroborated: bool,
) -> Vec<u8> {
    let original = factory(
        runtime,
        &[supplier_base.into(), supplier_base.into()],
        [None, None],
        [0, 1],
    );
    let mut document = original.bridge.routing.catalog_document().unwrap();
    document.routing.routes[0]
        .targets
        .retain(|target| target.upstream_id == 1);
    document.pricing.rules[0].rounding = Some(TokenRounding::LegacyGoRatioV1);
    let gateway = rebuild(
        &original,
        document,
        original.bridge.provider_policy.context_document(),
        runtime,
    )
    .with_legacy_precharge(vec![
        serde_json::from_value(json!({
            "source_revision":charging::legacy_precharge::SOURCE_REVISION,
            "upstream_id":1,"upstream_model":MODEL,"model":PUBLIC,
            "preconsumed_tokens":800,"count_token":true,"get_media_token":true,
            "get_media_token_not_stream":false,"source_free_model":false,
            "operator_max_billable":100000
        }))
        .unwrap(),
    ])
    .unwrap();
    let body = serde_json::to_vec(&json!({"model":PUBLIC,"input":"legacy checkpoint seed",
        "max_output_tokens":50,"store":false}))
    .unwrap();
    let request = relay::Request::parse(ClientProtocol::Responses, &body, &gateway.limits).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", fixture.key)).unwrap(),
    );
    let (principal, secret, groups) = gateway.authenticate(&headers).await.unwrap();
    assert_eq!(groups, ["allowed"]);
    gateway
        .authorize_request(&principal, &secret, PUBLIC, "allowed", None)
        .await
        .unwrap();
    let context = RequestContext {
        request_id: request_id.into(),
        actor_user_id: principal.user_id(),
        key_id: principal.key_id(),
        oauth_source: principal.oauth_source().cloned(),
    };
    let mut worker = [0; 32];
    rand::rand_core::UnwrapErr(SysRng).fill_bytes(&mut worker);
    assert!(
        gateway
            .bridge
            .active
            .lock()
            .unwrap()
            .insert(
                request_id.into(),
                Operation {
                    route_plan: None,
                    secret,
                    group: "allowed".into(),
                    worker,
                    fingerprint: Sha256::digest(&body).into(),
                    selected: None,
                    output_token_ceiling: 50,
                    peer: None,
                    oauth_issuer: None,
                }
            )
            .is_none()
    );
    let route = gateway.bridge.select(&context, &request).await.unwrap();
    let reservation = gateway
        .bridge
        .reserve(&context, &route, &request)
        .await
        .unwrap();
    let wire = request.upstream_body(&route).unwrap();
    gateway
        .bridge
        .dispatching(&context, &route, &wire)
        .await
        .unwrap();
    // The packet witness is the supplier's actual HTTP body, never reconstructed JSON.
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .post(route.endpoint.clone())
        .header(header::AUTHORIZATION, route.credential.clone())
        .header(header::CONTENT_TYPE, "application/json")
        .body(wire)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let bytes = response.bytes().await.unwrap().to_vec();
    let terminal: Value = serde_json::from_slice(&bytes).unwrap();
    let usage = relay::Usage {
        input_tokens: Some(terminal["usage"]["input_tokens"].as_u64().unwrap()),
        output_tokens: Some(terminal["usage"]["output_tokens"].as_u64().unwrap()),
        cached_input_tokens: Some(
            terminal["usage"]["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap(),
        ),
        ..Default::default()
    };
    if corroborated {
        gateway
            .bridge
            .persist_output(
                &reservation,
                &OutputCheckpoint {
                    sequence: 1,
                    packet_sha256: Sha256::digest(&bytes).into(),
                    packet_bytes: bytes.len(),
                    usage,
                    terminal_observed: true,
                },
            )
            .await
            .unwrap();
    } else {
        // The historical gap contains only a genuine worker-authorized checkpoint.
        // Do not add provider observation or terminal evidence to this branch.
        let mut tx = runtime.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SELECT core_model.checkpoint_output($1,$2,1,$3,$4,$5,true)")
            .bind(request_id)
            .bind(worker.as_slice())
            .bind(Sha256::digest(&bytes).as_slice())
            .bind(i64::try_from(bytes.len()).unwrap())
            .bind(serde_json::to_value(usage.checkpoint_usage()).unwrap())
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    gateway
        .bridge
        .charging
        .mark_unknown(request_id, &worker)
        .await
        .unwrap();
    bytes
}

async fn issue(issuer: &LmmIssuer, cookie: &str, client: &str, state: &str) -> String {
    const REDIRECT: &str = "http://127.0.0.1:38423/oauth/lmm/callback";
    const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-._~";
    let scope = if client == "lmm-pi" {
        "catalog:read balance:read usage:read models:invoke"
    } else {
        "catalog:read balance:read"
    };
    let page = issuer
        .begin(AuthorizationRequest {
            response_type: "code".into(),
            client_id: client.into(),
            redirect_uri: REDIRECT.into(),
            scope: scope.into(),
            resource: issuer.profile().resource().into(),
            code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER)),
            code_challenge_method: "S256".into(),
            state: state.into(),
        })
        .await
        .unwrap();
    let page = issuer
        .continue_authorization(&page.binding, &page.csrf, Some(cookie))
        .await
        .unwrap();
    let redirect = issuer
        .consent_with_budget(
            &page.binding,
            &page.csrf,
            Some(cookie),
            true,
            (client == "lmm-pi").then_some(20_000),
        )
        .await
        .unwrap();
    let url = reqwest::Url::parse(&redirect.uri).unwrap();
    let code = url
        .query_pairs()
        .find(|(name, _)| name == "code")
        .unwrap()
        .1
        .into_owned();
    issuer
        .token(TokenRequest::Code {
            client_id: client.into(),
            code,
            redirect: REDIRECT.into(),
            verifier: VERIFIER.into(),
        })
        .await
        .unwrap()
        .access_token
}

pub(super) async fn oauth_fixture(
    factory: GatewayFactory,
    fixture: &Fixture,
    runtime: &PgPool,
    supplier_bases: &[String],
) -> OAuthFixture {
    let gateway = factory(runtime, supplier_bases, [Some(true), Some(true)], [0, 1]);
    // Install only the normal OAuth runtime grant block, using the actual restricted role.
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(runtime)
        .await
        .unwrap();
    let quoted = format!("\"{}\"", role.replace('"', "\"\""));
    let grants = include_str!("../../../../deployment/postgres/grant-core-runtime.sql")
        .split_once("\\if :enable_oauth2\n")
        .unwrap()
        .1
        .split_once("\\endif")
        .unwrap()
        .0
        .replace(":\"runtime_role\"", &quoted);
    sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(&grants))
        .execute(&fixture.pool)
        .await
        .unwrap();
    let identity = IdentityStore::from_pool(runtime.clone())
        .with_dashboard_routing(gateway.bridge.routing.clone());
    let actor = identity.authorize(&fixture.session).await.unwrap();
    let hash = identity
        .hash_installer_password("independent fixture passphrase".into())
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) VALUES('review-oauth',$1,$2)")
        .bind(actor.user_id).bind(hash).execute(&fixture.pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_profiles(user_id,username,password_login_allowed) VALUES($1,'review-oauth',true) ON CONFLICT(user_id) DO UPDATE SET username=EXCLUDED.username,password_login_allowed=true")
        .bind(actor.user_id).execute(&fixture.pool).await.unwrap();
    let browser = BrowserAuth::new(identity.clone(), &[43; 32]).unwrap();
    let bundle = browser
        .login(
            "review-oauth",
            "independent fixture passphrase",
            &Default::default(),
        )
        .await
        .unwrap();
    let profile = LmmProfile::new(
        "https://continuity-oauth.example.test",
        vec!["allowed".into()],
    )
    .unwrap()
    .with_pi_budget_limit(100_000)
    .unwrap();
    let issuer = LmmIssuer::open(identity, profile, 900, 86400)
        .await
        .unwrap()
        .with_catalog_surface(gateway.oauth_catalog_surface());
    let mut tokens = Vec::new();
    for state in ["review-oauth-first", "review-oauth-second"] {
        let token = issue(&issuer, &bundle.refresh_token, "lmm-pi", state).await;
        let mut tx = runtime.begin().await.unwrap();
        let source = issuer
            .authorize_model_in_transaction(&mut tx, &token, None, None)
            .await
            .unwrap();
        tokens.push(OAuthToken {
            token,
            client_id: source.client_id().into(),
            grant_id: source.grant_id().into(),
        });
        tx.commit().await.unwrap();
    }
    let denied_scope_token = issue(
        &issuer,
        &bundle.refresh_token,
        "lmm",
        "review-oauth-readonly",
    )
    .await;
    let second = tokens.pop().unwrap();
    let first = tokens.pop().unwrap();
    OAuthFixture {
        gateway: gateway.with_oauth2_issuer(issuer),
        first,
        second,
        denied_scope_token,
    }
}

pub(super) async fn retry_fixture(
    factory: GatewayFactory,
    fixture: &Fixture,
    runtime: &PgPool,
    supplier_bases: &[String],
    layout: RetryLayout,
    cross_group_retry: bool,
) -> RetryFixture {
    let original = factory(runtime, supplier_bases, [None, Some(true)], [0, 1]);
    let mut document = original.bridge.routing.catalog_document().unwrap();
    let non_native = matches!(
        layout,
        RetryLayout::SameGroupNonNative | RetryLayout::LaterGroupNonNative
    );
    let later_group = matches!(
        layout,
        RetryLayout::LaterGroupNoContinuity | RetryLayout::LaterGroupNonNative
    );
    let mut contexts = original.bridge.provider_policy.context_document();
    if non_native {
        document.routing.upstreams[0].protocol = Protocol::OpenAiChat;
        contexts.retain(|context| context.upstream_id != 1);
    }
    if later_group {
        document.routing.groups = ["review-first", "review-second"]
            .into_iter()
            .map(|name| serde_json::from_value(json!({"name":name,"enabled":true})).unwrap())
            .collect();
        let mut routes = Vec::new();
        let mut prices = Vec::new();
        for (index, group) in ["review-first", "review-second"].into_iter().enumerate() {
            let mut route = document.routing.routes[0].clone();
            route.id = index as u64 + 11;
            route.group = group.into();
            route
                .targets
                .retain(|target| target.upstream_id == index as u64 + 1);
            routes.push(route);
            let mut price = document.pricing.rules[0].clone();
            price.id = index as u64 + 11;
            price.group = group.into();
            prices.push(price);
        }
        document.routing.routes = routes;
        document.pricing.rules = prices;
        let actor = IdentityStore::from_pool(runtime.clone())
            .authorize(&fixture.session)
            .await
            .unwrap();
        for group in ["review-first", "review-second"] {
            sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES($1,$2,true,'independent-continuity-review')")
                .bind(actor.user_id).bind(group).execute(&fixture.pool).await.unwrap();
        }
        sqlx::query("UPDATE core_identity.api_key_settings SET group_name='auto',auto_groups=$2,cross_group_retry=$3 WHERE credential_id=$1")
            .bind(fixture.key_id).bind(vec!["review-first","review-second"]).bind(cross_group_retry)
            .execute(&fixture.pool).await.unwrap();
    } else {
        // The key schema permits cross_group_retry only for an explicit auto group list.
        sqlx::query("UPDATE core_identity.api_key_settings SET group_name='auto',auto_groups=ARRAY['allowed'],cross_group_retry=$2 WHERE credential_id=$1")
            .bind(fixture.key_id).bind(cross_group_retry).execute(&fixture.pool).await.unwrap();
    }
    let gateway = rebuild(&original, document, contexts, runtime);
    let traces = Arc::new(Mutex::new(Vec::new()));
    RetryFixture {
        gateway: gateway.with_continuity_plan_observer(traces.clone()),
        traces,
    }
}

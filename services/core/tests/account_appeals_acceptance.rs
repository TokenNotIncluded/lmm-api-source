//! Independent #971 business API acceptance: real SQLx database and restricted
//! runtime role, merged production Axum routers (no browser/UI acceptance).
//! Evidence is emitted by scenario, without credentials or connection strings.
//! Run only in the main agent's serial cargo queue with the private PG fixture.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use lmm_core::{
    identity::{
        IdentityStore,
        admin_users::{self, AdminUsersStore},
    },
    legacy_auth::{self, LegacyAuthConfig},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

#[path = "common/runtime.rs"]
mod runtime;

const ORIGIN: &str = "https://independent-971.example";
const PASSWORD: &str = "acceptance-971";
const PUBLIC: &str = "Synthetic public suspension notice";
const PRIVATE: &str = "PRIVATE-971-investigation-only";
const NOTE: &str = "PRIVATE-971-reviewer-only";
const APPEAL: &str = "/api/user/account-action-requests/appeal";
const LIST: &str = "/api/account-action-requests";

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    json: Value,
}

async fn http(
    app: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
    bearer: Option<&str>,
    cookie: Option<&str>,
) -> Reply {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("origin", ORIGIN);
    if let Some(secret) = bearer {
        req = req.header("authorization", format!("Bearer {secret}"));
    }
    if let Some(cookie) = cookie {
        req = req.header("cookie", cookie);
    }
    let body = match body {
        Some(value) => {
            req = req.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    Reply {
        status,
        headers,
        json: serde_json::from_slice(&bytes).expect("HTTP response must be JSON"),
    }
}

fn expect(reply: &Reply, status: StatusCode, scenario: &str) {
    // Do not print successful auth bodies: they contain live fixture credentials.
    assert_eq!(reply.status, status, "{scenario}: unexpected HTTP status");
}

fn cookie(reply: &Reply, name: &str) -> String {
    reply
        .headers
        .get_all("set-cookie")
        .iter()
        .map(|h| h.to_str().unwrap())
        .find(|h| h.starts_with(&format!("{name}=")) && !h.contains("Max-Age=0"))
        .expect("expected non-cleared cookie")
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

fn private_projection(reply: &Reply) {
    let text = reply.json.to_string();
    for secret in [PRIVATE, NOTE, "lmma_"] {
        assert!(
            !text.contains(secret),
            "public JSON leaked private information"
        );
    }
    fn fields(value: &Value) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    assert!(
                        !matches!(
                            key.as_str(),
                            "admin_note"
                                | "reviewed_by"
                                | "token"
                                | "access_token"
                                | "refresh_token"
                        ),
                        "unexpected public authority/internal field: {key}"
                    );
                    fields(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    fields(value);
                }
            }
            _ => {}
        }
    }
    fields(&reply.json);
    assert!(
        !reply.headers.contains_key("location"),
        "proof must not travel in a URL"
    );
}

struct Applicant {
    id: i64,
    name: String,
    old_access: String,
    old_refresh: String,
    proof: String,
    request_id: i64,
}

async fn login(app: &Router, name: &str) -> Reply {
    http(
        app,
        "POST",
        "/api/user/login",
        Some(json!({"username":name,"password":PASSWORD})),
        None,
        None,
    )
    .await
}

async fn applicant(app: &Router, admin: &str, name: &str, public: bool) -> Applicant {
    let registered = http(
        app,
        "POST",
        "/api/user/register",
        Some(json!({"username":name,"password":PASSWORD,"accepted_legal":true})),
        None,
        None,
    )
    .await;
    expect(&registered, StatusCode::OK, "register synthetic applicant");
    let original = login(app, name).await;
    expect(&original, StatusCode::OK, "initial applicant login");
    let id = original.json["data"]["user"]["id"].as_i64().unwrap();
    let old_access = original.json["data"]["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let old_refresh = cookie(&original, "new_api_refresh");
    let security_path = format!("/core/v1/admin/user-security/{id}");
    let before = http(app, "GET", &security_path, None, Some(admin), None).await;
    expect(&before, StatusCode::OK, "read suspension version");
    let disabled = http(
        app,
        "POST",
        &format!("{security_path}/actions"),
        Some(json!({
        "expected_auth_version":before.json["auth_version"],
            "action":"disable", "reason":PRIVATE, "public_reason":public.then_some(PUBLIC)
        })),
        Some(admin),
        None,
    )
    .await;
    expect(
        &disabled,
        StatusCode::OK,
        "suspend applicant through admin HTTP",
    );
    let disabled_login = login(app, name).await;
    expect(&disabled_login, StatusCode::FORBIDDEN, "disabled login");
    assert_eq!(disabled_login.json["code"], "AUTH_USER_DISABLED");
    private_projection(&disabled_login);
    assert_eq!(
        disabled_login.json["data"]["public_reason_available"],
        public
    );
    assert_eq!(
        disabled_login.json["data"]["public_reason"],
        json!(public.then_some(PUBLIC))
    );
    let proof = cookie(&disabled_login, "lmm_account_appeal");
    let full = disabled_login
        .headers
        .get_all("set-cookie")
        .iter()
        .map(|h| h.to_str().unwrap())
        .find(|h| h.starts_with("lmm_account_appeal="))
        .unwrap();
    for attribute in [
        "HttpOnly",
        "Max-Age=600",
        "Secure",
        "Path=/api/user/account-action-requests",
    ] {
        assert!(
            full.contains(attribute),
            "missing appeal cookie attribute {attribute}"
        );
    }
    let submitted = http(
        app,
        "POST",
        APPEAL,
        Some(json!({"reason":format!("Please independently review {name}")})),
        None,
        Some(&proof),
    )
    .await;
    expect(&submitted, StatusCode::OK, "submit appeal over HTTP");
    private_projection(&submitted);
    Applicant {
        id,
        name: name.into(),
        old_access,
        old_refresh,
        proof,
        request_id: submitted.json["data"]["id"].as_i64().unwrap(),
    }
}

fn review(item: &Value, note: &str) -> Value {
    json!({"expected_revision":item["revision"], "expected_auth_version":item["auth_version"], "note":note})
}

async fn page(app: &Router, admin: &str, query: &str) -> Reply {
    let reply = http(
        app,
        "GET",
        &format!("{LIST}?{query}"),
        None,
        Some(admin),
        None,
    )
    .await;
    expect(&reply, StatusCode::OK, "admin list");
    assert_eq!(reply.json["success"], true);
    reply
}

async fn business_scenarios(owner: PgPool, runtime: PgPool, admin: String) {
    let restricted: bool = sqlx::query_scalar("SELECT NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolbypassrls FROM pg_roles WHERE rolname=current_user").fetch_one(&runtime).await.unwrap();
    assert!(
        restricted,
        "HTTP application must use a restricted runtime role"
    );
    let store = IdentityStore::from_pool(runtime).with_registration_enabled(true);
    let app = admin_users::router(AdminUsersStore::new(store.clone())).merge(legacy_auth::router(
        store,
        LegacyAuthConfig::new(ORIGIN, &[97; 32]).unwrap(),
    ));
    let a = applicant(&app, &admin, "accept971a", true).await;
    let b = applicant(&app, &admin, "accept971b", false).await;
    let c = applicant(&app, &admin, "accept971c", true).await;
    let ordinary = http(
        &app,
        "POST",
        "/api/user/register",
        Some(json!({"username":"accept971reader","password":PASSWORD,"accepted_legal":true})),
        None,
        None,
    )
    .await;
    expect(&ordinary, StatusCode::OK, "register ordinary reader");
    let ordinary = login(&app, "accept971reader").await;
    expect(&ordinary, StatusCode::OK, "ordinary reader login");
    let ordinary_secret = ordinary.json["data"]["access_token"].as_str().unwrap();
    let all = page(&app, &admin, "status=all").await;
    assert_eq!(all.json["pagination"]["total"], 3);
    let items = all.json["data"].as_array().unwrap();
    let item = |id| items.iter().find(|i| i["id"] == id).unwrap();
    let a_item = item(a.request_id);
    let b_item = item(b.request_id);
    let c_item = item(c.request_id);
    let a_path = format!("{LIST}/{}/approve", a.request_id);
    for (credential, status) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(ordinary_secret), StatusCode::FORBIDDEN),
    ] {
        let listed = http(&app, "GET", LIST, None, credential, None).await;
        expect(&listed, status, "admin list permission");
        for action in ["approve", "reject"] {
            let denied = http(
                &app,
                "POST",
                &format!("{LIST}/{}/{action}", a.request_id),
                Some(review(a_item, NOTE)),
                credential,
                None,
            )
            .await;
            expect(&denied, status, "admin review permission");
        }
    }
    let proof_as_bearer = a.proof.split_once('=').unwrap().1;
    for (method, path, body) in [
        ("GET", LIST.to_owned(), None),
        ("POST", a_path.clone(), Some(review(a_item, NOTE))),
    ] {
        let denied = http(&app, method, &path, body, Some(proof_as_bearer), None).await;
        expect(
            &denied,
            StatusCode::UNAUTHORIZED,
            "appeal proof is not admin authority",
        );
    }
    let mut seen = Vec::new();
    for p in 1..=4 {
        let listed = page(
            &app,
            &admin,
            &format!("status=pending&page={p}&page_size=1"),
        )
        .await;
        assert_eq!(
            listed.json["pagination"],
            json!({"page":p,"page_size":1,"total":3})
        );
        let rows = listed.json["data"].as_array().unwrap();
        assert_eq!(rows.len(), usize::from(p <= 3));
        seen.extend(rows.iter().map(|i| i["id"].as_i64().unwrap()));
    }
    assert_eq!(
        seen,
        vec![a.request_id, b.request_id, c.request_id],
        "pages must neither skip nor duplicate appeals"
    );
    for query in ["page=0", "page_size=0", "page_size=101", "status=unknown"] {
        let invalid = http(
            &app,
            "GET",
            &format!("{LIST}?{query}"),
            None,
            Some(&admin),
            None,
        )
        .await;
        expect(&invalid, StatusCode::BAD_REQUEST, "invalid list contract");
    }
    eprintln!(
        "evidence #971: restricted-role HTTP list pagination, totals, invalid filters and list/review permissions PASS"
    );

    for key in ["expected_revision", "expected_auth_version"] {
        let mut stale = review(a_item, NOTE);
        stale[key] = json!("999999");
        let rejected = http(&app, "POST", &a_path, Some(stale), Some(&admin), None).await;
        expect(&rejected, StatusCode::CONFLICT, "stale review CAS");
    }
    assert_eq!(
        page(&app, &admin, "status=pending").await.json["pagination"]["total"],
        3
    );
    let approved = http(
        &app,
        "POST",
        &a_path,
        Some(review(a_item, "")),
        Some(&admin),
        None,
    )
    .await;
    expect(&approved, StatusCode::OK, "approve current list snapshot");
    assert_eq!(approved.json["data"]["status"], "approved");
    let replay = http(
        &app,
        "POST",
        &a_path,
        Some(review(a_item, "")),
        Some(&admin),
        None,
    )
    .await;
    expect(&replay, StatusCode::OK, "exact HTTP review retry");
    assert_eq!(replay.json, approved.json);
    let conflicting = http(
        &app,
        "POST",
        &format!("{LIST}/{}/reject", a.request_id),
        Some(review(a_item, NOTE)),
        Some(&admin),
        None,
    )
    .await;
    expect(
        &conflicting,
        StatusCode::CONFLICT,
        "opposite review cannot overwrite approval",
    );

    let b_path = format!("{LIST}/{}/reject", b.request_id);
    for note in ["", "x", "  x  "] {
        let invalid = http(
            &app,
            "POST",
            &b_path,
            Some(review(b_item, note)),
            Some(&admin),
            None,
        )
        .await;
        expect(
            &invalid,
            StatusCode::BAD_REQUEST,
            "reject requires at least two trimmed characters",
        );
    }
    let (left, right) = tokio::join!(
        http(
            &app,
            "POST",
            &b_path,
            Some(review(b_item, NOTE)),
            Some(&admin),
            None
        ),
        http(
            &app,
            "POST",
            &b_path,
            Some(review(b_item, "different private reviewer decision")),
            Some(&admin),
            None
        )
    );
    assert!(
        (left.status == StatusCode::OK && right.status == StatusCode::CONFLICT)
            || (right.status == StatusCode::OK && left.status == StatusCode::CONFLICT),
        "competing HTTP reviewers must have one CAS winner"
    );
    let winner = if left.status == StatusCode::OK {
        &left
    } else {
        &right
    };
    let b_context = http(&app, "GET", APPEAL, None, None, Some(&b.proof)).await;
    expect(&b_context, StatusCode::OK, "rejected receipt");
    private_projection(&b_context);
    assert!(
        !b_context
            .json
            .to_string()
            .contains(winner.json["data"]["admin_note"].as_str().unwrap())
    );
    assert_eq!(b_context.json["data"]["request"]["status"], "rejected");
    assert_eq!(b_context.json["data"]["public_reason_available"], false);
    assert!(b_context.json["data"]["public_reason"].is_null());
    expect(
        &login(&app, &b.name).await,
        StatusCode::FORBIDDEN,
        "rejection keeps login disabled",
    );

    // Change suspension generation through the real admin boundary after a list
    // snapshot, rather than constructing a stale request row directly in SQL.
    let revoke = http(&app,"POST",&format!("/core/v1/admin/user-security/{}/actions",c.id),Some(json!({"expected_auth_version":c_item["auth_version"],"action":"revoke_credentials","reason":PRIVATE})),Some(&admin),None).await;
    expect(&revoke, StatusCode::OK, "new suspension generation");
    let current_disabled_login = login(&app, &c.name).await;
    expect(
        &current_disabled_login,
        StatusCode::FORBIDDEN,
        "disabled login after credential revocation",
    );
    private_projection(&current_disabled_login);
    assert_eq!(
        current_disabled_login.json["data"]["public_reason"], PUBLIC,
        "credential revocation must preserve this suspension's explicit public reason"
    );
    assert_eq!(
        current_disabled_login.json["data"]["public_reason_available"],
        true
    );
    let stale = http(
        &app,
        "POST",
        &format!("{LIST}/{}/approve", c.request_id),
        Some(review(c_item, NOTE)),
        Some(&admin),
        None,
    )
    .await;
    expect(
        &stale,
        StatusCode::CONFLICT,
        "list snapshot invalidated by new generation",
    );
    for (status, id) in [
        ("approved", a.request_id),
        ("rejected", b.request_id),
        ("superseded", c.request_id),
    ] {
        let filtered = page(&app, &admin, &format!("status={status}&page_size=1")).await;
        assert_eq!(filtered.json["pagination"]["total"], 1);
        assert_eq!(filtered.json["data"][0]["id"], id);
        assert_eq!(filtered.json["data"][0]["status"], status);
    }
    assert_eq!(page(&app, &admin, "").await.json["pagination"]["total"], 0);
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.audit WHERE action IN ('admin.user.appeal.approve','admin.user.appeal.reject')").fetch_one(&owner).await.unwrap();
    assert_eq!(
        audits, 2,
        "denied, stale and replayed reviews must not create extra decisions"
    );
    eprintln!(
        "evidence #971: HTTP stale revision/auth CAS, competing reviewers, retry and approved/rejected/superseded filters PASS"
    );

    // A new suspension without an explicit public reason must not reuse the
    // previous suspension's explanation, even after a credential-only change.
    let security_path = format!("/core/v1/admin/user-security/{}", c.id);
    for action in ["enable", "disable"] {
        let before = http(&app, "GET", &security_path, None, Some(&admin), None).await;
        expect(
            &before,
            StatusCode::OK,
            "read next suspension decision version",
        );
        let changed = http(&app, "POST", &format!("{security_path}/actions"), Some(json!({"expected_auth_version":before.json["auth_version"],"action":action,"reason":PRIVATE})), Some(&admin), None).await;
        expect(
            &changed,
            StatusCode::OK,
            "begin a distinct suspension round",
        );
    }
    let next_round = login(&app, &c.name).await;
    expect(&next_round, StatusCode::FORBIDDEN, "new suspension login");
    private_projection(&next_round);
    assert_eq!(next_round.json["data"]["public_reason_available"], false);
    assert!(
        next_round.json["data"]["public_reason"].is_null(),
        "a previous suspension's public reason must not cross an enable decision"
    );
    eprintln!(
        "evidence #971: reject note minimum, empty approval note, public reason survives revoke but never crosses suspension rounds PASS"
    );

    let public = http(&app, "GET", APPEAL, None, None, Some(&a.proof)).await;
    expect(&public, StatusCode::OK, "approved public receipt");
    private_projection(&public);
    assert_eq!(public.json["data"]["public_reason"], PUBLIC);
    assert_eq!(public.json["data"]["request"]["status"], "approved");
    for token in [&a.old_access, proof_as_bearer] {
        let unauthorized = http(
            &app,
            "GET",
            "/api/user/self/closure",
            None,
            Some(token),
            None,
        )
        .await;
        expect(
            &unauthorized,
            StatusCode::UNAUTHORIZED,
            "approval does not revive old authority",
        );
    }
    let refresh = http(
        &app,
        "POST",
        "/api/user/auth/refresh",
        None,
        None,
        Some(&a.old_refresh),
    )
    .await;
    expect(
        &refresh,
        StatusCode::UNAUTHORIZED,
        "old refresh stays revoked",
    );
    let fresh = login(&app, &a.name).await;
    expect(&fresh, StatusCode::OK, "fresh login after approval");
    let access = fresh.json["data"]["access_token"].as_str().unwrap();
    assert!(access != a.old_access);
    let preview = http(
        &app,
        "GET",
        "/api/user/self/closure",
        None,
        Some(access),
        None,
    )
    .await;
    expect(&preview, StatusCode::OK, "fresh bearer is usable");
    eprintln!(
        "evidence #971: public receipt hides ban reason/reviewer note; only fresh login restores HTTP authority PASS"
    );

    let closed = http(&app,"DELETE","/api/user/self",Some(json!({"expected_auth_version":preview.json["data"]["expected_auth_version"],"confirmation":a.name,"operation_key":"9719719719719719719719719719719719719719719719719719719719719719"})),Some(access),None).await;
    expect(
        &closed,
        StatusCode::OK,
        "close approved empty synthetic account",
    );
    assert_eq!(closed.json["data"]["closed"], true);
    let retry = http(
        &app,
        "POST",
        &a_path,
        Some(review(a_item, "")),
        Some(&admin),
        None,
    )
    .await;
    assert!(
        matches!(
            retry.status,
            StatusCode::OK | StatusCode::CONFLICT | StatusCode::FORBIDDEN
        ),
        "closed-account review retry must not fail internally"
    );
    // A historical idempotent receipt may be returned; terminal state is the
    // business assertion, regardless of that response's historical status.
    let terminal: (bool,bool,i64) = sqlx::query_as("SELECT u.active,a.active,(SELECT count(*) FROM core_account_lifecycle.closures cl WHERE cl.user_id=u.id) FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=$1").bind(a.id).fetch_one(&owner).await.unwrap();
    assert_eq!(terminal, (false, false, 1));
    let closed_login = login(&app, &a.name).await;
    expect(
        &closed_login,
        StatusCode::UNAUTHORIZED,
        "closed account cannot log in",
    );
    assert!(
        !closed_login
            .headers
            .get_all("set-cookie")
            .iter()
            .any(|h| h.to_str().unwrap().starts_with("lmm_account_appeal=")),
        "closed account must not get a new appeal proof"
    );
    let new_appeal = http(&app,"POST",APPEAL,Some(json!({"username":a.name,"password":PASSWORD,"reason":"Please restore the closed account"})),None,None).await;
    expect(
        &new_appeal,
        StatusCode::UNAUTHORIZED,
        "closure is not appealable",
    );
    let dead = http(
        &app,
        "GET",
        "/api/user/self/closure",
        None,
        Some(access),
        None,
    )
    .await;
    expect(
        &dead,
        StatusCode::UNAUTHORIZED,
        "closure revokes fresh session too",
    );
    eprintln!(
        "evidence #971: real HTTP closure followed by review replay/new appeal cannot restore user/account/session PASS"
    );
}

#[sqlx::test(migrations = false)]
async fn independent_native_appeal_business_http(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let admin = IdentityStore::from_pool(pool.clone())
        .bootstrap_user(900, 5)
        .await
        .unwrap()
        .secret;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("web_identity").await;
    // A failed assertion still closes the application pool and removes the
    // synthetic cluster-level role before surfacing the original task failure.
    let result = tokio::spawn(business_scenarios(pool, role.pool.clone(), admin)).await;
    role.cleanup().await;
    if let Err(error) = result {
        std::panic::resume_unwind(error.into_panic());
    }
}

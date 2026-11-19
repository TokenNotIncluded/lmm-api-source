//! Independent browser-facing merchant contract through real Core identity and Go storage.
use super::{Client, PgPool, StatusCode, Value, json};
use reqwest::Method;

pub struct Actors<'a> {
    pub seller: &'a str,
    pub buyer: &'a str,
    pub buyer_id: i64,
    pub admin: &'a str,
    pub host: &'a str,
}

async fn request(
    client: &Client,
    endpoint: &str,
    method: Method,
    token: Option<&str>,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = client.request(method, format!("{endpoint}/api/store{path}"));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let value = response.json().await.unwrap();
    (status, value)
}

fn success(result: (StatusCode, Value)) -> Value {
    assert_eq!(result.0, StatusCode::OK, "{}", result.1);
    assert_eq!(result.1["success"], true);
    result.1["data"].clone()
}

async fn stored_home(owner: &PgPool, seller: i64) -> Value {
    sqlx::query_scalar(
        "SELECT c.body FROM lmm_store.commands c JOIN lmm_store.shops s \
         ON s.id=c.shop_id WHERE c.id='@web:merchant-home' \
         AND s.body->'owner'->>'kind'='personal' \
         AND (s.body->'owner'->>'id')::bigint=$1",
    )
    .bind(seller)
    .fetch_one(owner)
    .await
    .unwrap()
}

fn profile(biography: &str, version: i64) -> Value {
    json!({"biography":biography,"announcement":"Synthetic merchant notice",
        "header_image":"","expected_version":version})
}

pub async fn before_restart(
    client: &Client,
    endpoint: &str,
    actors: Actors<'_>,
    owner: &PgPool,
) -> Value {
    for token in [None, Some(actors.host)] {
        assert_eq!(
            request(client, endpoint, Method::GET, token, "/my/home", None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let count_before: i64 = sqlx::query_scalar("SELECT count(*) FROM lmm_store.shops")
        .fetch_one(owner)
        .await
        .unwrap();
    let empty = success(
        request(
            client,
            endpoint,
            Method::GET,
            Some(actors.seller),
            "/my/home",
            None,
        )
        .await,
    );
    assert_eq!(empty["seller"]["id"], 9);
    assert_eq!(empty["version"], 0);
    assert_eq!(empty["biography"], "");
    assert_eq!(empty["announcement"], "");
    assert_eq!(empty["header_image"], "");
    assert!(empty["seller"].get("email").is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM lmm_store.shops")
            .fetch_one(owner)
            .await
            .unwrap(),
        count_before,
        "reading a profile must not create a shop"
    );
    // The native published shop already exists. Following its seller link must
    // not require an earlier visit to the profile editor.
    assert_eq!(
        success(request(client, endpoint, Method::GET, None, "/merchants/9", None).await),
        empty
    );
    assert_eq!(
        request(
            client,
            endpoint,
            Method::GET,
            None,
            "/merchants/999999",
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            client,
            endpoint,
            Method::GET,
            Some("invalid-synthetic-session"),
            "/merchants/9",
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );

    let initial = profile("Synthetic shop introduction", 0);
    let first = success(
        request(
            client,
            endpoint,
            Method::PUT,
            Some(actors.seller),
            "/my/home",
            Some(initial.clone()),
        )
        .await,
    );
    assert_eq!(first["version"], 1);
    assert_eq!(first["biography"], initial["biography"]);
    assert_eq!(first["announcement"], initial["announcement"]);
    assert_eq!(
        request(
            client,
            endpoint,
            Method::PUT,
            Some(actors.seller),
            "/my/home",
            Some(initial)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let one = request(
        client,
        endpoint,
        Method::PUT,
        Some(actors.seller),
        "/my/home",
        Some(profile("CAS winner A", 1)),
    );
    let two = request(
        client,
        endpoint,
        Method::PUT,
        Some(actors.seller),
        "/my/home",
        Some(profile("CAS winner B", 1)),
    );
    let (one, two) = tokio::join!(one, two);
    assert!(
        (one.0 == StatusCode::OK && two.0 == StatusCode::CONFLICT)
            || (two.0 == StatusCode::OK && one.0 == StatusCode::CONFLICT),
        "concurrent profile updates must have exactly one winner"
    );
    let home = if one.0 == StatusCode::OK {
        success(one)
    } else {
        success(two)
    };
    assert_eq!(home["version"], 2);
    let persisted = stored_home(owner, 9).await;
    assert_eq!(
        persisted,
        json!({"biography":home["biography"],"announcement":home["announcement"],
        "header_image":home["header_image"],"version":2})
    );
    assert_eq!(
        success(request(client, endpoint, Method::GET, None, "/merchants/9", None).await),
        home
    );

    let mut forged = profile("Cross-owner overwrite", 2);
    forged["owner"] = json!({"kind":"personal","id":9});
    assert_eq!(
        request(
            client,
            endpoint,
            Method::PUT,
            Some(actors.buyer),
            "/my/home",
            Some(forged)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(stored_home(owner, 9).await, persisted);
    let own = success(
        request(
            client,
            endpoint,
            Method::PUT,
            Some(actors.buyer),
            "/my/home",
            Some(profile("Different seller profile", 0)),
        )
        .await,
    );
    assert_eq!(own["seller"]["id"], actors.buyer_id);
    assert_eq!(own["version"], 1);
    assert_eq!(
        stored_home(owner, 9).await,
        persisted,
        "another seller cannot change this profile"
    );

    let mut unsafe_image = profile("Rejected image", 2);
    unsafe_image["header_image"] = json!("javascript:alert(1)");
    assert_eq!(
        request(
            client,
            endpoint,
            Method::PUT,
            Some(actors.seller),
            "/my/home",
            Some(unsafe_image)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(stored_home(owner, 9).await, persisted);

    let empty_notice =
        success(request(client, endpoint, Method::GET, None, "/announcement", None).await);
    assert_eq!(empty_notice["content"], "");
    let notice = json!({"content":"Synthetic store announcement\n\n**Published by the authorized administrator.**"});
    assert_eq!(
        request(
            client,
            endpoint,
            Method::PUT,
            Some(actors.seller),
            "/announcement",
            Some(notice.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        success(
            request(
                client,
                endpoint,
                Method::PUT,
                Some(actors.admin),
                "/announcement",
                Some(notice.clone())
            )
            .await
        ),
        notice
    );
    assert_eq!(
        success(request(client, endpoint, Method::GET, None, "/announcement", None).await),
        notice
    );
    let persisted_notice: Value = sqlx::query_scalar(
        "SELECT body FROM lmm_store.commands WHERE shop_id='@web-legal' AND id='@web:store-announcement'"
    ).fetch_one(owner).await.unwrap();
    assert_eq!(persisted_notice, notice);
    json!({"home":home,"persisted_home":persisted,"announcement":notice,
        "scope":"real Core Session and Go HTTP with separate PostgreSQL databases"})
}

pub async fn after_restart(
    client: &Client,
    endpoint: &str,
    revoked_seller: &str,
    owner: &PgPool,
    expected: &Value,
) {
    assert_eq!(
        success(request(client, endpoint, Method::GET, None, "/merchants/9", None).await),
        expected["home"]
    );
    assert_eq!(stored_home(owner, 9).await, expected["persisted_home"]);
    assert_eq!(
        success(request(client, endpoint, Method::GET, None, "/announcement", None).await),
        expected["announcement"]
    );
    assert_eq!(
        request(
            client,
            endpoint,
            Method::GET,
            Some(revoked_seller),
            "/my/home",
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED,
        "revoked Core sessions remain rejected after both products restart"
    );
}

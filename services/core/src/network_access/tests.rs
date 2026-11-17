use super::*;
use axum::{body::Body, http::Request, routing::get};
use tower::ServiceExt;

async fn pool_policy(pool: &PgPool, rules: &str) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(
        "CREATE SCHEMA core_console; CREATE TABLE core_console.options(key text PRIMARY KEY, value text NOT NULL);"
    )).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO core_console.options(key,value) VALUES ('IPAccessRoutingRules',$1)")
        .bind(rules)
        .execute(pool)
        .await
        .unwrap();
}

fn app(pool: PgPool, database: bool) -> Router {
    let config = TypedConfig {
        geoip_database: database.then(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/geoip/GeoIP2-Country-Test.mmdb")
        }),
    };
    NetworkAccess::new(Some(pool), config).unwrap().protect(
        Router::new()
            .route("/v1/models", get(|| async { StatusCode::OK }))
            .route("/health/live", get(|| async { StatusCode::NO_CONTENT })),
    )
}

fn request(peer: &str) -> Request<Body> {
    let mut request = Request::builder()
        .uri("/v1/models")
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    request
}

#[sqlx::test(migrations = false)]
async fn native_geoip_rejects_before_the_public_handler(pool: PgPool) {
    pool_policy(
        &pool,
        "ip(geoip:JP) -> direct\nfallback: reject\nunknown-country: reject",
    )
    .await;
    let app = app(pool, true);
    assert_eq!(
        app.clone()
            .oneshot(request("[2001:218::1]:4000"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let denied = app
        .clone()
        .oneshot(request("81.2.69.142:4000"))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(denied.headers()["cache-control"], "no-store");
    // IPv4-mapped peers use the same country lookup as IPv4.
    assert_eq!(
        app.clone()
            .oneshot(request("[::ffff:81.2.69.142]:4000"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.clone()
            .oneshot(request("192.0.2.10:4000"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let mut health = request("81.2.69.142:4000");
    *health.uri_mut() = "/health/live".parse().unwrap();
    assert_eq!(
        app.oneshot(health).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrations = false)]
async fn direct_requests_cannot_assert_a_country_or_client_ip(pool: PgPool) {
    pool_policy(
        &pool,
        "ip(203.0.113.7, geoip:JP) -> direct\nfallback: reject\nunknown-country: reject",
    )
    .await;
    let app = app(pool, false);
    let mut spoofed = request("198.51.100.8:4000");
    for (key, value) in [
        ("x-forwarded-for", "203.0.113.7"),
        ("x-real-ip", "203.0.113.7"),
        ("cf-connecting-ip", "203.0.113.7"),
        ("x-lmm-edge-client-ip", "203.0.113.7"),
        ("x-lmm-edge-country", "JP"),
        ("x-lmm-cn-source", "0"),
    ] {
        spoofed.headers_mut().insert(key, value.parse().unwrap());
    }
    assert_eq!(
        app.oneshot(spoofed).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrations = false)]
async fn policy_updates_apply_to_the_next_request_and_empty_is_unrestricted(pool: PgPool) {
    pool_policy(&pool, "").await;
    let app = app(pool.clone(), false);
    assert_eq!(
        app.clone()
            .oneshot(request("192.0.2.10:4000"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    sqlx::query("UPDATE core_console.options SET value='fallback: reject'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        app.clone()
            .oneshot(request("192.0.2.10:4000"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE core_console.options SET value='invalid rule'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        app.oneshot(request("192.0.2.10:4000"))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[cfg(unix)]
#[sqlx::test(migrations = false)]
async fn verified_edge_metadata_reaches_the_native_policy(pool: PgPool) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    pool_policy(
        &pool,
        "ip(geoip:JP) -> direct\nfallback: reject\nunknown-country: reject",
    )
    .await;
    let proof = "network-policy-test-proof-0123456789abcdef0123456789";
    let path =
        std::env::temp_dir().join(format!("lmm-network-proof-{:032x}", rand::random::<u128>()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    file.write_all(proof.as_bytes()).unwrap();
    drop(file);
    let edge = crate::edge_access::EdgeAccess::new(
        pool.clone(),
        crate::edge_access::TypedConfig {
            proof_file: path.clone(),
        },
    )
    .await;
    std::fs::remove_file(path).unwrap();
    let app = edge.unwrap().protect(app(pool, false));
    let mut allowed = request("127.0.0.1:4000");
    allowed
        .headers_mut()
        .insert(crate::edge_access::PROOF_HEADER, proof.parse().unwrap());
    allowed.headers_mut().insert(
        crate::edge_access::CLIENT_HEADER,
        "203.0.113.7".parse().unwrap(),
    );
    allowed
        .headers_mut()
        .insert("x-lmm-edge-country", "JP".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(allowed).await.unwrap().status(),
        StatusCode::OK
    );
    let mut denied = request("127.0.0.1:4000");
    denied
        .headers_mut()
        .insert(crate::edge_access::PROOF_HEADER, proof.parse().unwrap());
    denied.headers_mut().insert(
        crate::edge_access::CLIENT_HEADER,
        "203.0.113.7".parse().unwrap(),
    );
    denied
        .headers_mut()
        .insert("x-lmm-edge-country", "GB".parse().unwrap());
    assert_eq!(
        app.oneshot(denied).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[test]
fn configured_geoip_database_is_required_to_be_valid() {
    assert!(
        NetworkAccess::new(
            None,
            TypedConfig {
                geoip_database: Some("relative.mmdb".into())
            }
        )
        .is_err()
    );
    assert!(
        NetworkAccess::new(
            None,
            TypedConfig {
                geoip_database: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            }
        )
        .is_err()
    );
}

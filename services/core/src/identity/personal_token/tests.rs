use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

#[test]
fn generation_is_canonical_and_scope_cannot_be_supplied() {
    for value in ["-1", "01", "+1", "", "9223372036854775807"] {
        assert_eq!(
            RotateRequest {
                expected_generation: value.into()
            }
            .generation(),
            Err(IdentityError::Invalid)
        );
    }
    assert_eq!(
        RotateRequest {
            expected_generation: "0".into()
        }
        .generation(),
        Ok(0)
    );
    assert!(
        serde_json::from_value::<RotateRequest>(json!({"expected_generation":"0","scope":"admin"}))
            .is_err()
    );
}

#[sqlx::test]
async fn personal_token_rotation_is_session_only_and_never_native_authority(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let owner_store = IdentityStore::from_pool(pool.clone());
    let owner = owner_store.bootstrap_user(710, 5).await.unwrap();
    let other = owner_store.bootstrap_user(711, 0).await.unwrap();
    let role = crate::test_runtime::RuntimeRole::create(&pool).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT USAGE ON SCHEMA core_personal_tokens TO {}; GRANT SELECT,INSERT,UPDATE ON core_personal_tokens.tokens TO {}", role.name, role.name))).execute(&pool).await.unwrap();
    let store = IdentityStore::from_pool(role.pool.clone());
    let response = router(store.clone())
        .oneshot(
            Request::post("/core/v1/me/personal-token/rotate")
                .header("authorization", format!("Bearer {}", owner.secret))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"expected_generation":"0"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    let issued = RotatedToken {
        token: value["token"].as_str().unwrap().into(),
        generation: value["generation"].as_str().unwrap().into(),
        expires_at: value["expires_at"].as_i64().unwrap(),
        scope: SCOPE,
    };
    assert_eq!(issued.scope, SCOPE);
    assert_eq!(
        store
            .personal_token_profile(&issued.token)
            .await
            .unwrap()
            .user_id,
        "710"
    );
    assert!(store.authorize(&issued.token).await.is_err());
    assert!(
        super::super::admin_users::AdminUsersStore::new(store.clone())
            .users(&issued.token, &Default::default())
            .await
            .is_err()
    );
    assert!(
        store
            .rotate_personal_token(
                &issued.token,
                &RotateRequest {
                    expected_generation: "1".into()
                }
            )
            .await
            .is_err()
    );
    let request = RotateRequest {
        expected_generation: "1".into(),
    };
    let first = store.rotate_personal_token(&owner.secret, &request);
    let second = store.rotate_personal_token(&owner.secret, &request);
    let (first, second) = tokio::join!(first, second);
    let rotated = match (first, second) {
        (Ok(value), Err(IdentityError::Conflict)) | (Err(IdentityError::Conflict), Ok(value)) => {
            value
        }
        _ => panic!("exactly one rotation must commit"),
    };
    assert!(store.personal_token_profile(&issued.token).await.is_err());
    assert_eq!(
        store
            .personal_token_profile(&rotated.token)
            .await
            .unwrap()
            .user_id,
        "710"
    );
    assert!(store.authorize(&owner.secret).await.is_ok());
    let other_token = store
        .rotate_personal_token(
            &other.secret,
            &RotateRequest {
                expected_generation: "0".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .personal_token_profile(&other_token.token)
            .await
            .unwrap()
            .user_id,
        "711"
    );
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=710")
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.personal_token_profile(&rotated.token).await.is_err());
    role.cleanup().await;
}

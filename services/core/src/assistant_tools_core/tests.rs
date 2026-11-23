use super::*;
use crate::identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata, IdentityStore};
use serde_json::json;
use sqlx::PgPool;
#[test]
fn closed_read_arguments_reject_authority_and_unknown_names() {
    assert!(TypedReadArgs::parse("get_account_access", br#"{}"#).is_ok());
    for input in [
        br#"{"user_id":1}"#.as_slice(),
        br#"{"payer":1}"#,
        br#"{"root":true}"#,
    ] {
        assert!(TypedReadArgs::parse("get_account_access", input).is_err());
    }
    assert!(TypedReadArgs::parse("arbitrary_sql", br#"{}"#).is_err());
    assert!(TypedReadArgs::parse("list_my_api_keys", br#"{"secret":"x"}"#).is_err());
}
async fn browser(pool: &PgPool, name: &str) -> (IdentityStore, String, i64) {
    let store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![44; 32])])
        .unwrap();
    let browser = BrowserAuth::new(store.clone(), &[45; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: name,
            password: "test-password123",
            email: "",
        })
        .await
        .unwrap();
    let bundle = browser
        .login(
            name,
            "test-password123",
            &BrowserSessionMetadata {
                ip: "203.0.113.1".into(),
                user_agent: "synthetic-browser".into(),
            },
        )
        .await
        .unwrap();
    (
        store,
        bundle.data["access_token"].as_str().unwrap().to_owned(),
        bundle.data["user"]["id"].as_i64().unwrap(),
    )
}
#[sqlx::test(migrations = false)]
async fn read_scope_and_credential_revocation_are_live(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let (store, secret, user) = browser(&pool, "tool-alice").await;
    let (_, bob_secret, bob) = browser(&pool, "tool-bob").await;
    let mut tx = pool.begin().await.unwrap();
    let context = context::from_session(&mut tx, &secret).await.unwrap();
    let own = read_in_transaction(
        &mut tx,
        &context,
        &TypedReadArgs::UserOverview(account::UserTarget::default()),
    )
    .await
    .unwrap();
    assert_eq!(own.output["user"]["id"], user);
    assert!(
        read_in_transaction(
            &mut tx,
            &context,
            &TypedReadArgs::UserOverview(account::UserTarget {
                user_id: Some(bob),
                identifier: None
            })
        )
        .await
        .is_err()
    );
    let keys = read_in_transaction(
        &mut tx,
        &context,
        &TypedReadArgs::ListMyApiKeys(keys::ListMyApiKeys::default()),
    )
    .await
    .unwrap();
    assert_eq!(keys.output["credentials_returned"], false);
    let encoded = serde_json::to_string(&keys.output).unwrap();
    assert!(!encoded.contains(&secret));
    assert!(!encoded.contains(&bob_secret));
    let risk = read_in_transaction(&mut tx, &context, &TypedReadArgs::RegistrationRisk)
        .await
        .unwrap();
    assert_eq!(
        risk.output,
        json!({"ok":false,"status":"verification_unavailable"})
    );
    let support = read_in_transaction(
        &mut tx,
        &context,
        &TypedReadArgs::TechnicalSupportEligibility,
    )
    .await
    .unwrap();
    assert_eq!(support.output["eligible"], false);
    tx.commit().await.unwrap();
    store
        .revoke(&secret, context.credential_id())
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(context.revalidate(&mut tx).await.is_err());
    tx.rollback().await.unwrap();
}

#[test]
fn implemented_manifest_is_closed_and_self_hashed() {
    use sha2::{Digest, Sha256};
    let manifest = implemented_manifest().unwrap();
    for entry in manifest.as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let arguments = if name == "get_model_pricing" {
            br#"{"model_id":"synthetic-model"}"#.as_slice()
        } else {
            br#"{}"#
        };
        assert!(TypedReadArgs::parse(name, arguments).is_ok());
        let digest = Sha256::digest(serde_json::to_vec(&entry["parameters"]).unwrap());
        let digest: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(entry["schema_sha256"], digest);
        assert_eq!(entry["parameters"]["additionalProperties"], false);
        let price = Sha256::digest(serde_json::to_vec(&entry["price"]).unwrap());
        let price: String = price.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(entry["price_sha256"], price);
        assert_eq!(entry["domain"], "registered_core");
        assert_eq!(entry["effect"], "read_only");
    }
}

#[test]
fn administrative_catalog_arguments_cannot_choose_authority_or_mutations() {
    for name in [
        "get_admin_channels",
        "get_admin_model_inventory",
        "audit_admin_model_pricing",
    ] {
        assert!(TypedReadArgs::parse(name, br#"{}"#).is_ok());
        for raw in [
            br#"{"root":true}"#.as_slice(),
            br#"{"user_id":7}"#,
            br#"{"action":"publish"}"#,
            br#"{"endpoint":"https://example.com"}"#,
        ] {
            assert!(TypedReadArgs::parse(name, raw).is_err());
        }
    }
    assert!(TypedReadArgs::parse("audit_admin_model_pricing", br#"{"limit":1.5}"#).is_err());
    assert!(TypedReadArgs::parse("audit_admin_model_pricing", br#"{"model_ids":null}"#).is_err());
    assert!(TypedReadArgs::parse("publish_admin_model_pricing", br#"{}"#).is_err());
}

async fn catalog_browser_for_role(pool: &PgPool, name: &str, role: &str) -> (String, i64) {
    assert!(matches!(role, "admin" | "superadmin"));
    let (store, old_secret, user) = browser(pool, name).await;
    // The normal role trigger advances the generation. Issue a new browser
    // Session through real password login; never rewrite an old credential.
    sqlx::query("UPDATE core_identity.users SET platform_role=$2 WHERE id=$1")
        .bind(user)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    let mut stale = pool.begin().await.unwrap();
    assert!(matches!(
        context::from_session(&mut stale, &old_secret).await,
        Err(Error::Unauthorized)
    ));
    stale.rollback().await.unwrap();
    let auth = BrowserAuth::new(store, &[45; 32]).unwrap();
    let bundle = auth
        .login(
            name,
            "test-password123",
            &BrowserSessionMetadata {
                ip: "203.0.113.1".into(),
                user_agent: "synthetic-browser".into(),
            },
        )
        .await
        .unwrap();
    (
        bundle.data["access_token"].as_str().unwrap().to_owned(),
        user,
    )
}

async fn catalog_policy_fixture(c: &mut sqlx::PgConnection, user: i64, incomplete: bool) {
    use crate::identity::permissions;
    let entries: Vec<_> = permissions::Action::CATALOG
        .iter()
        .map(|action| {
            (
                action.catalog_name().unwrap().to_owned(),
                if *action == permissions::Action::ChannelRead {
                    "deny"
                } else {
                    "allow"
                }
                .to_owned(),
            )
        })
        .collect();
    let policy = permissions::complete_policy_snapshot(user, 1, &entries).unwrap();
    let payload = permissions::canonical_policy_payload(&policy).unwrap();
    let digest = permissions::policy_sha256(&policy).unwrap();
    sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES ($1,1,$2,$3,$4,TRUE)")
        .bind(user).bind(permissions::CONTRACT_VERSION).bind(digest.as_slice()).bind(payload)
        .execute(&mut *c).await.unwrap();
    for (action, effect) in entries {
        if incomplete && action == "ChannelRead" {
            continue;
        }
        sqlx::query("INSERT INTO core_identity.permission_policy_rows(user_id,action,effect) VALUES ($1,$2,$3)")
            .bind(user).bind(action).bind(effect).execute(&mut *c).await.unwrap();
    }
}

#[sqlx::test(migrations = false)]
async fn administrative_catalog_uses_live_session_policy_and_real_serving_prices(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let (_, ordinary_secret, _) = browser(&pool, "admin-catalog-reader").await;
    let gateway = crate::model_http::pricing::tests::gateway(pool.clone());
    let names = [
        "get_admin_channels",
        "get_admin_model_inventory",
        "audit_admin_model_pricing",
    ];
    let mut ordinary = pool.begin().await.unwrap();
    let ordinary_context = context::from_session(&mut ordinary, &ordinary_secret)
        .await
        .unwrap();
    for name in names {
        let args = TypedReadArgs::parse(name, br#"{}"#).unwrap();
        assert!(matches!(
            read_with_gateway_in_transaction(&mut ordinary, &ordinary_context, &args, &gateway)
                .await,
            Err(Error::Forbidden)
        ));
    }
    ordinary.rollback().await.unwrap();

    // Each role has a real Session issued after its generation changed.
    for (role, username) in [
        ("admin", "admin-catalog-admin"),
        ("superadmin", "admin-catalog-root"),
    ] {
        let (secret, user) = catalog_browser_for_role(&pool, username, role).await;
        let mut tx = pool.begin().await.unwrap();
        let context = context::from_session(&mut tx, &secret).await.unwrap();
        let ledger_before: i64 =
            sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        for name in names {
            let args = TypedReadArgs::parse(name, br#"{}"#).unwrap();
            let result = read_with_gateway_in_transaction(&mut tx, &context, &args, &gateway)
                .await
                .unwrap();
            assert_eq!(result.tool_name, name);
            assert_eq!(result.output["read_only"], true);
            assert_eq!(result.output["routing_version"], 9);
            assert_eq!(result.output["pricing_version"], 11);
            let encoded = result.output.to_string();
            for forbidden in [
                secret.as_str(),
                "synthetic-pricing-private-provider-credential",
                "127.0.0.1",
                "credential",
                "endpoint",
            ] {
                assert!(!encoded.contains(forbidden));
            }
            if name == "get_admin_channels" {
                assert_eq!(result.output["total_channels"], 5);
                assert!(result.output["channels"][0]["name"].is_null());
            } else if name == "get_admin_model_inventory" {
                assert!(
                    result.output["model_ids"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|id| id == "model")
                );
                assert!(result.output["missing_model_ids"].is_null());
                assert_eq!(result.output["missing_model_inventory_available"], false);
            } else {
                let model = result.output["models"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["model_id"] == "model")
                    .unwrap();
                let allowed = model["groups"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["group"] == "allowed")
                    .unwrap();
                assert_eq!(allowed["token_price"]["input"].to_string(), "2.5");
            }
        }
        let ledger_after: i64 =
            sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(ledger_after, ledger_before);

        // Offline policy records are immutable even to the owner. Each scenario
        // starts from the same no-policy snapshot by rolling back its inserts.
        sqlx::query("SAVEPOINT no_policy_fixture")
            .execute(&mut *tx)
            .await
            .unwrap();
        catalog_policy_fixture(&mut tx, user, false).await;
        for name in names {
            let args = TypedReadArgs::parse(name, br#"{}"#).unwrap();
            let actual = read_with_gateway_in_transaction(&mut tx, &context, &args, &gateway)
                .await
                .map(|_| ());
            assert_eq!(
                actual,
                Err(Error::Forbidden),
                "explicit-deny result: role={role}, tool={name}, Session issued by BrowserAuth::login after the role update"
            );
        }
        sqlx::query("ROLLBACK TO SAVEPOINT no_policy_fixture")
            .execute(&mut *tx)
            .await
            .unwrap();
        catalog_policy_fixture(&mut tx, user, true).await;
        assert_eq!(
            admin_catalog::authorize(&mut tx, &context).await,
            Err(Error::Forbidden)
        );
        sqlx::query("ROLLBACK TO SAVEPOINT no_policy_fixture")
            .execute(&mut *tx)
            .await
            .unwrap();

        let hash = [7_u8; 32];
        sqlx::query("INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) VALUES ($1,$2,$2,$2,$2,$3,$2,1,$2,$2,TRUE,FALSE,TRUE,TRUE,$2,$2,$4,FALSE,1)")
            .bind(user).bind(hash.as_slice()).bind(format!("admin-catalog-fixture-{user}")).bind(role)
            .execute(&mut *tx).await.unwrap();
        assert_eq!(
            admin_catalog::authorize(&mut tx, &context).await,
            Err(Error::Forbidden),
            "an imported administrator cannot fall back to role-only access"
        );
        sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=$1")
            .bind(user)
            .execute(&mut *tx)
            .await
            .unwrap();
        assert_eq!(
            admin_catalog::authorize(&mut tx, &context).await,
            Err(Error::Unauthorized)
        );
        tx.rollback().await.unwrap();
    }
}

//! Synthetic source facts exercise the normal owner protocol and actual PG guards.
//! These fixtures never claim to be an actual historical source import.
use super::super::{
    Approval, FrozenSource, PlatformRole, TargetInstance, digest_text, import_run_digest,
    permissions, snapshot_from_binding, system_id,
};
use super::*;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

fn h(byte: u8) -> [u8; 32] {
    [byte; 32]
}
struct Fixture {
    connection: sqlx::pool::PoolConnection<Postgres>,
    request: PlanRequest,
}
fn write(path: &std::path::Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
async fn fixture(pool: &sqlx::PgPool) -> Fixture {
    crate::core_schema::init(pool).await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("SET default_transaction_isolation='serializable'")
        .execute(&mut *conn)
        .await
        .unwrap();
    let oid: i64 =
        sqlx::query_scalar("SELECT oid::bigint FROM pg_database WHERE datname=current_database()")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    let identifier: i64 = sqlx::query_scalar("SELECT system_identifier FROM pg_control_system()")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    let nonce = h(9);
    sqlx::query("SELECT set_config('lmm.migration_instance_nonce',$1,false)")
        .bind(encode_hex_32(&nonce))
        .execute(&mut *conn)
        .await
        .unwrap();
    let entries = permissions::Action::CATALOG
        .iter()
        .map(|a| (a.catalog_name().unwrap().to_owned(), "allow".to_owned()))
        .collect::<Vec<_>>();
    let (snapshot, policy) = snapshot_from_binding(11, 3, &entries).unwrap();
    let approval = Approval {
        id: h(1),
        approver: h(2),
        expiry_unix: 4_000_000_000,
        source: FrozenSource {
            archive_sha256: h(3),
            a2_semantic_sha256: h(4),
            a2_file_sha256: h(5),
            a2_sql_sha256: h(6),
            migration_id: "synthetic-executor-mig-7".into(),
            user_id: 11,
            personal_account_id: 12,
            enabled: true,
            deleted: false,
            password_verifier_recognized: true,
            password_login_enabled: true,
            password_hash_sha256: digest_text("synthetic-password-verifier-not-a-credential"),
            login_name_sha256: digest_text("legacy-11"),
            second_factor_required: false,
            role: PlatformRole::Root,
            auth_version: 4,
        },
        target: TargetInstance {
            system_id: system_id(identifier),
            database_oid: oid,
            instance_nonce: nonce,
            import_run: import_run_digest("synthetic-executor-mig-7"),
        },
        policy,
        policy_snapshot: snapshot,
    };
    sqlx::query("INSERT INTO core_identity.accounts(id,kind,active) OVERRIDING SYSTEM VALUE VALUES (12,'personal',FALSE)")
        .execute(&mut *conn).await.unwrap();
    sqlx::query("INSERT INTO core_identity.users(id,personal_account_id,platform_role,active,auth_version) VALUES (11,12,'superadmin',FALSE,4)")
        .execute(&mut *conn).await.unwrap();
    // True A2 dormant gate is FALSE; the frozen source global flag is TRUE.
    sqlx::query("INSERT INTO core_identity.user_profiles(user_id,username,password_login_allowed,second_factor_required) VALUES (11,'root-import',FALSE,FALSE)")
        .execute(&mut *conn).await.unwrap();
    sqlx::query("INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) VALUES ('legacy-11',11,'synthetic-password-verifier-not-a-credential')")
        .execute(&mut *conn).await.unwrap();
    sqlx::query("INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) VALUES (11,$1,$2,$3,$4,$5,$6,$7,$8,$9,TRUE,FALSE,TRUE,TRUE,$10,$11,'superadmin',FALSE,4)")
        .bind(approval.source.archive_sha256.as_slice()).bind(approval.source.a2_semantic_sha256.as_slice())
        .bind(approval.source.a2_file_sha256.as_slice()).bind(approval.source.a2_sql_sha256.as_slice())
        .bind(&approval.source.migration_id).bind(approval.target.system_id.as_slice()).bind(oid)
        .bind(nonce.as_slice()).bind(approval.target.import_run.as_slice())
        .bind(approval.source.password_hash_sha256.as_slice()).bind(approval.source.login_name_sha256.as_slice())
        .execute(&mut *conn).await.unwrap();

    let mut directory_tag = [0_u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut directory_tag);
    let root = std::env::temp_dir().join(format!(
        "lmm-admin-key-continuity-{}",
        encode_hex_32(&directory_tag)
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let files = MaterialFiles {
        identity_manifest: root.join("identity.json"),
        identity_map: root.join("map.jsonl"),
        credential_receipt: root.join("credentials.json"),
        alias_manifest: root.join("aliases.json"),
        source_projection: root.join("source.json"),
    };
    let map = json!({"source_user_id":11,"target_user_id":11,"target_personal_account_id":12});
    let map_bytes = format!("{}\n", serde_json::to_string(&map).unwrap()).into_bytes();
    write(&files.identity_map, &map_bytes);
    let manifest = json!({"scope":"identity-only","source_archive_sha256":encode_hex_32(&approval.source.archive_sha256),
        "mapping_file_sha256":encode_hex_32(&approval.source.a2_file_sha256),"source_target_id_map_sha256":hash(&map_bytes)});
    write(
        &files.identity_manifest,
        &serde_json::to_vec(&manifest).unwrap(),
    );
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES($1,$2,$3,$4)")
        .bind(&approval.source.migration_id).bind(encode_hex_32(&approval.source.archive_sha256))
        .bind(encode_hex_32(&approval.source.a2_semantic_sha256)).bind(&manifest).execute(&mut *conn).await.unwrap();
    let raw_user = json!({"id":11,"status":1,"deleted_at":null,"role":100,"auth_version":4});
    let mut raw_tokens = Vec::new();
    let mut source_keys = Vec::new();
    let mut records = Vec::new();
    let mut aliases = Vec::new();
    for id in 100..106 {
        let secret = if id == 102 {
            format!("oauth_managed_{}", "C".repeat(50))
        } else {
            char::from_u32(65 + (id - 100) as u32)
                .unwrap()
                .to_string()
                .repeat(64)
        };
        let deleted = id == 101;
        let managed = id == 102;
        let status = if id == 103 { 2 } else { 1 };
        let expiry = if id == 104 { 2 } else { 253402300799_i64 };
        let digest = Sha256::digest(secret.as_bytes()).to_vec();
        let hex = encode_hex_32(&digest.clone().try_into().unwrap());
        sqlx::query("INSERT INTO core_identity.credentials(id,digest,kind,user_id,user_version,owner_account_id,expires_at,revoked_at,personal_billing_preference) OVERRIDING SYSTEM VALUE VALUES($1,$2,'api_key',11,4,12,to_timestamp($3),CASE WHEN $4 THEN clock_timestamp() ELSE NULL END,'wallet_only')")
            .bind(id).bind(&digest).bind(expiry as f64).bind(deleted||managed).execute(&mut *conn).await.unwrap();
        sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,enabled,one_time_reveal,group_name) VALUES($1,'source',$2,TRUE,'default')")
            .bind(id).bind(status==1).execute(&mut *conn).await.unwrap();
        sqlx::query("INSERT INTO core_identity.key_funding_rules(credential_id,position,payer_account_id) VALUES($1,0,12)")
            .bind(id).execute(&mut *conn).await.unwrap();
        sqlx::query("INSERT INTO core_charging.budgets(scope,key_id,issuer_user_id,period,limit_credits) VALUES('key',$1,11,'lifetime',100)")
            .bind(id).execute(&mut *conn).await.unwrap();
        sqlx::query("INSERT INTO core_charging.budget_opening_consumption(scope,key_id,period,start_at,end_at,consumed_credits,import_reference) VALUES('key',$1,'lifetime',-62135596800,253402300800,17,$2)")
            .bind(id).bind(format!("key-quota:fixture-keys:{id}")).execute(&mut *conn).await.unwrap();
        let raw = json!({"id":id,"user_id":11,"key":secret,"deleted_at":if deleted{json!("2020-01-01")}else{Value::Null},
            "oauth_managed":managed,"creation_source":"","status":status,"used_quota":17,"remain_quota":83,"unlimited_quota":false,"expired_time":if id==104{1}else{-1}});
        source_keys.push(SourceKey {
            source_key_id: id,
            source_user_id: 11,
            source_secret_sha256: hex.clone(),
            source_row_sha256: source_row_hash(&raw).unwrap(),
            deleted,
            managed,
            status,
            used_quota: 17,
            remain_quota: 83,
            unlimited_quota: false,
            expires_at_unix: expiry,
        });
        records.push(json!({"source_key_id":id,"target_credential_id":id,"target_user_id":11,"target_account_id":12,"source_secret_sha256":hex}));
        aliases.push(json!({"source_key_id":id,"target_credential_id":id,"target_user_id":11,"target_account_id":12,"source_secret_sha256":hex,"credential_user_version":4,"revoked_rekey_required":managed}));
        raw_tokens.push(raw);
    }
    let receipt = json!({"scope":"a02-credential-mapping-receipt","source_archive_sha256":encode_hex_32(&approval.source.archive_sha256),"records":records});
    let bytes = serde_json::to_vec(&receipt).unwrap();
    write(&files.credential_receipt, &bytes);
    write(&files.alias_manifest,&serde_json::to_vec(&json!({"version":1,"scope":"legacy-api-key-wire-aliases-v1",
        "source_archive_sha256":encode_hex_32(&approval.source.archive_sha256),"credential_receipt_sha256":hash(&bytes),
        "target":{"database_oid":oid,"instance_nonce":encode_hex_32(&nonce)},"keys":aliases})).unwrap());
    let source_receipt = root.join("source-receipt.json");
    let source_rows = root.join("source-rows.json");
    let source_sql = root.join("keys.sql");
    let source_receipt_bytes = serde_json::to_vec(
        &json!({"archive":{"sha256":encode_hex_32(&approval.source.archive_sha256)}}),
    )
    .unwrap();
    write(&source_receipt, &source_receipt_bytes);
    write(
        &source_rows,
        &serde_json::to_vec(&json!({"users":[raw_user],"tokens":raw_tokens})).unwrap(),
    );
    write(&source_sql, b"synthetic-normal-key-import");
    let projection = SourceProjection {
        scope: "legacy-admin-key-continuity-source.v1".into(),
        source_archive_sha256: encode_hex_32(&approval.source.archive_sha256),
        source_receipt_sha256: hash(&source_receipt_bytes),
        source_receipt_file: source_receipt,
        source_rows_file: source_rows,
        key_import_sql_file: source_sql,
        observed_at: 1,
        identity_migration_id: approval.source.migration_id.clone(),
        key_import_migration_id: "fixture-keys".into(),
        key_import_sql_sha256: hash(b"synthetic-normal-key-import"),
        source_user_id: 11,
        source_user_sha256: source_row_hash(&raw_user).unwrap(),
        source_status: 1,
        source_deleted: false,
        source_role: 100,
        source_auth_version: 4,
        keys: source_keys,
    };
    write(
        &files.source_projection,
        &serde_json::to_vec(&projection).unwrap(),
    );
    let request = PlanRequest {
        activation: PlanAdminActivationRequest {
            approval_id: encode_hex_32(&approval.id),
            approver: encode_hex_32(&approval.approver),
            expiry_unix: approval.expiry_unix,
            user_id: 11,
            policy_revision: 3,
            policy: approval
                .policy_snapshot
                .decisions
                .iter()
                .map(|d| {
                    (
                        d.action.catalog_name().unwrap().to_owned(),
                        d.effect.catalog_name().to_owned(),
                    )
                })
                .collect(),
        },
        materials: files,
    };
    Fixture {
        connection: conn,
        request,
    }
}

fn approved(response: PlanResponse) -> ApplyRequest {
    ApplyRequest {
        confirmation_sha256: response.confirmation_sha256,
        activation_confirmation_sha256: response.activation_confirmation_sha256,
        approval: response.approval,
        continuity: response.continuity,
    }
}

#[sqlx::test(migrations = false)]
async fn continuity_is_atomic_replay_safe_and_cannot_revive_or_chain(pool: sqlx::PgPool) {
    let mut f = fixture(&pool).await;
    let request = approved(plan(&mut f.connection, &f.request).await.unwrap());
    assert_eq!(request.continuity.keys.len(), 6);
    let result = apply(&mut f.connection, &request).await.unwrap();
    assert_eq!(result.status, "applied");
    assert_eq!(result.auth_version, Some(5));
    let key = crate::identity::authorize_in_transaction(&mut f.connection, &"A".repeat(64))
        .await
        .unwrap();
    assert_eq!(
        (
            key.user_id,
            key.owner_account_id,
            key.credential_generation()
        ),
        (11, 12, 4)
    );
    assert_eq!(
        apply(&mut f.connection, &request).await.unwrap().status,
        "replay"
    );
    for secret in [
        "B".repeat(64),
        format!("oauth_managed_{}", "C".repeat(50)),
        "D".repeat(64),
        "E".repeat(64),
    ] {
        assert!(matches!(
            crate::identity::authorize_in_transaction(&mut f.connection, &secret).await,
            Err(crate::identity::IdentityError::Unauthorized)
        ));
    }
    // Normal revocation is permanent, including after replay of this owner operation.
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=100")
        .execute(&mut *f.connection)
        .await
        .unwrap();
    assert_eq!(
        apply(&mut f.connection, &request).await.unwrap().status,
        "replay"
    );
    assert!(
        crate::identity::authorize_in_transaction(&mut f.connection, &"A".repeat(64))
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE core_identity.credentials SET revoked_at=NULL WHERE id=100")
            .execute(&mut *f.connection)
            .await
            .is_err()
    );
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=11")
        .execute(&mut *f.connection)
        .await
        .unwrap();
    assert!(
        crate::identity::authorize_in_transaction(&mut f.connection, &"F".repeat(64))
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE core_identity.credentials SET user_version=6 WHERE id=105")
            .execute(&mut *f.connection)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = false)]
async fn stale_settings_or_quota_refuses_entire_activation(pool: sqlx::PgPool) {
    let mut f = fixture(&pool).await;
    let request = approved(plan(&mut f.connection, &f.request).await.unwrap());
    sqlx::query(
        "UPDATE core_identity.api_key_settings SET group_name='changed' WHERE credential_id=100",
    )
    .execute(&mut *f.connection)
    .await
    .unwrap();
    assert!(matches!(
        apply(&mut f.connection, &request).await,
        Err(ApprovalError::VersionConflict)
    ));
    let active: bool = sqlx::query_scalar("SELECT active FROM core_identity.users WHERE id=11")
        .fetch_one(&mut *f.connection)
        .await
        .unwrap();
    assert!(!active);
    let request = approved(plan(&mut f.connection, &f.request).await.unwrap());
    sqlx::query(
        "UPDATE core_charging.budgets SET limit_credits=101,revision=revision+1 WHERE key_id=100",
    )
    .execute(&mut *f.connection)
    .await
    .unwrap();
    assert!(apply(&mut f.connection, &request).await.is_err());
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_identity.admin_activation_receipts")
            .fetch_one(&mut *f.connection)
            .await
            .unwrap();
    assert_eq!(receipts, 0);
}

#[sqlx::test(migrations = false)]
async fn ordinary_activation_cannot_be_retroactively_bridged(pool: sqlx::PgPool) {
    let mut f = fixture(&pool).await;
    let request = approved(plan(&mut f.connection, &f.request).await.unwrap());
    let ordinary = super::super::ApplyAdminActivationRequest {
        confirmation_sha256: request.activation_confirmation_sha256.clone(),
        approval: request.approval.clone(),
    };
    super::super::apply_admin_activation(&mut f.connection, &ordinary)
        .await
        .unwrap();
    assert!(matches!(
        apply(&mut f.connection, &request).await,
        Err(ApprovalError::ConfirmationMismatch)
    ));
    assert!(
        crate::identity::authorize_in_transaction(&mut f.connection, &"A".repeat(64))
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.credentials WHERE revoked_at IS NULL",
    )
    .fetch_one(&mut *f.connection)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

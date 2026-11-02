//! Strict conversion of legacy console policy inputs to Core pricing authority.
//! Dedicated owner-passkey authority gates the preview/challenge/confirm service.
use crate::billing::trust_pricing::{DiscountRatio, TrustPolicy, TrustTier};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod editor;
#[cfg(test)]
mod editor_tests;
pub mod http;
mod store;
pub use editor::{Challenge, Confirmation, EditorConfig};
pub use store::{
    CreatedPreview, EditorAvailability, Error, PolicyRead, RegisteredEditor, Resolution, Service,
    SigningKey,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConsolePolicy {
    version: u32,
    paid_activation_enabled: bool,
    decay_period_days: u16,
    tiers: Vec<ConsoleTier>,
    #[serde(default = "default_roles", deserialize_with = "nullable_roles")]
    role_tiers: Vec<ConsoleRole>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConsoleTier {
    level: u8,
    min_paid_credits: i64,
    discount_ratio: serde_json::Number,
    #[serde(default, deserialize_with = "nullable_benefits")]
    benefits: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConsoleRole {
    level: u8,
    role: u8,
    discount_ratio: serde_json::Number,
    #[serde(default, deserialize_with = "nullable_benefits")]
    benefits: Vec<String>,
}

fn nullable_benefits<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

fn nullable_roles<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ConsoleRole>, D::Error> {
    Ok(Option::<Vec<ConsoleRole>>::deserialize(deserializer)?.unwrap_or_else(default_roles))
}

// The frozen legacy parser supplies these role descriptions and discounts when
// role_tiers is omitted; this is not a reconstruction of an imported policy.
fn default_roles() -> Vec<ConsoleRole> {
    [
        (5, 10, "administrator_access"),
        (6, 100, "superadministrator_access"),
    ]
    .into_iter()
    .map(|(level, role, access)| ConsoleRole {
        level,
        role,
        discount_ratio: serde_json::Number::from_f64(0.9).expect("finite constant"),
        benefits: vec![access.into(), "usage_discount".into()],
    })
    .collect()
}

fn decimal_ratio(value: &serde_json::Number) -> Result<DiscountRatio, &'static str> {
    let text = value.to_string();
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (
            mantissa,
            exponent.parse::<i32>().map_err(|_| "invalid ratio")?,
        ),
        None => (text.as_str(), 0),
    };
    if mantissa.starts_with('-') {
        return Err("invalid ratio");
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    let numerator = digits.parse::<u128>().map_err(|_| "invalid ratio")?;
    let scale = i32::try_from(fraction.len())
        .map_err(|_| "invalid ratio")?
        .checked_sub(exponent)
        .ok_or("invalid ratio")?;
    let (mut n, mut d) = if scale >= 0 {
        (
            numerator,
            10u128.checked_pow(scale as u32).ok_or("invalid ratio")?,
        )
    } else {
        (
            numerator
                .checked_mul(
                    10u128
                        .checked_pow(scale.unsigned_abs())
                        .ok_or("invalid ratio")?,
                )
                .ok_or("invalid ratio")?,
            1,
        )
    };
    if n == 0 || n > d {
        return Err("invalid ratio");
    }
    let (mut a, mut b) = (n, d);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    n /= a;
    d /= a;
    DiscountRatio::new(
        u64::try_from(n).map_err(|_| "invalid ratio")?,
        u64::try_from(d).map_err(|_| "invalid ratio")?,
    )
    .map_err(|_| "invalid ratio")
}

fn valid_benefits(values: &[String], allowed: &[&str]) -> bool {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .all(|value| allowed.contains(&value.as_str()) && seen.insert(value))
}

impl ConsolePolicy {
    pub(super) fn parse(raw: &str) -> Result<(Self, TrustPolicy), &'static str> {
        if raw.len() > 16_384 {
            return Err("policy exceeds 16 KiB");
        }
        let source: Self = serde_json::from_str(raw).map_err(|_| "invalid policy")?;
        if source.tiers.len() != 5 || source.role_tiers.len() != 2 {
            return Err("invalid policy levels");
        }
        let mut tiers = Vec::with_capacity(5);
        for (i, tier) in source.tiers.iter().enumerate() {
            if usize::from(tier.level) != i
                || !valid_benefits(
                    &tier.benefits,
                    &["standard_access", "developer_access", "usage_discount"],
                )
            {
                return Err("invalid tier benefits");
            }
            tiers.push(TrustTier {
                level: tier.level,
                min_paid_credits: tier.min_paid_credits,
                discount: decimal_ratio(&tier.discount_ratio)?,
            });
        }
        for (i, tier) in source.role_tiers.iter().enumerate() {
            let (level, role, benefit) = if i == 0 {
                (5, 10, "administrator_access")
            } else {
                (6, 100, "superadministrator_access")
            };
            if tier.level != level
                || tier.role != role
                || !valid_benefits(&tier.benefits, &[benefit, "usage_discount"])
            {
                return Err("invalid role benefits");
            }
        }
        let policy = TrustPolicy {
            version: source.version,
            paid_activation_enabled: source.paid_activation_enabled,
            decay_period_days: source.decay_period_days,
            tiers: tiers.try_into().map_err(|_| "invalid policy levels")?,
            admin_discount: decimal_ratio(&source.role_tiers[0].discount_ratio)?,
            superadmin_discount: decimal_ratio(&source.role_tiers[1].discount_ratio)?,
        };
        policy.validate().map_err(|_| "invalid policy")?;
        Ok((source, policy))
    }
}

/// Build a deterministic description of the exact policy transition. The digest
/// is a change-binding value, not a credential or a substitute for authorization.
#[derive(Debug, Serialize)]
pub struct PolicyPreview {
    pub actor_user_id: i64,
    pub session_credential_id: i64,
    pub base_revision: i64,
    pub current: TrustPolicy,
    // None means the historical console descriptions were not preserved.
    pub current_console: Option<ConsolePolicy>,
    pub proposed: TrustPolicy,
    pub proposed_console: ConsolePolicy,
    pub digest: String,
}

impl PolicyPreview {
    pub(super) fn build(
        actor_user_id: i64,
        session_credential_id: i64,
        base_revision: i64,
        current: TrustPolicy,
        current_console_raw: Option<&str>,
        raw: &str,
    ) -> Result<Self, &'static str> {
        use sha2::{Digest, Sha256};
        if actor_user_id <= 0
            || session_credential_id <= 0
            || base_revision <= 0
            || current.validate().is_err()
        {
            return Err("invalid current policy");
        }
        let current_console = current_console_raw
            .map(ConsolePolicy::parse)
            .transpose()?
            .map(|(source, authority)| {
                if authority == current {
                    Ok(source)
                } else {
                    Err("current policy source mismatch")
                }
            })
            .transpose()?;
        let (proposed_console, proposed) = ConsolePolicy::parse(raw)?;
        let binding = serde_json::to_vec(&(
            "core-trust-policy-preview-v1",
            actor_user_id,
            session_credential_id,
            base_revision,
            &current,
            &current_console,
            &proposed,
            &proposed_console,
        ))
        .map_err(|_| "invalid preview")?;
        let digest = Sha256::digest(binding)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self {
            actor_user_id,
            session_credential_id,
            base_revision,
            current,
            current_console,
            proposed,
            proposed_console,
            digest,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[allow(dead_code)]
    pub(super) mod runtime_fixture {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/common/runtime.rs"
        ));
    }

    pub(super) type TestDevice = webauthn_authenticator_rs::WebauthnAuthenticator<
        webauthn_authenticator_rs::softpasskey::SoftPasskey,
    >;
    pub(super) fn test_device() -> TestDevice {
        webauthn_authenticator_rs::WebauthnAuthenticator::new(
            webauthn_authenticator_rs::softpasskey::SoftPasskey::new(true),
        )
    }
    pub(super) fn test_origin() -> webauthn_rs::prelude::Url {
        webauthn_rs::prelude::Url::parse("https://localhost").unwrap()
    }
    pub(super) fn test_config() -> EditorConfig {
        EditorConfig::new("localhost", "https://localhost").unwrap()
    }
    pub(super) fn test_service(pool: &sqlx::PgPool) -> Service {
        Service::new(pool.clone(), test_config(), SigningKey::new([0x0b; 32]))
    }
    pub(super) async fn enroll_editor(
        pool: &sqlx::PgPool,
        device: &mut TestDevice,
        editor: &str,
        version: i64,
    ) {
        let owner = editor::Owner::new(pool.clone(), test_config());
        let begin = owner.begin_registration(editor, 1, version).await.unwrap();
        let response = device
            .do_registration(
                test_origin(),
                serde_json::from_value(begin.options).unwrap(),
            )
            .unwrap();
        let review = owner
            .review_registration(&begin.enrollment, &response)
            .await
            .unwrap();
        assert_eq!(
            owner
                .approve_registration(&begin.enrollment, &review.fingerprint)
                .await
                .unwrap(),
            version + 1
        );
    }
    pub(super) async fn fixture_confirmation(
        service: &Service,
        device: &mut TestDevice,
        secret: &str,
        token: &str,
        digest: &str,
    ) -> Result<Confirmation, Error> {
        let challenge = service.challenge(secret, token, digest, "owner").await?;
        let assertion = device
            .do_authentication(
                test_origin(),
                serde_json::from_value(challenge.options).unwrap(),
            )
            .unwrap();
        Ok(Confirmation {
            token: token.into(),
            digest: digest.into(),
            challenge: challenge.challenge,
            assertion,
        })
    }
    pub(super) async fn fixture_confirm(
        service: &Service,
        device: &mut TestDevice,
        secret: &str,
        token: &str,
        digest: &str,
    ) -> Result<i64, Error> {
        let input = fixture_confirmation(service, device, secret, token, digest).await?;
        service.confirm(secret, &input).await
    }

    pub(super) fn fixture() -> serde_json::Value {
        json!({"version":1,"paid_activation_enabled":true,"decay_period_days":90,
            "tiers":(0..5).map(|i| json!({"level":i,"min_paid_credits":i,"discount_ratio":0.97,"benefits":["standard_access"]})).collect::<Vec<_>>(),
            "role_tiers":[{"level":5,"role":10,"discount_ratio":0.9,"benefits":["administrator_access"]},{"level":6,"role":100,"discount_ratio":1,"benefits":["superadministrator_access"]}]})
    }

    #[test]
    fn translates_without_floating_point_arithmetic() {
        let (_, policy) = ConsolePolicy::parse(&fixture().to_string()).unwrap();
        assert_eq!(
            (
                policy.tiers[0].discount.numerator(),
                policy.tiers[0].discount.denominator()
            ),
            (97, 100)
        );
        assert_eq!(
            (
                policy.admin_discount.numerator(),
                policy.admin_discount.denominator()
            ),
            (9, 10)
        );
        assert_eq!(
            (
                decimal_ratio(&serde_json::from_str("1e-18").unwrap())
                    .unwrap()
                    .numerator(),
                decimal_ratio(&serde_json::from_str("1e-18").unwrap())
                    .unwrap()
                    .denominator()
            ),
            (1, 1_000_000_000_000_000_000)
        );
        assert!(decimal_ratio(&serde_json::from_str("1e-19").unwrap()).is_err());
    }

    #[test]
    fn missing_role_tiers_use_legacy_parser_defaults() {
        let mut input = fixture();
        input.as_object_mut().unwrap().remove("role_tiers");
        let (_, policy) = ConsolePolicy::parse(&input.to_string()).unwrap();
        assert_eq!(
            (
                policy.admin_discount.numerator(),
                policy.admin_discount.denominator()
            ),
            (9, 10)
        );
        assert_eq!(policy.admin_discount, policy.superadmin_discount);
    }

    #[test]
    fn null_optional_lists_match_legacy_nil_semantics() {
        let mut input = fixture();
        input["role_tiers"] = serde_json::Value::Null;
        input["tiers"][0]["benefits"] = serde_json::Value::Null;
        let (source, policy) = ConsolePolicy::parse(&input.to_string()).unwrap();
        assert!(source.tiers[0].benefits.is_empty());
        assert_eq!(policy.admin_discount.numerator(), 9);
        input["role_tiers"] = json!([]);
        assert!(ConsolePolicy::parse(&input.to_string()).is_err());
    }

    #[test]
    fn preview_binds_actor_session_revision_current_and_proposal() {
        let raw = fixture().to_string();
        let (_, current) = ConsolePolicy::parse(&raw).unwrap();
        let preview = PolicyPreview::build(1, 10, 1, current.clone(), None, &raw).unwrap();
        assert_eq!(preview.digest.len(), 64);
        assert!(preview.current_console.is_none());
        assert_eq!(
            preview.digest,
            PolicyPreview::build(1, 10, 1, current.clone(), None, &raw)
                .unwrap()
                .digest
        );
        for (actor, session, revision) in [(2, 10, 1), (1, 11, 1), (1, 10, 2)] {
            assert_ne!(
                preview.digest,
                PolicyPreview::build(actor, session, revision, current.clone(), None, &raw)
                    .unwrap()
                    .digest
            );
        }
        let known = PolicyPreview::build(1, 10, 1, current.clone(), Some(&raw), &raw).unwrap();
        assert!(known.current_console.is_some());
        assert_ne!(preview.digest, known.digest);
        let mut altered = fixture();
        altered["tiers"][1]["benefits"] = json!(["developer_access"]);
        assert_ne!(
            preview.digest,
            PolicyPreview::build(1, 10, 1, current.clone(), None, &altered.to_string())
                .unwrap()
                .digest
        );
        let mut altered_current = current.clone();
        altered_current.decay_period_days = 91;
        assert_ne!(
            preview.digest,
            PolicyPreview::build(1, 10, 1, altered_current.clone(), None, &raw)
                .unwrap()
                .digest
        );
        assert!(PolicyPreview::build(1, 10, 1, altered_current, Some(&raw), &raw).is_err());
        for (actor, session, revision) in [(0, 10, 1), (1, 0, 1), (1, 10, 0)] {
            assert!(
                PolicyPreview::build(actor, session, revision, current.clone(), None, &raw)
                    .is_err()
            );
        }
    }

    #[sqlx::test(migrations = false)]
    async fn staged_database_validator_agrees_with_rust_on_boundaries(pool: sqlx::PgPool) {
        crate::core_schema::init(&pool).await.unwrap();
        install_policy_fixture(&pool).await;
        let (_, valid) = ConsolePolicy::parse(&fixture().to_string()).unwrap();
        let base = serde_json::to_value(valid).unwrap();
        let mut cases = vec![base.clone(), json!(null), json!([]), json!({})];
        for (path, replacement) in [
            ("version", json!(2)),
            ("version", json!(1.0)),
            ("decay_period_days", json!(3651)),
            ("decay_period_days", json!(-1)),
            ("paid_activation_enabled", json!(null)),
            ("unexpected", json!(true)),
        ] {
            let mut item = base.clone();
            item[path] = replacement;
            cases.push(item);
        }
        for replacement in [
            json!(0),
            json!(-1),
            json!(9007199254740991_i64),
            json!(9007199254740992_i64),
            json!(1.5),
            json!(null),
        ] {
            let mut item = base.clone();
            item["tiers"][4]["min_paid_credits"] = replacement;
            cases.push(item);
        }
        for replacement in [json!(0), json!(1), json!(-1), json!(1.0), json!(null)] {
            let mut item = base.clone();
            item["tiers"][1]["level"] = replacement;
            cases.push(item);
        }
        for replacement in [
            json!({"numerator":0,"denominator":1}),
            json!({"numerator":2,"denominator":1}),
            json!({"numerator":1,"denominator":1000000000000000001_i64}),
            json!({"numerator":1,"denominator":1,"other":0}),
            json!({"numerator":1.0,"denominator":1}),
            json!(null),
        ] {
            let mut item = base.clone();
            item["admin_discount"] = replacement;
            cases.push(item);
        }
        for item in cases {
            let rust = serde_json::from_value::<TrustPolicy>(item.clone())
                .is_ok_and(|policy| policy.validate().is_ok());
            let database: bool =
                sqlx::query_scalar("SELECT core_trust_pricing.valid_policy_for_edit($1)")
                    .bind(sqlx::types::Json(&item))
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(database, rust, "policy: {item}");
        }
        let public_privileges: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce(c.relacl,pg_catalog.acldefault('r',c.relowner))) a WHERE n.nspname='core_trust_pricing' AND c.relname IN ('policy_edit_previews','policy_edit_audit') AND a.grantee=0",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(public_privileges, 0);
        let identity = crate::identity::IdentityStore::from_pool(pool.clone());
        identity.bootstrap_user(1, 6).await.unwrap();
        let credential: i64 = sqlx::query_scalar(
            "SELECT id FROM core_identity.credentials WHERE user_id=1 AND kind='session'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let digest = vec![7_u8; 32];
        let preview_sql = "INSERT INTO core_trust_pricing.policy_edit_previews(token_digest,actor_user_id,session_credential_id,base_revision,before_spec,after_spec,after_console,transition_digest,expires_at) VALUES ($1,1,$2,1,$3,$3,'{}'::jsonb,$1,statement_timestamp()+interval '5 minutes')";
        sqlx::query(preview_sql)
            .bind(&digest)
            .bind(credential)
            .bind(sqlx::types::Json(&base))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO core_trust_pricing.policy_edit_audit(revision,actor_user_id,session_credential_id,preview_digest,transition_digest,before_source_digest,after_source_digest,before_spec,after_spec,after_console) VALUES (2,1,$1,$2,$2,$2,$2,$3,$3,'{}'::jsonb)")
            .bind(credential).bind(&digest).bind(sqlx::types::Json(&base)).execute(&pool).await.unwrap();
        for mutation in [
            "UPDATE core_trust_pricing.policy_edit_audit SET revision=3",
            "DELETE FROM core_trust_pricing.policy_edit_audit",
            // Include dependent confirmation rows so the immutability trigger,
            // rather than PostgreSQL's FK precheck, is the protection exercised.
            "TRUNCATE core_trust_pricing.policy_edit_audit CASCADE",
        ] {
            assert_eq!(
                sqlx::query(mutation)
                    .execute(&pool)
                    .await
                    .unwrap_err()
                    .as_database_error()
                    .and_then(|e| e.code())
                    .as_deref(),
                Some("55000")
            );
        }
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM core_trust_pricing.policy_edit_audit")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = false)]
    async fn staged_update_is_atomic_and_rejects_replay_and_stale_preview(pool: sqlx::PgPool) {
        use sqlx::Row;
        crate::core_schema::init(&pool).await.unwrap();
        install_policy_fixture(&pool).await;
        let identity = crate::identity::IdentityStore::from_pool(pool.clone());
        identity.bootstrap_user(1, 6).await.unwrap();
        let credential: i64 = sqlx::query_scalar(
            "SELECT id FROM core_identity.credentials WHERE user_id=1 AND kind='session'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let before = sqlx::query("SELECT revision,spec,source_digest,source_cutoff,native_user_floor,native_since::text AS native_since FROM core_trust_pricing.policies WHERE singleton")
            .fetch_one(&pool).await.unwrap();
        let original: serde_json::Value = before.get("spec");
        let mut proposed = original.clone();
        proposed["decay_period_days"] = json!(91);
        let token = vec![1_u8; 32];
        let stale = vec![2_u8; 32];
        let transition = vec![3_u8; 32];
        let insert = "SELECT core_trust_pricing.create_policy_edit_preview($1,1,$2,$3,$4,$5,NULL,'{}'::jsonb,$6)";
        for id in [&token, &stale] {
            sqlx::query(insert)
                .bind(id)
                .bind(credential)
                .bind(before.get::<i64, _>("revision"))
                .bind(sqlx::types::Json(&original))
                .bind(sqlx::types::Json(&proposed))
                .bind(&transition)
                .execute(&pool)
                .await
                .unwrap();
        }
        let apply = "SELECT core_trust_pricing.apply_policy_edit($1,1,$2,$3)";
        // Use the actual quote authority while holding its serializable
        // admission transaction open. The update must wait; rolling the write
        // back must preserve the quote policy and leave the preview reusable.
        let mut reader = pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *reader)
            .await
            .unwrap();
        crate::billing::trust_pricing::quote_in_transaction(&mut reader, 1)
            .await
            .unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let writer_pool = pool.clone();
        let writer_token = token.clone();
        let writer_transition = transition.clone();
        let mut writer = tokio::spawn(async move {
            let mut tx = writer_pool.begin().await.unwrap();
            started_tx.send(()).unwrap();
            let revision: i64 = sqlx::query_scalar(apply)
                .bind(writer_token)
                .bind(credential)
                .bind(writer_transition)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
            tx.rollback().await.unwrap();
            revision
        });
        started_rx.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(250), &mut writer)
                .await
                .is_err()
        );
        let unchanged: i64 =
            sqlx::query_scalar("SELECT revision FROM core_trust_pricing.policies WHERE singleton")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(unchanged, before.get::<i64, _>("revision"));
        reader.commit().await.unwrap();
        assert_eq!(writer.await.unwrap(), unchanged + 1);
        let audit_after_rollback: i64 =
            sqlx::query_scalar("SELECT count(*) FROM core_trust_pricing.policy_edit_audit")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(audit_after_rollback, 0);
        let consumed_after_rollback: bool = sqlx::query_scalar("SELECT consumed_at IS NOT NULL FROM core_trust_pricing.policy_edit_previews WHERE token_digest=$1")
            .bind(&token).fetch_one(&pool).await.unwrap();
        assert!(!consumed_after_rollback);
        let expired = vec![4_u8; 32];
        sqlx::query("INSERT INTO core_trust_pricing.policy_edit_previews(token_digest,actor_user_id,session_credential_id,base_revision,before_spec,after_spec,after_console,transition_digest,created_at,expires_at) VALUES ($1,1,$2,$3,$4,$5,'{}'::jsonb,$6,statement_timestamp()-interval '10 minutes',statement_timestamp()-interval '5 minutes')")
            .bind(&expired).bind(credential).bind(before.get::<i64,_>("revision"))
            .bind(sqlx::types::Json(&original)).bind(sqlx::types::Json(&proposed)).bind(&transition)
            .execute(&pool).await.unwrap();
        let wrong_transition = vec![9_u8; 32];
        for (id, supplied) in [(&expired, &transition), (&token, &wrong_transition)] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(apply)
                    .bind(id)
                    .bind(credential)
                    .bind(supplied)
                    .fetch_one(&pool)
                    .await
                    .unwrap_err()
                    .as_database_error()
                    .and_then(|e| e.code())
                    .as_deref(),
                Some("22023")
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT core_trust_pricing.apply_policy_edit($1,2,$2,$3)")
                .bind(&token)
                .bind(credential)
                .bind(&transition)
                .fetch_one(&pool)
                .await
                .unwrap_err()
                .as_database_error()
                .and_then(|e| e.code())
                .as_deref(),
            Some("42501")
        );
        let first = sqlx::query_scalar::<_, i64>(apply)
            .bind(&token)
            .bind(credential)
            .bind(&transition)
            .fetch_one(&pool);
        let second = sqlx::query_scalar::<_, i64>(apply)
            .bind(&stale)
            .bind(credential)
            .bind(&transition)
            .fetch_one(&pool);
        let (first, second) = tokio::join!(first, second);
        let (revision, winner, failure) = match (first, second) {
            (Ok(revision), Err(error)) => (revision, &token, error),
            (Err(error), Ok(revision)) => (revision, &stale, error),
            other => panic!("expected one policy winner, got {other:?}"),
        };
        assert_eq!(revision, before.get::<i64, _>("revision") + 1);
        assert_eq!(
            failure
                .as_database_error()
                .and_then(|e| e.code())
                .as_deref(),
            Some("40001")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(apply)
                .bind(winner)
                .bind(credential)
                .bind(&transition)
                .fetch_one(&pool)
                .await
                .unwrap_err()
                .as_database_error()
                .and_then(|e| e.code())
                .as_deref(),
            Some("22023")
        );
        let stale_creation = sqlx::query(insert)
            .bind(&vec![8_u8; 32])
            .bind(credential)
            .bind(before.get::<i64, _>("revision"))
            .bind(sqlx::types::Json(&original))
            .bind(sqlx::types::Json(&proposed))
            .bind(&transition)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(
            stale_creation
                .as_database_error()
                .and_then(|e| e.code())
                .as_deref(),
            Some("40001")
        );
        let after = sqlx::query("SELECT revision,spec,source_digest,source_cutoff,native_user_floor,native_since::text AS native_since FROM core_trust_pricing.policies WHERE singleton")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(after.get::<i64, _>("revision"), revision);
        assert_eq!(after.get::<serde_json::Value, _>("spec"), proposed);
        assert_ne!(
            before.get::<Vec<u8>, _>("source_digest"),
            after.get::<Vec<u8>, _>("source_digest")
        );
        assert_eq!(
            before.get::<i64, _>("source_cutoff"),
            after.get::<i64, _>("source_cutoff")
        );
        assert_eq!(
            before.get::<i64, _>("native_user_floor"),
            after.get::<i64, _>("native_user_floor")
        );
        assert_eq!(
            before.get::<String, _>("native_since"),
            after.get::<String, _>("native_since")
        );
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM core_trust_pricing.policy_edit_audit")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
        let consumed: i64 = sqlx::query_scalar("SELECT count(*) FROM core_trust_pricing.policy_edit_previews WHERE consumed_at IS NOT NULL AND resulting_revision=$1")
            .bind(revision).fetch_one(&pool).await.unwrap();
        assert_eq!(consumed, 1);

        // A commit result cannot be resolved while its original transaction
        // still holds the policy fence. Once it commits, the same token proves
        // exactly which revision was written; no second mutation is issued.
        let third = vec![5_u8; 32];
        sqlx::query(insert)
            .bind(&third)
            .bind(credential)
            .bind(revision)
            .bind(sqlx::types::Json(&proposed))
            .bind(sqlx::types::Json(&proposed))
            .bind(&transition)
            .execute(&pool)
            .await
            .unwrap();
        let mut committing = pool.begin().await.unwrap();
        let next: i64 = sqlx::query_scalar(apply)
            .bind(&third)
            .bind(credential)
            .bind(&transition)
            .fetch_one(&mut *committing)
            .await
            .unwrap();
        assert_eq!(next, revision + 1);
        let resolver_pool = pool.clone();
        let resolver_token = third.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let mut resolver = tokio::spawn(async move {
            started_tx.send(()).unwrap();
            sqlx::query_as::<_, (String, Option<i64>)>(
                "SELECT state,revision FROM core_trust_pricing.resolve_policy_edit($1,1,$2)",
            )
            .bind(resolver_token)
            .bind(credential)
            .fetch_one(&resolver_pool)
            .await
            .unwrap()
        });
        started_rx.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(250), &mut resolver)
                .await
                .is_err()
        );
        committing.commit().await.unwrap();
        assert_eq!(
            resolver.await.unwrap(),
            ("confirmed".to_owned(), Some(next))
        );
    }

    pub(super) async fn install_policy_fixture(pool: &sqlx::PgPool) {
        let installed: bool = sqlx::query_scalar(
            "SELECT to_regclass('core_trust_pricing.policy_edit_previews') IS NOT NULL",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if !installed {
            sqlx::raw_sql(include_str!("../../schema/trust_policy_edit.sql"))
                .execute(pool)
                .await
                .unwrap();
        }
    }
    async fn install_proof_schema(pool: &sqlx::PgPool) {
        let installed: bool = sqlx::query_scalar(
            "SELECT to_regclass('core_trust_pricing.policy_edit_signing_key') IS NOT NULL",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if !installed {
            sqlx::raw_sql(include_str!("../../schema/trust_policy_edit_proof.sql"))
                .execute(pool)
                .await
                .unwrap();
        }
    }
    pub(super) async fn install_proof_fixture(pool: &sqlx::PgPool) -> TestDevice {
        install_proof_schema(pool).await;
        sqlx::query("INSERT INTO core_trust_pricing.policy_edit_signing_key(secret) VALUES ($1)")
            .bind(vec![0x0bu8; 32])
            .execute(pool)
            .await
            .unwrap();
        let installed: bool = sqlx::query_scalar(
            "SELECT to_regclass('core_trust_pricing.policy_editors') IS NOT NULL",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if !installed {
            sqlx::raw_sql(include_str!("../../schema/trust_policy_edit_editor.sql"))
                .execute(pool)
                .await
                .unwrap();
        }
        let mut device = test_device();
        enroll_editor(pool, &mut device, "owner", 0).await;
        device
    }

    #[sqlx::test(migrations = false)]
    async fn staged_service_requires_root_session_preview_and_confirmation(pool: sqlx::PgPool) {
        crate::core_schema::init(&pool).await.unwrap();
        install_policy_fixture(&pool).await;
        let mut device = install_proof_fixture(&pool).await;
        let service = test_service(&pool);
        let identity = crate::identity::IdentityStore::from_pool(pool.clone());
        let root = identity.bootstrap_user(1, 6).await.unwrap();
        let admin = identity.bootstrap_user(2, 5).await.unwrap();
        let raw = fixture().to_string();
        assert!(matches!(
            service.preview(&admin.secret, &raw).await,
            Err(store::Error::Auth(
                crate::identity::IdentityError::Forbidden
            ))
        ));
        let created = service.preview(&root.secret, &raw).await.unwrap();
        assert!(created.preview.current_console.is_none());
        let pending = service.resolve(&root.secret, &created.token).await.unwrap();
        assert_eq!(
            (pending.state.as_str(), pending.revision),
            ("pending", None)
        );
        let unknown = service
            .resolve(&root.secret, &"00".repeat(32))
            .await
            .unwrap();
        assert_eq!(
            (unknown.state.as_str(), unknown.revision),
            ("unknown", None)
        );
        assert!(matches!(
            service.resolve(&admin.secret, &created.token).await,
            Err(store::Error::Auth(
                crate::identity::IdentityError::Forbidden
            ))
        ));
        let before: i64 =
            sqlx::query_scalar("SELECT revision FROM core_trust_pricing.policies WHERE singleton")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(before, 1);
        let persisted: String = sqlx::query_scalar(
            "SELECT encode(token_digest,'hex') FROM core_trust_pricing.policy_edit_previews",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_ne!(persisted, created.token);
        assert!(matches!(
            fixture_confirm(
                &service,
                &mut device,
                &admin.secret,
                &created.token,
                &created.preview.digest
            )
            .await,
            Err(store::Error::Auth(
                crate::identity::IdentityError::Forbidden
            ))
        ));
        assert!(matches!(
            fixture_confirm(
                &service,
                &mut device,
                &root.secret,
                &created.token,
                &"00".repeat(32)
            )
            .await,
            Err(store::Error::Invalid)
        ));
        assert_eq!(
            fixture_confirm(
                &service,
                &mut device,
                &root.secret,
                &created.token,
                &created.preview.digest
            )
            .await
            .unwrap(),
            2
        );
        assert!(matches!(
            fixture_confirm(
                &service,
                &mut device,
                &root.secret,
                &created.token,
                &created.preview.digest
            )
            .await,
            Err(store::Error::Invalid)
        ));
        let resolved = service.resolve(&root.secret, &created.token).await.unwrap();
        assert_eq!(
            (resolved.state.as_str(), resolved.revision),
            ("confirmed", Some(2))
        );
        let next = service.preview(&root.secret, &raw).await.unwrap();
        assert!(next.preview.current_console.is_some());
        assert_eq!(next.preview.base_revision, 2);
        for _ in 0..7 {
            service.preview(&root.secret, &raw).await.unwrap();
        }
        assert!(matches!(
            service.preview(&root.secret, &raw).await,
            Err(store::Error::Limited)
        ));
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM core_trust_pricing.policy_edit_previews WHERE consumed_at IS NULL")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(pending, 8);
    }

    #[sqlx::test(migrations = false)]
    async fn staged_service_works_with_restricted_runtime_without_table_writes(pool: sqlx::PgPool) {
        crate::core_schema::init(&pool).await.unwrap();
        install_policy_fixture(&pool).await;
        let mut device = install_proof_fixture(&pool).await;
        let identity = crate::identity::IdentityStore::from_pool(pool.clone());
        let root = identity.bootstrap_user(1, 6).await.unwrap();
        let runtime = runtime_fixture::RuntimeRole::create(&pool).await;
        let name = &runtime.name;
        let grants = format!(
            "GRANT EXECUTE ON FUNCTION core_trust_pricing.read_policy_edit_description(bigint), core_trust_pricing.policy_edit_binding(text,jsonb), core_trust_pricing.authorized_policy_preview(bytea,bigint,bigint,bigint,jsonb,jsonb,jsonb,jsonb,bytea,bytea), core_trust_pricing.authorized_policy_confirm(jsonb,bytea), core_trust_pricing.authorized_policy_challenge(jsonb,bytea), core_trust_pricing.policy_editor_context(text,bigint), core_trust_pricing.policy_challenge_context(bytea,bytea,bigint,bigint), core_trust_pricing.authorized_policy_resolve(bytea,bigint,bigint,bytea) TO {name}"
        );
        sqlx::raw_sql(&grants).execute(&pool).await.unwrap();
        for sql in [
            "UPDATE core_trust_pricing.policies SET revision=revision",
            "DELETE FROM core_trust_pricing.user_bases WHERE false",
            "SELECT * FROM core_trust_pricing.policy_edit_previews",
            "SELECT * FROM core_trust_pricing.policy_edit_audit",
        ] {
            let error = sqlx::query(sql).execute(&runtime.pool).await.unwrap_err();
            assert_eq!(
                error.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("42501"),
                "{sql}"
            );
        }
        let raw = fixture().to_string();
        let created = test_service(&runtime.pool)
            .preview(&root.secret, &raw)
            .await
            .unwrap();
        assert_eq!(
            fixture_confirm(
                &test_service(&runtime.pool),
                &mut device,
                &root.secret,
                &created.token,
                &created.preview.digest
            )
            .await
            .unwrap(),
            2
        );
        let resolution = test_service(&runtime.pool)
            .resolve(&root.secret, &created.token)
            .await
            .unwrap();
        assert_eq!(
            (resolution.state.as_str(), resolution.revision),
            ("confirmed", Some(2))
        );
        let audit: i64 =
            sqlx::query_scalar("SELECT count(*) FROM core_trust_pricing.policy_edit_audit")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(audit, 1);
        runtime.cleanup().await;
    }

    #[sqlx::test(migrations = false)]
    async fn staged_proof_uses_hmac_and_binds_the_top_level_transaction(pool: sqlx::PgPool) {
        use hmac::{Hmac, Mac};
        crate::core_schema::init(&pool).await.unwrap();
        install_proof_schema(&pool).await;
        let key = vec![0x0bu8; 32];
        let message = b"Hi There";
        let sql: Vec<u8> = sqlx::query_scalar("SELECT core_trust_pricing.policy_edit_hmac($1,$2)")
            .bind(&key)
            .bind(message.as_slice())
            .fetch_one(&pool)
            .await
            .unwrap();
        let mut oracle = Hmac::<sha2::Sha256>::new_from_slice(&key).unwrap();
        oracle.update(message);
        assert_eq!(sql, oracle.finalize().into_bytes().to_vec());
        sqlx::query("INSERT INTO core_trust_pricing.policy_edit_signing_key(secret) VALUES ($1)")
            .bind(&key)
            .execute(&pool)
            .await
            .unwrap();
        let payload = json!({"actor":1,"revision":1});
        let mut tx = pool.begin().await.unwrap();
        let first: Vec<u8> =
            sqlx::query_scalar("SELECT core_trust_pricing.policy_edit_binding($1,$2)")
                .bind("preview")
                .bind(&payload)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        let mut signer = Hmac::<sha2::Sha256>::new_from_slice(&key).unwrap();
        signer.update(&first);
        let proof = signer.finalize().into_bytes().to_vec();
        let valid: bool =
            sqlx::query_scalar("SELECT core_trust_pricing.policy_edit_verify($1,$2,$3)")
                .bind("preview")
                .bind(&payload)
                .bind(&proof)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert!(valid);
        let wrong_action: bool =
            sqlx::query_scalar("SELECT core_trust_pricing.policy_edit_verify($1,$2,$3)")
                .bind("confirm")
                .bind(&payload)
                .bind(&proof)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert!(!wrong_action);
        let wrong_payload: bool =
            sqlx::query_scalar("SELECT core_trust_pricing.policy_edit_verify($1,$2,$3)")
                .bind("preview")
                .bind(json!({"actor":1,"revision":2}))
                .bind(&proof)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert!(!wrong_payload);
        let malformed: bool =
            sqlx::query_scalar("SELECT core_trust_pricing.policy_edit_verify($1,$2,$3)")
                .bind("preview")
                .bind(&payload)
                .bind(vec![0u8; 31])
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert!(!malformed);
        tx.commit().await.unwrap();
        let replay: bool =
            sqlx::query_scalar("SELECT core_trust_pricing.policy_edit_verify($1,$2,$3)")
                .bind("preview")
                .bind(&payload)
                .bind(&proof)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!replay);
        let runtime = runtime_fixture::RuntimeRole::create(&pool).await;
        let denied = sqlx::query("SELECT secret FROM core_trust_pricing.policy_edit_signing_key")
            .execute(&runtime.pool)
            .await
            .unwrap_err();
        assert_eq!(
            denied.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
        runtime.cleanup().await;
    }

    // A database runtime credential must not itself confer the authority of an
    // unrelated superadministrator session, even if it knows both numeric IDs.
    #[sqlx::test(migrations = false)]
    async fn restricted_runtime_cannot_forge_a_superadmin_preview(pool: sqlx::PgPool) {
        crate::core_schema::init(&pool).await.unwrap();
        install_policy_fixture(&pool).await;
        let _device = install_proof_fixture(&pool).await;
        let identity = crate::identity::IdentityStore::from_pool(pool.clone());
        let root = identity.bootstrap_user(1, 6).await.unwrap();
        let runtime = runtime_fixture::RuntimeRole::create(&pool).await;
        sqlx::raw_sql(&format!("GRANT EXECUTE ON FUNCTION core_trust_pricing.authorized_policy_preview(bytea,bigint,bigint,bigint,jsonb,jsonb,jsonb,jsonb,bytea,bytea) TO {}",runtime.name)).execute(&pool).await.unwrap();
        let spec: serde_json::Value =
            sqlx::query_scalar("SELECT spec FROM core_trust_pricing.policies WHERE singleton")
                .fetch_one(&pool)
                .await
                .unwrap();
        let direct = sqlx::query(
            "SELECT core_trust_pricing.create_policy_edit_preview($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(vec![9u8; 32])
        .bind(1i64)
        .bind(root.id)
        .bind(1i64)
        .bind(&spec)
        .bind(&spec)
        .bind(Option::<serde_json::Value>::None)
        .bind(json!({}))
        .bind(vec![8u8; 32])
        .execute(&runtime.pool)
        .await
        .unwrap_err();
        assert_eq!(
            direct.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
        let forged = sqlx::query(
            "SELECT core_trust_pricing.authorized_policy_preview($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(vec![9u8; 32])
        .bind(1i64)
        .bind(root.id)
        .bind(1i64)
        .bind(&spec)
        .bind(&spec)
        .bind(Option::<serde_json::Value>::None)
        .bind(json!({}))
        .bind(vec![8u8; 32])
        .bind(vec![7u8; 32])
        .execute(&runtime.pool)
        .await;
        runtime.cleanup().await;
        let error = forged
            .expect_err("runtime forged superadmin preview without presenting a session secret");
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
    }

    #[sqlx::test(migrations = false)]
    async fn staged_http_requires_preview_and_confirms_the_exact_transition(pool: sqlx::PgPool) {
        use axum::{
            body::{Body, to_bytes},
            http::{Request, StatusCode, header},
        };
        use tower::ServiceExt;
        crate::core_schema::init(&pool).await.unwrap();
        install_policy_fixture(&pool).await;
        let mut device = install_proof_fixture(&pool).await;
        let service = test_service(&pool);
        let identity = crate::identity::IdentityStore::from_pool(pool.clone());
        let root = identity.bootstrap_user(1, 6).await.unwrap();
        let admin = identity.bootstrap_user(2, 5).await.unwrap();
        let key = identity
            .issue_key(
                &root.secret,
                crate::accounts::Account {
                    kind: crate::accounts::AccountKind::Personal,
                    id: 1,
                },
                crate::funding::FundingPolicy::new_personal_key(),
                3600,
            )
            .await
            .unwrap();
        let app = http::router(service.clone());
        let request = |path: &str, secret: &str, payload: serde_json::Value| {
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {secret}"))
                .body(Body::from(payload.to_string()))
                .unwrap()
        };
        let payload = json!({"value":fixture().to_string()});
        let denied = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/preview",
                &admin.secret,
                payload.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let key_denied = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/preview",
                &key.secret,
                payload.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(key_denied.status(), StatusCode::FORBIDDEN);
        let preview = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/preview",
                &root.secret,
                payload,
            ))
            .await
            .unwrap();
        assert_eq!(preview.status(), StatusCode::OK);
        assert_eq!(
            preview.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        let preview: serde_json::Value =
            serde_json::from_slice(&to_bytes(preview.into_body(), 100_000).await.unwrap()).unwrap();
        let token = preview["data"]["token"].as_str().unwrap();
        let digest = preview["data"]["preview"]["digest"].as_str().unwrap();
        let revision: i64 =
            sqlx::query_scalar("SELECT revision FROM core_trust_pricing.policies WHERE singleton")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(revision, 1);
        let challenge = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/challenge",
                &root.secret,
                json!({"token":token,"digest":digest,"editor":"owner"}),
            ))
            .await
            .unwrap();
        assert_eq!(challenge.status(), StatusCode::OK);
        let challenge: serde_json::Value =
            serde_json::from_slice(&to_bytes(challenge.into_body(), 100_000).await.unwrap())
                .unwrap();
        let assertion = device
            .do_authentication(
                test_origin(),
                serde_json::from_value(challenge["data"]["options"].clone()).unwrap(),
            )
            .unwrap();
        let confirm = json!({"token":token,"digest":digest,"challenge":challenge["data"]["challenge"],"assertion":assertion});
        let denied = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/confirm",
                &admin.secret,
                confirm.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let accepted = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/confirm",
                &root.secret,
                confirm.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
        let replay = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/confirm",
                &root.secret,
                confirm.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
        let resolved = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/resolve",
                &root.secret,
                json!({"token":token}),
            ))
            .await
            .unwrap();
        assert_eq!(resolved.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(resolved.into_body(), 100_000).await.unwrap())
                .unwrap();
        assert_eq!(body["data"], json!({"state":"confirmed","revision":2}));
        let next = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/preview",
                &root.secret,
                json!({"value":fixture().to_string()}),
            ))
            .await
            .unwrap();
        assert_eq!(next.status(), StatusCode::OK);
        let next: serde_json::Value =
            serde_json::from_slice(&to_bytes(next.into_body(), 100_000).await.unwrap()).unwrap();
        let refreshed = identity.refresh_session(&root.secret).await.unwrap();
        let revoked = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/confirm",
                &root.secret,
                json!({"token":next["data"]["token"],"digest":next["data"]["preview"]["digest"],"challenge":confirm["challenge"],"assertion":confirm["assertion"]}),
            ))
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
        // A renewed session can recover the old outcome, but cannot confirm a
        // preview bound to the now-revoked session.
        let recovered = app
            .clone()
            .oneshot(request(
                "/api/option/trust-policy/resolve",
                &refreshed.secret,
                json!({"token":token}),
            ))
            .await
            .unwrap();
        assert_eq!(recovered.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(recovered.into_body(), 100_000).await.unwrap())
                .unwrap();
        assert_eq!(body["data"], json!({"state":"confirmed","revision":2}));
        let new_session_confirm = app
            .oneshot(request(
                "/api/option/trust-policy/confirm",
                &refreshed.secret,
                json!({"token":next["data"]["token"],"digest":next["data"]["preview"]["digest"],"challenge":confirm["challenge"],"assertion":confirm["assertion"]}),
            ))
            .await
            .unwrap();
        assert_eq!(new_session_confirm.status(), StatusCode::BAD_REQUEST);
        let final_revision: i64 =
            sqlx::query_scalar("SELECT revision FROM core_trust_pricing.policies WHERE singleton")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(final_revision, 2);
    }

    #[test]
    fn rejects_invalid_authority_and_thresholds() {
        for change in 0..6 {
            let mut policy = fixture();
            match change {
                0 => policy["tiers"][1]["min_paid_credits"] = json!(0),
                1 => policy["tiers"][1]["benefits"] = json!(["administrator_access"]),
                2 => policy["tiers"][1]["benefits"] = json!(["standard_access", "standard_access"]),
                3 => policy["role_tiers"][0]["role"] = json!(100),
                4 => policy["tiers"][0]["discount_ratio"] = json!(0),
                _ => policy["unexpected"] = json!(true),
            }
            assert!(
                ConsolePolicy::parse(&policy.to_string()).is_err(),
                "case {change}"
            );
        }
    }
}

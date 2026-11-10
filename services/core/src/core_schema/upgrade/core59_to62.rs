//! Explicit Core59 -> Core62. Pending captures authorize no connection/DDL.
//! Uses the existing offline session fence, data/ACL preservation, CAS and replay.
use super::*;
pub(super) const PROCEDURE: &str = "core-59-to-62-v1";
const TARGET: &str = include_str!("core59_to62_target.json");
const PRE_MANIFEST: &str = "cd54e4a35500c482ad0385b625edefc160e5dfdeb554b2e0038c00ff81df320c";
const PRE_CATALOG: &str = "38df997ce287e5bfe31b05a50cc1c7a831189b719e3690b7127a3d6080ad5640";
const ENGINE: suffix::Suffix = suffix::Suffix {
    procedure: PROCEDURE,
    predecessor_count: 59,
    target_count: 62,
    predecessor_manifest: PRE_MANIFEST,
    predecessor_catalog: PRE_CATALOG,
    target_json: TARGET,
    receipt: "core_meta.core59_to62_upgrades",
};
const GRANTS: &[(&str, &str)] = &[
    (
        "deployment/postgres/grant-core-runtime.sql",
        include_str!("../../../../../deployment/postgres/grant-core-runtime.sql"),
    ),
    (
        "services/core/schema/commerce_partial_refunds_grants.sql",
        include_str!("../../../schema/commerce_partial_refunds_grants.sql"),
    ),
    (
        "services/core/schema/subscription_stripe_grants.sql",
        include_str!("../../../schema/subscription_stripe_grants.sql"),
    ),
];
const SQL: &[(&str, &str)] = &[
    (
        "federated-auth",
        include_str!("../../../schema/federated_auth.sql"),
    ),
    (
        "commerce-partial-refunds",
        include_str!("../../../schema/commerce_partial_refunds.sql"),
    ),
    (
        "subscription-stripe",
        include_str!("../../../schema/subscription_stripe.sql"),
    ),
];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    component_count: usize,
    manifest_sha256: Option<String>,
    catalog_sha256: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    format_version: u32,
    predecessor: Pin,
    successor: Pin,
    runtime_grants_sha256: Option<BTreeMap<String, String>>,
    frozen_sql_sha256: BTreeMap<String, Option<String>>,
}
fn parse_captured(raw: &str) -> Result<(String, String), UpgradeError> {
    let t: Target = serde_json::from_str(raw).map_err(|_| UpgradeError::Storage)?;
    let Some(grants) = t.runtime_grants_sha256 else {
        return Err(reject("CORE62_TARGET_NOT_CAPTURED"));
    };
    if t.format_version != 1
        || t.predecessor.component_count != 59
        || t.predecessor.manifest_sha256.as_deref() != Some(PRE_MANIFEST)
        || t.predecessor.catalog_sha256.as_deref() != Some(PRE_CATALOG)
        || t.successor.component_count != 62
        || grants.len() != GRANTS.len()
        || t.frozen_sql_sha256.len() != SQL.len()
    {
        return Err(reject("CORE62_CAPTURE_BINDING_INVALID"));
    }
    for (path, bytes) in GRANTS {
        if grants.get(*path) != Some(&hex(&Sha256::digest(bytes.as_bytes()))) {
            return Err(reject("CORE62_REVIEWED_GRANTS_CHANGED"));
        }
    }
    for (name, bytes) in SQL {
        if t.frozen_sql_sha256.get(*name).and_then(|v| v.as_deref())
            != Some(hex(&Sha256::digest(bytes.as_bytes())).as_str())
        {
            return Err(reject("CORE62_REVIEWED_SQL_CHANGED"));
        }
    }
    let manifest = t
        .successor
        .manifest_sha256
        .filter(|v| digest_hex(v))
        .ok_or_else(|| reject("CORE62_TARGET_NOT_CAPTURED"))?;
    let catalog = t
        .successor
        .catalog_sha256
        .filter(|v| digest_hex(v))
        .ok_or_else(|| reject("CORE62_TARGET_NOT_CAPTURED"))?;
    Ok((manifest, catalog))
}
pub(super) fn captured_target() -> Result<(String, String), UpgradeError> {
    parse_captured(TARGET)
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured_target()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    let (manifest, catalog) = captured_target()?;
    let fresh: Pin = serde_json::from_str(include_str!("fresh_target.json"))
        .map_err(|_| UpgradeError::Storage)?;
    if fresh.component_count != 62
        || fresh.manifest_sha256.as_deref() != Some(manifest.as_str())
        || fresh.catalog_sha256.as_deref() != Some(catalog.as_str())
    {
        return Err(reject("CORE62_FRESH_UPGRADE_TARGET_CONFLICT"));
    }
    if COMPONENTS.len() != 62
        || COMPONENTS.get(59).map(|v| v.0) != Some("federated-auth")
        || COMPONENTS.get(60).map(|v| v.0) != Some("commerce-partial-refunds")
        || COMPONENTS.get(61).map(|v| v.0) != Some("subscription-stripe")
        || COMPONENTS[61].1
            != super::super::navigation_options::SUBSCRIPTION_STRIPE_WITH_UPGRADE_RECEIPT
    {
        return Err(reject("CORE62_COMPILED_COMPONENTS_CHANGED"));
    }
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    authorize_target()?;
    ENGINE.connected(c, r).await
}
// Only the reviewed delta grants are installed; never evaluate psql feature
// flags or rerun broad grants on predecessor objects. Existing ACLs stay fenced.
fn auth_fragment(raw: &str) -> Result<&str, UpgradeError> {
    let (before, rest) = raw
        .split_once("\\if :enable_federated_auth\n")
        .ok_or_else(|| reject("CORE62_AUTH_GRANTS_STRUCTURE_CHANGED"))?;
    let (body, after) = rest
        .split_once("\\endif")
        .ok_or_else(|| reject("CORE62_AUTH_GRANTS_STRUCTURE_CHANGED"))?;
    if before.contains("\\if :enable_federated_auth\n")
        || after.contains("\\if :enable_federated_auth\n")
        || body.contains('\\')
    {
        return Err(reject("CORE62_AUTH_GRANTS_STRUCTURE_CHANGED"));
    }
    Ok(body)
}
pub(super) async fn grant_runtime(c: &mut PgConnection, role: &str) -> Result<(), UpgradeError> {
    captured_target()?;
    for raw in [auth_fragment(GRANTS[0].1)?, GRANTS[1].1, GRANTS[2].1] {
        let sql = raw.replace(":\"runtime_role\"", &quote(role));
        sqlx::raw_sql(&sql).execute(&mut *c).await?;
    }
    // Match root's new metadata read baseline only on the NEW receipt. Do not
    // repair permissions on any predecessor table inside this upgrade.
    sqlx::raw_sql(&format!(
        "GRANT SELECT ON core_meta.core59_to62_upgrades TO {}",
        quote(role)
    ))
    .execute(&mut *c)
    .await?;
    Ok(())
}
// The coordinated runtime baseline deliberately permits only named SELECT.
pub(super) async fn receipt_privileges(
    c: &mut PgConnection,
    runtime: &str,
) -> Result<(), UpgradeError> {
    let safe: bool = sqlx::query_scalar(
        r#"
SELECT NOT EXISTS(SELECT FROM pg_class c,
 LATERAL aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a
 WHERE c.oid='core_meta.core59_to62_upgrades'::regclass AND a.grantee<>c.relowner
 AND (a.grantee<>coalesce((SELECT oid FROM pg_roles WHERE rolname=$1),0)
      OR a.privilege_type<>'SELECT' OR a.is_grantable))
 AND NOT EXISTS(SELECT FROM pg_attribute t JOIN pg_class c ON c.oid=t.attrelid,
 LATERAL aclexplode(t.attacl) a
 WHERE c.oid='core_meta.core59_to62_upgrades'::regclass AND a.grantee<>c.relowner)
"#,
    )
    .bind(runtime)
    .fetch_one(c)
    .await?;
    if !safe {
        return Err(reject("CORE62_RECEIPT_PRIVILEGES_INVALID"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_core62_cannot_authorize_database_access() {
        let mut value: serde_json::Value = serde_json::from_str(TARGET).unwrap();
        value["runtime_grants_sha256"] = serde_json::Value::Null;
        assert!(parse_captured(&value.to_string()).is_err());
    }
    #[test]
    fn capture_binds_old_predecessor_and_all_reviewed_sql_and_grants() {
        let mut value: serde_json::Value = serde_json::from_str(TARGET).unwrap();
        value["runtime_grants_sha256"] = serde_json::json!(
            GRANTS
                .iter()
                .map(|(p, b)| ((*p).to_owned(), hex(&Sha256::digest(b.as_bytes()))))
                .collect::<BTreeMap<_, _>>()
        );
        value["frozen_sql_sha256"] = serde_json::json!(
            SQL.iter()
                .map(|(p, b)| ((*p).to_owned(), hex(&Sha256::digest(b.as_bytes()))))
                .collect::<BTreeMap<_, _>>()
        );
        value["successor"]["manifest_sha256"] = serde_json::json!("ab".repeat(32));
        value["successor"]["catalog_sha256"] = serde_json::json!("cd".repeat(32));
        assert!(parse_captured(&value.to_string()).is_ok());
        for (section, key) in [
            ("predecessor", "catalog_sha256"),
            ("predecessor", "manifest_sha256"),
            ("runtime_grants_sha256", GRANTS[0].0),
            ("frozen_sql_sha256", SQL[0].0),
        ] {
            let mut drift = value.clone();
            drift[section][key] = serde_json::json!("00".repeat(32));
            assert!(parse_captured(&drift.to_string()).is_err());
        }
    }
    #[test]
    fn auth_grants_reject_nested_or_duplicate_psql_blocks() {
        assert!(auth_fragment(GRANTS[0].1).is_ok());
        assert!(auth_fragment("\\if :enable_federated_auth\n\\if true\n\\endif").is_err());
        assert!(auth_fragment("\\if :enable_federated_auth\nSELECT 1;\n\\endif\n\\if :enable_federated_auth\nSELECT 2;\n\\endif").is_err());
    }
}

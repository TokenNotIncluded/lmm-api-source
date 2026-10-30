//! Fixed 28 -> 29 component upgrade. No balances or provider evidence are imported.
use super::{
    finance::{data, data_hash, object_ids, permissions},
    *,
};

pub(super) const PROCEDURE: &str = "core-payment-gateways-v1";
// Observed from the exact 6523c4b predecessor on PostgreSQL 18.6, UTF-8.
const PRE_MANIFEST: &str = "207bc6bf3f39dc9803177e6cebda491150fb66370a5ff3fc37b26b863cfec97b";
const PRE_CATALOG: &str = "5521fb8224fbdfc994421f0fe04756df99a4e341ceec78b6302882a1f145d2d0";
const PRE_TABLES: usize = 175;
const TARGET: &str = include_str!("payment_gateways_target.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    manifest_sha256: String,
    catalog_sha256: String,
    definitions: Vec<String>,
}

fn target() -> Result<Target, UpgradeError> {
    let target: Target = serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if target.manifest_sha256 != manifest_sha256()?
        || target.catalog_sha256 != hex(&fingerprint(&target.definitions))
        || target.definitions.is_empty()
    {
        return Err(reject("PAYMENT_GATEWAYS_COMPILED_TARGET_CHANGED"));
    }
    Ok(target)
}

pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    let roles = [
        &r.roles.installer,
        &r.roles.owner,
        &r.roles.runtime,
        &r.roles.extensions,
    ];
    if r.version != 1
        || r.procedure != PROCEDURE
        || !(16..=96).contains(&r.upgrade_id.len())
        || !r
            .upgrade_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        || !identifier(&r.database.name)
        || r.database.oid == 0
        || !(180_000..190_000).contains(&r.database.server_version_num)
        || r.database.system_identifier.len() > 20
        || !r
            .database
            .system_identifier
            .bytes()
            .all(|c| c.is_ascii_digit())
        || r.database
            .system_identifier
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0)
            .is_none()
        || roles.iter().any(|v| !identifier(v))
        || roles
            .iter()
            .enumerate()
            .any(|(i, v)| roles[..i].contains(v))
        || r.predecessor_manifest_sha256 != PRE_MANIFEST
        || r.predecessor_catalog_sha256 != PRE_CATALOG
        || r.successor_manifest_sha256 != manifest_sha256()?
    {
        return Err(reject("PAYMENT_GATEWAYS_UPGRADE_CONTRACT_INVALID"));
    }
    Ok(())
}

pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.len() != 29 || COMPONENTS[28].0 != "payment-gateways" {
        return Err(reject("PAYMENT_GATEWAYS_COMPILED_COMPONENTS_CHANGED"));
    }
    let mut predecessor = schema_manifest();
    predecessor.components.truncate(28);
    let mut bytes = serde_json::to_vec(&predecessor).map_err(|_| UpgradeError::Storage)?;
    bytes.push(b'\n');
    if hex(&Sha256::digest(bytes)) != PRE_MANIFEST {
        return Err(reject("PAYMENT_GATEWAYS_COMPILED_PREDECESSOR_CHANGED"));
    }
    target()?;
    Ok(())
}

async fn privileges(c: &mut PgConnection, r: &UpgradeRequest) -> Result<(), UpgradeError> {
    // Default grants must not turn an owner-only financial upgrade receipt into
    // a runtime, extension, PUBLIC or unrelated-role data source.
    let safe: bool = sqlx::query_scalar(r#"
SELECT NOT has_table_privilege($1,'core_meta.payment_gateway_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
 AND NOT has_any_column_privilege($1,'core_meta.payment_gateway_upgrades','SELECT,INSERT,UPDATE,REFERENCES')
 AND NOT has_table_privilege($2,'core_meta.payment_gateway_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
 AND NOT has_any_column_privilege($2,'core_meta.payment_gateway_upgrades','SELECT,INSERT,UPDATE,REFERENCES')
 AND NOT EXISTS(SELECT FROM pg_class c CROSS JOIN LATERAL aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a
   WHERE c.oid='core_meta.payment_gateway_upgrades'::regclass AND a.grantee<>c.relowner)
 AND NOT EXISTS(SELECT FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid CROSS JOIN LATERAL aclexplode(a.attacl) acl
   WHERE c.oid='core_meta.payment_gateway_upgrades'::regclass AND acl.grantee<>c.relowner)
"#).bind(&r.roles.runtime).bind(&r.roles.extensions).fetch_one(c).await?;
    if !safe {
        return Err(reject("PAYMENT_GATEWAYS_RECEIPT_PRIVILEGES_INVALID"));
    }
    Ok(())
}

pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    let target = target()?;
    // Shared guard reauthorizes the target before touching the connection,
    // fences the installer session, verifies offline roles and locks all tables.
    let (mut tx, backend_pid, tables) = begin_offline_upgrade(c, r).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_meta.payment_gateway_upgrades') IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    if exists {
        // Resolve only the original request after an unknown COMMIT outcome.
        // A fresh install, new ID or changed request must never authorize DDL.
        let row = sqlx::query("SELECT request_sha256,request,successor_catalog_sha256 FROM core_meta.payment_gateway_upgrades WHERE upgrade_id=$1")
            .bind(&r.upgrade_id).fetch_optional(&mut *tx).await?
            .ok_or_else(|| reject("PAYMENT_GATEWAYS_ORIGINAL_RECEIPT_ABSENT"))?;
        let request: sqlx::types::Json<serde_json::Value> = row.try_get("request")?;
        if row.try_get::<Vec<u8>, _>("request_sha256")? != r.digest()?
            || request.0 != serde_json::to_value(r).map_err(|_| UpgradeError::Storage)?
            || hex(&row.try_get::<Vec<u8>, _>("successor_catalog_sha256")?) != target.catalog_sha256
        {
            return Err(reject("PAYMENT_GATEWAYS_RECEIPT_CONFLICT"));
        }
        if definitions(&mut tx).await? != target.definitions {
            return Err(reject("PAYMENT_GATEWAYS_TARGET_CATALOG_CHANGED"));
        }
        super::super::check_connection(&mut tx).await?;
        privileges(&mut tx, r).await?;
        tx.rollback().await?;
        return report(r, &target.catalog_sha256, "already_committed");
    }
    if hex(&fingerprint(&definitions(&mut tx).await?)) != PRE_CATALOG || tables.len() != PRE_TABLES
    {
        return Err(reject("PAYMENT_GATEWAYS_PREDECESSOR_CATALOG_MISMATCH"));
    }
    let recorded: Vec<u8> = sqlx::query_scalar(
        "SELECT catalog_fingerprint FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?;
    if hex(&recorded) != PRE_CATALOG {
        return Err(reject("PAYMENT_GATEWAYS_PREDECESSOR_RECEIPT_MISMATCH"));
    }
    let expected: BTreeMap<String, Vec<u8>> = COMPONENTS[..28]
        .iter()
        .map(|(n, sql)| (n.to_string(), Sha256::digest(sql.as_bytes()).to_vec()))
        .collect();
    let actual: BTreeMap<String, Vec<u8>> =
        sqlx::query("SELECT name,fingerprint FROM core_meta.component_contract")
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .map(|row| Ok((row.try_get("name")?, row.try_get("fingerprint")?)))
            .collect::<Result<_, sqlx::Error>>()?;
    if actual != expected {
        return Err(reject("PAYMENT_GATEWAYS_PREDECESSOR_COMPONENTS_MISMATCH"));
    }
    let reused: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_payments.intents WHERE provider_transaction IS NOT NULL GROUP BY provider,merchant,environment,provider_transaction HAVING count(*)>1)")
        .fetch_one(&mut *tx).await?;
    if reused {
        return Err(reject("PAYMENT_GATEWAYS_EXISTING_TRANSACTION_REUSED"));
    }
    let relations = object_ids(&mut tx, "relation").await?;
    let types = object_ids(&mut tx, "type").await?;
    let old_permissions = permissions(&mut tx, &relations, &types).await?;
    let old_data = data(&mut tx, &tables).await?;
    sqlx::raw_sql(COMPONENTS[28].1).execute(&mut *tx).await?;
    privileges(&mut tx, r).await?;
    if definitions(&mut tx).await? != target.definitions
        || permissions(&mut tx, &relations, &types).await? != old_permissions
        || data(&mut tx, &tables).await? != old_data
    {
        return Err(reject("PAYMENT_GATEWAYS_PRESERVATION_OR_TARGET_MISMATCH"));
    }
    sqlx::query(
        "INSERT INTO core_meta.component_contract(name,fingerprint) VALUES('payment-gateways',$1)",
    )
    .bind(Sha256::digest(COMPONENTS[28].1.as_bytes()).to_vec())
    .execute(&mut *tx)
    .await?;
    let changed = sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton AND catalog_fingerprint=$2")
        .bind(fingerprint(&target.definitions)).bind(&recorded).execute(&mut *tx).await?;
    if changed.rows_affected() != 1 {
        return Err(reject("PAYMENT_GATEWAYS_RUNTIME_RECEIPT_CONFLICT"));
    }
    sqlx::query("INSERT INTO core_meta.payment_gateway_upgrades(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(&r.upgrade_id).bind(r.digest()?).bind(sqlx::types::Json(serde_json::to_value(r).map_err(|_|UpgradeError::Storage)?))
        .bind(data_hash(&old_data)).bind(Sha256::digest(serde_json::to_vec(&old_permissions).map_err(|_|UpgradeError::Storage)?).to_vec())
        .bind(fingerprint(&target.definitions)).execute(&mut *tx).await?;
    super::super::check_connection(&mut tx).await?;
    preflight(&mut tx, r).await?;
    commit_offline_upgrade(tx, r, backend_pid).await?;
    report(r, &target.catalog_sha256, "committed")
}

fn report(
    r: &UpgradeRequest,
    catalog: &str,
    status: &'static str,
) -> Result<UpgradeReport, UpgradeError> {
    Ok(UpgradeReport {
        version: 1,
        protocol: "core.upgrade-db.v1",
        status,
        upgrade_id: r.upgrade_id.clone(),
        request_sha256: hex(&r.digest()?),
        predecessor_catalog_sha256: PRE_CATALOG,
        successor_catalog_sha256: catalog.to_string(),
        components: 29,
    })
}

#[cfg(test)]
#[path = "payment_gateways_tests.rs"]
mod tests;

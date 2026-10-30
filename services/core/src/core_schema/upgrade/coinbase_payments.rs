//! Fixed 31 -> 32 component upgrade; existing money and payment proofs stay intact.
use super::{
    finance::{data_hash, object_ids, permissions},
    request_operations::preservation::data,
    *,
};

pub(super) const PROCEDURE: &str = "core-coinbase-payments-v1";
// Exact b521f557 predecessor and appended component, observed on PostgreSQL 18.6.
const PRE_MANIFEST: &str = "6991ca03dbef6ea9ec788bf191d022f8e6a45f5f7ecbb1190255e5dd9c1b5735";
const PRE_CATALOG: &str = "68cedab606bceb54212a78fb595b65755188ee17f8155e3a9588e554548e482e";
const TARGET_MANIFEST: &str = "9c9726a7fa204a150d9fe18139d4580d23feb4cec40145fcb829338331a4cd15";
const TARGET_CATALOG: &str = "abe3979b376da869d4a05b32dda8c3ad33197497db7af16970d8c9b1153bc6ce";

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
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || !identifier(&r.database.name)
        || r.database.oid == 0
        || !(180_000..190_000).contains(&r.database.server_version_num)
        || r.database.system_identifier.len() > 20
        || !r
            .database
            .system_identifier
            .bytes()
            .all(|b| b.is_ascii_digit())
        || r.database
            .system_identifier
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .is_none()
        || roles.iter().any(|role| !identifier(role))
        || roles
            .iter()
            .enumerate()
            .any(|(i, role)| roles[..i].contains(role))
        || r.predecessor_manifest_sha256 != PRE_MANIFEST
        || r.predecessor_catalog_sha256 != PRE_CATALOG
        || r.successor_manifest_sha256 != TARGET_MANIFEST
    {
        return Err(reject("COINBASE_UPGRADE_CONTRACT_INVALID"));
    }
    Ok(())
}

pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.len() != 32
        || COMPONENTS[31].0 != "coinbase-payments"
        || manifest_sha256()? != TARGET_MANIFEST
    {
        return Err(reject("COINBASE_COMPILED_TARGET_CHANGED"));
    }
    let mut prior = schema_manifest();
    prior.components.truncate(31);
    let mut bytes = serde_json::to_vec(&prior).map_err(|_| UpgradeError::Storage)?;
    bytes.push(b'\n');
    if hex(&Sha256::digest(bytes)) != PRE_MANIFEST {
        return Err(reject("COINBASE_COMPILED_PREDECESSOR_CHANGED"));
    }
    Ok(())
}

async fn privileges(c: &mut PgConnection, r: &UpgradeRequest) -> Result<(), UpgradeError> {
    let safe: bool = sqlx::query_scalar(r#"
SELECT NOT has_table_privilege($1,'core_meta.coinbase_payment_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
 AND NOT has_any_column_privilege($1,'core_meta.coinbase_payment_upgrades','SELECT,INSERT,UPDATE,REFERENCES')
 AND NOT has_table_privilege($2,'core_meta.coinbase_payment_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
 AND NOT has_any_column_privilege($2,'core_meta.coinbase_payment_upgrades','SELECT,INSERT,UPDATE,REFERENCES')
 AND NOT EXISTS(SELECT FROM pg_class c CROSS JOIN LATERAL aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a
   WHERE c.oid='core_meta.coinbase_payment_upgrades'::regclass AND a.grantee<>c.relowner)
 AND NOT EXISTS(SELECT FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid CROSS JOIN LATERAL aclexplode(a.attacl) acl
   WHERE c.oid='core_meta.coinbase_payment_upgrades'::regclass AND acl.grantee<>c.relowner)
"#).bind(&r.roles.runtime).bind(&r.roles.extensions).fetch_one(c).await?;
    if !safe {
        return Err(reject("COINBASE_RECEIPT_PRIVILEGES_INVALID"));
    }
    Ok(())
}

pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    let (mut tx, backend_pid, tables) = begin_offline_upgrade(c, r).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_meta.coinbase_payment_upgrades') IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    if exists {
        // An unknown COMMIT resolves only against the original durable request.
        let row = sqlx::query("SELECT request_sha256,request,successor_catalog_sha256 FROM core_meta.coinbase_payment_upgrades WHERE upgrade_id=$1")
            .bind(&r.upgrade_id).fetch_optional(&mut *tx).await?
            .ok_or_else(|| reject("COINBASE_ORIGINAL_RECEIPT_ABSENT"))?;
        let request: sqlx::types::Json<serde_json::Value> = row.try_get("request")?;
        if row.try_get::<Vec<u8>, _>("request_sha256")? != r.digest()?
            || request.0 != serde_json::to_value(r).map_err(|_| UpgradeError::Storage)?
            || hex(&row.try_get::<Vec<u8>, _>("successor_catalog_sha256")?) != TARGET_CATALOG
            || hex(&fingerprint(&definitions(&mut tx).await?)) != TARGET_CATALOG
        {
            return Err(reject("COINBASE_RECEIPT_OR_CATALOG_CONFLICT"));
        }
        super::super::check_connection(&mut tx).await?;
        privileges(&mut tx, r).await?;
        tx.rollback().await?;
        return report(r, "already_committed");
    }
    if hex(&fingerprint(&definitions(&mut tx).await?)) != PRE_CATALOG {
        return Err(reject("COINBASE_PREDECESSOR_CATALOG_MISMATCH"));
    }
    let recorded: Vec<u8> = sqlx::query_scalar(
        "SELECT catalog_fingerprint FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?;
    if hex(&recorded) != PRE_CATALOG {
        return Err(reject("COINBASE_PREDECESSOR_RECEIPT_MISMATCH"));
    }
    let expected: BTreeMap<String, Vec<u8>> = COMPONENTS[..31]
        .iter()
        .map(|(name, ddl)| (name.to_string(), Sha256::digest(ddl.as_bytes()).to_vec()))
        .collect();
    let actual: BTreeMap<String, Vec<u8>> =
        sqlx::query("SELECT name,fingerprint FROM core_meta.component_contract")
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .map(|row| Ok((row.try_get("name")?, row.try_get("fingerprint")?)))
            .collect::<Result<_, sqlx::Error>>()?;
    if actual != expected {
        return Err(reject("COINBASE_PREDECESSOR_COMPONENTS_MISMATCH"));
    }
    let relations = object_ids(&mut tx, "relation").await?;
    let types = object_ids(&mut tx, "type").await?;
    let sequences: Vec<String> = sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname COLLATE \"C\",c.relname COLLATE \"C\"")
        .fetch_all(&mut *tx).await?;
    let old_permissions = permissions(&mut tx, &relations, &types).await?;
    let old_data = data(&mut tx, &tables, &sequences).await?;
    sqlx::raw_sql(COMPONENTS[31].1).execute(&mut *tx).await?;
    privileges(&mut tx, r).await?;
    let catalog = fingerprint(&definitions(&mut tx).await?);
    if hex(&catalog) != TARGET_CATALOG
        || permissions(&mut tx, &relations, &types).await? != old_permissions
        || data(&mut tx, &tables, &sequences).await? != old_data
    {
        return Err(reject("COINBASE_PRESERVATION_OR_TARGET_MISMATCH"));
    }
    sqlx::query(
        "INSERT INTO core_meta.component_contract(name,fingerprint) VALUES('coinbase-payments',$1)",
    )
    .bind(Sha256::digest(COMPONENTS[31].1.as_bytes()).to_vec())
    .execute(&mut *tx)
    .await?;
    let changed = sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton AND catalog_fingerprint=$2")
        .bind(&catalog).bind(&recorded).execute(&mut *tx).await?;
    if changed.rows_affected() != 1 {
        return Err(reject("COINBASE_RUNTIME_RECEIPT_CONFLICT"));
    }
    sqlx::query("INSERT INTO core_meta.coinbase_payment_upgrades(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(&r.upgrade_id).bind(r.digest()?).bind(sqlx::types::Json(serde_json::to_value(r).map_err(|_| UpgradeError::Storage)?))
        .bind(data_hash(&old_data)).bind(Sha256::digest(serde_json::to_vec(&old_permissions).map_err(|_| UpgradeError::Storage)?).to_vec())
        .bind(catalog).execute(&mut *tx).await?;
    super::super::check_connection(&mut tx).await?;
    preflight(&mut tx, r).await?;
    commit_offline_upgrade(tx, r, backend_pid).await?;
    report(r, "committed")
}

fn report(r: &UpgradeRequest, status: &'static str) -> Result<UpgradeReport, UpgradeError> {
    Ok(UpgradeReport {
        version: 1,
        protocol: "core.upgrade-db.v1",
        status,
        upgrade_id: r.upgrade_id.clone(),
        request_sha256: hex(&r.digest()?),
        predecessor_catalog_sha256: PRE_CATALOG,
        successor_catalog_sha256: TARGET_CATALOG.into(),
        components: 32,
    })
}

#[cfg(test)]
#[path = "coinbase_payments_tests.rs"]
mod tests;

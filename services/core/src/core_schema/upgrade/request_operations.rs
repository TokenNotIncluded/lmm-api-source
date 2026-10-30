//! Fixed additive 29 -> 31 upgrade. Preserves all existing data and authority.
use super::{
    finance::{data_hash, object_ids},
    *,
};
pub(super) mod preservation;
use preservation::{data, permissions, sequences};

pub(super) const PROCEDURE: &str = "core-request-operations-v1";
const PRE_MANIFEST: &str = "48578c69632d80d7d4e41a5fb36c67f365875e8bb67e0695f62e1d688177f530";
const PRE_CATALOG: &str = "cb9349ca60bafbd49204dc237a88eec5ca10ac4654e654e7c816ea59673bc21e";
const PREDECESSOR: &str = include_str!("payment_gateways_target.json");
// New foreign keys add precisely these RI triggers on the existing parent.
// Treat definitions as a multiset: policy_events has an identically named
// predecessor constraint, so remove only one newly introduced occurrence.
const PARENT_ADDITIONS: [&str; 4] = [
    r#"["internal-trigger", "core_identity", "users", "log_cleanup_jobs_created_by_fkey", "pg_catalog", "RI_FKey_noaction_del", 9, "O", false, false, "", "", null]"#,
    r#"["internal-trigger", "core_identity", "users", "log_cleanup_jobs_created_by_fkey", "pg_catalog", "RI_FKey_noaction_upd", 17, "O", false, false, "", "", null]"#,
    r#"["internal-trigger", "core_identity", "users", "policy_events_actor_user_id_fkey", "pg_catalog", "RI_FKey_noaction_del", 9, "O", false, false, "", "", null]"#,
    r#"["internal-trigger", "core_identity", "users", "policy_events_actor_user_id_fkey", "pg_catalog", "RI_FKey_noaction_upd", 17, "O", false, false, "", "", null]"#,
];

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
        return Err(reject("REQUEST_OPERATIONS_UPGRADE_CONTRACT_INVALID"));
    }
    Ok(())
}
fn predecessor_definitions() -> Result<Vec<String>, UpgradeError> {
    let value: serde_json::Value =
        serde_json::from_str(PREDECESSOR).map_err(|_| UpgradeError::Storage)?;
    let defs: Vec<String> =
        serde_json::from_value(value["definitions"].clone()).map_err(|_| UpgradeError::Storage)?;
    if value["manifest_sha256"] != PRE_MANIFEST
        || value["catalog_sha256"] != PRE_CATALOG
        || hex(&fingerprint(&defs)) != PRE_CATALOG
    {
        return Err(reject("REQUEST_OPERATIONS_PREDECESSOR_CHANGED"));
    }
    Ok(defs)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.len() != 31
        || COMPONENTS[29].0 != "request-limits"
        || COMPONENTS[30].0 != "operations"
    {
        return Err(reject("REQUEST_OPERATIONS_COMPILED_COMPONENTS_CHANGED"));
    }
    let mut prior = schema_manifest();
    prior.components.truncate(29);
    let mut bytes = serde_json::to_vec(&prior).map_err(|_| UpgradeError::Storage)?;
    bytes.push(b'\n');
    if hex(&Sha256::digest(bytes)) != PRE_MANIFEST {
        return Err(reject("REQUEST_OPERATIONS_PREDECESSOR_CHANGED"));
    }
    predecessor_definitions()?;
    Ok(())
}
fn preserves_catalog(defs: &[String]) -> Result<(), UpgradeError> {
    let mut original = Vec::new();
    let mut additions = PARENT_ADDITIONS.to_vec();
    for definition in defs {
        let v: serde_json::Value =
            serde_json::from_str(definition).map_err(|_| UpgradeError::Storage)?;
        if !matches!(
            v[1].as_str(),
            Some("core_request_limits" | "core_operations")
        ) {
            if let Some(at) = additions
                .iter()
                .position(|expected| *expected == definition)
            {
                additions.remove(at);
                continue;
            }
            original.push(definition.clone());
        }
    }
    if !additions.is_empty() || original != predecessor_definitions()? {
        return Err(reject("REQUEST_OPERATIONS_EXISTING_CATALOG_CHANGED"));
    }
    Ok(())
}

async fn grant(c: &mut PgConnection, r: &UpgradeRequest) -> Result<(), UpgradeError> {
    let role = quote(&r.roles.runtime);
    sqlx::raw_sql(&format!(
        r#"
        GRANT USAGE ON SCHEMA core_operations,core_request_limits TO {role};
        GRANT SELECT ON core_request_limits.policy,core_request_limits.auto_policy TO {role};
        GRANT EXECUTE ON FUNCTION core_request_limits.admit(bigint,text,text),
          core_request_limits.write_policy(bigint,boolean,integer,integer,jsonb,bigint),
          core_request_limits.write_auto_groups(bigint,text[],bigint) TO {role};
        GRANT SELECT,INSERT ON core_operations.log_cleanup_jobs TO {role};
        GRANT USAGE ON SEQUENCE core_operations.log_cleanup_jobs_id_seq TO {role};
        GRANT SELECT,INSERT,UPDATE,DELETE ON core_operations.instances TO {role};
        GRANT EXECUTE ON FUNCTION core_operations.run_log_cleanup_batch() TO {role};
    "#
    ))
    .execute(c)
    .await?;
    Ok(())
}
async fn privileges(c: &mut PgConnection, r: &UpgradeRequest) -> Result<(), UpgradeError> {
    let unsafe_rights:bool=sqlx::query_scalar(r#"
        SELECT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
          WHERE n.nspname IN ('core_operations','core_request_limits') AND c.relkind='r' AND (
            has_table_privilege($1,c.oid,'TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
            OR has_table_privilege($2,c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
            OR has_any_column_privilege($2,c.oid,'SELECT,INSERT,UPDATE,REFERENCES')
            OR (n.nspname='core_request_limits' AND c.relname IN ('policy','auto_policy') AND (has_table_privilege($1,c.oid,'INSERT,UPDATE,DELETE') OR has_any_column_privilege($1,c.oid,'INSERT,UPDATE,REFERENCES')))
            OR ((n.nspname='core_request_limits' AND c.relname NOT IN ('policy','auto_policy') OR n.nspname='core_operations' AND c.relname='upgrade_receipts') AND (has_table_privilege($1,c.oid,'SELECT,INSERT,UPDATE,DELETE') OR has_any_column_privilege($1,c.oid,'SELECT,INSERT,UPDATE,REFERENCES')))
            OR (n.nspname='core_operations' AND c.relname='log_cleanup_jobs' AND (has_table_privilege($1,c.oid,'UPDATE,DELETE') OR has_any_column_privilege($1,c.oid,'UPDATE,REFERENCES')))))
        OR EXISTS(SELECT FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
          WHERE n.nspname IN ('core_operations','core_request_limits') AND has_function_privilege($2,p.oid,'EXECUTE'))
        OR EXISTS(SELECT FROM pg_class c CROSS JOIN LATERAL aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a
          WHERE c.oid='core_operations.upgrade_receipts'::regclass AND a.grantee<>c.relowner)
        OR EXISTS(SELECT FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid CROSS JOIN LATERAL aclexplode(a.attacl) acl
          WHERE c.oid='core_operations.upgrade_receipts'::regclass AND acl.grantee<>c.relowner)
    "#).bind(&r.roles.runtime).bind(&r.roles.extensions).fetch_one(c).await?;
    if unsafe_rights {
        return Err(reject("REQUEST_OPERATIONS_PRIVILEGES_INVALID"));
    }
    Ok(())
}

pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    let (mut tx, backend_pid, tables) = begin_offline_upgrade(c, r).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_operations.upgrade_receipts') IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    if exists {
        let row=sqlx::query("SELECT request_sha256,request,successor_catalog_sha256 FROM core_operations.upgrade_receipts WHERE upgrade_id=$1")
            .bind(&r.upgrade_id).fetch_optional(&mut *tx).await?.ok_or_else(||reject("REQUEST_OPERATIONS_ORIGINAL_RECEIPT_ABSENT"))?;
        let request: sqlx::types::Json<serde_json::Value> = row.try_get("request")?;
        let catalog: Vec<u8> = row.try_get("successor_catalog_sha256")?;
        let defs = definitions(&mut tx).await?;
        if row.try_get::<Vec<u8>, _>("request_sha256")? != r.digest()?
            || request.0 != serde_json::to_value(r).map_err(|_| UpgradeError::Storage)?
            || catalog != fingerprint(&defs)
        {
            return Err(reject("REQUEST_OPERATIONS_RECEIPT_CONFLICT"));
        }
        preserves_catalog(&defs)?;
        super::super::check_connection(&mut tx).await?;
        privileges(&mut tx, r).await?;
        tx.rollback().await?;
        return report(r, &catalog, "already_committed");
    }
    if definitions(&mut tx).await? != predecessor_definitions()? {
        return Err(reject("REQUEST_OPERATIONS_PREDECESSOR_CATALOG_MISMATCH"));
    }
    let recorded: Vec<u8> = sqlx::query_scalar(
        "SELECT catalog_fingerprint FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?;
    if hex(&recorded) != PRE_CATALOG {
        return Err(reject("REQUEST_OPERATIONS_PREDECESSOR_RECEIPT_MISMATCH"));
    }
    let expected: BTreeMap<String, Vec<u8>> = COMPONENTS[..29]
        .iter()
        .map(|(n, s)| (n.to_string(), Sha256::digest(s.as_bytes()).to_vec()))
        .collect();
    let actual: BTreeMap<String, Vec<u8>> =
        sqlx::query("SELECT name,fingerprint FROM core_meta.component_contract")
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .map(|row| Ok((row.try_get("name")?, row.try_get("fingerprint")?)))
            .collect::<Result<_, sqlx::Error>>()?;
    if actual != expected {
        return Err(reject("REQUEST_OPERATIONS_PREDECESSOR_COMPONENTS_MISMATCH"));
    }
    let relations = object_ids(&mut tx, "relation").await?;
    let types = object_ids(&mut tx, "type").await?;
    let old_permissions = permissions(&mut tx, &relations, &types).await?;
    let old_sequences = sequences(&mut tx).await?;
    let old_data = data(&mut tx, &tables, &old_sequences).await?;
    for (_, ddl) in &COMPONENTS[29..] {
        sqlx::raw_sql(ddl).execute(&mut *tx).await?;
    }
    grant(&mut tx, r).await?;
    privileges(&mut tx, r).await?;
    let defs = definitions(&mut tx).await?;
    preserves_catalog(&defs)?;
    if permissions(&mut tx, &relations, &types).await? != old_permissions
        || data(&mut tx, &tables, &old_sequences).await? != old_data
    {
        return Err(reject(
            "REQUEST_OPERATIONS_EXISTING_DATA_OR_PERMISSIONS_CHANGED",
        ));
    }
    for (name, ddl) in &COMPONENTS[29..] {
        sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES($1,$2)")
            .bind(name)
            .bind(Sha256::digest(ddl.as_bytes()).to_vec())
            .execute(&mut *tx)
            .await?;
    }
    let catalog = fingerprint(&defs);
    let changed=sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton AND catalog_fingerprint=$2")
        .bind(&catalog).bind(&recorded).execute(&mut *tx).await?;
    if changed.rows_affected() != 1 {
        return Err(reject("REQUEST_OPERATIONS_RUNTIME_RECEIPT_CONFLICT"));
    }
    sqlx::query("INSERT INTO core_operations.upgrade_receipts(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(&r.upgrade_id).bind(r.digest()?).bind(sqlx::types::Json(serde_json::to_value(r).map_err(|_|UpgradeError::Storage)?))
        .bind(data_hash(&old_data)).bind(Sha256::digest(serde_json::to_vec(&old_permissions).map_err(|_|UpgradeError::Storage)?).to_vec())
        .bind(&catalog).execute(&mut *tx).await?;
    super::super::check_connection(&mut tx).await?;
    preflight(&mut tx, r).await?;
    commit_offline_upgrade(tx, r, backend_pid).await?;
    report(r, &catalog, "committed")
}
fn report(
    r: &UpgradeRequest,
    catalog: &[u8],
    status: &'static str,
) -> Result<UpgradeReport, UpgradeError> {
    Ok(UpgradeReport {
        version: 1,
        protocol: "core.upgrade-db.v1",
        status,
        upgrade_id: r.upgrade_id.clone(),
        request_sha256: hex(&r.digest()?),
        predecessor_catalog_sha256: PRE_CATALOG,
        successor_catalog_sha256: hex(catalog),
        components: 31,
    })
}

#[cfg(test)]
mod tests;

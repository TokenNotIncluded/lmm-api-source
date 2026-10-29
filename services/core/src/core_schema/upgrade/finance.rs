//! Fixed 27-component -> site-finance upgrade. No source facts are imported.
use super::*;

pub(super) const PROCEDURE: &str = "core-site-finance-v1";
const PRE_MANIFEST: &str = "47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078";
const PRE_CATALOG: &str = "557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d";
const TARGET: &str = include_str!("finance_target.json");
const PREDECESSOR_CATALOG_QUERY: &str = include_str!("finance_predecessor_catalog.sql");

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
        return Err(reject("SITE_FINANCE_COMPILED_TARGET_CHANGED"));
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
        return Err(reject("SITE_FINANCE_UPGRADE_CONTRACT_INVALID"));
    }
    Ok(())
}

pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.len() != 28 || COMPONENTS[27].0 != "site-finance" {
        return Err(reject("SITE_FINANCE_COMPILED_COMPONENTS_CHANGED"));
    }
    let mut predecessor = schema_manifest();
    predecessor.components.truncate(27);
    predecessor.catalog.query_utf8 = PREDECESSOR_CATALOG_QUERY;
    predecessor.catalog.query_sha256 = hex(&Sha256::digest(PREDECESSOR_CATALOG_QUERY.as_bytes()));
    let mut bytes = serde_json::to_vec(&predecessor).map_err(|_| UpgradeError::Storage)?;
    bytes.push(b'\n');
    if hex(&Sha256::digest(bytes)) != PRE_MANIFEST {
        return Err(reject("SITE_FINANCE_COMPILED_PREDECESSOR_CHANGED"));
    }
    target()?;
    Ok(())
}

async fn object_ids(c: &mut PgConnection, catalog: &str) -> Result<Vec<i64>, UpgradeError> {
    // Caller selects only these two static pg catalogs, never request SQL.
    let sql = match catalog {
        "relation" => {
            "SELECT c.oid::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' ORDER BY c.oid"
        }
        "type" => {
            "SELECT t.oid::bigint FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' ORDER BY t.oid"
        }
        _ => return Err(UpgradeError::Storage),
    };
    Ok(sqlx::query_scalar(sql).fetch_all(c).await?)
}

async fn permissions(
    c: &mut PgConnection,
    rels: &[i64],
    types: &[i64],
) -> Result<Vec<String>, UpgradeError> {
    Ok(sqlx::query_scalar(r#"
WITH ns AS (SELECT * FROM pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema') AND left(nspname,3)<>'pg_'), facts AS (
 SELECT json_build_array('schema',nspname,nspowner,nspacl)::text AS fact FROM ns
 UNION ALL SELECT json_build_array('relation',n.nspname,c.relname,c.relowner,c.relacl)::text
 FROM pg_class c JOIN ns n ON n.oid=c.relnamespace WHERE c.oid::bigint=ANY($1)
 UNION ALL SELECT json_build_array('column',n.nspname,c.relname,a.attnum,a.attacl)::text
 FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN ns n ON n.oid=c.relnamespace
 WHERE c.oid::bigint=ANY($1) AND a.attnum>0 AND NOT a.attisdropped
 UNION ALL SELECT json_build_array('function',n.nspname,p.oid::regprocedure::text,p.proowner,p.proacl)::text
 FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace
 UNION ALL SELECT json_build_array('type',n.nspname,t.typname,t.typowner,t.typacl)::text
 FROM pg_type t JOIN ns n ON n.oid=t.typnamespace WHERE t.oid::bigint=ANY($2)
 UNION ALL SELECT json_build_array('default',defaclrole,defaclnamespace,defaclobjtype,defaclacl)::text FROM pg_default_acl
 UNION ALL SELECT json_build_array('database',datname,datdba,datacl,datallowconn,datconnlimit)::text FROM pg_database WHERE datname=current_database()
 UNION ALL SELECT json_build_array('role',oid,rolname,rolsuper,rolinherit,rolcreaterole,rolcreatedb,rolcanlogin,rolreplication,rolbypassrls,rolconnlimit,rolvaliduntil)::text FROM pg_roles
 UNION ALL SELECT json_build_array('membership',roleid,member,grantor,admin_option,inherit_option,set_option)::text FROM pg_auth_members
) SELECT fact FROM facts ORDER BY fact COLLATE "C"
"#).bind(rels).bind(types).fetch_all(c).await?)
}

async fn data(
    c: &mut PgConnection,
    tables: &[String],
) -> Result<BTreeMap<String, Vec<u8>>, UpgradeError> {
    let mut result = BTreeMap::new();
    for table in tables {
        if matches!(
            table.as_str(),
            "core_meta.component_contract" | "core_meta.runtime_contract"
        ) {
            continue;
        }
        sqlx::query(&format!("DECLARE site_finance_rows NO SCROLL CURSOR FOR SELECT json_build_array(encode(h,'hex'),count(*)::text)::text FROM (SELECT sha256(convert_to(to_jsonb(t)::text,'UTF8')) h FROM {table} t) q GROUP BY h ORDER BY h"))
            .execute(&mut *c).await?;
        let mut hash = Sha256::new();
        loop {
            let rows: Vec<String> = sqlx::query_scalar("FETCH FORWARD 256 FROM site_finance_rows")
                .fetch_all(&mut *c)
                .await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                hash.update((row.len() as u64).to_be_bytes());
                hash.update(row.as_bytes());
            }
        }
        sqlx::query("CLOSE site_finance_rows")
            .execute(&mut *c)
            .await?;
        result.insert(table.clone(), hash.finalize().to_vec());
    }
    let sequences: Vec<String> = sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname COLLATE \"C\",c.relname COLLATE \"C\"")
        .fetch_all(&mut *c).await?;
    if sequences.len() != 22 {
        return Err(reject("SITE_FINANCE_SEQUENCE_SET_CHANGED"));
    }
    for sequence in sequences {
        let state: String = sqlx::query_scalar(&format!("SELECT json_build_array(last_value::text,log_cnt::text,is_called)::text FROM {sequence}"))
            .fetch_one(&mut *c).await?;
        result.insert(
            format!("sequence:{sequence}"),
            Sha256::digest(state.as_bytes()).to_vec(),
        );
    }
    Ok(result)
}

fn data_hash(data: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut hash = Sha256::new();
    for (key, value) in data {
        hash.update((key.len() as u64).to_be_bytes());
        hash.update(key.as_bytes());
        hash.update(value);
    }
    hash.finalize().to_vec()
}

async fn privileges(c: &mut PgConnection, r: &UpgradeRequest) -> Result<(), UpgradeError> {
    let safe: bool = sqlx::query_scalar(r#"
SELECT NOT has_table_privilege($1,'core_meta.site_finance_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
 AND NOT has_any_column_privilege($1,'core_meta.site_finance_upgrades','SELECT,INSERT,UPDATE,REFERENCES')
 AND NOT EXISTS(SELECT 1 FROM (VALUES('finance_sources'),('finance_users'),('finance_topups'),('finance_native_payments')) t(name)
  WHERE NOT has_table_privilege($1,'core_history.'||name,'SELECT')
   OR has_table_privilege($1,'core_history.'||name,'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
   OR has_any_column_privilege($1,'core_history.'||name,'INSERT,UPDATE,REFERENCES'))
"#).bind(&r.roles.runtime).fetch_one(c).await?;
    if !safe {
        return Err(reject("SITE_FINANCE_RUNTIME_PRIVILEGES_INVALID"));
    }
    Ok(())
}

pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    let target = target()?;
    let (mut tx, backend_pid, tables) = begin_offline_upgrade(c, r).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_meta.site_finance_upgrades') IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    if exists {
        let row = sqlx::query("SELECT request_sha256,request,successor_catalog_sha256 FROM core_meta.site_finance_upgrades WHERE upgrade_id=$1")
            .bind(&r.upgrade_id).fetch_optional(&mut *tx).await?.ok_or_else(||reject("SITE_FINANCE_ORIGINAL_RECEIPT_ABSENT"))?;
        let request: sqlx::types::Json<serde_json::Value> = row.try_get("request")?;
        if row.try_get::<Vec<u8>, _>("request_sha256")? != r.digest()?
            || request.0 != serde_json::to_value(r).map_err(|_| UpgradeError::Storage)?
            || hex(&row.try_get::<Vec<u8>, _>("successor_catalog_sha256")?) != target.catalog_sha256
        {
            return Err(reject("SITE_FINANCE_RECEIPT_CONFLICT"));
        }
        if definitions(&mut tx).await? != target.definitions {
            return Err(reject("SITE_FINANCE_TARGET_CATALOG_CHANGED"));
        }
        super::super::check_connection(&mut tx).await?;
        privileges(&mut tx, r).await?;
        tx.rollback().await?;
        return report(r, &target.catalog_sha256, "already_committed");
    }
    if hex(&fingerprint(&definitions(&mut tx).await?)) != PRE_CATALOG || tables.len() != 171 {
        return Err(reject("SITE_FINANCE_PREDECESSOR_CATALOG_MISMATCH"));
    }
    let recorded: Vec<u8> = sqlx::query_scalar(
        "SELECT catalog_fingerprint FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?;
    if hex(&recorded) != PRE_CATALOG {
        return Err(reject("SITE_FINANCE_PREDECESSOR_RECEIPT_MISMATCH"));
    }
    let expected: BTreeMap<String, Vec<u8>> = COMPONENTS[..27]
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
        return Err(reject("SITE_FINANCE_PREDECESSOR_COMPONENTS_MISMATCH"));
    }
    let relations = object_ids(&mut tx, "relation").await?;
    let types = object_ids(&mut tx, "type").await?;
    let old_permissions = permissions(&mut tx, &relations, &types).await?;
    let old_data = data(&mut tx, &tables).await?;
    sqlx::raw_sql(COMPONENTS[27].1).execute(&mut *tx).await?;
    sqlx::query(&format!("GRANT SELECT ON core_history.finance_sources,core_history.finance_users,core_history.finance_topups,core_history.finance_native_payments TO {}",quote(&r.roles.runtime))).execute(&mut *tx).await?;
    privileges(&mut tx, r).await?;
    if definitions(&mut tx).await? != target.definitions
        || permissions(&mut tx, &relations, &types).await? != old_permissions
        || data(&mut tx, &tables).await? != old_data
    {
        return Err(reject("SITE_FINANCE_PRESERVATION_OR_TARGET_MISMATCH"));
    }
    sqlx::query(
        "INSERT INTO core_meta.component_contract(name,fingerprint) VALUES('site-finance',$1)",
    )
    .bind(Sha256::digest(COMPONENTS[27].1.as_bytes()).to_vec())
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton")
        .bind(fingerprint(&target.definitions))
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO core_meta.site_finance_upgrades(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(&r.upgrade_id).bind(r.digest()?).bind(sqlx::types::Json(serde_json::to_value(r).map_err(|_|UpgradeError::Storage)?))
        .bind(data_hash(&old_data)).bind(Sha256::digest(serde_json::to_vec(&old_permissions).map_err(|_|UpgradeError::Storage)?).to_vec())
        .bind(fingerprint(&target.definitions)).execute(&mut *tx).await?;
    super::super::check_connection(&mut tx).await?;
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
        components: 28,
    })
}

//! Shared exact-predecessor suffix engine for frozen 32/41/42 -> 43 upgrades.
//! Original rows, sequence states, ownership and ACLs must survive unchanged.
use super::{
    finance::{data_hash, object_ids, permissions},
    request_operations::preservation::data,
    *,
};
use std::collections::BTreeSet;

pub(super) struct Suffix {
    pub procedure: &'static str,
    pub predecessor_count: usize,
    pub predecessor_manifest: &'static str,
    pub predecessor_catalog: &'static str,
    pub target_json: &'static str,
    pub receipt: &'static str,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    manifest_sha256: String,
    catalog_sha256: String,
}
// Keep the existing procedures' public rejection codes stable while sharing logic.
macro_rules! reject_suffix {
    ($spec:expr, $code:literal) => {
        reject(match $spec.procedure {
            "core-functional-completion-v1" => concat!("COMPLETION_", $code),
            "core-account-appeals-v1" => concat!("ACCOUNT_APPEALS_", $code),
            _ => concat!("CHANNEL_ACCOUNTING_", $code),
        })
    };
}

impl Suffix {
    fn target(&self) -> Result<Target, UpgradeError> {
        let target: Target =
            serde_json::from_str(self.target_json).map_err(|_| UpgradeError::Storage)?;
        if target.manifest_sha256 != manifest_sha256()? || !digest_hex(&target.catalog_sha256) {
            return Err(reject_suffix!(self, "COMPILED_TARGET_CHANGED"));
        }
        Ok(target)
    }
    pub(super) fn validate(&self, r: &UpgradeRequest) -> Result<(), UpgradeError> {
        let roles = [
            &r.roles.installer,
            &r.roles.owner,
            &r.roles.runtime,
            &r.roles.extensions,
        ];
        if r.version != 1
            || r.procedure != self.procedure
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
                .filter(|n| *n > 0)
                .is_none()
            || roles.iter().any(|role| !identifier(role))
            || roles
                .iter()
                .enumerate()
                .any(|(i, role)| roles[..i].contains(role))
            || r.predecessor_manifest_sha256 != self.predecessor_manifest
            || r.predecessor_catalog_sha256 != self.predecessor_catalog
            || r.successor_manifest_sha256 != manifest_sha256()?
        {
            return Err(reject_suffix!(self, "UPGRADE_CONTRACT_INVALID"));
        }
        Ok(())
    }
    pub(super) fn authorize_target(&self) -> Result<(), UpgradeError> {
        if COMPONENTS.len() != 43
            || COMPONENTS[40].0 != "completion-upgrade"
            || COMPONENTS[41].0 != "account-appeals"
            || COMPONENTS[42].0 != "channel-accounting"
        {
            return Err(reject_suffix!(self, "COMPILED_COMPONENTS_CHANGED"));
        }
        let mut prior = schema_manifest();
        prior.components.truncate(self.predecessor_count);
        let mut bytes = serde_json::to_vec(&prior).map_err(|_| UpgradeError::Storage)?;
        bytes.push(b'\n');
        if hex(&Sha256::digest(bytes)) != self.predecessor_manifest {
            return Err(reject_suffix!(self, "COMPILED_PREDECESSOR_CHANGED"));
        }
        self.target()?;
        Ok(())
    }

    async fn receipt_privileges(&self, c: &mut PgConnection) -> Result<(), UpgradeError> {
        let safe:bool=sqlx::query_scalar(&format!(r#"
SELECT NOT EXISTS(SELECT FROM pg_class c CROSS JOIN LATERAL aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a
 WHERE c.oid='{receipt}'::regclass AND a.grantee<>c.relowner)
 AND NOT EXISTS(SELECT FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid CROSS JOIN LATERAL aclexplode(a.attacl) acl
 WHERE c.oid='{receipt}'::regclass AND acl.grantee<>c.relowner)
"#, receipt=self.receipt)).fetch_one(c).await?;
        if !safe {
            return Err(reject_suffix!(self, "RECEIPT_PRIVILEGES_INVALID"));
        }
        Ok(())
    }
    pub(super) async fn connected(
        &self,
        c: &mut PgConnection,
        r: &UpgradeRequest,
    ) -> Result<UpgradeReport, UpgradeError> {
        let target = self.target()?;
        let (mut tx, backend_pid, tables) = begin_offline_upgrade(c, r).await?;
        let exists: bool = sqlx::query_scalar(&format!(
            "SELECT to_regclass('{}') IS NOT NULL",
            self.receipt
        ))
        .fetch_one(&mut *tx)
        .await?;
        if exists {
            let row=sqlx::query(&format!("SELECT request_sha256,request,successor_catalog_sha256 FROM {receipt} WHERE upgrade_id=$1", receipt=self.receipt))
            .bind(&r.upgrade_id).fetch_optional(&mut *tx).await?.ok_or_else(||reject_suffix!(self, "ORIGINAL_RECEIPT_ABSENT"))?;
            let request: sqlx::types::Json<serde_json::Value> = row.try_get("request")?;
            if row.try_get::<Vec<u8>, _>("request_sha256")? != r.digest()?
                || request.0 != serde_json::to_value(r).map_err(|_| UpgradeError::Storage)?
                || hex(&row.try_get::<Vec<u8>, _>("successor_catalog_sha256")?)
                    != target.catalog_sha256
                || hex(&fingerprint(&definitions(&mut tx).await?)) != target.catalog_sha256
            {
                return Err(reject_suffix!(self, "RECEIPT_OR_CATALOG_CONFLICT"));
            }
            super::super::check_connection(&mut tx).await?;
            self.receipt_privileges(&mut tx).await?;
            tx.rollback().await?;
            return self.report(r, "already_committed", target.catalog_sha256);
        }
        if hex(&fingerprint(&definitions(&mut tx).await?)) != self.predecessor_catalog {
            return Err(reject_suffix!(self, "PREDECESSOR_CATALOG_MISMATCH"));
        }
        let recorded: Vec<u8> = sqlx::query_scalar(
            "SELECT catalog_fingerprint FROM core_meta.runtime_contract WHERE singleton",
        )
        .fetch_one(&mut *tx)
        .await?;
        if hex(&recorded) != self.predecessor_catalog {
            return Err(reject_suffix!(self, "PREDECESSOR_RECEIPT_MISMATCH"));
        }
        let expected: BTreeMap<String, Vec<u8>> = COMPONENTS[..self.predecessor_count]
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
            return Err(reject_suffix!(self, "PREDECESSOR_COMPONENTS_MISMATCH"));
        }
        let rels = object_ids(&mut tx, "relation").await?;
        let types = object_ids(&mut tx, "type").await?;
        let old_permissions = permissions(&mut tx, &rels, &types).await?;
        let mut keys = BTreeSet::new();
        for fact in &old_permissions {
            if let Some(key) = permission_key(fact)? {
                keys.insert(key);
            }
        }
        let sequences:Vec<String>=sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname COLLATE \"C\",c.relname COLLATE \"C\"").fetch_all(&mut *tx).await?;
        let old_data = data(&mut tx, &tables, &sequences).await?;
        for (name, ddl) in &COMPONENTS[self.predecessor_count..] {
            sqlx::raw_sql(ddl).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES($1,$2)")
                .bind(name)
                .bind(Sha256::digest(ddl.as_bytes()).to_vec())
                .execute(&mut *tx)
                .await?;
        }
        let catalog = fingerprint(&definitions(&mut tx).await?);
        if hex(&catalog) != target.catalog_sha256
            || preserved_permissions(&mut tx, &rels, &types, &keys).await? != old_permissions
            || data(&mut tx, &tables, &sequences).await? != old_data
        {
            return Err(reject_suffix!(self, "PRESERVATION_OR_TARGET_MISMATCH"));
        }
        self.receipt_privileges(&mut tx).await?;
        let changed=sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton AND catalog_fingerprint=$2").bind(&catalog).bind(&recorded).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(reject_suffix!(self, "RUNTIME_RECEIPT_CONFLICT"));
        }
        sqlx::query(&format!("INSERT INTO {receipt}(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256) VALUES($1,$2,$3,$4,$5,$6)", receipt=self.receipt))
        .bind(&r.upgrade_id).bind(r.digest()?).bind(sqlx::types::Json(serde_json::to_value(r).map_err(|_|UpgradeError::Storage)?))
        .bind(data_hash(&old_data)).bind(Sha256::digest(serde_json::to_vec(&old_permissions).map_err(|_|UpgradeError::Storage)?).to_vec()).bind(catalog).execute(&mut *tx).await?;
        super::super::check_connection(&mut tx).await?;
        preflight(&mut tx, r).await?;
        commit_offline_upgrade(tx, r, backend_pid).await?;
        self.report(r, "committed", target.catalog_sha256)
    }
    fn report(
        &self,
        r: &UpgradeRequest,
        status: &'static str,
        catalog: String,
    ) -> Result<UpgradeReport, UpgradeError> {
        Ok(UpgradeReport {
            version: 1,
            protocol: "core.upgrade-db.v1",
            status,
            upgrade_id: r.upgrade_id.clone(),
            request_sha256: hex(&r.digest()?),
            predecessor_catalog_sha256: self.predecessor_catalog,
            successor_catalog_sha256: catalog,
            components: COMPONENTS.len(),
        })
    }
}

// The shared snapshot already pins old relation/type OIDs. Restrict its schema
// and function rows to the exact old names, so new objects do not masquerade as
// changed permissions on old ones. Replaced routing functions retain their ACLs.
fn permission_key(fact: &str) -> Result<Option<String>, UpgradeError> {
    let v: serde_json::Value = serde_json::from_str(fact).map_err(|_| UpgradeError::Storage)?;
    match v[0].as_str() {
        Some("schema") => Ok(Some(format!("schema:{}", v[1]))),
        Some("function") => Ok(Some(format!("function:{}:{}", v[1], v[2]))),
        _ => Ok(None),
    }
}
async fn preserved_permissions(
    c: &mut PgConnection,
    rels: &[i64],
    types: &[i64],
    keys: &BTreeSet<String>,
) -> Result<Vec<String>, UpgradeError> {
    let mut result = Vec::new();
    for fact in permissions(c, rels, types).await? {
        if permission_key(&fact)?.is_none_or(|key| keys.contains(&key)) {
            result.push(fact);
        }
    }
    Ok(result)
}

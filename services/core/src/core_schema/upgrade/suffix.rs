//! Shared exact-predecessor engine with an individually frozen successor per procedure.
//! Original rows, sequence states, ownership and ACLs must survive unchanged.
use super::{
    finance::{data_hash, object_ids, permissions},
    request_operations::preservation::data_projected,
    *,
};
use std::collections::BTreeSet;

pub(super) struct Suffix {
    pub hooks: &'static registry::Registration,
    pub procedure: &'static str,
    pub predecessor_count: usize,
    pub target_count: usize,
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
impl Suffix {
    fn error(&self, error: registry::SuffixError) -> UpgradeError {
        reject((self.hooks.suffix().error)(error))
    }

    pub(super) fn target_manifest(&self) -> Result<String, UpgradeError> {
        if self.target_count > COMPONENTS.len() || self.target_count < 44 {
            return Err(self.error(registry::SuffixError::CompiledComponentsChanged));
        }
        let mut manifest = schema_manifest();
        manifest.components.truncate(self.target_count);
        let mut bytes = serde_json::to_vec(&manifest).map_err(|_| UpgradeError::Storage)?;
        bytes.push(b'\n');
        Ok(hex(Sha256::digest(bytes)))
    }

    fn target(&self) -> Result<Target, UpgradeError> {
        let target: Target = match self.hooks.suffix().policy.target {
            registry::TargetSource::Captured(parse) => {
                let (manifest_sha256, catalog_sha256) = parse()?;
                Target {
                    manifest_sha256,
                    catalog_sha256,
                }
            }
            registry::TargetSource::FrozenJson => {
                serde_json::from_str(self.target_json).map_err(|_| UpgradeError::Storage)?
            }
        };
        if target.manifest_sha256 != self.target_manifest()? || !digest_hex(&target.catalog_sha256)
        {
            return Err(self.error(registry::SuffixError::CompiledTargetChanged));
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
            || r.successor_manifest_sha256 != self.target_manifest()?
        {
            return Err(self.error(registry::SuffixError::UpgradeContractInvalid));
        }
        Ok(())
    }
    pub(super) fn authorize_target(&self) -> Result<(), UpgradeError> {
        if COMPONENTS.len() < self.target_count
            || self.target_count < 44
            || COMPONENTS[40].0 != "completion-upgrade"
            || COMPONENTS[41].0 != "account-appeals"
            || COMPONENTS[42].0 != "channel-accounting"
            || COMPONENTS[43].0 != "channel-projection"
        {
            return Err(self.error(registry::SuffixError::CompiledComponentsChanged));
        }
        let mut prior = schema_manifest();
        prior.components.truncate(self.predecessor_count);
        let mut bytes = serde_json::to_vec(&prior).map_err(|_| UpgradeError::Storage)?;
        bytes.push(b'\n');
        if hex(Sha256::digest(bytes)) != self.predecessor_manifest {
            return Err(self.error(registry::SuffixError::CompiledPredecessorChanged));
        }
        self.target()?;
        Ok(())
    }

    async fn check_target(&self, c: &mut PgConnection) -> Result<(), UpgradeError> {
        IdentityStore::check_schema_connection(c).await?;
        super::super::check_component_set(c, &COMPONENTS[..self.target_count], false).await?;
        super::super::check_usage_index(c).await?;
        let saved: Vec<u8> = sqlx::query_scalar(
            "SELECT catalog_fingerprint FROM core_meta.runtime_contract WHERE singleton",
        )
        .fetch_one(&mut *c)
        .await?;
        if saved != super::super::catalog(c).await? {
            return Err(self.error(registry::SuffixError::RuntimeReceiptConflict));
        }
        Ok(())
    }

    pub(super) async fn receipt_owner_only(
        &self,
        c: &mut PgConnection,
        _r: &UpgradeRequest,
    ) -> Result<(), UpgradeError> {
        let safe:bool=sqlx::query_scalar(sqlx::AssertSqlSafe(format!(r#"
SELECT NOT EXISTS(SELECT FROM pg_class c CROSS JOIN LATERAL aclexplode(coalesce(c.relacl,acldefault('r',c.relowner))) a
 WHERE c.oid='{receipt}'::regclass AND a.grantee<>c.relowner)
 AND NOT EXISTS(SELECT FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid CROSS JOIN LATERAL aclexplode(a.attacl) acl
 WHERE c.oid='{receipt}'::regclass AND acl.grantee<>c.relowner)
"#, receipt=self.receipt))).fetch_one(c).await?;
        if !safe {
            return Err(self.error(registry::SuffixError::ReceiptPrivilegesInvalid));
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
        let exists: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT to_regclass('{}') IS NOT NULL",
            self.receipt
        )))
        .fetch_one(&mut *tx)
        .await?;
        if exists {
            let row=sqlx::query(sqlx::AssertSqlSafe(format!("SELECT request_sha256,request,successor_catalog_sha256 FROM {receipt} WHERE upgrade_id=$1", receipt=self.receipt)))
            .bind(&r.upgrade_id).fetch_optional(&mut *tx).await?.ok_or_else(||self.error(registry::SuffixError::OriginalReceiptAbsent))?;
            let request: sqlx::types::Json<serde_json::Value> = row.try_get("request")?;
            if row.try_get::<Vec<u8>, _>("request_sha256")? != r.digest()?
                || request.0 != serde_json::to_value(r).map_err(|_| UpgradeError::Storage)?
                || hex(&row.try_get::<Vec<u8>, _>("successor_catalog_sha256")?)
                    != target.catalog_sha256
                || hex(fingerprint(&definitions(&mut tx).await?)) != target.catalog_sha256
            {
                return Err(self.error(registry::SuffixError::ReceiptOrCatalogConflict));
            }
            self.check_target(&mut tx).await?;
            if self.predecessor_count < 44 {
                super::channel_projection::check_ready(&mut tx).await?;
            }
            (self.hooks.hooks.receipt_privileges)(self, &mut tx, r).await?;
            tx.rollback().await?;
            return self.report(r, "already_committed", target.catalog_sha256);
        }
        if hex(fingerprint(&definitions(&mut tx).await?)) != self.predecessor_catalog {
            return Err(self.error(registry::SuffixError::PredecessorCatalogMismatch));
        }
        let recorded: Vec<u8> = sqlx::query_scalar(
            "SELECT catalog_fingerprint FROM core_meta.runtime_contract WHERE singleton",
        )
        .fetch_one(&mut *tx)
        .await?;
        if hex(&recorded) != self.predecessor_catalog {
            return Err(self.error(registry::SuffixError::PredecessorReceiptMismatch));
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
            return Err(self.error(registry::SuffixError::PredecessorComponentsMismatch));
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
        let subscription =
            self.hooks.suffix().policy.projection == registry::DataProjection::SubscriptionEpay;
        let old_data = data_projected(&mut tx, &tables, &sequences, subscription).await?;
        for (name, ddl) in &COMPONENTS[self.predecessor_count..self.target_count] {
            sqlx::raw_sql(*ddl).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES($1,$2)")
                .bind(name)
                .bind(Sha256::digest(ddl.as_bytes()).to_vec())
                .execute(&mut *tx)
                .await?;
        }
        #[cfg(test)]
        if let Err(error) = (self.hooks.suffix().policy.after_ddl)() {
            tx.rollback().await?;
            return Err(error);
        }
        (self.hooks.hooks.grant_runtime)(&mut tx, r).await?;
        // One explicit owner-only backfill, inside the existing migration fence
        // and transaction. Failure rolls back the new DDL as well as projection.
        if self.predecessor_count < 44 {
            super::channel_projection::backfill(&mut tx).await?;
        }
        let catalog = fingerprint(&definitions(&mut tx).await?);
        if hex(&catalog) != target.catalog_sha256
            || preserved_permissions(&mut tx, &rels, &types, &keys, subscription).await?
                != old_permissions
            || data_projected(&mut tx, &tables, &sequences, subscription).await? != old_data
        {
            return Err(self.error(registry::SuffixError::PreservationOrTargetMismatch));
        }
        (self.hooks.hooks.receipt_privileges)(self, &mut tx, r).await?;
        let changed=sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton AND catalog_fingerprint=$2").bind(&catalog).bind(&recorded).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(self.error(registry::SuffixError::RuntimeReceiptConflict));
        }
        sqlx::query(sqlx::AssertSqlSafe(format!("INSERT INTO {receipt}(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256) VALUES($1,$2,$3,$4,$5,$6)", receipt=self.receipt)))
        .bind(&r.upgrade_id).bind(r.digest()?).bind(sqlx::types::Json(serde_json::to_value(r).map_err(|_|UpgradeError::Storage)?))
        .bind(data_hash(&old_data)).bind(Sha256::digest(serde_json::to_vec(&old_permissions).map_err(|_|UpgradeError::Storage)?).to_vec()).bind(catalog).execute(&mut *tx).await?;
        self.check_target(&mut tx).await?;
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
            components: self.target_count,
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
    subscription: bool,
) -> Result<Vec<String>, UpgradeError> {
    let added_column: Option<i16> = if subscription {
        Some(sqlx::query_scalar("SELECT attnum FROM pg_attribute WHERE attrelid='core_payments.provider_identifiers'::regclass AND attname='subscription_order_id' AND NOT attisdropped AND attacl IS NULL")
            .fetch_optional(&mut *c).await?.ok_or_else(||reject("SUBSCRIPTION_EPAY_COLUMN_ACL_CHANGED"))?)
    } else {
        None
    };
    let mut result = Vec::new();
    for fact in permissions(c, rels, types).await? {
        if let Some(number) = added_column {
            let v: serde_json::Value =
                serde_json::from_str(&fact).map_err(|_| UpgradeError::Storage)?;
            if v[0] == "column"
                && v[1] == "core_payments"
                && v[2] == "provider_identifiers"
                && v[3] == number
                && v[4].is_null()
            {
                continue;
            }
        }
        if permission_key(&fact)?.is_none_or(|key| keys.contains(&key)) {
            result.push(fact);
        }
    }
    Ok(result)
}

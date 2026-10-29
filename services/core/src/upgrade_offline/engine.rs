//! Native v3 schema engine over reviewed v5/v6 predecessors.
//!
//! Executed only by the v3 controller after the external start is consumed.
//! Historical v2 inputs and receipts remain unchanged.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};
use std::{collections::BTreeMap, fmt};

pub const PROCEDURE: &str = "core-offline-upgrade-v3";
const OLD: &str = include_str!("console_predecessor.json");
const AFTER_V1: &str = include_str!("console_bridge_v1_successor.json");
const TARGET: &str = include_str!("target.json");
const PREVIOUS_V6: &str = include_str!("previous_v6_manifest.json");
const RECEIPT: &str = include_str!("../../schema/offline_upgrade.sql");
// PostgreSQL 18.6, exact formal fresh SQL and schema-manifest 47ad3692...03078.
// Observed independently from the old native installer plus additive metadata; never
// substitute the observed post-upgrade hash for this reviewed target pin.
pub(super) const TARGET_CATALOG: &str =
    "557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d";
const IDENTITY_V5: &str = include_str!("historical/identity_v5.sql");
const IDENTITY_V6: &str = include_str!("../../schema/identity.sql");
const CONSOLE_OLD: &str = include_str!("historical/console.sql");
const CONSOLE: &str = include_str!("../../schema/console.sql");
const AGENT: &str = include_str!("../../schema/assistant_agent.sql");
const V1_RECEIPT: &str = include_str!("../core_schema/upgrade/console_receipt.sql");
const PRECONDITIONS: &str = include_str!("historical/v5_preconditions.sql");

#[derive(Debug)]
pub enum BridgeError {
    Rejected(&'static str),
    Database(sqlx::Error),
}
impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(code) => f.write_str(code),
            // SQLx errors may contain data; do not put them in a public report.
            Self::Database(_) => f.write_str("CONSOLE_BRIDGE_DATABASE_CHECK_FAILED"),
        }
    }
}
impl std::error::Error for BridgeError {}
impl From<sqlx::Error> for BridgeError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}
type Result<T> = std::result::Result<T, BridgeError>;
fn require(ok: bool, code: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(BridgeError::Rejected(code))
    }
}
fn sha(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
pub(super) fn framed(rows: &[String]) -> String {
    let mut hash = Sha256::new();
    for row in rows {
        hash.update((row.len() as u64).to_be_bytes());
        hash.update(row.as_bytes());
    }
    format!("{:x}", hash.finalize())
}
fn json(text: &str) -> Result<Value> {
    serde_json::from_str(text)
        .map_err(|_| BridgeError::Rejected("CONSOLE_BRIDGE_BAD_COMPILED_JSON"))
}
fn component_map(manifest: &Value) -> Result<BTreeMap<String, String>> {
    let rows = manifest["components"]
        .as_array()
        .ok_or(BridgeError::Rejected("CONSOLE_BRIDGE_BAD_COMPONENTS"))?;
    let mut result = BTreeMap::new();
    for row in rows {
        let name = row["name"]
            .as_str()
            .ok_or(BridgeError::Rejected("CONSOLE_BRIDGE_BAD_COMPONENTS"))?;
        let digest = row["raw_schema_sha256"]
            .as_str()
            .ok_or(BridgeError::Rejected("CONSOLE_BRIDGE_BAD_COMPONENTS"))?;
        require(
            result.insert(name.into(), digest.into()).is_none(),
            "CONSOLE_BRIDGE_DUPLICATE_COMPONENT",
        )?;
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Predecessor {
    FreshV5Core26,
    ConsoleV1V5Core26,
    FreshV6Core27,
}

/// Distinct procedure and predecessor enum: a v6/27 target cannot impersonate
/// the historical v1 installer. No caller SQL, relaxed hashes or skip flags.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeRequest {
    pub version: u32,
    pub procedure: String,
    pub predecessor: Predecessor,
    pub predecessor_manifest_sha256: String,
    pub predecessor_catalog_sha256: String,
    pub successor_manifest_sha256: String,
    pub database_name: String,
    pub database_oid: u32,
    pub system_identifier: String,
    pub installer: String,
    pub owner: String,
    pub runtime: String,
    pub extensions: String,
}

pub struct BridgePlan {
    pub(super) request: BridgeRequest,
    expected: BTreeMap<String, String>,
    target: BTreeMap<String, String>,
    catalog_query: String,
    tables: usize,
    identity_delta: &'static str,
    console_delta: &'static str,
}

impl BridgePlan {
    /// `compiled_manifest` must come from the real core schema_manifest().
    /// This also rejects unrelated component or installer metadata drift.
    pub fn new(request: BridgeRequest, compiled_manifest: &Value) -> Result<Self> {
        let old = json(OLD)?;
        let target = json(TARGET)?;
        require(
            compiled_manifest == &target && request.successor_manifest_sha256 == sha(TARGET),
            "CONSOLE_BRIDGE_COMPILED_TARGET_DRIFT",
        )?;
        require(
            request.version == 3 && request.procedure == PROCEDURE,
            "CONSOLE_BRIDGE_REQUEST_VERSION_INVALID",
        )?;
        let (manifest, manifest_hash, catalog_hash, tables) = match request.predecessor {
            Predecessor::FreshV6Core27 => (
                json(PREVIOUS_V6)?,
                Value::String(sha(PREVIOUS_V6)),
                Value::String(
                    "acbee20015ea1397adfda06232f25c811956823e00b15e574d74794702f418e5".into(),
                ),
                0,
            ),
            Predecessor::FreshV5Core26 => (
                old["manifest"].clone(),
                old["schema_manifest_stdout_sha256"].clone(),
                old["catalog_sha256"].clone(),
                160,
            ),
            Predecessor::ConsoleV1V5Core26 => (
                json(AFTER_V1)?,
                old["successor"]["schema_manifest_stdout_sha256"].clone(),
                old["successor"]["catalog_sha256"].clone(),
                161,
            ),
        };
        require(
            request.predecessor_manifest_sha256 == manifest_hash
                && request.predecessor_catalog_sha256 == catalog_hash,
            "CONSOLE_BRIDGE_PREDECESSOR_IDENTITY_MISMATCH",
        )?;
        let expected = component_map(&manifest)?;
        let target_components = component_map(&target)?;
        let v6 = matches!(request.predecessor, Predecessor::FreshV6Core27);
        require(
            expected.len() == if v6 { 27 } else { 26 }
                && target_components.len() == 27
                && manifest["identity"]["version"] == if v6 { 6 } else { 5 }
                && target["identity"]["version"] == 6
                && expected["identity"] == sha(if v6 { IDENTITY_V6 } else { IDENTITY_V5 })
                && target_components["identity"] == sha(IDENTITY_V6)
                && target_components["console"] == sha(CONSOLE)
                && target_components["assistant-agent"] == sha(AGENT)
                && manifest["catalog"] == target["catalog"],
            "CONSOLE_BRIDGE_SCHEMA_BYTES_DRIFT",
        )?;
        for (name, digest) in &expected {
            if name != "identity" && name != "console" {
                require(
                    target_components.get(name) == Some(digest),
                    "CONSOLE_BRIDGE_UNREVIEWED_COMPONENT_CHANGE",
                )?;
            }
        }
        let identity_delta = IDENTITY_V6
            .strip_prefix(IDENTITY_V5)
            .filter(|s| !s.is_empty())
            .ok_or(BridgeError::Rejected("CONSOLE_BRIDGE_NONADDITIVE_IDENTITY"))?;
        let (prefix, _) = CONSOLE_OLD
            .split_once("REVOKE ALL ON SCHEMA core_console")
            .ok_or(BridgeError::Rejected("CONSOLE_BRIDGE_BAD_CONSOLE_ANCHOR"))?;
        let console_delta = CONSOLE
            .strip_prefix(prefix)
            .and_then(|s| s.strip_suffix(&CONSOLE_OLD[prefix.len()..]))
            .filter(|s| !s.is_empty())
            .ok_or(BridgeError::Rejected("CONSOLE_BRIDGE_NONADDITIVE_CONSOLE"))?;
        require(
            expected["console"]
                == match request.predecessor {
                    Predecessor::FreshV5Core26 => sha(CONSOLE_OLD),
                    Predecessor::ConsoleV1V5Core26 | Predecessor::FreshV6Core27 => sha(CONSOLE),
                },
            "CONSOLE_BRIDGE_CONSOLE_PREDECESSOR_DRIFT",
        )?;
        let roles = [
            &request.installer,
            &request.owner,
            &request.runtime,
            &request.extensions,
        ];
        let identifier =
            |s: &str| !s.is_empty() && s.len() <= 63 && !s.chars().any(char::is_control);
        require(
            roles.iter().all(|s| identifier(s))
                && roles
                    .iter()
                    .enumerate()
                    .all(|(i, s)| !roles[..i].contains(s))
                && identifier(&request.database_name)
                && request.database_oid > 0
                && request
                    .system_identifier
                    .parse::<u64>()
                    .is_ok_and(|id| id > 0),
            "CONSOLE_BRIDGE_DATABASE_OR_ROLES_INVALID",
        )?;
        Ok(Self {
            request,
            expected,
            target: target_components,
            catalog_query: target["catalog"]["query_utf8"]
                .as_str()
                .ok_or(BridgeError::Rejected("CONSOLE_BRIDGE_BAD_CATALOG"))?
                .into(),
            tables,
            identity_delta,
            console_delta,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct BridgePreview {
    pub procedure: &'static str,
    pub status: &'static str,
    pub predecessor_catalog_sha256: String,
    pub successor_catalog_sha256: String,
    pub preserved_data_sha256: String,
    pub preserved_permissions_sha256: String,
    pub components: usize,
    pub identity_version: i32,
}

pub(super) fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub(super) async fn offline(c: &mut PgConnection, plan: &BridgePlan) -> Result<()> {
    let r = &plan.request;
    let valid: bool = sqlx::query_scalar(
        r#"
        SELECT current_database()=$1 AND d.oid=$2::bigint::oid
        AND current_setting('server_version_num')::int BETWEEN 180000 AND 189999
        AND session_user=$3 AND current_user=$4 AND NOT datallowconn
        AND NOT pg_is_in_recovery()
        AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid())
        AND NOT EXISTS(SELECT FROM pg_prepared_xacts WHERE database=current_database())
        AND NOT EXISTS(SELECT FROM pg_event_trigger WHERE evtenabled<>'D')
        FROM pg_database d WHERE datname=current_database()
    "#,
    )
    .bind(&r.database_name)
    .bind(i64::from(r.database_oid))
    .bind(&r.installer)
    .bind(&r.owner)
    .fetch_one(&mut *c)
    .await?;
    require(valid, "CONSOLE_BRIDGE_OFFLINE_DATABASE_REQUIRED")?;
    let system: String =
        sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
            .fetch_one(&mut *c)
            .await?;
    require(
        system == r.system_identifier,
        "CONSOLE_BRIDGE_DATABASE_SYSTEM_MISMATCH",
    )?;
    let safe: bool = sqlx::query_scalar(r#"
        WITH RECURSIVE authorities(oid) AS (
          SELECT oid FROM pg_roles WHERE rolname IN ($2,$3)
          UNION SELECT m.member FROM pg_auth_members m JOIN authorities a ON m.roleid=a.oid
        )
        SELECT count(*)=4 AND bool_and(NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
        AND bool_and(CASE WHEN rolname=$1 THEN rolcanlogin AND NOT rolinherit
                         WHEN rolname IN ($2,$3) THEN NOT rolcanlogin ELSE TRUE END)
        AND pg_has_role($1,$2,'SET') AND NOT pg_has_role($1,$2,'USAGE')
        AND NOT EXISTS(SELECT FROM pg_auth_members m JOIN pg_roles p ON p.oid=m.member WHERE p.rolname=$3)
        AND NOT pg_has_role($4,$2,'MEMBER')
        AND NOT EXISTS(SELECT FROM authorities a JOIN pg_roles p ON p.oid=a.oid WHERE p.rolcanlogin AND p.rolname<>$1)
        AND NOT EXISTS(SELECT FROM pg_roles p WHERE p.rolcanlogin AND NOT p.rolsuper AND p.rolname<>$1
                       AND has_database_privilege(p.oid,current_database(),'CONNECT'))
        AND NOT has_database_privilege($4,current_database(),'CONNECT')
        AND NOT has_function_privilege($3,'pg_control_system()','EXECUTE')
        AND has_schema_privilege($3,'core_console','USAGE')
        AND has_table_privilege($3,'core_console.options','SELECT')
        AND NOT has_table_privilege($3,'core_console.options','INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
        AND NOT has_any_column_privilege($3,'core_console.options','INSERT,UPDATE,REFERENCES')
        FROM pg_roles WHERE rolname IN ($1,$2,$3,$4)
    "#).bind(&r.installer).bind(&r.owner).bind(&r.runtime).bind(&r.extensions).fetch_one(&mut *c).await?;
    require(safe, "CONSOLE_BRIDGE_OFFLINE_PERMISSIONS_REQUIRED")?;
    let owners: bool = sqlx::query_scalar(r#"
        WITH ns AS (SELECT * FROM pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema','public') AND left(nspname,3)<>'pg_')
        SELECT NOT EXISTS(SELECT FROM ns WHERE pg_get_userbyid(nspowner)<>current_user)
        AND NOT EXISTS(SELECT FROM pg_class c JOIN ns n ON n.oid=c.relnamespace WHERE pg_get_userbyid(c.relowner)<>current_user)
        AND NOT EXISTS(SELECT FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace WHERE pg_get_userbyid(p.proowner)<>current_user)
        AND NOT EXISTS(SELECT FROM pg_type t JOIN ns n ON n.oid=t.typnamespace WHERE pg_get_userbyid(t.typowner)<>current_user)
    "#).fetch_one(c).await?;
    require(owners, "CONSOLE_BRIDGE_FOREIGN_OWNER")
}

// Each fact includes object identity. New objects can add facts but cannot
// replace/remove old owners, ACLs, default grants or role memberships.
pub(super) async fn permissions(c: &mut PgConnection) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(r#"
      WITH ns AS (SELECT * FROM pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema') AND left(nspname,3)<>'pg_'), facts AS (
      SELECT json_build_array('schema',oid,nspname,nspowner,nspacl)::text AS fact FROM ns
      UNION ALL SELECT json_build_array('relation',c.oid,c.relowner,c.relacl)::text FROM pg_class c JOIN ns n ON n.oid=c.relnamespace
      UNION ALL SELECT json_build_array('column',c.oid,a.attnum,a.attacl)::text FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN ns n ON n.oid=c.relnamespace WHERE a.attnum>0 AND NOT a.attisdropped
      UNION ALL SELECT json_build_array('function',p.oid,p.proowner,p.proacl)::text FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace
      UNION ALL SELECT json_build_array('type',t.oid,t.typowner,t.typacl)::text FROM pg_type t JOIN ns n ON n.oid=t.typnamespace
      UNION ALL SELECT json_build_array('default',defaclrole,defaclnamespace,defaclobjtype,defaclacl)::text FROM pg_default_acl
      UNION ALL SELECT json_build_array('database',oid,datdba,datacl,datallowconn,datconnlimit)::text FROM pg_database WHERE datname=current_database()
      UNION ALL SELECT json_build_array('role',oid,rolname,rolsuper,rolinherit,rolcreaterole,rolcreatedb,rolcanlogin,rolreplication,rolbypassrls,rolconnlimit,rolvaliduntil)::text FROM pg_roles
      UNION ALL SELECT json_build_array('member',roleid,member,grantor,admin_option,inherit_option,set_option)::text FROM pg_auth_members
      ) SELECT fact FROM facts ORDER BY fact COLLATE "C"
    "#).fetch_all(c).await?)
}

pub(super) struct OldRows {
    pub(super) columns: BTreeMap<String, String>,
    sequences: Vec<String>,
}
impl OldRows {
    pub(super) async fn capture(c: &mut PgConnection) -> Result<Self> {
        let rows = sqlx::query(r#"
          SELECT format('%I.%I',n.nspname,c.relname) AS name,
          string_agg(format('%I',a.attname),',' ORDER BY a.attnum) AS columns
          FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
          JOIN pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped
          WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='r'
          GROUP BY n.nspname,c.relname ORDER BY n.nspname COLLATE "C",c.relname COLLATE "C"
        "#).fetch_all(&mut *c).await?;
        let columns = rows
            .into_iter()
            .map(|r| Ok((r.try_get("name")?, r.try_get("columns")?)))
            .collect::<std::result::Result<_, sqlx::Error>>()?;
        let sequences = sqlx::query_scalar(r#"SELECT format('%I.%I',n.nspname,c.relname)
          FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
          WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S'
          ORDER BY n.nspname COLLATE "C",c.relname COLLATE "C""#).fetch_all(c).await?;
        Ok(Self { columns, sequences })
    }

    async fn digest(&self, c: &mut PgConnection) -> Result<String> {
        let mut facts = Vec::new();
        for (table, columns) in &self.columns {
            // Compare contract rows separately with exact CAS below. All other
            // old columns/rows, including historical receipts, remain immutable.
            if matches!(
                table.as_str(),
                "core_meta.component_contract"
                    | "core_meta.runtime_contract"
                    | "core_meta.schema_contract"
            ) {
                continue;
            }
            sqlx::query(&format!("DECLARE console_bridge_rows NO SCROLL CURSOR FOR SELECT json_build_array(encode(d,'hex'),count(*)::text)::text FROM (SELECT sha256(convert_to(to_jsonb(t)::text,'UTF8')) d FROM (SELECT {columns} FROM {table}) t) s GROUP BY d ORDER BY d"))
                .execute(&mut *c).await?;
            let mut hash = Sha256::new();
            loop {
                let rows: Vec<String> =
                    sqlx::query_scalar("FETCH FORWARD 256 FROM console_bridge_rows")
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
            sqlx::query("CLOSE console_bridge_rows")
                .execute(&mut *c)
                .await?;
            facts.push(format!("{table}:{hash:x}", hash = hash.finalize()));
        }
        for seq in &self.sequences {
            let state: String = sqlx::query_scalar(&format!(
                "SELECT json_build_array(last_value,log_cnt,is_called)::text FROM {seq}"
            ))
            .fetch_one(&mut *c)
            .await?;
            facts.push(format!("{seq}:{state}"));
            let definition: String = sqlx::query_scalar("SELECT json_build_array(seqtypid,seqstart,seqincrement,seqmax,seqmin,seqcache,seqcycle)::text FROM pg_sequence WHERE seqrelid=$1::regclass")
                .bind(seq).fetch_one(&mut *c).await?;
            facts.push(format!("{seq}:definition:{definition}"));
        }
        Ok(framed(&facts))
    }
}

async fn components(c: &mut PgConnection) -> Result<BTreeMap<String, String>> {
    Ok(sqlx::query(
        "SELECT name,encode(fingerprint,'hex') AS hash FROM core_meta.component_contract",
    )
    .fetch_all(c)
    .await?
    .into_iter()
    .map(|r| Ok((r.try_get("name")?, r.try_get("hash")?)))
    .collect::<std::result::Result<_, sqlx::Error>>()?)
}

// Private: no caller can commit this stage through the public module surface.
pub(super) async fn stage(
    c: &mut PgConnection,
    plan: &BridgePlan,
    rows: &OldRows,
) -> Result<BridgePreview> {
    let before: Vec<String> = sqlx::query_scalar(&plan.catalog_query)
        .fetch_all(&mut *c)
        .await?;
    require(
        framed(&before) == plan.request.predecessor_catalog_sha256
            && (plan.tables == 0 || rows.columns.len() == plan.tables)
            && rows.sequences.len()
                == if matches!(plan.request.predecessor, Predecessor::FreshV6Core27) {
                    22
                } else {
                    21
                },
        "CONSOLE_BRIDGE_PREDECESSOR_CATALOG_MISMATCH",
    )?;
    let saved: String = sqlx::query_scalar(
        "SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&mut *c)
    .await?;
    require(
        saved == plan.request.predecessor_catalog_sha256 && components(c).await? == plan.expected,
        "CONSOLE_BRIDGE_PREDECESSOR_RECEIPTS_MISMATCH",
    )?;
    let native = sqlx::query("SELECT version,encode(fingerprint,'hex') AS hash,installed_at::text AS installed FROM core_meta.schema_contract WHERE singleton")
        .fetch_one(&mut *c).await?;
    require(
        native.try_get::<i32, _>("version")?
            == if matches!(plan.request.predecessor, Predecessor::FreshV6Core27) {
                6
            } else {
                5
            }
            && native.try_get::<String, _>("hash")?
                == sha(
                    if matches!(plan.request.predecessor, Predecessor::FreshV6Core27) {
                        IDENTITY_V6
                    } else {
                        IDENTITY_V5
                    },
                ),
        "CONSOLE_BRIDGE_IDENTITY_V5_REQUIRED",
    )?;
    let v6 = matches!(plan.request.predecessor, Predecessor::FreshV6Core27);
    if !v6 {
        sqlx::raw_sql(PRECONDITIONS).execute(&mut *c).await?;
    }
    let data = rows.digest(c).await?;
    let acl = permissions(c).await?;
    if !v6 {
        sqlx::raw_sql(plan.identity_delta).execute(&mut *c).await?;
    }
    let no_promoted_email: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT FROM core_identity.email_addresses)
         AND NOT EXISTS(SELECT FROM core_identity.user_profiles WHERE email_version<>0)",
    )
    .fetch_one(&mut *c)
    .await?;
    require(
        v6 || no_promoted_email,
        "CONSOLE_BRIDGE_EMAIL_AUTHORITY_PROMOTED",
    )?;
    if !v6 {
        sqlx::raw_sql(AGENT).execute(&mut *c).await?;
    }
    if matches!(plan.request.predecessor, Predecessor::FreshV5Core26) {
        sqlx::raw_sql(plan.console_delta).execute(&mut *c).await?;
        // Preserve the historical v1 table definition, but do NOT create a v1
        // success row for this different procedure.
        sqlx::raw_sql(V1_RECEIPT).execute(&mut *c).await?;
    }
    sqlx::raw_sql(RECEIPT).execute(&mut *c).await?;
    let after: Vec<String> = sqlx::query_scalar(&plan.catalog_query)
        .fetch_all(&mut *c)
        .await?;
    let after_acl = permissions(c).await?;
    require(
        before.iter().all(|x| after.contains(x))
            && acl.iter().all(|x| after_acl.contains(x))
            && rows.digest(c).await? == data,
        "CONSOLE_BRIDGE_OLD_DATA_DEFINITIONS_OR_ACL_CHANGED",
    )?;
    if !v6 {
        for name in ["identity", "console"] {
            let n = sqlx::query("UPDATE core_meta.component_contract SET fingerprint=decode($1,'hex') WHERE name=$2 AND fingerprint=decode($3,'hex')")
            .bind(&plan.target[name]).bind(name).bind(&plan.expected[name]).execute(&mut *c).await?.rows_affected();
            require(n == 1, "CONSOLE_BRIDGE_COMPONENT_CAS_FAILED")?;
        }
        sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES('assistant-agent',decode($1,'hex'))")
        .bind(&plan.target["assistant-agent"]).execute(&mut *c).await?;
        let n = sqlx::query("UPDATE core_meta.schema_contract SET version=6,fingerprint=decode($1,'hex') WHERE singleton AND version=5 AND fingerprint=decode($2,'hex')")
        .bind(sha(IDENTITY_V6)).bind(sha(IDENTITY_V5)).execute(&mut *c).await?.rows_affected();
        require(n == 1, "CONSOLE_BRIDGE_IDENTITY_CAS_FAILED")?;
    }
    let successor = framed(&after);
    require(
        successor == TARGET_CATALOG,
        "CONSOLE_BRIDGE_SUCCESSOR_CATALOG_MISMATCH",
    )?;
    let n = sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=decode($1,'hex') WHERE singleton AND catalog_fingerprint=decode($2,'hex')")
        .bind(&successor).bind(&saved).execute(&mut *c).await?.rows_affected();
    require(
        n == 1 && components(c).await? == plan.target,
        "CONSOLE_BRIDGE_TARGET_RECEIPTS_MISMATCH",
    )?;
    let installed: String = sqlx::query_scalar(
        "SELECT installed_at::text FROM core_meta.schema_contract WHERE singleton",
    )
    .fetch_one(&mut *c)
    .await?;
    require(
        installed == native.try_get::<String, _>("installed")?,
        "CONSOLE_BRIDGE_INSTALL_TIMESTAMP_CHANGED",
    )?;
    Ok(BridgePreview {
        procedure: PROCEDURE,
        status: "staged",
        predecessor_catalog_sha256: saved,
        successor_catalog_sha256: successor,
        preserved_data_sha256: data,
        preserved_permissions_sha256: framed(&acl),
        components: 27,
        identity_version: 6,
    })
}

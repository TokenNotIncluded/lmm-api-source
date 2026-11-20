//! Retained Core CLIs for frozen upgrade contracts.
//! The current fresh-install binary deliberately refuses these historical families.
use lmm_core::core_schema::{UpgradeError, UpgradeReport, UpgradeRequest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{ConnectOptions, Connection, PgConnection, postgres::PgConnectOptions};
use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::OnceLock,
};

#[derive(Clone, Copy)]
struct Family {
    revision: &'static str,
    binary_variable: &'static str,
    digest_variable: &'static str,
    component_count: usize,
    last_component: &'static str,
    target: &'static str,
}
fn family() -> Family {
    match super::PROCEDURE {
        "core-subscription-pancake-language-optional-v1" => Family {
            revision: "0510c2455dabcb63e33c719ba52ac189d62612bc",
            binary_variable: "LMM_PRESERVED_PANCAKE67_CORE_BIN",
            digest_variable: "LMM_PRESERVED_PANCAKE67_CORE_SHA256",
            component_count: 67,
            last_component: "subscription-pancake-language-optional",
            target: include_str!(
                "../../src/core_schema/upgrade/subscription_pancake_language_target.json"
            ),
        },
        "core-functional-completion-v1"
        | "core-account-appeals-v1"
        | "core-channel-accounting-v1"
        | "core-channel-projection-v1"
        | "core-trust-policy-edit-v1"
        | "core-channel-observations-v1"
        | "core-navigation-options-v1"
        | "core-commerce-display-rate-v1" => Family {
            revision: "2a0c7bd380f58a553dc903d399255821c1a65d0f",
            binary_variable: "LMM_PRESERVED_EARLY58_CORE_BIN",
            digest_variable: "LMM_PRESERVED_EARLY58_CORE_SHA256",
            component_count: 58,
            last_component: "display-exchange-rate",
            target: include_str!("../../src/core_schema/upgrade/commerce_display_rate_target.json"),
        },
        _ => panic!("no retained executor for this historical procedure"),
    }
}
fn target_count() -> usize {
    match super::PROCEDURE {
        "core-subscription-pancake-language-optional-v1" => 67,
        "core-commerce-display-rate-v1" => 58,
        "core-navigation-options-v1" => 47,
        "core-channel-observations-v1" => 46,
        "core-trust-policy-edit-v1" => 45,
        _ => 44,
    }
}
fn procedure_target() -> &'static str {
    match super::PROCEDURE {
        "core-subscription-pancake-language-optional-v1" => {
            include_str!("../../src/core_schema/upgrade/subscription_pancake_language_target.json")
        }
        "core-commerce-display-rate-v1" => {
            include_str!("../../src/core_schema/upgrade/commerce_display_rate_target.json")
        }
        "core-navigation-options-v1" => {
            include_str!("../../src/core_schema/upgrade/navigation_options_target.json")
        }
        "core-channel-observations-v1" => {
            include_str!("../../src/core_schema/upgrade/channel_observations_target.json")
        }
        "core-trust-policy-edit-v1" => {
            include_str!("../../src/core_schema/upgrade/trust_policy_target.json")
        }
        _ => include_str!("../../src/core_schema/upgrade/completion_target.json"),
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) format_version: u32,
    pub(crate) scope: String,
    pub(crate) components: Vec<Component>,
    pub(crate) identity: Identity,
    pub(crate) installer_contract_sql_sha256: String,
    pub(crate) catalog: Catalog,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Component {
    pub(crate) name: String,
    pub(crate) raw_schema_sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    pub(crate) version: i32,
    pub(crate) fingerprint_sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    pub(crate) query_utf8: String,
    pub(crate) query_sha256: String,
    pub(crate) query_hash_domain: String,
    pub(crate) fingerprint_hash_domain: String,
    pub(crate) definition_order: String,
}
struct Executor {
    binary: PathBuf,
    manifest: Manifest,
}
fn executor() -> &'static Executor {
    static EXECUTOR: OnceLock<Executor> = OnceLock::new();
    EXECUTOR.get_or_init(|| {
        let spec = family();
        let binary = PathBuf::from(
            std::env::var_os(spec.binary_variable)
                .unwrap_or_else(|| panic!("build preserved Core from commit {}", spec.revision)),
        );
        assert!(binary.is_absolute() && binary.is_file());
        assert_eq!(
            binary.canonicalize().unwrap(),
            binary,
            "preserved Core cannot use a symlink"
        );
        let expected = std::env::var(spec.digest_variable)
            .expect("preserved Core requires its source-bound binary SHA256");
        assert_eq!(expected.len(), 64);
        assert!(
            expected
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_eq!(
            crate::hex_codec::encode(Sha256::digest(fs::read(&binary).unwrap())),
            expected
        );
        let output = Command::new(&binary)
            .arg("schema-manifest")
            .env_clear()
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success() && output.stderr.is_empty());
        let manifest: Manifest = serde_json::from_slice(&output.stdout).unwrap();
        let mut canonical = serde_json::to_vec(&manifest).unwrap();
        canonical.push(b'\n');
        assert_eq!(
            output.stdout, canonical,
            "preserved manifest serialization changed"
        );
        let target: serde_json::Value = serde_json::from_str(family().target).unwrap();
        assert_eq!(manifest.components.len(), spec.component_count);
        assert_eq!(
            manifest.components[spec.component_count - 1].name,
            spec.last_component
        );
        assert_eq!(
            crate::hex_codec::encode(Sha256::digest(&canonical)),
            target["manifest_sha256"]
        );
        assert_eq!(
            crate::hex_codec::encode(Sha256::digest(manifest.catalog.query_utf8.as_bytes())),
            manifest.catalog.query_sha256
        );
        Executor { binary, manifest }
    })
}
pub(crate) fn manifest() -> &'static Manifest {
    &executor().manifest
}
pub(crate) fn manifest_sha256(count: usize) -> String {
    let mut manifest = manifest().clone();
    assert!(count <= manifest.components.len());
    manifest.components.truncate(count);
    let mut bytes = serde_json::to_vec(&manifest).unwrap();
    bytes.push(b'\n');
    crate::hex_codec::encode(Sha256::digest(bytes))
}
pub(crate) fn catalog_query() -> &'static str {
    &manifest().catalog.query_utf8
}
struct PrivateDirectory(PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn private_file(path: &std::path::Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}
pub(crate) fn invoke(url: &str, command: &str, input: Option<Vec<u8>>) -> Output {
    let parsed = reqwest::Url::parse(url).unwrap();
    assert!(matches!(parsed.host_str(), Some("localhost" | "127.0.0.1")));
    let path = std::env::temp_dir().join(format!(
        "lmm-preserved-core-{:032x}",
        rand::random::<u128>()
    ));
    DirBuilder::new().mode(0o700).create(&path).unwrap();
    let private = PrivateDirectory(path);
    let database = private.0.join("database-url");
    private_file(&database, url.as_bytes());
    let config = private.0.join("core.toml");
    private_file(
        &config,
        format!(
            "version=1\n[database]\nurl_file={}\n",
            serde_json::to_string(database.to_str().unwrap()).unwrap()
        )
        .as_bytes(),
    );
    let mut child = Command::new(&executor().binary)
        .args(["--config", config.to_str().unwrap(), command])
        .env_clear()
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(&input).unwrap();
    }
    child.wait_with_output().unwrap()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    version: u32,
    protocol: String,
    status: String,
    upgrade_id: String,
    request_sha256: String,
    predecessor_catalog_sha256: String,
    successor_catalog_sha256: String,
    components: usize,
}
async fn maintenance_snapshot(request: &UpgradeRequest) -> Result<serde_json::Value, &'static str> {
    let maintenance_url =
        std::env::var("DATABASE_URL").map_err(|_| "maintenance_url_unavailable")?;
    let options: PgConnectOptions = maintenance_url
        .parse()
        .map_err(|_| "maintenance_options_invalid")?;
    let options = options.disable_statement_logging();
    // Reject an absent/default or target database before opening any connection.
    match options.get_database() {
        Some(database) if !database.is_empty() && database != request.database.name.as_str() => {}
        _ => return Err("maintenance_database_not_separate"),
    }
    let mut connection = PgConnection::connect_with(&options)
        .await
        .map_err(|_| "maintenance_connect_failed")?;
    let snapshot = sqlx::query_scalar::<_, sqlx::types::Json<serde_json::Value>>(
        r#"SELECT jsonb_build_object(
            'backends', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'pid', pid, 'backend_type', backend_type, 'state', state
                ) ORDER BY pid)
                FROM pg_stat_activity WHERE datid=$1::bigint::oid
            ), '[]'::jsonb),
            'prepared_count', (
                SELECT count(*) FROM pg_prepared_xacts p
                JOIN pg_database d ON d.datname=p.database
                WHERE d.oid=$1::bigint::oid
            )
        )"#,
    )
    .bind(i64::from(request.database.oid))
    .fetch_one(&mut connection)
    .await;
    let closed = connection.close().await;
    let snapshot = snapshot.map_err(|_| "maintenance_observation_failed")?;
    closed.map_err(|_| "maintenance_close_failed")?;
    Ok(snapshot.0)
}

async fn diagnose_unexpected_refusal(
    (ordinal, phase): (u8, &'static str),
    request: &UpgradeRequest,
) {
    // The original refusal has already occurred. Never sample before the call,
    // retry it, or turn a diagnostic failure into a different gate failure.
    let observation = match tokio::time::timeout(
        std::time::Duration::from_secs(1),
        maintenance_snapshot(request),
    )
    .await
    {
        Ok(Ok(snapshot)) => serde_json::json!({"status": "observed", "snapshot": snapshot}),
        Ok(Err(status)) => serde_json::json!({"status": status}),
        Err(_) => serde_json::json!({"status": "observation_timeout"}),
    };
    let _ = writeln!(
        std::io::stderr().lock(),
        "preserved_upgrade_diagnostic {}",
        serde_json::json!({"ordinal": ordinal, "phase": phase, "observation": observation})
    );
}

// completion_upgrade uses this original entry point; suffix targets use the
// staged entry below when compiling their own copy of this shared helper.
#[allow(dead_code)]
pub(super) async fn upgrade_database(
    url: &str,
    request: UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    upgrade_database_with_stage(url, request, None).await
}

pub(super) async fn upgrade_database_with_stage(
    url: &str,
    request: UpgradeRequest,
    stage: Option<(u8, &'static str)>,
) -> Result<UpgradeReport, UpgradeError> {
    let input = serde_json::to_vec(&request).unwrap();
    let expected_digest = crate::hex_codec::encode(Sha256::digest(&input));
    let url = url.to_owned();
    let output = tokio::task::spawn_blocking(move || invoke(&url, "upgrade-db", Some(input)))
        .await
        .unwrap();
    if !output.status.success() {
        let refusal: serde_json::Value = serde_json::from_slice(&output.stdout)
            .expect("preserved upgrade rejection must be structured JSON");
        assert_eq!(refusal["version"], 1);
        assert_eq!(refusal["protocol"], "core.upgrade-db.v1");
        assert_eq!(refusal["status"], "rejected");
        let code = refusal["code"].as_str().unwrap();
        assert!(
            code.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        );
        const EXPECTED_REFUSALS: &[&str] = &[
            "COMPLETION_ORIGINAL_RECEIPT_ABSENT",
            "COMPLETION_RECEIPT_OR_CATALOG_CONFLICT",
            "COMPLETION_RECEIPT_PRIVILEGES_INVALID",
            "ACCOUNT_APPEALS_ORIGINAL_RECEIPT_ABSENT",
            "ACCOUNT_APPEALS_RECEIPT_OR_CATALOG_CONFLICT",
            "ACCOUNT_APPEALS_RECEIPT_PRIVILEGES_INVALID",
            "CHANNEL_ACCOUNTING_ORIGINAL_RECEIPT_ABSENT",
            "CHANNEL_ACCOUNTING_RECEIPT_OR_CATALOG_CONFLICT",
            "CHANNEL_ACCOUNTING_RECEIPT_PRIVILEGES_INVALID",
            "CHANNEL_PROJECTION_ORIGINAL_RECEIPT_ABSENT",
            "CHANNEL_PROJECTION_RECEIPT_OR_CATALOG_CONFLICT",
            "CHANNEL_PROJECTION_RECEIPT_PRIVILEGES_INVALID",
            "TRUST_POLICY_EDIT_ORIGINAL_RECEIPT_ABSENT",
            "TRUST_POLICY_EDIT_RECEIPT_OR_CATALOG_CONFLICT",
            "TRUST_POLICY_EDIT_RECEIPT_PRIVILEGES_INVALID",
            "CHANNEL_OBSERVATIONS_ORIGINAL_RECEIPT_ABSENT",
            "CHANNEL_OBSERVATIONS_RECEIPT_OR_CATALOG_CONFLICT",
            "CHANNEL_OBSERVATIONS_RECEIPT_PRIVILEGES_INVALID",
            "NAVIGATION_OPTIONS_ORIGINAL_RECEIPT_ABSENT",
            "NAVIGATION_OPTIONS_RECEIPT_OR_CATALOG_CONFLICT",
            "NAVIGATION_OPTIONS_RECEIPT_PRIVILEGES_INVALID",
            "COMMERCE_DISPLAY_RATE_ORIGINAL_RECEIPT_ABSENT",
            "COMMERCE_DISPLAY_RATE_RECEIPT_OR_CATALOG_CONFLICT",
            "COMMERCE_DISPLAY_RATE_RECEIPT_PRIVILEGES_INVALID",
            "PANCAKE_LANGUAGE_ORIGINAL_RECEIPT_ABSENT",
            "PANCAKE_LANGUAGE_RECEIPT_OR_CATALOG_CONFLICT",
            "PANCAKE_LANGUAGE_RECEIPT_PRIVILEGES_INVALID",
        ];
        let Some(code) = EXPECTED_REFUSALS
            .iter()
            .copied()
            .find(|expected| *expected == code)
        else {
            if let Some(stage) = stage {
                diagnose_unexpected_refusal(stage, &request).await;
            }
            panic!("unexpected preserved upgrade rejection: {code}");
        };
        return Err(UpgradeError::Rejected(code));
    }
    assert!(output.stderr.is_empty());
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.version, 1);
    assert_eq!(report.protocol, "core.upgrade-db.v1");
    assert_eq!(report.upgrade_id, request.upgrade_id);
    assert_eq!(report.request_sha256, expected_digest);
    assert_eq!(report.predecessor_catalog_sha256, super::PRE_CATALOG);
    assert_eq!(report.components, target_count());
    let target: serde_json::Value = serde_json::from_str(procedure_target()).unwrap();
    assert_eq!(report.successor_catalog_sha256, target["catalog_sha256"]);
    let status = match report.status.as_str() {
        "committed" => "committed",
        "already_committed" => "already_committed",
        _ => panic!("unexpected successful preserved upgrade status"),
    };
    Ok(UpgradeReport {
        version: 1,
        protocol: "core.upgrade-db.v1",
        status,
        upgrade_id: report.upgrade_id,
        request_sha256: report.request_sha256,
        predecessor_catalog_sha256: super::PRE_CATALOG,
        successor_catalog_sha256: report.successor_catalog_sha256,
        components: report.components,
    })
}

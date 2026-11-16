//! Source-bound historical Core67 CLI for the frozen Pancake upgrade contract.
//! The current fresh-install binary deliberately does not implement this family.
use lmm_core::core_schema::{UpgradeError, UpgradeReport, UpgradeRequest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::OnceLock,
};

const TARGET: &str =
    include_str!("../../src/core_schema/upgrade/subscription_pancake_language_target.json");
const PRE_CATALOG: &str = "be0b981025782d501487bcc8b9cd9c59bccf9cae2351c77708a1c1ce16b0358e";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format_version: u32,
    scope: String,
    components: Vec<Component>,
    identity: Identity,
    installer_contract_sql_sha256: String,
    catalog: Catalog,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Component {
    name: String,
    raw_schema_sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    version: i32,
    fingerprint_sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    query_utf8: String,
    query_sha256: String,
    query_hash_domain: String,
    fingerprint_hash_domain: String,
    definition_order: String,
}
struct Executor {
    binary: PathBuf,
    manifest: Manifest,
}
fn executor() -> &'static Executor {
    static EXECUTOR: OnceLock<Executor> = OnceLock::new();
    EXECUTOR.get_or_init(|| {
        let binary =
            PathBuf::from(std::env::var_os("LMM_PRESERVED_PANCAKE67_CORE_BIN").expect(
                "build preserved Core67 from commit 0510c2455dabcb63e33c719ba52ac189d62612bc",
            ));
        assert!(binary.is_absolute() && binary.is_file());
        assert_eq!(
            binary.canonicalize().unwrap(),
            binary,
            "preserved Core cannot use a symlink"
        );
        let expected = std::env::var("LMM_PRESERVED_PANCAKE67_CORE_SHA256")
            .expect("preserved Core67 requires its source-bound binary SHA256");
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
        let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
        assert_eq!(manifest.components.len(), 67);
        assert_eq!(
            manifest.components[66].name,
            "subscription-pancake-language-optional"
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
pub(crate) fn manifest_sha256(count: usize) -> String {
    let mut manifest = executor().manifest.clone();
    assert!(count <= manifest.components.len());
    manifest.components.truncate(count);
    let mut bytes = serde_json::to_vec(&manifest).unwrap();
    bytes.push(b'\n');
    crate::hex_codec::encode(Sha256::digest(bytes))
}
pub(crate) fn catalog_query() -> &'static str {
    &executor().manifest.catalog.query_utf8
}
pub(crate) async fn assert_installed_target(pool: &sqlx::PgPool) {
    let actual: Vec<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT name,fingerprint FROM core_meta.component_contract ORDER BY name COLLATE \"C\"",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut expected: Vec<_> = executor()
        .manifest
        .components
        .iter()
        .map(|component| (component.name.clone(), component.raw_schema_sha256.clone()))
        .collect();
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    let actual: Vec<_> = actual
        .into_iter()
        .map(|(name, fingerprint)| (name, crate::hex_codec::encode(fingerprint)))
        .collect();
    assert_eq!(
        actual, expected,
        "fixture must begin with the real retained Core67 installer"
    );
    let definitions: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(catalog_query()))
        .fetch_all(pool)
        .await
        .unwrap();
    let mut digest = Sha256::new();
    for definition in definitions {
        digest.update((definition.len() as u64).to_be_bytes());
        digest.update(definition.as_bytes());
    }
    let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
    assert_eq!(
        crate::hex_codec::encode(digest.finalize()),
        target["catalog_sha256"]
    );
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
fn invoke(url: &str, command: &str, input: Option<Vec<u8>>) -> Output {
    let parsed = reqwest::Url::parse(url).unwrap();
    assert!(matches!(parsed.host_str(), Some("localhost" | "127.0.0.1")));
    let path = std::env::temp_dir().join(format!(
        "lmm-preserved-pancake67-{:032x}",
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
pub(super) async fn initialize(url: &str, owner: &str) {
    let mut url = reqwest::Url::parse(url).unwrap();
    url.query_pairs_mut()
        .append_pair("options", &format!("-c role={owner}"));
    let output = tokio::task::spawn_blocking(move || invoke(url.as_str(), "init-db", None))
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "preserved Core67 init-db failed with exit {:?}",
        output.status.code()
    );
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
pub(super) async fn upgrade_database(
    url: &str,
    request: UpgradeRequest,
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
        return Err(UpgradeError::Rejected(match code {
            "PANCAKE_LANGUAGE_ORIGINAL_RECEIPT_ABSENT" => {
                "PANCAKE_LANGUAGE_ORIGINAL_RECEIPT_ABSENT"
            }
            "PANCAKE_LANGUAGE_RECEIPT_OR_CATALOG_CONFLICT" => {
                "PANCAKE_LANGUAGE_RECEIPT_OR_CATALOG_CONFLICT"
            }
            "PANCAKE_LANGUAGE_RECEIPT_PRIVILEGES_INVALID" => {
                "PANCAKE_LANGUAGE_RECEIPT_PRIVILEGES_INVALID"
            }
            other => panic!("unexpected preserved upgrade rejection: {other}"),
        }));
    }
    assert!(output.stderr.is_empty());
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.version, 1);
    assert_eq!(report.protocol, "core.upgrade-db.v1");
    assert_eq!(report.upgrade_id, request.upgrade_id);
    assert_eq!(report.request_sha256, expected_digest);
    assert_eq!(report.predecessor_catalog_sha256, PRE_CATALOG);
    assert_eq!(report.components, 67);
    let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
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
        predecessor_catalog_sha256: PRE_CATALOG,
        successor_catalog_sha256: report.successor_catalog_sha256,
        components: report.components,
    })
}

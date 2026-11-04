//! Explicit owner-only measurement, not a business acceptance test.
use super::*;
use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf};

#[sqlx::test(migrations = false)]
#[ignore = "explicit isolated owner database and new private capture path required"]
async fn capture_fresh_install_catalog(pool: PgPool) {
    let output = PathBuf::from(std::env::var("LMM_FRESH_CATALOG_CAPTURE").unwrap());
    let parent = output.parent().unwrap().canonicalize().unwrap();
    let backups = PathBuf::from(std::env::var("HOME").unwrap())
        .join(".local/state/lmm-api")
        .canonicalize()
        .unwrap();
    assert!(parent.starts_with(backups));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&output)
        .unwrap();
    init(&pool).await.unwrap();
    let catalog: String = sqlx::query_scalar(
        "SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut manifest = serde_json::to_vec(&schema_manifest()).unwrap();
    manifest.push(b'\n');
    let record = serde_json::json!({
        "manifest_sha256":format!("{:x}",Sha256::digest(manifest)),
        "catalog_sha256":catalog,
    });
    writeln!(file, "{}", serde_json::to_string_pretty(&record).unwrap()).unwrap();
    file.sync_all().unwrap();
}

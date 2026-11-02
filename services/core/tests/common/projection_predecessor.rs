//! Frozen 43-component SQLx setup; never the latest initializer.
#[path = "accounting_predecessor.rs"]
mod frozen42;
use sha2::{Digest, Sha256};
pub async fn install(pool: &sqlx::PgPool) {
    frozen42::install(pool).await;
    let mut tx = pool.begin().await.unwrap();
    let ddl = include_str!("../../schema/channel_accounting.sql");
    sqlx::raw_sql(ddl).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES('channel-accounting',$1)")
        .bind(Sha256::digest(ddl.as_bytes()).to_vec()).execute(&mut *tx).await.unwrap();
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../src/core_schema/upgrade/projection_predecessor_manifest.json"
    ))
    .unwrap();
    let definitions: Vec<String> =
        sqlx::query_scalar(manifest["catalog"]["query_utf8"].as_str().unwrap())
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    let mut hash = Sha256::new();
    for definition in definitions {
        hash.update((definition.len() as u64).to_be_bytes());
        hash.update(definition.as_bytes());
    }
    let catalog = hash.finalize().to_vec();
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(
                include_str!("../../src/core_schema/upgrade/projection_predecessor_manifest.json")
                    .as_bytes()
            )
        ),
        "f12d5e81ca219acdc59ac07be931d52e46f0b64974f1e2b3154ad102b5cc0c1c"
    );
    assert_eq!(
        catalog
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "1f9ef4951b101320e4016f2e8347985c7a321b4c3405fe756864abba1bf7c680"
    );
    sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton")
        .bind(catalog)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

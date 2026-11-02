//! Exact frozen 45 installation; no component46 or simulated observations.
#[path = "trust_policy_predecessor.rs"]
mod frozen44;
use sha2::{Digest, Sha256};
pub async fn install(pool: &sqlx::PgPool) {
    frozen44::install(pool).await;
    let mut tx = pool.begin().await.unwrap();
    let ddl = concat!(
        include_str!("../../schema/trust_policy_edit.sql"),
        "\n",
        include_str!("../../schema/trust_policy_edit_proof.sql"),
        "\n",
        include_str!("../../schema/trust_policy_edit_editor.sql"),
        "\n",
        include_str!("../../schema/trust_policy_upgrade.sql")
    );
    sqlx::raw_sql(ddl).execute(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO core_meta.component_contract(name,fingerprint) VALUES('trust-policy-edit',$1)",
    )
    .bind(Sha256::digest(ddl.as_bytes()).to_vec())
    .execute(&mut *tx)
    .await
    .unwrap();
    let mut manifest = lmm_core::core_schema::schema_manifest();
    manifest.components.truncate(45);
    let mut bytes = serde_json::to_vec(&manifest).unwrap();
    bytes.push(b'\n');
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "567d84fa06adf6916a62b6a5de0ae4234da1c0876972254f8393f0a55cd8d9c8"
    );
    let definitions: Vec<String> = sqlx::query_scalar(manifest.catalog.query_utf8)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    let mut hash = Sha256::new();
    for definition in definitions {
        hash.update((definition.len() as u64).to_be_bytes());
        hash.update(definition.as_bytes());
    }
    let catalog = hash.finalize();
    assert_eq!(
        format!("{catalog:x}"),
        "b3e3db14753bcb575eba90541b09cf9fac2955a22e513741a91b1f11019dbe6f"
    );
    sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton")
        .bind(catalog.as_slice())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

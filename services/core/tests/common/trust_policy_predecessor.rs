//! Exact frozen 44 installation, including pending projection work.
#[path = "projection_predecessor.rs"]
mod frozen43;
use sha2::{Digest, Sha256};
pub async fn install(pool: &sqlx::PgPool) {
    frozen43::install(pool).await;
    let mut tx = pool.begin().await.unwrap();
    let ddl = concat!(
        include_str!("../../schema/channel_projection.sql"),
        "\n",
        include_str!("../../schema/channel_openings.sql")
    );
    sqlx::raw_sql(ddl).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES('channel-projection',$1)")
        .bind(Sha256::digest(ddl.as_bytes()).to_vec()).execute(&mut *tx).await.unwrap();
    let mut manifest = lmm_core::core_schema::schema_manifest();
    manifest.components.truncate(44);
    let mut bytes = serde_json::to_vec(&manifest).unwrap();
    bytes.push(b'\n');
    assert_eq!(
        hex_codec::encode(Sha256::digest(bytes)),
        "5f6b75aaae3110c460c58270aac77028d8d4c32360481a52831872515131261d"
    );
    let definitions: Vec<String> =
        sqlx::query_scalar(sqlx::AssertSqlSafe(manifest.catalog.query_utf8))
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
        hex_codec::encode(catalog),
        "d4cb2dbfd24de133c92dbd8128ea691d50dc93f72c4753ec995135d081b8f584"
    );
    sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton")
        .bind(catalog.as_slice())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

use crate::hex_codec;

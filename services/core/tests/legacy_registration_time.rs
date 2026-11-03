//! Execute the exact Python import statement against the compiled fresh Core.
use lmm_core::core_schema;
use sqlx::PgPool;
#[sqlx::test(migrations = false)]
async fn imported_registration_time_preserves_seconds_and_marks_unknown(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let script = r#"
import importlib.util,json,pathlib,sys
p=pathlib.Path('scripts/migrate-legacy-snapshot.py')
s=importlib.util.spec_from_file_location('registration_fixture',p)
m=importlib.util.module_from_spec(s);sys.modules[s.name]=m;s.loader.exec_module(m)
users=[{'id':i,'username':f'synthetic-{i}','password':'$2b$10$'+'.'*53,'role':1,'status':1,'auth_version':1,'deleted_at':None} for i in (1,2)]
users[0]['created_at']=1700000001
records=m.map_identity(users,[],source_sha256='a'*64)['records']
print(json.dumps({'sql':m.ACCOUNT_IMPORT_SQL,'records':records}))
"#;
    let result = std::process::Command::new("python3")
        .args(["-c", script])
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let fixture: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('lmm.test_payload',$1,true)")
        .bind(fixture["records"].to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let statement = format!(
        "DO $fixture$ DECLARE payload jsonb := current_setting('lmm.test_payload')::jsonb; BEGIN {} END $fixture$",
        fixture["sql"].as_str().unwrap()
    );
    sqlx::query(&statement).execute(&mut *tx).await.unwrap();
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT id,extract(epoch FROM created_at)::bigint FROM core_identity.accounts ORDER BY id",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert_eq!(rows, vec![(1, 1700000001), (2, 0)]);
    tx.rollback().await.unwrap();
}

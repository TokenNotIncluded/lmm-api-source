//! Independent synthetic receipt scale fixture. Never writes projection tables.
//! Each cloned receipt derives from a real HTTP settlement, with canonical command
//! identities rewritten. This proves receipt validation/scale, not a million
//! real funding transactions; the small seed stage proves funding separately.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::time::Instant;

pub const ROWS: usize = 1_000_000;
pub const BATCH: usize = 1000;
pub const DAY: i64 = 1_700_006_400; // UTC midnight; verify this invariant in the test.

pub fn metric(name: &str, start: Instant, rows: usize) {
    println!(
        "PROJECTION_MEASURE {}",
        json!({"name":name,"elapsed_ms":start.elapsed().as_millis(),"rows":rows})
    );
}

/// No DSN is printed, no database is created, and the existing fixture must be
/// explicitly named by Main. The runner also binds the binary to an approval file.
pub async fn approved_database() -> PgPool {
    let url = std::env::var("LMM_PROJECTION_DATABASE_URL")
        .expect("Main-coordinated isolated PG URL required");
    let name =
        std::env::var("LMM_PROJECTION_DATABASE_NAME").expect("isolated database name required");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect approved isolated fixture");
    let actual: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(actual, name, "approved database identity mismatch");
    let occupied: bool =
        sqlx::query_scalar("SELECT to_regclass('core_charging.requests') IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !occupied,
        "fixture needs an empty Main-approved database; never clear another run"
    );
    pool
}

/// Complete source ledger rows, including balances and idempotency receipts.
/// Small actual funding seed only: bulk fixtures must leave these rows untouched.
pub async fn financial_snapshot(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(\
      'commands',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY scope,operation_key),'[]') FROM core_billing.ledger_operations t),\
      'journals',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY id),'[]') FROM core_billing.ledger_journals t),\
      'entries',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY journal_id,ledger_account_id),'[]') FROM core_billing.ledger_entries t),\
      'balances',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY ledger_account_id),'[]') FROM core_billing.balance_state t),\
      'holds',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY request_id,position),'[]') FROM core_charging.ledger_holds t),\
      'checks',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY request_id,request_revision,budget_id),'[]') FROM core_charging.budget_checks t))")
        .fetch_one(pool).await.unwrap()
}

/// Calls the real owner entry, which locks source facts and validates all receipts.
/// No runtime/test-only bypass or directly seeded rollup is accepted.
pub async fn rebuild(pool: &PgPool) {
    let start = Instant::now();
    let rows = lmm_core::core_schema::rebuild_channel_usage(pool)
        .await
        .unwrap();
    let ready: bool =
        sqlx::query_scalar("SELECT ready FROM core_channels.projection_state WHERE singleton")
            .fetch_one(pool)
            .await
            .unwrap();
    let dirty: i64 = sqlx::query_scalar("SELECT count(*) FROM core_channels.projection_dirty")
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(ready);
    assert_eq!(dirty, 0);
    metric("owner-validator-rebuild", start, rows as usize);
}

/// Inspect the actual Native footer/proof before deriving any scale material.
/// Full semantic validation remains the product owner's non-skippable backfill.
pub async fn assert_seed(pool: &PgPool, id: &str, channel: i64) {
    let row = sqlx::query("SELECT r.settled,r.refunded,r.price,\
        (SELECT detail FROM core_charging.usage_events WHERE request_id=r.id AND event_id='final') AS final,\
        (SELECT detail FROM core_charging.usage_events WHERE request_id=r.id AND event_id='reservation-contract:v1') AS proof,\
        (SELECT detail FROM core_charging.usage_events WHERE request_id=r.id AND event_id='reservation-admission:v1') AS admission,\
        (SELECT count(*) FROM core_charging.ledger_commands WHERE request_id=r.id) AS commands \
        FROM core_charging.requests r WHERE id=$1").bind(id).fetch_one(pool).await.unwrap();
    assert_eq!(row.get::<i64, _>("settled"), 41);
    assert_eq!(row.get::<i64, _>("refunded"), 5);
    assert!(row.get::<i64, _>("commands") >= 3);
    let final_event: Value = row.get("final");
    assert_eq!(final_event["kind"], "final");
    assert_eq!(final_event["usage"]["input"], 40);
    assert_eq!(final_event["usage"]["output"], 1);
    let proof: Value = row.get("proof");
    let admission: Value = row.get("admission");
    assert_eq!(proof["binding"]["upstream_id"], channel);
    assert_eq!(proof["prepared_sha256"], admission["prepared_sha256"]);
    assert_eq!(
        proof["frozen_price_sha256"],
        admission["frozen_price_sha256"]
    );
    assert_eq!(
        proof["admission_spec_sha256"],
        admission["admission_spec_sha256"]
    );
    assert_eq!(proof["frozen_price_sha256"].as_str().unwrap().len(), 64);
}

struct Template {
    request: Value,
    events: Vec<(String, Value)>,
    commands: Vec<(String, Value)>,
    refunds: Vec<Value>,
}
impl Template {
    async fn load(pool: &PgPool, id: &str) -> Self {
        let request =
            sqlx::query_scalar("SELECT to_jsonb(r) FROM core_charging.requests r WHERE id=$1")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap();
        let events = sqlx::query_as("SELECT event_id,detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id<>'performance-v1' ORDER BY event_id")
            .bind(id).fetch_all(pool).await.unwrap();
        let refunds: Vec<Value> = sqlx::query_scalar("SELECT to_jsonb(r)-'request_id' FROM core_charging.refunds r WHERE request_id=$1 ORDER BY refund_id")
            .bind(id).fetch_all(pool).await.unwrap();
        let raw: Vec<(String, Value)> = sqlx::query_as("SELECT operation_id,command FROM core_charging.ledger_commands WHERE request_id=$1 ORDER BY operation_id")
            .bind(id).fetch_all(pool).await.unwrap();
        let commands = raw
            .into_iter()
            .map(|(op, command)| {
                let tag = [
                    format!(
                        "reserve:{}",
                        command["change"]["total"].as_i64().unwrap_or(0)
                    ),
                    "settle".to_owned(),
                    "release".to_owned(),
                ]
                .into_iter()
                .chain(
                    refunds
                        .iter()
                        .map(|r| format!("refund:{}", r["refund_id"].as_str().unwrap())),
                )
                .find(|tag| operation_id(id, tag) == op)
                .expect("unsupported real seed command tag");
                (tag, command)
            })
            .collect();
        Self {
            request,
            events,
            commands,
            refunds,
        }
    }
    fn receipt(&self, id: &str, at: i64) -> Value {
        let mut request = self.request.clone();
        request["id"] = json!(id);
        request["authorized_at"] = json!(at);
        request["lease_until"] = json!(at + 60);
        request["hard_deadline"] = json!(at + 3600);
        let commands: Vec<Value> = self
            .commands
            .iter()
            .map(|(tag, original)| {
                let mut command = original.clone();
                command["request_id"] = json!(id);
                command["operation_id"] = json!(operation_id(id, tag));
                command
            })
            .collect();
        json!({"request":request,"events":self.events,"commands":commands,"refunds":self.refunds})
    }
}
fn operation_id(id: &str, tag: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&("core-charging-v1", id, tag)).unwrap())
    )
}

/// 1m requests on channel 1 plus ten sparse channel-2 requests. Both queries
/// therefore see a site above the old 100k bound, without doubling the dataset.
/// No trigger disabling or receipt mutation: every source fact is append-only.
pub async fn bulk(pool: &PgPool, seeds: [&str; 2]) {
    assert_eq!(DAY % 86400, 0);
    let templates = [
        Template::load(pool, seeds[0]).await,
        Template::load(pool, seeds[1]).await,
    ];
    let columns: Vec<String> = sqlx::query_scalar("SELECT column_name FROM information_schema.columns WHERE table_schema='core_charging' AND table_name='requests' AND is_generated='NEVER' ORDER BY ordinal_position")
        .fetch_all(pool).await.unwrap();
    assert!(
        columns
            .iter()
            .all(|c| c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    );
    let names = columns.join(",");
    let selected = columns
        .iter()
        .map(|c| format!("r.{c}"))
        .collect::<Vec<_>>()
        .join(",");
    let insert = format!(
        "INSERT INTO core_charging.requests({names}) SELECT {selected} FROM jsonb_array_elements($1) x CROSS JOIN LATERAL jsonb_populate_record(NULL::core_charging.requests,x->'request') r"
    );
    let start = Instant::now();
    for low in (0..ROWS + 10).step_by(BATCH) {
        let high = (low + BATCH).min(ROWS + 10);
        let batch: Vec<Value> = (low..high)
            .map(|n| {
                let sparse = n >= ROWS;
                let at = DAY + [-1, 0, 1, 86399, 86400][n % 5];
                let id = if sparse {
                    format!("projection-sparse:{n:07}")
                } else {
                    format!("projection-scale:{n:07}")
                };
                templates[usize::from(sparse)].receipt(&id, at)
            })
            .collect();
        let batch = json!(batch);
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query(&insert)
                .bind(&batch)
                .execute(&mut *tx)
                .await
                .unwrap()
                .rows_affected(),
            (high - low) as u64
        );
        sqlx::query("INSERT INTO core_charging.usage_events(request_id,event_id,detail) SELECT x->'request'->>'id',e->>0,e->1 FROM jsonb_array_elements($1) x CROSS JOIN LATERAL jsonb_array_elements(x->'events') e")
            .bind(&batch).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO core_charging.ledger_commands(operation_id,request_id,command) SELECT c->>'operation_id',x->'request'->>'id',c FROM jsonb_array_elements($1) x CROSS JOIN LATERAL jsonb_array_elements(x->'commands') c")
            .bind(&batch).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO core_charging.refunds(request_id,refund_id,amount,evidence,created_at) SELECT x->'request'->>'id',r->>'refund_id',(r->>'amount')::bigint,r->>'evidence',(r->>'created_at')::timestamptz FROM jsonb_array_elements($1) x CROSS JOIN LATERAL jsonb_array_elements(x->'refunds') r")
            .bind(&batch).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
    }
    metric("bulk-single-million-plus-sparse-ten", start, ROWS + 10);
}

/// Two-way relational equality catches stale extra rows and missing/incorrect
/// buckets. SHA256's first six low bits, both lifetime (-1) and UTC daily keys.
pub async fn assert_rollups(pool: &PgPool, scale: bool) {
    // Compute bucket independently in Rust; do not copy the production SQL hash.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("CREATE TEMP TABLE expected_projection_buckets(request_id text PRIMARY KEY,bucket integer NOT NULL) ON COMMIT DROP")
        .execute(&mut *tx).await.unwrap();
    for low in (0..if scale { ROWS } else { 0 }).step_by(BATCH) {
        let ids: Vec<String> = (low..low + BATCH)
            .map(|n| format!("projection-scale:{n:07}"))
            .collect();
        let buckets: Vec<i32> = ids
            .iter()
            .map(|id| i32::from(Sha256::digest(id.as_bytes())[0] & 63))
            .collect();
        sqlx::query("INSERT INTO expected_projection_buckets SELECT * FROM unnest($1::text[],$2::integer[])")
            .bind(ids).bind(buckets).execute(&mut *tx).await.unwrap();
    }
    // Small real seeds share the same rollup table and must be included too.
    let ids: Vec<String> = sqlx::query_scalar("SELECT DISTINCT request_id FROM core_channels.usage_contributions WHERE request_id NOT LIKE 'projection-scale:%'")
        .fetch_all(&mut *tx).await.unwrap();
    for id in ids {
        let bucket = i32::from(Sha256::digest(id.as_bytes())[0] & 63);
        sqlx::query("INSERT INTO expected_projection_buckets VALUES($1,$2)")
            .bind(id)
            .bind(bucket)
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    let delta: i64 = sqlx::query_scalar("WITH expected AS (\
      SELECT channel_id,day,bucket,sum(gross_credits) gross_credits,sum(refunded_credits) refunded_credits,\
       sum(settled_requests) settled_requests,sum(pending_requests) pending_requests,sum(unknown_requests) unknown_requests \
      FROM core_channels.usage_contributions c JOIN expected_projection_buckets b USING(request_id)\
      CROSS JOIN LATERAL (VALUES(-1::bigint),(c.authorized_at/86400)) d(day) GROUP BY channel_id,day,bucket),\
      actual AS (SELECT channel_id,day,bucket,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests FROM core_channels.usage_rollups),\
      difference AS ((SELECT * FROM expected EXCEPT SELECT * FROM actual) UNION ALL (SELECT * FROM actual EXCEPT SELECT * FROM expected))\
      SELECT count(*) FROM difference")
        .fetch_one(&mut *tx).await.unwrap();
    assert_eq!(delta, 0, "contribution/rollup relational difference");
    tx.commit().await.unwrap();
}

//! Only synthetic version documents and audit facts in a private SQLx database.
pub async fn seed_history(pool: &sqlx::PgPool) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(
        r#"
INSERT INTO public.routing_price_versions(version,document,sealed) VALUES(1,'{"version":1}',true);
INSERT INTO public.routing_config_versions(version,price_version,document,sealed) VALUES
 (1,1,'{"version":1,"upstreams":[{"id":101},{"id":102},{"id":104}]}',true),
 (2,1,'{"version":2,"upstreams":[{"id":102}]}',true),
 (3,1,'{"version":3,"upstreams":[{"id":101},{"id":103}]}',true);
UPDATE public.routing_active_release SET config_version=3 WHERE singleton;
INSERT INTO core_identity.audit(actor_user_id,action,target) VALUES
 (7001,'channel.delete','{"change":{"id":103}}'),
 (7001,'channel.delete','{"change":{"id":106}}'),
 (7001,'channel.create','{"change":{"id":107}}'),
 (7001,'channel.copy','{"change":{"id":108}}');
"#,
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    seed_native_history(pool).await;
}
pub async fn seed_native_history(pool: &sqlx::PgPool) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(r#"INSERT INTO core_charging.requests(id,actor_user_id,key_id,owner_account_id,payer_account_id,
 fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved)
SELECT 'synthetic-bootstrap-text',7001,c.id,u.personal_account_id,u.personal_account_id,
 decode(repeat('01',32),'hex'),decode(repeat('02',32),'hex'),decode(repeat('03',32),'hex'),
 '{}',1,2,3,'reserved',0 FROM core_identity.users u JOIN core_identity.credentials c ON c.user_id=u.id
 WHERE u.id=7001 ORDER BY c.id LIMIT 1;
INSERT INTO core_charging.usage_events(request_id,event_id,detail) VALUES
 ('synthetic-bootstrap-text','reservation-contract:v1',
 '{"kind":"model_reservation_contract","version":1,"binding":{"upstream_id":201},"fallbacks":[{"binding":{"upstream_id":202}}]}');
-- Create the actual Native API-key source before its deferred charging FK.
INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at)
SELECT decode(repeat('07',32),'hex'),'api_key',id,auth_version,personal_account_id,
 '9999-12-31 UTC'::timestamptz FROM core_identity.users WHERE id=7001;
INSERT INTO core_image.operations(id,source_kind,key_id,actor_user_id,owner_account_id,auth_version,
 input,fingerprint,price,state,created_at,expires_at,confirmed_at)
SELECT 'image:'||repeat('a',64),'api_key',c.id,c.user_id,c.owner_account_id,c.user_version,
 '{}',decode(repeat('04',32),'hex'),
 '{"version":"synthetic-image-v1","model":"synthetic-image","group":"default",
   "input_per_million":0,"cached_per_million":0,"output_per_million":0,
   "image":{"revision":1,"reservation_credits":0,
     "request":{"upstream_id":203,"vendor":"openai","billing_group":"default",
       "upstream_model":"synthetic-image","semantic_model":"synthetic-image",
       "endpoint":"generations","size":"1024x1024","quality":"standard","n":1},
     "rule":{"upstream_id":203,"vendor":"openai","billing_group":"default",
       "upstream_model":"synthetic-image","semantic_model":"synthetic-image",
       "endpoint":"generations","size":"1024x1024","quality":"standard",
       "usd_micros_per_image":{"numerator":0,"denominator":1},
       "group_multiplier":{"numerator":1,"denominator":1},"max_n":1,
       "count_basis":"requested","max_budget":0,"source_revision":"synthetic-frozen42"}}}',
 'confirmed',1,253402300799,1
 FROM core_identity.credentials c WHERE c.digest=decode(repeat('07',32),'hex');
INSERT INTO core_charging.requests(id,actor_user_id,key_id,image_operation_id,owner_account_id,payer_account_id,
 fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved)
SELECT id,actor_user_id,key_id,id,owner_account_id,owner_account_id,
 fingerprint,decode(repeat('05',32),'hex'),decode(repeat('06',32),'hex'),
 price,1,2,3,'reserved',(price#>>'{image,reservation_credits}')::bigint
 FROM core_image.operations WHERE id='image:'||repeat('a',64);
"#).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}
pub async fn check_history(c: &mut sqlx::PgConnection) {
    let ids: Vec<(i64,String,bool,bool)> = sqlx::query_as(
        "SELECT id,origin,history_ambiguous,retired_at IS NOT NULL FROM core_channels.identities ORDER BY id")
        .fetch_all(&mut *c).await.unwrap();
    assert_eq!(
        ids,
        vec![
            (101, "migrated".into(), true, false),
            (102, "migrated".into(), false, true),
            (103, "migrated".into(), true, true),
            (104, "migrated".into(), false, true),
            (106, "migrated".into(), false, true),
            (107, "migrated".into(), false, false),
            (108, "migrated".into(), false, false),
            (201, "migrated".into(), false, true),
            (202, "migrated".into(), false, true),
            (203, "migrated".into(), false, true),
        ]
    );
    let baselines: Vec<(i64,String,String,bool)> = sqlx::query_as(
        "SELECT channel_id,gross_credits::text,refunded_credits::text,historical_coverage FROM core_channels.counter_baselines ORDER BY channel_id")
        .fetch_all(&mut *c).await.unwrap();
    assert_eq!(baselines, vec![(101, "0".into(), "0".into(), false)]);
    let unknown: Vec<(i64,bool)> = sqlx::query_as(
        "SELECT channel_id,kind='unknown' AND balance IS NULL AND observed_at IS NULL AND credential_id IS NULL AND credential_revision IS NULL FROM core_channels.balance_observations ORDER BY channel_id")
        .fetch_all(&mut *c).await.unwrap();
    assert_eq!(unknown, vec![(101, true)]);
}

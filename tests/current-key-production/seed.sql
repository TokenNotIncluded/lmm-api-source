-- Disposable synthetic Core78 fixture only; never a production migration.
BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on;
INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id AS fixture_account_id \gset
INSERT INTO core_billing.account_policies(account_id,preference) VALUES(:fixture_account_id,'wallet_only');
INSERT INTO core_identity.users(personal_account_id,platform_role,active,auth_version)
VALUES(:fixture_account_id,'user',true,1) RETURNING id AS fixture_user_id \gset
INSERT INTO core_identity.user_profiles(user_id,username) VALUES(:fixture_user_id,'synthetic-current-runtime-keys');
INSERT INTO core_identity.developer_access(user_id,granted,source_reference)
VALUES(:fixture_user_id,true,'synthetic-current-runtime-keys');
-- Use the same official immutable publication function as channel management.
-- The endpoint is a synthetic numeric loopback trap; no provider is reachable.
SELECT public.routing_control_publish(
  '{"version":1,"groups":[{"name":"paid","enabled":true}],
    "upstreams":[{"id":1,"endpoint":"https://127.0.0.1:KEY_SEAM_PROVIDER_PORT/custom/v1","protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true}],
    "routes":[{"id":1,"model":"synthetic-model","group":"paid","enabled":true,"speeds":["standard"],
      "targets":[{"id":1,"upstream_id":1,"upstream_model":"provider-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
      "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]}',
  '{"version":1,"rules":[{"id":1,"model":"synthetic-model","group":"paid","speed":"standard",
    "rates":{"input":2000000,"output":4000000,"cache_read":500000,"cache_write":0,"request":0},
    "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]}',
  NULL, NULL);
INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference)
VALUES(:fixture_user_id,'paid',true,'synthetic-current-runtime-keys');
WITH fact AS (
  SELECT jsonb_build_object('scope','synthetic-current-runtime-keys-fixture',
    'account_id',:fixture_account_id::bigint,'wallet_units',1000000,
    'reserved_units',0,'affiliate_units',0) body
), bound AS (
  SELECT encode(sha256(convert_to(body::text,'UTF8')),'hex') h FROM fact
)
SELECT core_billing.import_opening_balance(jsonb_build_object(
  'migration_id','synthetic-current-runtime-key-seed',
  'source_entity_key','synthetic-user:'||:fixture_user_id::text,
  'source_archive_sha256',h,'mapping_sha256',h,'source_row_sha256',h,
  'account_id',:fixture_account_id::bigint,'available_units',1000000,
  'reserved_units',0,'affiliate_units',0,'observed_at',1000)) FROM bound;
COMMIT;
SELECT jsonb_build_object('user_id',:fixture_user_id::bigint,'account_id',:fixture_account_id::bigint);

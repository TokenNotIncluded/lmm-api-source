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
INSERT INTO public.routing_price_versions(version,document) VALUES(1,'{"version":1}');
INSERT INTO public.routing_config_versions(version,price_version,document) VALUES(1,1,'{"version":1}');
INSERT INTO public.routing_groups(version,name,enabled) VALUES(1,'paid',true);
UPDATE public.routing_price_versions SET sealed=true WHERE version=1;
UPDATE public.routing_config_versions SET sealed=true WHERE version=1;
UPDATE public.routing_active_release SET config_version=1 WHERE singleton;
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

-- Component 87: accept the already implemented invitation confirmation.
-- The generic browser nonce/state contract still applies. Invitations do not
-- receive the Core key/market mutation receipt shape or any spending authority.
ALTER TABLE core_assistant_tools.confirmations
 DROP CONSTRAINT confirmations_tool_name_check;
ALTER TABLE core_assistant_tools.confirmations
 ADD CONSTRAINT confirmations_tool_name_check CHECK(tool_name IN (
  'set_overview_greeting','create_site_issue','update_site_issue',
  'request_human_support','prepare_admin_site_policy_change',
  'prepare_api_key_action','prepare_user_action','request_create_key','connect_market_tool',
  'send_invitation'));

-- Preserve every frozen85 execution assertion. A historical prepared result
-- keeps its original URL; the native browser now uses the refresh-cookie path.
CREATE OR REPLACE FUNCTION core_assistant_tools.check_action_execution()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE q core_assistant_tools.confirmations%ROWTYPE;
DECLARE a core_identity.audit%ROWTYPE;
DECLARE o core_assistant.agent_tool_operations%ROWTYPE;
BEGIN
 SELECT * INTO STRICT q FROM core_assistant_tools.confirmations WHERE id=NEW.id;
 IF q.tool_name NOT IN ('prepare_api_key_action','prepare_user_action','request_create_key','connect_market_tool') OR q.state<>'approved' THEN
  RETURN NULL;
 END IF;
 SELECT * INTO a FROM core_identity.audit WHERE id=(q.execution_receipt->>'audit_id')::bigint;
 SELECT * INTO o FROM core_assistant.agent_tool_operations WHERE id=q.operation_id;
 IF a.id IS NULL OR a.actor_user_id<>q.actor_user_id OR a.credential_id IS DISTINCT FROM q.credential_id
  OR a.action<>'assistant.confirmed_action'
  OR a.target IS DISTINCT FROM (q.execution_receipt-ARRAY['executed','actor_user_id','audit_id'])
  OR q.execution_receipt->>'after_sha256' IS DISTINCT FROM encode(sha256(convert_to((q.execution_receipt->'after')::text,'UTF8')),'hex')
  OR q.execution_receipt->>'before_sha256' IS DISTINCT FROM encode(sha256(convert_to(
      (CASE q.tool_name WHEN 'prepare_api_key_action' THEN q.preview->'before' WHEN 'request_create_key' THEN q.preview->'before'
       WHEN 'connect_market_tool' THEN q.preview->'market'
       ELSE jsonb_build_object('display_name',q.preview->'before_display_name') END)::text,'UTF8')),'hex')
  OR (CASE q.tool_name WHEN 'prepare_api_key_action' THEN
      q.preview->>'kind'='api_key'
      AND q.execution_receipt->'resource_id'=q.preview#>'{before,id}'
      AND q.execution_receipt->'action'=q.preview->'action'
      AND q.execution_receipt->'after'=(q.preview->'before' || CASE q.preview->>'action'
          WHEN 'disable' THEN '{"enabled":false,"status":2}'::jsonb
          WHEN 'delete' THEN '{"revoked":true,"status":2}'::jsonb END)
      AND EXISTS(SELECT FROM core_identity.credentials k JOIN core_identity.api_key_settings s ON s.credential_id=k.id
        WHERE k.id=(q.execution_receipt->>'resource_id')::bigint AND k.kind='api_key'
          AND k.user_id=q.actor_user_id AND k.owner_account_id=q.account_id
          AND s.name=q.preview#>>'{before,name}' AND s.group_name=q.preview#>>'{before,group}'
          AND extract(epoch FROM k.expires_at)::bigint=(q.preview#>>'{before,expires_at_unix}')::bigint
          AND ((q.preview->>'action'='disable' AND NOT s.enabled AND k.revoked_at IS NULL)
            OR (q.preview->>'action'='delete' AND k.revoked_at IS NOT NULL)))
    WHEN 'request_create_key' THEN
      q.preview->>'kind'='create_key'
      AND q.execution_receipt->'after'=jsonb_build_object(
        'id',q.execution_receipt->'resource_id','name',q.preview#>'{before,name}',
        'group',q.preview#>'{before,group}','one_time_reveal',TRUE,'expired_time',-1,
        'unlimited_quota',TRUE,'model_limits_enabled',FALSE,'cross_group_retry',FALSE,'card_id',q.id)
      AND EXISTS(SELECT FROM core_identity.credentials k
        JOIN core_identity.api_key_settings s ON s.credential_id=k.id
        JOIN core_assistant_tools.key_creation_cards p ON p.credential_id=k.id AND p.confirmation_id=q.id
        WHERE k.id=(q.execution_receipt->>'resource_id')::bigint AND k.kind='api_key'
          AND k.user_id=q.actor_user_id AND k.owner_account_id=q.account_id AND k.user_version=q.auth_version
          AND k.revoked_at IS NULL AND k.personal_billing_preference='subscription_first'
          AND extract(epoch FROM k.expires_at)::bigint=253402300799
          AND s.name=q.preview#>>'{before,name}' AND s.group_name=q.preview#>>'{before,group}'
          AND s.enabled AND s.one_time_reveal AND s.encrypted_secret IS NULL
          AND NOT s.model_limits_enabled AND NOT s.cross_group_retry
          AND cardinality(s.model_limits)=0 AND cardinality(s.allow_ips)=0 AND cardinality(s.auto_groups)=0
          AND EXISTS(SELECT FROM core_identity.key_funding_rules f WHERE f.credential_id=k.id
            AND f.position=0 AND f.payer_account_id=q.account_id)
          AND NOT EXISTS(SELECT FROM core_identity.key_funding_rules f WHERE f.credential_id=k.id AND f.position<>0))
    WHEN 'connect_market_tool' THEN
      core_assistant_tools.market_confirmation_bound(q.id)
    ELSE q.preview->>'kind'='display_name' AND q.preview->'user_id'=to_jsonb(q.actor_user_id)
      AND q.execution_receipt->'after'=jsonb_build_object('display_name',q.preview->'display_name')
      AND EXISTS(SELECT FROM core_identity.user_profiles p
        WHERE p.user_id=q.actor_user_id AND p.display_name=q.preview->>'display_name')
    END) IS NOT TRUE
  OR o.id IS NULL OR o.domain<>'registered_core' OR o.state<>'completed'
  OR o.tool_name<>q.tool_name OR o.tool_version<>q.tool_version
  OR o.turn_id<>q.turn_id OR o.arguments_sha256<>q.arguments_sha256
  OR o.result_sha256<>sha256(o.result_json) OR o.claim_key IS DISTINCT FROM 'core-'||o.id
  OR o.domain_receipt_id IS DISTINCT FROM 'core-prepared-'||o.id
  OR convert_from(o.result_json,'UTF8')::jsonb IS DISTINCT FROM jsonb_build_object(
    'ok',TRUE,'status','prepared','executed',FALSE,'confirmation_id',q.id,
    'browser_path',CASE WHEN convert_from(o.result_json,'UTF8')::jsonb->>'browser_path'
        ='/api/user/auth/assistant/confirmations/'||q.id
      THEN '/api/user/auth/assistant/confirmations/'||q.id
      ELSE '/api/user/assistant/confirmations/'||q.id END,
    'draft_sha256',encode(q.draft_sha256,'hex'),'expires_at_unix',q.expires_at)
 THEN
  RAISE EXCEPTION 'assistant execution audit or immutable proposal mismatch' USING ERRCODE='23514';
 END IF;
 RETURN NULL;
END $$;

CREATE TABLE core_meta.assistant_invitation_confirmation_upgrades (
 upgrade_id TEXT PRIMARY KEY CHECK(octet_length(upgrade_id) BETWEEN 16 AND 96),
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL,
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER assistant_invitation_confirmation_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.assistant_invitation_confirmation_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER assistant_invitation_confirmation_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.assistant_invitation_confirmation_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.assistant_invitation_confirmation_upgrades FROM PUBLIC;

-- Forward component85. Existing confirmation rows and original Go consent
-- semantics are retained. Startup never executes this asset.
ALTER TABLE core_assistant_tools.confirmations
 DROP CONSTRAINT confirmations_tool_name_check;
ALTER TABLE core_assistant_tools.confirmations
 ADD CONSTRAINT confirmations_tool_name_check CHECK(tool_name IN (
  'set_overview_greeting','create_site_issue','update_site_issue',
  'request_human_support','prepare_admin_site_policy_change',
  'prepare_api_key_action','prepare_user_action','request_create_key','connect_market_tool')),
 ADD COLUMN execution_receipt JSONB,
 ADD COLUMN execution_sha256 BYTEA,
 ADD COLUMN executed_at BIGINT,
 ADD CONSTRAINT assistant_confirmation_execution_shape CHECK ((
  CASE WHEN tool_name IN ('prepare_api_key_action','prepare_user_action','request_create_key','connect_market_tool') AND state='approved' THEN
   execution_receipt IS NOT NULL AND execution_sha256 IS NOT NULL AND executed_at IS NOT NULL
   AND jsonb_typeof(execution_receipt)='object'
   AND octet_length(execution_receipt::text)<=16384
   AND execution_receipt ?& ARRAY['version','operation_id','confirmation_id','action','resource_kind',
     'resource_id','account_id','before_sha256','after_sha256','after','executed','actor_user_id','audit_id']
   AND (execution_receipt-ARRAY['version','operation_id','confirmation_id','action','resource_kind',
     'resource_id','account_id','before_sha256','after_sha256','after','executed','actor_user_id','audit_id'])='{}'::jsonb
   AND execution_receipt->'version'='1'::jsonb AND execution_receipt->'executed'='true'::jsonb
   AND execution_receipt->>'operation_id'=operation_id AND execution_receipt->>'confirmation_id'=id
   AND execution_receipt->'actor_user_id'=to_jsonb(actor_user_id)
   AND execution_receipt->'account_id'=to_jsonb(account_id)
   AND execution_receipt->>'before_sha256' ~ '^[0-9a-f]{64}$'
   AND execution_receipt->>'after_sha256' ~ '^[0-9a-f]{64}$'
   AND jsonb_typeof(execution_receipt->'after')='object'
   AND jsonb_typeof(execution_receipt->'audit_id')='number'
   AND (execution_receipt->>'audit_id')::bigint>0
   AND (CASE WHEN tool_name='connect_market_tool' THEN
      jsonb_typeof(execution_receipt->'resource_id')='string'
      AND execution_receipt->>'resource_id' ~ '^[0-9a-f]{64}$'
     ELSE jsonb_typeof(execution_receipt->'resource_id')='number'
      AND (execution_receipt->>'resource_id')::bigint>0 END)
   AND execution_sha256=sha256(convert_to(execution_receipt::text,'UTF8'))
   AND executed_at=confirmed_at AND executed_at<expires_at AND nonce_sha256 IS NOT NULL
   AND ((tool_name='prepare_api_key_action'
      AND execution_receipt->>'action' IN ('disable','delete')
      AND execution_receipt->>'resource_kind'='api_key')
     OR (tool_name='request_create_key'
      AND execution_receipt->>'action'='create'
      AND execution_receipt->>'resource_kind'='api_key')
     OR (tool_name='connect_market_tool'
      AND execution_receipt->>'action'='connect'
      AND execution_receipt->>'resource_kind'='market_consent')
     OR (tool_name='prepare_user_action'
      AND execution_receipt->>'action'='change_display_name'
      AND execution_receipt->>'resource_kind'='user_profile'
      AND execution_receipt->'resource_id'=to_jsonb(actor_user_id)))
  ELSE execution_receipt IS NULL AND execution_sha256 IS NULL AND executed_at IS NULL END) IS TRUE
 );

CREATE OR REPLACE FUNCTION core_assistant_tools.guard_confirmation()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' OR OLD.state<>'pending' OR
  (to_jsonb(OLD)-ARRAY['nonce_sha256','state','confirmed_at','execution_receipt','execution_sha256','executed_at'])
   IS DISTINCT FROM
  (to_jsonb(NEW)-ARRAY['nonce_sha256','state','confirmed_at','execution_receipt','execution_sha256','executed_at'])
  OR (NEW.execution_receipt IS NOT NULL AND
     (NEW.tool_name NOT IN ('prepare_api_key_action','prepare_user_action','request_create_key','connect_market_tool') OR NEW.state<>'approved')) THEN
  RAISE EXCEPTION 'immutable assistant confirmation' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;

CREATE TABLE core_assistant_tools.key_creation_cards (
 confirmation_id TEXT PRIMARY KEY REFERENCES core_assistant_tools.confirmations(id),
 credential_id BIGINT NOT NULL UNIQUE REFERENCES core_identity.credentials(id),
 ciphertext BYTEA,
 created_at BIGINT NOT NULL CHECK(created_at>0),
 expires_at BIGINT NOT NULL CHECK(expires_at=created_at+600),
 nonce_sha256 BYTEA CHECK(nonce_sha256 IS NULL OR octet_length(nonce_sha256)=32),
 consumed_at BIGINT,
 CONSTRAINT assistant_key_card_secret_shape CHECK ((
  (consumed_at IS NULL AND ciphertext IS NOT NULL AND octet_length(ciphertext) BETWEEN 44 AND 512)
  OR (consumed_at IS NOT NULL AND ciphertext IS NULL AND nonce_sha256 IS NOT NULL
      AND consumed_at>=created_at AND consumed_at<expires_at)) IS TRUE)
);
CREATE FUNCTION core_assistant_tools.guard_key_creation_card()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' OR OLD.consumed_at IS NOT NULL
  OR (to_jsonb(OLD)-ARRAY['ciphertext','nonce_sha256','consumed_at'])
    IS DISTINCT FROM (to_jsonb(NEW)-ARRAY['ciphertext','nonce_sha256','consumed_at'])
  OR (NEW.consumed_at IS NULL AND (NEW.ciphertext IS DISTINCT FROM OLD.ciphertext
      OR OLD.expires_at<=extract(epoch FROM clock_timestamp())::bigint))
  OR (NEW.consumed_at IS NOT NULL AND (NEW.ciphertext IS NOT NULL
      OR NEW.nonce_sha256 IS DISTINCT FROM OLD.nonce_sha256)) THEN
  RAISE EXCEPTION 'immutable or consumed assistant key card' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER assistant_key_card_immutable BEFORE UPDATE OR DELETE
 ON core_assistant_tools.key_creation_cards FOR EACH ROW
 EXECUTE FUNCTION core_assistant_tools.guard_key_creation_card();
CREATE TRIGGER assistant_key_card_no_truncate BEFORE TRUNCATE
 ON core_assistant_tools.key_creation_cards FOR EACH STATEMENT
 EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE FUNCTION core_assistant_tools.check_key_creation_card()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE p core_assistant_tools.key_creation_cards%ROWTYPE;
BEGIN
 SELECT * INTO STRICT p FROM core_assistant_tools.key_creation_cards WHERE confirmation_id=NEW.confirmation_id;
 IF NOT EXISTS (
  SELECT FROM core_assistant_tools.confirmations q
  JOIN core_identity.credentials k ON k.id=p.credential_id
  JOIN core_identity.api_key_settings s ON s.credential_id=k.id
  WHERE q.id=p.confirmation_id AND q.tool_name='request_create_key' AND q.state='approved'
   AND q.execution_receipt->>'action'='create'
   AND q.execution_receipt->'resource_id'=to_jsonb(k.id)
   AND q.execution_receipt#>>'{after,card_id}'=q.id
   AND k.kind='api_key' AND k.user_id=q.actor_user_id AND k.owner_account_id=q.account_id
   AND k.user_version=q.auth_version AND k.revoked_at IS NULL
   AND s.one_time_reveal AND s.encrypted_secret IS NULL
   AND p.created_at<=q.executed_at
 ) THEN
  RAISE EXCEPTION 'assistant key card requires exact browser execution receipt' USING ERRCODE='23514';
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER assistant_key_card_execution_binding
 AFTER INSERT OR UPDATE ON core_assistant_tools.key_creation_cards
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
 EXECUTE FUNCTION core_assistant_tools.check_key_creation_card();
REVOKE ALL ON core_assistant_tools.key_creation_cards FROM PUBLIC;
REVOKE ALL ON FUNCTION core_assistant_tools.guard_key_creation_card() FROM PUBLIC;
REVOKE ALL ON FUNCTION core_assistant_tools.check_key_creation_card() FROM PUBLIC;


-- Both directions bind the original browser action to the native immutable
-- credit operation and consent. Ordinary nonassistant credit rows are unchanged.
CREATE FUNCTION core_assistant_tools.market_confirmation_bound(confirmation_id TEXT)
RETURNS BOOLEAN LANGUAGE SQL STABLE SET search_path=pg_catalog AS $$
 SELECT EXISTS(
  SELECT FROM core_assistant_tools.confirmations q
  JOIN core_assistant.agent_turns turn ON turn.id=q.turn_id
  JOIN core_tool_market.credit_consents c ON c.id=q.execution_receipt->>'resource_id'
  JOIN core_tool_market.credit_quotes quote ON quote.id=c.quote_id
  JOIN core_tool_market.credit_access_operations op
    ON op.actor_user_id=q.actor_user_id AND op.operation_id=q.operation_id
  WHERE q.id=confirmation_id AND q.tool_name='connect_market_tool' AND q.state='approved'
   AND q.preview->>'kind'='market_tool'
   AND (q.preview-ARRAY['kind','market'])='{}'::jsonb
   AND q.execution_receipt->>'resource_kind'='market_consent'
   AND q.execution_receipt->>'action'='connect'
   AND jsonb_typeof(q.execution_receipt#>'{after,browser_limits}')='object'
   AND ((q.execution_receipt#>'{after,browser_limits}')-ARRAY[
     'max_price_quota','max_total_quota','max_calls','lifetime_seconds'])='{}'::jsonb
   AND ((q.execution_receipt->'after')-ARRAY['consent','browser_limits','request_sha256'])='{}'::jsonb
   AND q.execution_receipt#>>'{after,request_sha256}' ~ '^[0-9a-f]{64}$'
   AND c.actor_user_id=q.actor_user_id AND c.payer_account_id=q.account_id
   -- Only immutable creation fields belong to the historical browser proof.
   -- Revocation and subsequent use are checked by the live call authority.
   AND c.created_at=q.executed_at
   AND c.request=jsonb_build_object(
     'owner',jsonb_build_object('user',q.actor_user_id::text,'account',q.actor_user_id::text),
     'operation_id',q.operation_id,'client_id','builtin-assistant',
     'tool_id',q.preview#>'{market,preview,request,tool_id}',
     'version_id',q.preview#>'{market,preview,request,version_id}',
     'max_price_quota',q.execution_receipt#>'{after,browser_limits,max_price_quota}',
     'max_total_quota',q.execution_receipt#>'{after,browser_limits,max_total_quota}',
     'max_calls',q.execution_receipt#>'{after,browser_limits,max_calls}',
     'expires_at',q.executed_at+
       (q.execution_receipt#>>'{after,browser_limits,lifetime_seconds}')::bigint)
   AND (c.request->>'max_price_quota')::bigint BETWEEN quote.ceiling_credits AND 9007199254740991
   AND (c.request->>'max_total_quota')::bigint BETWEEN (c.request->>'max_price_quota')::bigint AND 9007199254740991
   AND (c.request->>'max_calls')::bigint BETWEEN 1 AND 100
   AND (q.execution_receipt#>>'{after,browser_limits,lifetime_seconds}')::bigint BETWEEN 60 AND 86400
   AND (c.request->>'expires_at')::bigint <= (quote.snapshot->>'expires')::bigint
   AND c.quote_id=q.preview#>>'{market,preview,quote_id}'
   AND quote.snapshot->'service_id'=q.preview#>'{market,preview,service_id}'
   AND quote.snapshot->'tool_id'=c.request->'tool_id'
   AND quote.snapshot->'version_id'=c.request->'version_id'
   AND quote.snapshot->'price'=q.preview#>'{market,price}'
   AND q.execution_receipt#>'{after,consent}'=(c.request || jsonb_build_object(
     'quote_id',c.quote_id,'frozen_price_quota',quote.ceiling_credits,
     'rules_hash',quote.snapshot->'pricing_digest','id',c.id,'created_at',c.created_at,
     'revoked_at',0,'reserved_quota',0,'spent_quota',0,'reserved_calls',0,'successful_calls',0))
   AND op.kind='consent-create' AND op.target_id=c.id AND op.created_at=q.executed_at
   AND op.details=jsonb_build_object('assistant_confirmation_id',q.id,
      'assistant_tool_operation_id',q.operation_id,
      'policy_revision',turn.host_policy#>'{market,revision}')
   AND op.request_hash=q.execution_receipt#>>'{after,request_sha256}'
   -- Exact field order is ConsentRequest/Owner's existing serde contract.
   AND op.request_hash=encode(sha256(convert_to(
     '{"owner":{"user":'||to_json(c.request#>>'{owner,user}')::text||
     ',"account":'||to_json(c.request#>>'{owner,account}')::text||
     '},"operation_id":'||to_json(c.request->>'operation_id')::text||
     ',"client_id":'||to_json(c.request->>'client_id')::text||
     ',"tool_id":'||to_json(c.request->>'tool_id')::text||
     ',"version_id":'||to_json(c.request->>'version_id')::text||
     ',"max_price_quota":'||(c.request->'max_price_quota')::text||
     ',"max_total_quota":'||(c.request->'max_total_quota')::text||
     ',"max_calls":'||(c.request->'max_calls')::text||
     ',"expires_at":'||(c.request->'expires_at')::text||'}','UTF8')),'hex')
 )
$$;
CREATE FUNCTION core_assistant_tools.check_market_confirmation_reverse()
RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE confirmation_id TEXT;
BEGIN
 IF TG_TABLE_NAME='credit_consents' THEN
  IF NEW.request->>'client_id' IS DISTINCT FROM 'builtin-assistant' THEN RETURN NULL; END IF;
  SELECT q.id INTO confirmation_id FROM core_assistant_tools.confirmations q
   WHERE q.tool_name='connect_market_tool' AND q.execution_receipt->>'resource_id'=NEW.id;
 ELSIF TG_TABLE_NAME='credit_access_operations' THEN
  IF NOT COALESCE(NEW.details ? 'assistant_confirmation_id',FALSE) THEN RETURN NULL; END IF;
  confirmation_id:=NEW.details->>'assistant_confirmation_id';
  -- Bind this INSERT's primary key before validating the canonical operation.
  -- The complete bound() check below now examines this exact immutable row,
  -- including its kind, target, request hash, timestamp and details.
  IF NOT EXISTS(SELECT FROM core_assistant_tools.confirmations q
    WHERE q.id=confirmation_id AND NEW.actor_user_id=q.actor_user_id
      AND NEW.operation_id=q.operation_id) THEN
   RAISE EXCEPTION 'assistant market operation must belong to its original browser action' USING ERRCODE='23514';
  END IF;
 ELSE
  RAISE EXCEPTION 'invalid assistant market trigger target' USING ERRCODE='23514';
 END IF;
 IF confirmation_id IS NULL OR
    core_assistant_tools.market_confirmation_bound(confirmation_id) IS NOT TRUE THEN
  RAISE EXCEPTION 'assistant market consent requires exact original browser execution' USING ERRCODE='23514';
 END IF;
 -- This deferred check runs only for the creation INSERTs. Later legitimate
 -- reserve/settle/revoke operations do not rewrite the original approval.
 IF NOT EXISTS(SELECT FROM core_tool_market.credit_consents c
   JOIN core_assistant_tools.confirmations q ON q.execution_receipt->>'resource_id'=c.id
   WHERE q.id=confirmation_id AND c.revoked_at=0 AND c.reserved_calls=0
    AND c.successful_calls=0 AND c.reserved_credits=0 AND c.spent_credits=0) THEN
  RAISE EXCEPTION 'assistant market approval must create an unused consent' USING ERRCODE='23514';
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER assistant_market_consent_confirmation_binding
 AFTER INSERT ON core_tool_market.credit_consents
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
 EXECUTE FUNCTION core_assistant_tools.check_market_confirmation_reverse();
CREATE CONSTRAINT TRIGGER assistant_market_access_confirmation_binding
 AFTER INSERT ON core_tool_market.credit_access_operations
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
 EXECUTE FUNCTION core_assistant_tools.check_market_confirmation_reverse();
REVOKE ALL ON FUNCTION core_assistant_tools.market_confirmation_bound(TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION core_assistant_tools.check_market_confirmation_reverse() FROM PUBLIC;

CREATE FUNCTION core_assistant_tools.check_action_execution()
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
    'browser_path','/api/user/assistant/confirmations/'||q.id,
    'draft_sha256',encode(q.draft_sha256,'hex'),'expires_at_unix',q.expires_at)
 THEN
  RAISE EXCEPTION 'assistant execution audit or immutable proposal mismatch' USING ERRCODE='23514';
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER assistant_confirmation_execution_binding
 AFTER INSERT OR UPDATE ON core_assistant_tools.confirmations
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
 EXECUTE FUNCTION core_assistant_tools.check_action_execution();
CREATE UNIQUE INDEX assistant_one_confirmed_action_audit
 ON core_identity.audit((target->>'operation_id'))
 WHERE action='assistant.confirmed_action';
REVOKE ALL ON FUNCTION core_assistant_tools.check_action_execution() FROM PUBLIC;

CREATE TABLE core_meta.assistant_confirmed_actions_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER assistant_confirmed_actions_upgrades_immutable BEFORE UPDATE OR DELETE
 ON core_meta.assistant_confirmed_actions_upgrades FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER assistant_confirmed_actions_upgrades_no_truncate BEFORE TRUNCATE
 ON core_meta.assistant_confirmed_actions_upgrades FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.assistant_confirmed_actions_upgrades FROM PUBLIC;

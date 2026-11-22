-- Component83: the typed Agent gift result and its canonical ledger effect
-- commit together. Existing PersonalText streaming authority is unchanged.
-- No business table, wallet, grant, predecessor row, sequence or ACL is rewritten.

CREATE FUNCTION core_gift.check_agent_gift_binding() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE o RECORD; d RECORD; t RECORD; r RECORD; q RECORD; g RECORD;
        output JSONB; arguments JSONB;
BEGIN
 IF TG_TABLE_SCHEMA='core_assistant' THEN
  IF NEW.tool_name<>'prepare_new_user_gift' THEN RETURN NULL; END IF;
  SELECT * INTO STRICT o FROM core_assistant.agent_tool_operations WHERE id=NEW.id;
  IF o.domain<>'registered_core' THEN
   RAISE EXCEPTION 'agent gift requires the fixed Core tool' USING ERRCODE='23514';
  END IF;
  IF o.state<>'completed' THEN RETURN NULL; END IF;
  output:=convert_from(o.result_json,'UTF8')::jsonb;
  IF jsonb_typeof(output) IS DISTINCT FROM 'object' THEN
   RAISE EXCEPTION 'agent gift result must be typed' USING ERRCODE='23514';
  END IF;
  IF output->'ok'='false'::jsonb AND coalesce(output->'issued','false'::jsonb)='false'::jsonb
     AND output-ARRAY['ok','issued','error','reason']='{}'::jsonb
     AND output->>'error' IN ('gift_not_issued','tool_arguments_or_scope_denied')
     AND o.domain_receipt_id IN ('core-gift-denied-'||o.id,'core-policy-'||o.id) THEN
   RETURN NULL;
  END IF;
  IF output->'ok' IS DISTINCT FROM 'true'::jsonb OR output->'issued' IS DISTINCT FROM 'true'::jsonb THEN
   RAISE EXCEPTION 'agent gift cannot report unissued success' USING ERRCODE='23514';
  END IF;
  SELECT * INTO d FROM core_gift.decisions WHERE id=output->>'gift_id';
  IF NOT FOUND THEN
   RAISE EXCEPTION 'agent gift result lacks its decision' USING ERRCODE='23514';
  END IF;
 ELSE
  SELECT * INTO STRICT d FROM core_gift.decisions WHERE id=NEW.id;
  IF NOT EXISTS(SELECT FROM core_assistant.agent_rounds WHERE model_request_id=d.model_request_id) THEN
   RETURN NULL;
  END IF;
  SELECT * INTO o FROM core_assistant.agent_tool_operations
   WHERE tool_name='prepare_new_user_gift' AND domain='registered_core' AND state='completed'
     AND domain_receipt_id='core-gift-'||d.id;
  IF NOT FOUND THEN
   RAISE EXCEPTION 'agent gift decision lacks its completed tool' USING ERRCODE='23514';
  END IF;
  output:=convert_from(o.result_json,'UTF8')::jsonb;
 END IF;
 SELECT * INTO STRICT r FROM core_assistant.agent_rounds WHERE id=o.round_id;
 SELECT * INTO STRICT t FROM core_assistant.agent_turns WHERE id=o.turn_id;
 SELECT * INTO STRICT q FROM core_charging.requests WHERE id=r.model_request_id;
 SELECT * INTO STRICT g FROM core_assistant.grants WHERE id=q.assistant_grant_id;
 arguments:=convert_from(o.arguments_json,'UTF8')::jsonb;
 IF o.state<>'completed' OR o.domain<>'registered_core' OR o.tool_name<>'prepare_new_user_gift'
    OR o.tool_version<>'core-agent-gift-v1'
    OR o.schema_sha256<>decode('f8c557232f18438cd5488e7d6570a54109898ebe81a4e7a2dc175eceefdeffa1','hex')
    OR o.arguments_sha256<>sha256(o.arguments_json) OR o.result_sha256<>sha256(o.result_json)
    OR o.claim_key IS DISTINCT FROM 'core-'||o.id
    OR o.domain_receipt_id IS DISTINCT FROM 'core-gift-'||d.id
    OR o.resource_kind<>'' OR o.resource_id<>'' OR o.resource_version<>'' OR o.scope_grant_id<>''
    OR o.resource_schema_sha256 IS NOT NULL OR o.resource_price_sha256 IS NOT NULL
    OR jsonb_typeof(arguments) IS DISTINCT FROM 'object'
    OR arguments-ARRAY['amount_credits','reason']<>'{}'::jsonb
    OR jsonb_typeof(arguments->'amount_credits') IS DISTINCT FROM 'number'
    OR (arguments->>'amount_credits')::bigint IS DISTINCT FROM d.amount_credits
    OR d.amount_credits NOT BETWEEN 1 AND 9007199254740991
    OR (arguments ? 'reason' AND (jsonb_typeof(arguments->'reason') IS DISTINCT FROM 'string'
       OR length(arguments->>'reason')>240))
    OR d.claimed_at IS NULL OR d.ledger_journal_id IS NULL
    OR output IS DISTINCT FROM jsonb_build_object('ok',TRUE,'issued',TRUE,'gift_id',d.id,
       'credit_amount',d.amount_credits,'ledger_transaction_id',d.ledger_journal_id,'claimed_at_unix',d.claimed_at)
    OR r.turn_id<>t.id OR r.state<>'settled' OR q.state<>'settled' OR q.outcome IS DISTINCT FROM 'completed'
    OR r.provider_response_sha256 IS NULL OR r.provider_response_sha256<>sha256(r.provider_response_json)
    OR q.id<>d.model_request_id OR q.actor_user_id<>d.user_id OR q.owner_account_id<>t.owner_account_id
    OR t.owner_user_id<>d.user_id OR d.conversation_id<>t.conversation_id
    OR g.conversation_id<>t.conversation_id OR g.session_id<>t.session_id OR g.auth_version<>t.auth_version
    OR g.policy IS DISTINCT FROM t.host_policy->'model'
    OR g.revoked OR g.expires_at<=extract(epoch FROM clock_timestamp())::bigint
    OR t.expires_at<=extract(epoch FROM clock_timestamp())::bigint
    OR t.phase<>'tools_pending'
    OR (g.policy->>'gift_tools')::boolean IS DISTINCT FROM TRUE
    OR ((t.host_policy->'allowed_tools') ? 'prepare_new_user_gift') IS NOT TRUE
    OR NOT EXISTS(SELECT FROM core_assistant.operations original WHERE original.request_id=q.id AND original.grant_id=g.id)
    OR NOT EXISTS(SELECT FROM core_identity.credentials k JOIN core_identity.users u ON u.id=k.user_id
      JOIN core_identity.accounts a ON a.id=u.personal_account_id
      LEFT JOIN core_identity.web_session_credentials m ON m.credential_id=k.id
      LEFT JOIN core_identity.web_sessions s ON s.sid=m.sid
      WHERE k.id=t.session_id AND k.user_id=t.owner_user_id AND k.owner_account_id=t.owner_account_id
        AND k.kind='session' AND k.revoked_at IS NULL AND k.expires_at>clock_timestamp()
        AND k.user_version=t.auth_version AND u.auth_version=t.auth_version
        AND u.active AND a.active AND a.kind='personal' AND u.personal_account_id=t.owner_account_id
        AND (m.sid IS NULL OR (s.sid=t.source_web_session_sid AND s.current_credential_id=k.id
          AND s.user_id=u.id AND s.user_version=u.auth_version AND s.revoked_at IS NULL
          AND s.expires_at>clock_timestamp() AND (NOT s.weekly_logout OR s.created_at>clock_timestamp()-interval '7 days'))))
    OR NOT EXISTS(SELECT FROM core_gift.policy p WHERE p.singleton AND p.enabled AND d.amount_credits<p.exclusive_cap)
    OR NOT EXISTS(SELECT FROM jsonb_array_elements(convert_from(r.provider_response_json,'UTF8')::jsonb #> '{choices,0,message,tool_calls}') call
      WHERE call->>'id'=o.provider_call_id AND call#>>'{function,name}'=o.tool_name
        AND convert_to(call#>>'{function,arguments}','UTF8')=o.arguments_json)
    OR (q.payer_account_id=t.owner_account_id
      OR q.payer_account_id=(g.policy->'sponsor'->>'treasury_account_id')::bigint
      OR (q.payer_account_id=(g.policy->'service_funding'->>'payer_account_id')::bigint AND EXISTS(
        SELECT FROM core_identity.users payer JOIN core_identity.accounts a ON a.id=payer.personal_account_id
        WHERE payer.id=(g.policy->'service_funding'->>'payer_user_id')::bigint AND a.id=q.payer_account_id AND a.kind='personal'))) IS NOT TRUE
    OR NOT EXISTS(SELECT FROM core_billing.ledger_journals j
      JOIN core_billing.ledger_accounts a ON a.id=j.payee_id
      WHERE j.id=d.ledger_journal_id AND j.kind='gift' AND j.scope='core-assistant-gift'
        AND j.operation_key=d.id AND j.actor_user_id=d.user_id AND a.owner_account_id=t.owner_account_id
        AND j.amount_units=d.amount_credits AND j.payment_reference IS NULL) THEN
  RAISE EXCEPTION 'agent gift model, authority, tool, or ledger binding mismatch' USING ERRCODE='23514';
 END IF;
 RETURN NULL;
END $$;

-- The trigger makes this exact receipt shape equivalent to issued=true.
-- Negative/status results have another prefix and do not consume this key.
CREATE UNIQUE INDEX agent_gift_one_issued_receipt
 ON core_assistant.agent_tool_operations(domain_receipt_id)
 WHERE tool_name='prepare_new_user_gift' AND domain='registered_core' AND state='completed'
   AND domain_receipt_id ~ '^core-gift-[0-9a-f]{64}$';
CREATE CONSTRAINT TRIGGER agent_gift_tool_binding
 AFTER INSERT OR UPDATE ON core_assistant.agent_tool_operations
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_gift.check_agent_gift_binding();
CREATE CONSTRAINT TRIGGER agent_gift_decision_binding
 AFTER INSERT OR UPDATE ON core_gift.decisions
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_gift.check_agent_gift_binding();
REVOKE ALL ON FUNCTION core_gift.check_agent_gift_binding() FROM PUBLIC;

CREATE TABLE core_meta.assistant_agent_gift_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER assistant_agent_gift_upgrades_immutable BEFORE UPDATE OR DELETE
 ON core_meta.assistant_agent_gift_upgrades FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER assistant_agent_gift_upgrades_no_truncate BEFORE TRUNCATE
 ON core_meta.assistant_agent_gift_upgrades FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.assistant_agent_gift_upgrades FROM PUBLIC;

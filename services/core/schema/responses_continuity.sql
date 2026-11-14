-- Native HTTP provider context handles only. Never a response body store.
CREATE TABLE core_model.responses_continuations (
 request_id TEXT PRIMARY KEY REFERENCES core_charging.requests(id),
 handle TEXT NOT NULL UNIQUE CHECK(handle ~ '^resp_lmm_[0-9a-f]{64}$'),
 provider_response_id TEXT NOT NULL CHECK(length(provider_response_id) BETWEEN 1 AND 512
   AND provider_response_id !~ '[[:cntrl:]]'),
 parent_request_id TEXT REFERENCES core_model.responses_continuations(request_id),
 public_model TEXT NOT NULL CHECK(length(public_model) BETWEEN 1 AND 256),
 policy_sha256 BYTEA NOT NULL CHECK(octet_length(policy_sha256)=32),
 binding JSONB NOT NULL CHECK(jsonb_typeof(binding)='object' AND pg_column_size(binding)<=8192),
 terminal_sequence BIGINT NOT NULL,
 active BOOLEAN NOT NULL DEFAULT FALSE,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 activated_at TIMESTAMPTZ,
 CHECK(active=(activated_at IS NOT NULL)),
 FOREIGN KEY(request_id,terminal_sequence)
   REFERENCES core_model.output_checkpoints(request_id,sequence)
);
-- Recovery skips activated history while preserving request_id cursor order.
CREATE INDEX responses_continuations_inactive
 ON core_model.responses_continuations(request_id) WHERE NOT active;
CREATE FUNCTION core_model.guard_continuation() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP<>'UPDATE' THEN
   RAISE EXCEPTION 'immutable continuation' USING ERRCODE='23514';
 END IF;
 IF (to_jsonb(NEW)-'active'-'activated_at') IS DISTINCT FROM
    (to_jsonb(OLD)-'active'-'activated_at') OR OLD.active OR NOT NEW.active THEN
   RAISE EXCEPTION 'immutable continuation binding' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER responses_continuations_immutable BEFORE UPDATE OR DELETE
 ON core_model.responses_continuations FOR EACH ROW EXECUTE FUNCTION core_model.guard_continuation();
CREATE TRIGGER responses_continuations_no_truncate BEFORE TRUNCATE
 ON core_model.responses_continuations FOR EACH STATEMENT EXECUTE FUNCTION core_charging.no_delete();

CREATE FUNCTION core_model.record_continuation(p_request_id TEXT,p_worker BYTEA,
 p_provider_response_id TEXT,p_parent_request_id TEXT,p_public_model TEXT,
 p_policy_sha256 BYTEA,p_terminal_sequence BIGINT) RETURNS VOID
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE r core_charging.requests%ROWTYPE;
 old core_model.responses_continuations%ROWTYPE;
 b JSONB;
 proof JSONB;
 h TEXT;
BEGIN
 IF current_setting('transaction_isolation')<>'serializable' THEN
   RAISE EXCEPTION 'continuation requires serializable' USING ERRCODE='25001';
 END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('core_charging:'||p_request_id,0));
 SELECT * INTO STRICT r FROM core_charging.requests WHERE id=p_request_id FOR UPDATE;
 IF p_worker IS NULL OR octet_length(p_worker)<>32 OR r.worker_digest<>sha256(p_worker)
   OR r.key_id IS NULL THEN
   RAISE EXCEPTION 'wrong continuation worker' USING ERRCODE='42501';
 END IF;
 SELECT detail INTO proof FROM core_charging.usage_events
   WHERE request_id=p_request_id AND event_id='reservation-contract:v1';
 b:=proof->'binding';
 IF proof->>'mode' IS DISTINCT FROM 'context_max' OR b IS NULL
   OR b->>'protocol' IS DISTINCT FROM 'responses' OR p_provider_response_id IS NULL
   OR p_public_model IS NULL OR p_policy_sha256 IS NULL OR p_terminal_sequence IS NULL
   OR NOT EXISTS(SELECT 1 FROM core_model.output_checkpoints
     WHERE request_id=p_request_id AND sequence=p_terminal_sequence AND terminal_observed) THEN
   RAISE EXCEPTION 'missing continuation terminal contract' USING ERRCODE='23514';
 END IF;
 h:='resp_lmm_'||encode(sha256(convert_to(p_request_id,'UTF8')),'hex');
 SELECT * INTO old FROM core_model.responses_continuations WHERE request_id=p_request_id;
 IF FOUND THEN
   IF (old.provider_response_id,old.parent_request_id,old.public_model,old.policy_sha256,
       old.binding,old.terminal_sequence) IS DISTINCT FROM
      (p_provider_response_id,p_parent_request_id,p_public_model,p_policy_sha256,b,p_terminal_sequence) THEN
     RAISE EXCEPTION 'conflicting continuation receipt' USING ERRCODE='23505';
   END IF;
   RETURN;
 END IF;
 IF r.state<>'streaming' THEN
   RAISE EXCEPTION 'continuation worker is not live' USING ERRCODE='42501';
 END IF;
 IF p_parent_request_id IS NOT NULL AND NOT EXISTS(
   SELECT 1 FROM core_model.responses_continuations c JOIN core_charging.requests pr ON pr.id=c.request_id
   WHERE c.request_id=p_parent_request_id AND c.active AND pr.state='settled'
     AND pr.key_id=r.key_id AND pr.actor_user_id=r.actor_user_id
     AND pr.owner_account_id=r.owner_account_id) THEN
   RAISE EXCEPTION 'continuation parent denied' USING ERRCODE='42501';
 END IF;
 INSERT INTO core_model.responses_continuations(request_id,handle,provider_response_id,
   parent_request_id,public_model,policy_sha256,binding,terminal_sequence)
 VALUES(p_request_id,h,p_provider_response_id,p_parent_request_id,p_public_model,
   p_policy_sha256,b,p_terminal_sequence);
END $$;

CREATE FUNCTION core_model.activate_continuation(p_request_id TEXT) RETURNS BOOLEAN
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE c core_model.responses_continuations%ROWTYPE;
BEGIN
 SELECT * INTO c FROM core_model.responses_continuations WHERE request_id=p_request_id FOR UPDATE;
 IF NOT FOUND THEN RETURN FALSE; END IF;
 IF c.active THEN RETURN TRUE; END IF;
 IF NOT EXISTS(SELECT 1 FROM core_charging.requests r
   JOIN core_model.output_checkpoints e ON e.request_id=r.id AND e.sequence=c.terminal_sequence
   WHERE r.id=p_request_id AND r.state='settled' AND r.outcome='completed' AND e.terminal_observed) THEN
   RETURN FALSE;
 END IF;
 UPDATE core_model.responses_continuations SET active=TRUE,activated_at=clock_timestamp()
   WHERE request_id=p_request_id;
 RETURN TRUE;
END $$;

CREATE FUNCTION core_model.resolve_continuation(p_handle TEXT,p_secret TEXT) RETURNS JSONB
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE c core_model.responses_continuations%ROWTYPE;
 r core_charging.requests%ROWTYPE;
BEGIN
 SELECT t.* INTO c FROM core_model.responses_continuations t
   JOIN core_charging.requests q ON q.id=t.request_id
   JOIN core_identity.credentials k ON k.id=q.key_id AND k.user_id=q.actor_user_id
     AND k.owner_account_id=q.owner_account_id
   WHERE t.handle=p_handle AND k.digest=sha256(convert_to(p_secret,'UTF8'))
     AND k.kind='api_key' AND k.revoked_at IS NULL AND k.expires_at>clock_timestamp();
 IF NOT FOUND THEN RETURN NULL; END IF;
 IF NOT core_model.activate_continuation(c.request_id) THEN RETURN NULL; END IF;
 SELECT * INTO STRICT r FROM core_charging.requests WHERE id=c.request_id;
 RETURN jsonb_build_object('request_id',c.request_id,'provider_response_id',c.provider_response_id,
   'public_model',c.public_model,'canonical_model',r.price->>'model','group_name',r.price->>'group',
   'binding',c.binding,'policy_sha256',encode(c.policy_sha256,'hex'));
END $$;

CREATE FUNCTION core_model.activate_continuations_after(p_limit INTEGER,p_after TEXT)
RETURNS TABLE(request_id TEXT,activated BOOLEAN) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE id TEXT;
BEGIN
 IF p_limit IS NULL OR p_limit NOT BETWEEN 1 AND 1000 THEN
   RAISE EXCEPTION 'invalid continuation recovery page' USING ERRCODE='23514';
 END IF;
 FOR id IN SELECT c.request_id FROM core_model.responses_continuations c
   JOIN core_charging.requests r ON r.id=c.request_id
   WHERE NOT c.active AND r.state='settled' AND (p_after IS NULL OR c.request_id>p_after)
   ORDER BY c.request_id LIMIT p_limit LOOP
   activated:=core_model.activate_continuation(id); request_id:=id; RETURN NEXT;
 END LOOP;
END $$;

REVOKE ALL ON core_model.responses_continuations FROM PUBLIC;
REVOKE ALL ON FUNCTION core_model.guard_continuation(),
 core_model.record_continuation(text,bytea,text,text,text,bytea,bigint),
 core_model.activate_continuation(text),core_model.resolve_continuation(text,text),
 core_model.activate_continuations_after(integer,text) FROM PUBLIC;

CREATE TABLE core_meta.responses_continuity_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER responses_continuity_upgrades_immutable BEFORE UPDATE OR DELETE
 ON core_meta.responses_continuity_upgrades FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER responses_continuity_upgrades_no_truncate BEFORE TRUNCATE
 ON core_meta.responses_continuity_upgrades FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.responses_continuity_upgrades FROM PUBLIC;

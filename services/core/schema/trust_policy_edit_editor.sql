-- Third part of the single trust-policy-edit component. Never grant table writes
-- or the legacy apply_policy_edit primitive to the runtime role.
CREATE TABLE core_trust_pricing.policy_editors (
    editor_id text PRIMARY KEY CHECK (length(editor_id) BETWEEN 1 AND 80),
    actor_user_id bigint NOT NULL CHECK (actor_user_id > 0),
    version bigint NOT NULL CHECK (version > 0),
    credential_id bytea NOT NULL UNIQUE CHECK (octet_length(credential_id) BETWEEN 1 AND 4096),
    passkey jsonb NOT NULL CHECK (jsonb_typeof(passkey->'cred') = 'object'),
    rp_id text NOT NULL,
    origin text NOT NULL,
    revoked boolean NOT NULL DEFAULT false
);
CREATE TABLE core_trust_pricing.policy_editor_enrollments (
    enrollment_id bytea PRIMARY KEY CHECK (octet_length(enrollment_id)=32),
    editor_id text NOT NULL,
    actor_user_id bigint NOT NULL CHECK (actor_user_id>0),
    expected_version bigint NOT NULL CHECK (expected_version>=0),
    rp_id text NOT NULL,
    origin text NOT NULL,
    registration jsonb NOT NULL,
    candidate jsonb,
    candidate_digest bytea CHECK (octet_length(candidate_digest)=32),
    expires_at timestamptz NOT NULL,
    approved_at timestamptz,
    CHECK ((candidate IS NULL) = (candidate_digest IS NULL))
);
CREATE TABLE core_trust_pricing.policy_editor_challenges (
    challenge_id bytea PRIMARY KEY CHECK (octet_length(challenge_id)=32),
    preview_digest bytea NOT NULL UNIQUE REFERENCES core_trust_pricing.policy_edit_previews(token_digest) ON DELETE CASCADE,
    editor_id text NOT NULL REFERENCES core_trust_pricing.policy_editors(editor_id),
    editor_version bigint NOT NULL,
    authentication jsonb NOT NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz
);
CREATE TABLE core_trust_pricing.policy_editor_confirmations (
    revision bigint PRIMARY KEY REFERENCES core_trust_pricing.policy_edit_audit(revision),
    editor_id text NOT NULL,
    editor_version bigint NOT NULL,
    credential_id bytea NOT NULL,
    challenge_id bytea NOT NULL UNIQUE REFERENCES core_trust_pricing.policy_editor_challenges(challenge_id),
    assertion_digest bytea NOT NULL CHECK (octet_length(assertion_digest)=32),
    verified_result jsonb NOT NULL
);
CREATE TRIGGER policy_editor_confirmations_append_only BEFORE UPDATE OR DELETE OR TRUNCATE
ON core_trust_pricing.policy_editor_confirmations FOR EACH STATEMENT
EXECUTE FUNCTION core_trust_pricing.prevent_policy_edit_audit_mutation();
REVOKE ALL ON core_trust_pricing.policy_editors,core_trust_pricing.policy_editor_enrollments,
core_trust_pricing.policy_editor_challenges,core_trust_pricing.policy_editor_confirmations FROM PUBLIC;

-- Public-key material only; a root actor/session alone cannot mutate it.
CREATE FUNCTION core_trust_pricing.policy_editor_context(p_editor text,p_actor bigint)
RETURNS jsonb LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT to_jsonb(e) - 'credential_id' FROM core_trust_pricing.policy_editors e
 WHERE editor_id=p_editor AND actor_user_id=p_actor AND NOT revoked
$$;
REVOKE ALL ON FUNCTION core_trust_pricing.policy_editor_context(text,bigint) FROM PUBLIC;

CREATE FUNCTION core_trust_pricing.authorized_policy_challenge(p jsonb,proof bytea)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE e core_trust_pricing.policy_editors; v core_trust_pricing.policy_edit_previews;
BEGIN
 IF NOT core_trust_pricing.policy_edit_verify('challenge',p,proof) THEN
  RAISE EXCEPTION 'invalid editor proof' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock_shared(741803219);
 SELECT * INTO e FROM core_trust_pricing.policy_editors WHERE editor_id=p->>'editor' FOR SHARE;
 SELECT * INTO v FROM core_trust_pricing.policy_edit_previews WHERE token_digest=decode(p->>'token','hex') FOR UPDATE;
 IF e.editor_id IS NULL OR e.revoked OR e.version IS DISTINCT FROM (p->>'version')::bigint
 OR e.actor_user_id IS DISTINCT FROM (p->>'actor')::bigint
 OR v.actor_user_id IS DISTINCT FROM e.actor_user_id
 OR v.session_credential_id IS DISTINCT FROM (p->>'session')::bigint
 OR v.transition_digest IS DISTINCT FROM decode(p->>'digest','hex')
 OR v.consumed_at IS NOT NULL OR v.expires_at<=clock_timestamp()
 OR jsonb_typeof(p->'authentication') IS DISTINCT FROM 'object' THEN
  RAISE EXCEPTION 'invalid editor challenge' USING ERRCODE='22023'; END IF;
 INSERT INTO core_trust_pricing.policy_editor_challenges VALUES
 (decode(p->>'challenge','hex'),v.token_digest,e.editor_id,e.version,p->'authentication',
 least(v.expires_at,clock_timestamp()+interval '5 minutes'),NULL);
END $$;
REVOKE ALL ON FUNCTION core_trust_pricing.authorized_policy_challenge(jsonb,bytea) FROM PUBLIC;

CREATE FUNCTION core_trust_pricing.policy_challenge_context(p_id bytea,p_token bytea,p_actor bigint,p_session bigint)
RETURNS jsonb LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE c core_trust_pricing.policy_editor_challenges; v core_trust_pricing.policy_edit_previews; e core_trust_pricing.policy_editors;
BEGIN
 -- Also serializes counter updates and revocation. Same order as owner commands.
 PERFORM pg_advisory_xact_lock(741803219);
 SELECT * INTO c FROM core_trust_pricing.policy_editor_challenges WHERE challenge_id=p_id FOR UPDATE;
 SELECT * INTO v FROM core_trust_pricing.policy_edit_previews WHERE token_digest=p_token FOR UPDATE;
 SELECT * INTO e FROM core_trust_pricing.policy_editors WHERE editor_id=c.editor_id FOR UPDATE;
 IF c.challenge_id IS NULL OR v.token_digest IS NULL OR e.editor_id IS NULL OR e.revoked
 OR e.actor_user_id IS DISTINCT FROM p_actor OR v.actor_user_id IS DISTINCT FROM p_actor
 OR v.session_credential_id IS DISTINCT FROM p_session OR c.preview_digest IS DISTINCT FROM p_token
 OR c.editor_version IS DISTINCT FROM e.version OR c.consumed_at IS NOT NULL
 OR v.consumed_at IS NOT NULL OR least(c.expires_at,v.expires_at)<=clock_timestamp() THEN
  RAISE EXCEPTION 'invalid editor confirmation' USING ERRCODE='22023'; END IF;
 RETURN jsonb_build_object('editor',to_jsonb(e)-'credential_id','authentication',c.authentication,
 'digest',encode(v.transition_digest,'hex'));
END $$;
REVOKE ALL ON FUNCTION core_trust_pricing.policy_challenge_context(bytea,bytea,bigint,bigint) FROM PUBLIC;

CREATE FUNCTION core_trust_pricing.authorized_policy_confirm(p jsonb,proof bytea)
RETURNS bigint LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE context jsonb; e core_trust_pricing.policy_editors; next_revision bigint; counter bigint;
BEGIN
 IF NOT core_trust_pricing.policy_edit_verify('confirm-passkey-v1',p,proof) THEN
  RAISE EXCEPTION 'invalid editor proof' USING ERRCODE='42501'; END IF;
 context:=core_trust_pricing.policy_challenge_context(decode(p->>'challenge','hex'),decode(p->>'token','hex'),
 (p->>'actor')::bigint,(p->>'session')::bigint);
 SELECT * INTO e FROM core_trust_pricing.policy_editors WHERE editor_id=context->'editor'->>'editor_id';
 counter:=(p->>'counter')::bigint;
 IF p->>'digest' IS DISTINCT FROM context->>'digest' OR p->>'editor' IS DISTINCT FROM e.editor_id
 OR (p->>'version')::bigint IS DISTINCT FROM e.version
 OR decode(p->>'credential','hex') IS DISTINCT FROM e.credential_id
 OR p->'user_verified' IS DISTINCT FROM 'true'::jsonb
 OR counter IS NULL OR counter<0 OR counter>4294967295
 OR (counter>0 OR (e.passkey#>>'{cred,counter}')::bigint>0) AND counter<=(e.passkey#>>'{cred,counter}')::bigint
 OR jsonb_typeof(p->'backup_state') IS DISTINCT FROM 'boolean'
 OR jsonb_typeof(p->'backup_eligible') IS DISTINCT FROM 'boolean'
 OR ((e.passkey#>>'{cred,backup_eligible}')::boolean AND NOT (p->>'backup_eligible')::boolean)
 OR ((p->>'backup_state')::boolean AND NOT (p->>'backup_eligible')::boolean)
 THEN RAISE EXCEPTION 'invalid passkey result' USING ERRCODE='22023'; END IF;
 next_revision:=core_trust_pricing.apply_policy_edit(decode(p->>'token','hex'),(p->>'actor')::bigint,
 (p->>'session')::bigint,decode(p->>'digest','hex'));
 UPDATE core_trust_pricing.policy_editors SET passkey=
 jsonb_set(jsonb_set(jsonb_set(passkey,'{cred,counter}',to_jsonb(counter),false),
 '{cred,backup_state}',p->'backup_state',false),'{cred,backup_eligible}',p->'backup_eligible',false)
 WHERE editor_id=e.editor_id;
 UPDATE core_trust_pricing.policy_editor_challenges SET consumed_at=clock_timestamp()
 WHERE challenge_id=decode(p->>'challenge','hex');
 INSERT INTO core_trust_pricing.policy_editor_confirmations VALUES(next_revision,e.editor_id,e.version,e.credential_id,
 decode(p->>'challenge','hex'),decode(p->>'assertion_digest','hex'),p);
 RETURN next_revision;
END $$;
REVOKE ALL ON FUNCTION core_trust_pricing.authorized_policy_confirm(jsonb,bytea) FROM PUBLIC;

CREATE TABLE core_trust_pricing.policy_editor_events (
 editor_id text NOT NULL, version bigint NOT NULL, action text NOT NULL CHECK(action IN ('approve','revoke')),
 owner_role text NOT NULL, at timestamptz NOT NULL DEFAULT clock_timestamp(), PRIMARY KEY(editor_id,version)
);
REVOKE ALL ON core_trust_pricing.policy_editor_events FROM PUBLIC;
CREATE TRIGGER policy_editor_events_append_only BEFORE UPDATE OR DELETE OR TRUNCATE
ON core_trust_pricing.policy_editor_events FOR EACH STATEMENT
EXECUTE FUNCTION core_trust_pricing.prevent_policy_edit_audit_mutation();

-- Actor-scoped metadata only. No credential/public key or registration state is
-- exposed by the initialization endpoint; never infer entries from identity rows.
CREATE FUNCTION core_trust_pricing.read_policy_editors(p_actor bigint)
RETURNS TABLE(editor_id text,version bigint,revoked boolean,rp_id text,origin text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT e.editor_id,e.version,e.revoked,e.rp_id,e.origin
 FROM core_trust_pricing.policy_editors e WHERE e.actor_user_id=p_actor
 ORDER BY e.editor_id
$$;
REVOKE ALL ON FUNCTION core_trust_pricing.read_policy_editors(bigint) FROM PUBLIC;

-- Fixed-domain readiness check only: no caller-selected action or payload, no
-- user/session dependency and no writes. Missing owner key verifies as false.
CREATE FUNCTION core_trust_pricing.policy_editor_ready(proof bytea)
RETURNS boolean LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT core_trust_pricing.policy_edit_verify('ready','{}'::jsonb,proof)
$$;
REVOKE ALL ON FUNCTION core_trust_pricing.policy_editor_ready(bytea) FROM PUBLIC;

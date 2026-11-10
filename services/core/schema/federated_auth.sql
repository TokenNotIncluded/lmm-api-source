-- AUTH01: OIDC, the legacy WeChat verification-code bridge, and custom OAuth.
-- Independent appended component; historical identity/flow SQL is unchanged.
CREATE SCHEMA core_federated_auth;

CREATE TABLE core_federated_auth.providers (
 id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY CHECK(id>0),
 kind TEXT NOT NULL CHECK(kind IN ('oidc','wechat','custom_oauth')),
 slug TEXT NOT NULL UNIQUE CHECK(slug ~ '^[a-z0-9][a-z0-9-]{0,63}$'),
 name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 64),
 icon TEXT NOT NULL DEFAULT '' CHECK(length(icon)<=128),
 enabled BOOLEAN NOT NULL DEFAULT false,
 configuration JSONB NOT NULL CHECK(jsonb_typeof(configuration)='object'
   AND octet_length(configuration::text)<=1048576
   AND NOT configuration ?| ARRAY['client_secret','server_token']),
 secret_context BYTEA NOT NULL UNIQUE CHECK(octet_length(secret_context)=32),
 secret_key_id TEXT NOT NULL CHECK(secret_key_id ~ '^[a-zA-Z0-9_-]{1,64}$'),
 sealed_secret BYTEA NOT NULL CHECK(octet_length(sealed_secret) BETWEEN 29 AND 65536),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
 created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
 deleted_at TIMESTAMPTZ,
 CHECK(deleted_at IS NULL OR NOT enabled),
 CHECK((kind='oidc' AND slug='oidc') OR (kind='wechat' AND slug='wechat')
   OR (kind='custom_oauth' AND slug NOT IN ('oidc','wechat','github','discord','linuxdo','telegram','email','state','passkey')))
);
CREATE UNIQUE INDEX federated_builtin_kind ON core_federated_auth.providers(kind)
 WHERE kind IN ('oidc','wechat') AND deleted_at IS NULL;

-- The digest bounds index keys even for a legacy bridge's long opaque ID.
-- Consumers compare the original subject too; collisions never select an owner.
CREATE TABLE core_federated_auth.identities (
 provider_id BIGINT NOT NULL REFERENCES core_federated_auth.providers(id),
 subject TEXT NOT NULL CHECK(octet_length(subject) BETWEEN 1 AND 65536),
 subject_digest BYTEA NOT NULL CHECK(octet_length(subject_digest)=32
   AND subject_digest=sha256(convert_to(subject,'UTF8'))),
 user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 revision BIGINT NOT NULL DEFAULT 1 CHECK(revision>0),
 created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
 revoked_at TIMESTAMPTZ,
 PRIMARY KEY(provider_id,subject_digest)
);
CREATE UNIQUE INDEX federated_active_user ON core_federated_auth.identities(user_id,provider_id)
 WHERE revoked_at IS NULL;

CREATE TABLE core_federated_auth.flows (
 digest BYTEA PRIMARY KEY CHECK(octet_length(digest)=32),
 provider_id BIGINT NOT NULL REFERENCES core_federated_auth.providers(id),
 provider_revision BIGINT NOT NULL CHECK(provider_revision>0),
 purpose TEXT NOT NULL CHECK(purpose IN ('login','bind')),
 binding_digest BYTEA NOT NULL CHECK(octet_length(binding_digest)=32),
 nonce_digest BYTEA CHECK(octet_length(nonce_digest)=32),
 pkce_verifier TEXT CHECK(length(pkce_verifier) BETWEEN 43 AND 128),
 user_id BIGINT REFERENCES core_identity.users(id),
 user_version BIGINT CHECK(user_version>0),
 sid TEXT REFERENCES core_identity.web_sessions(sid),
 accepted_legal BOOLEAN NOT NULL DEFAULT false,
 affiliate TEXT NOT NULL DEFAULT '' CHECK(length(affiliate)<=256),
 created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()+interval '10 minutes',
 consumed_at TIMESTAMPTZ,
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '10 minutes'),
 CHECK((purpose='login' AND user_id IS NULL AND user_version IS NULL AND sid IS NULL)
   OR (purpose='bind' AND user_id IS NOT NULL AND user_version IS NOT NULL AND sid IS NOT NULL))
);
CREATE INDEX federated_flow_expiry ON core_federated_auth.flows(expires_at);

-- HMAC of provider ID and bridge code, shared by login and binding. Neither
-- raw small-space codes nor unsalted hashes enter durable storage.
CREATE TABLE core_federated_auth.assertions (
 digest BYTEA PRIMARY KEY CHECK(octet_length(digest)=32),
 created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()+interval '10 minutes',
 CHECK(expires_at>created_at AND expires_at<=created_at+interval '10 minutes')
);
CREATE INDEX federated_assertion_expiry ON core_federated_auth.assertions(expires_at);

-- Configuration mutation recovery contains no provider secret or auth bundle.
CREATE TABLE core_federated_auth.operations (
 actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 operation_key TEXT NOT NULL CHECK(operation_key ~ '^[a-zA-Z0-9_-]{16,128}$'),
 sid TEXT NOT NULL REFERENCES core_identity.web_sessions(sid),
 request_digest BYTEA NOT NULL CHECK(octet_length(request_digest)=32),
 action TEXT NOT NULL CHECK(action IN ('create','update','delete')),
 provider_id BIGINT NOT NULL REFERENCES core_federated_auth.providers(id),
 provider_revision BIGINT NOT NULL CHECK(provider_revision>0),
 created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
 PRIMARY KEY(actor_user_id,operation_key)
);

CREATE FUNCTION core_federated_auth.protect_provider() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') THEN
  RAISE EXCEPTION 'retain provider identity namespace' USING ERRCODE='23514';
 END IF;
 IF TG_OP='UPDATE' THEN
  IF (NEW.id,NEW.kind,NEW.secret_context,NEW.created_at) IS DISTINCT FROM (OLD.id,OLD.kind,OLD.secret_context,OLD.created_at)
    OR OLD.deleted_at IS NOT NULL OR OLD.revision=9223372036854775807
    OR NEW.revision<>OLD.revision+1 THEN
   RAISE EXCEPTION 'immutable provider or invalid revision' USING ERRCODE='23514';
  END IF;
  -- A new issuer cannot reinterpret already-owned subjects under an old ID.
  IF OLD.kind='oidc' AND NEW.configuration->>'issuer' IS DISTINCT FROM OLD.configuration->>'issuer'
    AND EXISTS(SELECT FROM core_federated_auth.identities i WHERE i.provider_id=OLD.id) THEN
   RAISE EXCEPTION 'OIDC issuer has identity history' USING ERRCODE='23514';
  END IF;
  IF NEW.deleted_at IS NOT NULL
    AND EXISTS(SELECT FROM core_federated_auth.identities i WHERE i.provider_id=OLD.id AND i.revoked_at IS NULL) THEN
   RAISE EXCEPTION 'provider still has active bindings' USING ERRCODE='23514';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER federated_provider_guard BEFORE INSERT OR UPDATE OR DELETE ON core_federated_auth.providers
 FOR EACH ROW EXECUTE FUNCTION core_federated_auth.protect_provider();
CREATE TRIGGER federated_provider_no_truncate BEFORE TRUNCATE ON core_federated_auth.providers
 FOR EACH STATEMENT EXECUTE FUNCTION core_federated_auth.protect_provider();

CREATE FUNCTION core_federated_auth.protect_identity() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP IN ('DELETE','TRUNCATE') THEN
  RAISE EXCEPTION 'retain federated identity ownership' USING ERRCODE='23514';
 END IF;
 IF TG_OP='UPDATE' AND (
   (NEW.provider_id,NEW.subject,NEW.subject_digest,NEW.user_id,NEW.created_at) IS DISTINCT FROM (OLD.provider_id,OLD.subject,OLD.subject_digest,OLD.user_id,OLD.created_at)
   OR OLD.revision=9223372036854775807 OR NEW.revision<>OLD.revision+1) THEN
  RAISE EXCEPTION 'immutable federated identity or invalid revision' USING ERRCODE='23514';
 END IF;
 IF NEW.revoked_at IS NULL THEN
  -- Serializes binding/reactivation with deletion and issuer changes. A
  -- disabled provider may retain imported bindings without granting login.
  PERFORM 1 FROM core_federated_auth.providers p
   WHERE p.id=NEW.provider_id AND p.deleted_at IS NULL FOR SHARE;
  IF NOT FOUND THEN
   RAISE EXCEPTION 'provider no longer exists' USING ERRCODE='23514';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER federated_identity_guard BEFORE INSERT OR UPDATE OR DELETE ON core_federated_auth.identities
 FOR EACH ROW EXECUTE FUNCTION core_federated_auth.protect_identity();
CREATE TRIGGER federated_identity_no_truncate BEFORE TRUNCATE ON core_federated_auth.identities
 FOR EACH STATEMENT EXECUTE FUNCTION core_federated_auth.protect_identity();

CREATE FUNCTION core_federated_auth.protect_ceremony() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN
  IF OLD.expires_at>clock_timestamp() THEN
   RAISE EXCEPTION 'retain unexpired ceremony replay protection' USING ERRCODE='23514';
  END IF;
  RETURN OLD;
 END IF;
 IF TG_OP='UPDATE' AND (OLD.consumed_at IS NOT NULL OR NEW.consumed_at IS NULL
   OR NEW.consumed_at<OLD.created_at OR NEW.consumed_at>clock_timestamp()
   OR (to_jsonb(NEW)-'consumed_at') IS DISTINCT FROM (to_jsonb(OLD)-'consumed_at')) THEN
  RAISE EXCEPTION 'ceremony already consumed or immutable evidence changed' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER federated_flow_guard BEFORE UPDATE OR DELETE ON core_federated_auth.flows
 FOR EACH ROW EXECUTE FUNCTION core_federated_auth.protect_ceremony();
CREATE TRIGGER federated_assertion_guard BEFORE DELETE ON core_federated_auth.assertions
 FOR EACH ROW EXECUTE FUNCTION core_federated_auth.protect_ceremony();

CREATE FUNCTION core_federated_auth.retain_operation() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 RAISE EXCEPTION 'immutable provider operation receipt' USING ERRCODE='23514';
END $$;
CREATE TRIGGER federated_operation_guard BEFORE UPDATE OR DELETE ON core_federated_auth.operations
 FOR EACH ROW EXECUTE FUNCTION core_federated_auth.retain_operation();
CREATE TRIGGER federated_operation_no_truncate BEFORE TRUNCATE ON core_federated_auth.operations
 FOR EACH STATEMENT EXECUTE FUNCTION core_federated_auth.retain_operation();

REVOKE ALL ON SCHEMA core_federated_auth FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_federated_auth FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_federated_auth FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_federated_auth FROM PUBLIC;
-- Exact runtime grants/checker inventory is maintained in runtime_federated_auth.rs.

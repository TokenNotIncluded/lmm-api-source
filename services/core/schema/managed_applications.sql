-- Component70: explicit fresh installation or pinned owner69->70 only.
-- Historical component SQL is immutable; these exact catalog changes are the delta.

CREATE OR REPLACE FUNCTION core_oauth.client_scope_set(client_ TEXT,values_ TEXT[],initial_ BOOLEAN) RETURNS BOOLEAN
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
DECLARE filtered_ TEXT[];
BEGIN
 IF client_='lmm-cc-switch' THEN
  IF cardinality(values_)<>(SELECT count(DISTINCT v) FROM unnest(values_) v) THEN RETURN FALSE; END IF;
  IF initial_ THEN RETURN cardinality(values_)=4 AND values_ @> ARRAY['balance:read','catalog:read','models:invoke','usage:read']::text[]; END IF;
  RETURN values_ @> ARRAY['catalog:read','models:invoke']::text[]
    AND core_oauth.readonly_scope_set(array_remove(array_remove(values_,'models:invoke'),'usage:read'));
 END IF;
 IF cardinality(values_)<>(SELECT count(DISTINCT v) FROM unnest(values_) v) THEN RETURN FALSE; END IF;
 IF initial_ THEN
  IF client_='lmm' THEN RETURN cardinality(values_)=2 AND values_ @> ARRAY['catalog:read','balance:read']::text[]; END IF;
  RETURN client_='lmm-pi' AND cardinality(values_) BETWEEN 4 AND 7
   AND values_ @> ARRAY['catalog:read','balance:read','usage:read','models:invoke']::text[]
   AND values_ <@ ARRAY['catalog:read','balance:read','usage:read','models:invoke','mcp:bounties','mcp:drawing','remote:control']::text[];
 END IF;
 IF client_='lmm' THEN RETURN core_oauth.readonly_scope_set(values_); END IF;
 IF client_<>'lmm-pi' OR cardinality(values_) NOT BETWEEN 2 AND 68
    OR NOT (values_ && ARRAY['catalog:read','balance:read','usage:read','models:invoke']::text[]) THEN RETURN FALSE; END IF;
 filtered_:=array_remove(array_remove(values_,'usage:read'),'models:invoke');
 IF NOT (filtered_ && ARRAY['catalog:read','balance:read']::text[]) THEN filtered_:=filtered_||'catalog:read'::text; END IF;
 RETURN core_oauth.readonly_scope_set(filtered_);
END $$;

ALTER TABLE core_oauth.families DROP CONSTRAINT families_client_id_check;
ALTER TABLE core_oauth.families ADD CONSTRAINT families_client_id_check CHECK(client_id IN ('lmm','lmm-pi','lmm-cc-switch'));

ALTER TABLE core_oauth.authorization_requests DROP CONSTRAINT authorization_requests_client_id_check;
ALTER TABLE core_oauth.authorization_requests ADD CONSTRAINT authorization_requests_client_id_check CHECK(client_id IN ('lmm','lmm-pi','lmm-cc-switch'));

ALTER TABLE core_oauth.authorization_codes DROP CONSTRAINT authorization_codes_client_id_check;
ALTER TABLE core_oauth.authorization_codes ADD CONSTRAINT authorization_codes_client_id_check CHECK(client_id IN ('lmm','lmm-pi','lmm-cc-switch'));

ALTER TABLE core_oauth.families DROP CONSTRAINT families_check1;
ALTER TABLE core_oauth.families ADD CONSTRAINT families_check1 CHECK ((issuance_kind = 'foundation'::text AND purpose = 'foundation'::text AND budget_credits IS NULL AND client_id = 'lmm-pi'::text AND cardinality(scopes) >= 1 AND cardinality(scopes) <= 7 AND scopes <@ ARRAY['catalog:read'::text, 'balance:read'::text, 'usage:read'::text, 'models:invoke'::text, 'mcp:bounties'::text, 'mcp:drawing'::text, 'remote:control'::text] OR issuance_kind = 'browser'::text AND client_id = 'lmm'::text AND purpose = 'read_only'::text AND budget_credits IS NULL AND core_oauth.readonly_scope_set(scopes) OR issuance_kind = 'browser'::text AND client_id = 'lmm-pi'::text AND core_oauth.client_scope_set(client_id, scopes, false) AND (purpose = 'normal_model'::text AND budget_credits IS NOT NULL OR purpose = 'honeypot_only'::text AND budget_credits IS NULL)) OR (client_id='lmm-cc-switch' AND issuance_kind='browser' AND purpose='normal_model' AND budget_credits IS NOT NULL AND core_oauth.client_scope_set(client_id,scopes,false)));

ALTER TABLE core_oauth.authorization_requests DROP CONSTRAINT authorization_requests_check3;
ALTER TABLE core_oauth.authorization_requests ADD CONSTRAINT authorization_requests_check3 CHECK ((user_id IS NULL AND owner_account_id IS NULL AND user_version IS NULL AND sid IS NULL AND cardinality(granted_scopes) = 0 OR user_id IS NOT NULL AND owner_account_id IS NOT NULL AND user_version IS NOT NULL AND sid IS NOT NULL AND ARRAY['catalog:read'::text, 'balance:read'::text] <@ granted_scopes AND core_oauth.client_scope_set(client_id, granted_scopes, false) AND (client_id = 'lmm'::text OR ARRAY['usage:read'::text, 'models:invoke'::text] <@ granted_scopes)) OR (client_id='lmm-cc-switch' AND user_id IS NOT NULL AND owner_account_id IS NOT NULL AND user_version IS NOT NULL AND sid IS NOT NULL AND core_oauth.client_scope_set(client_id,granted_scopes,false)));

ALTER TABLE core_oauth.authorization_codes DROP CONSTRAINT authorization_codes_check1;
ALTER TABLE core_oauth.authorization_codes ADD CONSTRAINT authorization_codes_check1 CHECK ((ARRAY['catalog:read'::text, 'balance:read'::text] <@ scopes AND core_oauth.client_scope_set(client_id, scopes, false) AND (client_id = 'lmm'::text OR ARRAY['usage:read'::text, 'models:invoke'::text] <@ scopes)) OR (client_id='lmm-cc-switch' AND core_oauth.client_scope_set(client_id,scopes,false)));

ALTER TABLE core_charging.requests DROP CONSTRAINT requests_oauth_source_check;
ALTER TABLE core_charging.requests ADD CONSTRAINT requests_oauth_source_check CHECK (oauth_source IS NULL OR (jsonb_typeof(oauth_source) = 'object'::text AND oauth_source ?& ARRAY['grant_id'::text, 'access_id'::text, 'client_id'::text, 'issuer'::text, 'resource'::text, 'user_id'::text, 'personal_account_id'::text, 'user_version'::text, 'consent_version'::text, 'scopes'::text, 'expires_at'::text, 'budget_credits'::text] AND (oauth_source - ARRAY['grant_id'::text, 'access_id'::text, 'client_id'::text, 'issuer'::text, 'resource'::text, 'user_id'::text, 'personal_account_id'::text, 'user_version'::text, 'consent_version'::text, 'scopes'::text, 'expires_at'::text, 'budget_credits'::text]) = '{}'::jsonb AND (oauth_source ->> 'client_id'::text) = ANY (ARRAY['lmm-pi'::text,'lmm-cc-switch'::text]) AND (oauth_source ->> 'grant_id'::text) ~ '^lmm_gr_[0-9a-f]{64}$'::text AND (oauth_source ->> 'access_id'::text) ~ '^lmm_ac_[0-9a-f]{64}$'::text AND jsonb_typeof(oauth_source -> 'scopes'::text) = 'array'::text AND jsonb_typeof(oauth_source -> 'grant_id'::text) = 'string'::text AND jsonb_typeof(oauth_source -> 'access_id'::text) = 'string'::text AND jsonb_typeof(oauth_source -> 'client_id'::text) = 'string'::text AND jsonb_typeof(oauth_source -> 'issuer'::text) = 'string'::text AND jsonb_typeof(oauth_source -> 'resource'::text) = 'string'::text AND (oauth_source -> 'scopes'::text) ? 'models:invoke'::text AND jsonb_array_length(oauth_source -> 'scopes'::text) >= 2 AND jsonb_array_length(oauth_source -> 'scopes'::text) <= 68 AND (oauth_source ->> 'expires_at'::text) ~ '^[1-9][0-9]{0,11}$'::text AND ((oauth_source ->> 'expires_at'::text)::numeric) <= '253402300799'::bigint::numeric AND jsonb_typeof(oauth_source -> 'budget_credits'::text) = 'number'::text AND jsonb_typeof(oauth_source -> 'user_id'::text) = 'number'::text AND jsonb_typeof(oauth_source -> 'personal_account_id'::text) = 'number'::text AND jsonb_typeof(oauth_source -> 'user_version'::text) = 'number'::text AND jsonb_typeof(oauth_source -> 'consent_version'::text) = 'number'::text AND jsonb_typeof(oauth_source -> 'expires_at'::text) = 'number'::text AND ((oauth_source ->> 'budget_credits'::text)::numeric) >= 1::numeric AND ((oauth_source ->> 'budget_credits'::text)::numeric) <= '9007199254740991'::bigint::numeric) IS TRUE);

CREATE TABLE core_oauth.managed_authorization_bindings (
 binding_digest BYTEA PRIMARY KEY REFERENCES core_oauth.authorization_requests(binding_digest),
 client_id TEXT NOT NULL CHECK(client_id='lmm-cc-switch'),
 application TEXT NOT NULL CHECK(application='cc-switch'),
 installation_id TEXT NOT NULL CHECK(installation_id ~ '^[0-9a-f]{64}$'),
 display_label TEXT NOT NULL CHECK(octet_length(display_label) BETWEEN 1 AND 160
   AND display_label COLLATE "C" !~ U&'[\0001-\001F\007F-\009F]')
);
CREATE TABLE core_oauth.managed_family_bindings (
 family_id TEXT PRIMARY KEY REFERENCES core_oauth.families(id),
 authorization_binding_digest BYTEA NOT NULL REFERENCES core_oauth.managed_authorization_bindings(binding_digest),
 client_id TEXT NOT NULL CHECK(client_id='lmm-cc-switch'),
 application TEXT NOT NULL CHECK(application='cc-switch'),
 installation_id TEXT NOT NULL CHECK(installation_id ~ '^[0-9a-f]{64}$'),
 display_label TEXT NOT NULL CHECK(octet_length(display_label) BETWEEN 1 AND 160
   AND display_label COLLATE "C" !~ U&'[\0001-\001F\007F-\009F]')
);
CREATE INDEX managed_family_installation ON core_oauth.managed_family_bindings(client_id,installation_id,family_id);
CREATE TABLE core_oauth.managed_refresh_lineage (
 old_refresh_digest BYTEA PRIMARY KEY REFERENCES core_oauth.refresh_tokens(digest),
 family_id TEXT NOT NULL REFERENCES core_oauth.families(id),
 rotation_id TEXT NOT NULL CHECK(rotation_id ~ '^[0-9a-f]{64}$'),
 request_digest BYTEA NOT NULL CHECK(octet_length(request_digest)=32),
 successor_access_id TEXT NOT NULL UNIQUE REFERENCES core_oauth.access_tokens(id),
 successor_refresh_digest BYTEA NOT NULL UNIQUE REFERENCES core_oauth.refresh_tokens(digest),
 created_at TIMESTAMPTZ NOT NULL,
 recover_until TIMESTAMPTZ NOT NULL,
 UNIQUE(family_id,rotation_id),
 CHECK(old_refresh_digest<>successor_refresh_digest),
 CHECK(recover_until=created_at+interval '600 seconds')
);
CREATE INDEX managed_refresh_recover_until ON core_oauth.managed_refresh_lineage(recover_until,old_refresh_digest);
CREATE TABLE core_oauth.managed_refresh_responses (
 old_refresh_digest BYTEA PRIMARY KEY REFERENCES core_oauth.managed_refresh_lineage(old_refresh_digest),
 key_id TEXT NOT NULL CHECK(key_id ~ '^[A-Za-z0-9_.-]{1,64}$'),
 encrypted_response BYTEA NOT NULL CHECK(octet_length(encrypted_response) BETWEEN 29 AND 16384)
);

CREATE FUNCTION core_oauth.immutable_managed_binding() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 RAISE EXCEPTION 'immutable managed authorization lineage' USING ERRCODE='23514';
END $$;
CREATE TRIGGER managed_authorization_immutable BEFORE UPDATE OR DELETE ON core_oauth.managed_authorization_bindings
 FOR EACH ROW EXECUTE FUNCTION core_oauth.immutable_managed_binding();
CREATE TRIGGER managed_authorization_no_truncate BEFORE TRUNCATE ON core_oauth.managed_authorization_bindings
 FOR EACH STATEMENT EXECUTE FUNCTION core_oauth.immutable_managed_binding();
CREATE TRIGGER managed_family_immutable BEFORE UPDATE OR DELETE ON core_oauth.managed_family_bindings
 FOR EACH ROW EXECUTE FUNCTION core_oauth.immutable_managed_binding();
CREATE TRIGGER managed_family_no_truncate BEFORE TRUNCATE ON core_oauth.managed_family_bindings
 FOR EACH STATEMENT EXECUTE FUNCTION core_oauth.immutable_managed_binding();
CREATE TRIGGER managed_lineage_immutable BEFORE UPDATE OR DELETE ON core_oauth.managed_refresh_lineage
 FOR EACH ROW EXECUTE FUNCTION core_oauth.immutable_managed_binding();
CREATE TRIGGER managed_lineage_no_truncate BEFORE TRUNCATE ON core_oauth.managed_refresh_lineage
 FOR EACH STATEMENT EXECUTE FUNCTION core_oauth.immutable_managed_binding();
CREATE TRIGGER managed_response_no_update BEFORE UPDATE ON core_oauth.managed_refresh_responses
 FOR EACH ROW EXECUTE FUNCTION core_oauth.immutable_managed_binding();

CREATE FUNCTION core_oauth.validate_managed_binding() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE valid_ BOOLEAN;
BEGIN
 IF TG_TABLE_NAME='managed_authorization_bindings' THEN
  SELECT client_id=NEW.client_id INTO valid_ FROM core_oauth.authorization_requests
   WHERE binding_digest=NEW.binding_digest;
 ELSE
  SELECT f.client_id=NEW.client_id AND f.client_id=b.client_id AND r.client_id=f.client_id
    AND b.application=NEW.application AND b.installation_id=NEW.installation_id AND b.display_label=NEW.display_label
    AND f.user_id=r.user_id AND f.owner_account_id=r.owner_account_id AND f.user_version=r.user_version
    AND f.issuer=r.issuer AND f.resource=r.resource AND r.sid IS NOT NULL
    AND f.scopes<@r.granted_scopes AND f.issuance_kind='browser' AND f.purpose='normal_model'
    AND f.budget_credits BETWEEN 1 AND 9007199254740991
  INTO valid_ FROM core_oauth.families f
  JOIN core_oauth.managed_authorization_bindings b ON b.binding_digest=NEW.authorization_binding_digest
  JOIN core_oauth.authorization_requests r ON r.binding_digest=b.binding_digest
  WHERE f.id=NEW.family_id;
 END IF;
 IF valid_ IS DISTINCT FROM TRUE THEN
  RAISE EXCEPTION 'managed authorization binding mismatch' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER managed_authorization_validate BEFORE INSERT ON core_oauth.managed_authorization_bindings
 FOR EACH ROW EXECUTE FUNCTION core_oauth.validate_managed_binding();
CREATE TRIGGER managed_family_validate BEFORE INSERT ON core_oauth.managed_family_bindings
 FOR EACH ROW EXECUTE FUNCTION core_oauth.validate_managed_binding();

CREATE FUNCTION core_oauth.validate_managed_rotation() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE valid_ BOOLEAN;
BEGIN
 IF TG_TABLE_NAME='managed_refresh_responses' THEN
  SELECT clock_timestamp()<recover_until INTO valid_ FROM core_oauth.managed_refresh_lineage
   WHERE old_refresh_digest=NEW.old_refresh_digest;
 ELSE
  SELECT o.family_id=NEW.family_id AND a.family_id=o.family_id AND n.family_id=o.family_id
    AND o.consumed_at IS NOT NULL AND n.consumed_at IS NULL
    AND o.consent_version=a.consent_version AND o.consent_version=n.consent_version
    AND a.scopes=n.scopes AND n.scopes<@o.scopes
  INTO valid_ FROM core_oauth.refresh_tokens o
  JOIN core_oauth.managed_family_bindings b ON b.family_id=o.family_id
  JOIN core_oauth.access_tokens a ON a.id=NEW.successor_access_id
  JOIN core_oauth.refresh_tokens n ON n.digest=NEW.successor_refresh_digest
  WHERE o.digest=NEW.old_refresh_digest;
  NEW.created_at:=clock_timestamp();
  NEW.recover_until:=NEW.created_at+interval '600 seconds';
 END IF;
 IF valid_ IS DISTINCT FROM TRUE THEN
  RAISE EXCEPTION 'managed rotation binding mismatch or expired response' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER managed_lineage_validate BEFORE INSERT ON core_oauth.managed_refresh_lineage
 FOR EACH ROW EXECUTE FUNCTION core_oauth.validate_managed_rotation();
CREATE TRIGGER managed_response_validate BEFORE INSERT ON core_oauth.managed_refresh_responses
 FOR EACH ROW EXECUTE FUNCTION core_oauth.validate_managed_rotation();

-- Explicit maintenance only. Runtime receives neither EXECUTE nor DELETE.
-- Keep all lineage, even after its encrypted response has been retired.
CREATE FUNCTION core_oauth.cleanup_managed_refresh_responses(limit_ INTEGER) RETURNS INTEGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE deleted_ INTEGER;
BEGIN
 IF limit_ NOT BETWEEN 1 AND 256 OR limit_ IS NULL THEN
  RAISE EXCEPTION 'invalid managed cleanup batch' USING ERRCODE='22023';
 END IF;
 IF NOT pg_has_role(current_user,(SELECT relowner FROM pg_class WHERE oid='core_oauth.managed_refresh_responses'::regclass),'USAGE') THEN
  RAISE EXCEPTION 'managed cleanup requires table owner' USING ERRCODE='42501';
 END IF;
 WITH selected AS MATERIALIZED (
  SELECT r.old_refresh_digest FROM core_oauth.managed_refresh_responses r
   JOIN core_oauth.managed_refresh_lineage l USING(old_refresh_digest)
   WHERE clock_timestamp()>=l.recover_until ORDER BY l.recover_until,r.old_refresh_digest
   LIMIT limit_ FOR UPDATE OF r SKIP LOCKED
 ), removed AS (
  DELETE FROM core_oauth.managed_refresh_responses r USING selected s
   WHERE r.old_refresh_digest=s.old_refresh_digest RETURNING r.old_refresh_digest
 ) SELECT count(*)::integer INTO deleted_ FROM removed;
 RETURN deleted_;
END $$;
REVOKE ALL ON core_oauth.managed_authorization_bindings,core_oauth.managed_family_bindings,
 core_oauth.managed_refresh_lineage,core_oauth.managed_refresh_responses FROM PUBLIC;
REVOKE ALL ON FUNCTION core_oauth.immutable_managed_binding(),core_oauth.validate_managed_binding(),
 core_oauth.validate_managed_rotation(),core_oauth.cleanup_managed_refresh_responses(integer) FROM PUBLIC;

CREATE TABLE core_meta.managed_applications_upgrades (
 upgrade_id TEXT PRIMARY KEY CHECK(octet_length(upgrade_id) BETWEEN 16 AND 96),
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL,
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER managed_applications_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.managed_applications_upgrades
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER managed_applications_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.managed_applications_upgrades
 FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.managed_applications_upgrades FROM PUBLIC;

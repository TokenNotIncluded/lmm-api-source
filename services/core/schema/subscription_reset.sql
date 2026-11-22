-- Core79, appended to the exact Core78 contract. Existing grants, requests,
-- opening consumption and wallet journals are never rewritten by a reset.
CREATE TABLE core_subscriptions.reset_codes (
    digest BYTEA PRIMARY KEY CHECK(octet_length(digest)=32),
    plan_id BIGINT NOT NULL REFERENCES core_subscriptions.plans(id),
    issuer_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    source_reference TEXT NOT NULL UNIQUE CHECK(length(source_reference) BETWEEN 1 AND 200),
    expires_at BIGINT NOT NULL CHECK(expires_at BETWEEN 0 AND 253402300799),
    voucher_expires_at BIGINT NOT NULL CHECK(voucher_expires_at BETWEEN 1 AND 253402300799),
    enabled BOOLEAN NOT NULL
);
-- An imported grant's plan is an explicit owner-attested fact. Numeric IDs,
-- titles and grant_reference substrings are not evidence of a plan relation.
CREATE TABLE core_subscriptions.reset_imported_grant_plans (
    subscription_id BIGINT PRIMARY KEY REFERENCES core_charging.subscriptions(id),
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    plan_id BIGINT NOT NULL REFERENCES core_subscriptions.plans(id),
    source_reference TEXT NOT NULL UNIQUE CHECK(length(source_reference) BETWEEN 1 AND 200)
);
CREATE TABLE core_subscriptions.reset_vouchers (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    plan_id BIGINT NOT NULL REFERENCES core_subscriptions.plans(id),
    operation_id TEXT NOT NULL CHECK(length(operation_id) BETWEEN 1 AND 200),
    code_digest BYTEA UNIQUE REFERENCES core_subscriptions.reset_codes(digest),
    expires_at BIGINT NOT NULL CHECK(expires_at BETWEEN 1 AND 253402300799),
    created_by BIGINT NOT NULL REFERENCES core_identity.users(id),
    created_at BIGINT NOT NULL CHECK(created_at>0),
    UNIQUE(user_id,plan_id,operation_id)
);
CREATE INDEX reset_vouchers_user ON core_subscriptions.reset_vouchers(user_id,id DESC);
CREATE TABLE core_subscriptions.reset_voucher_redemptions (
    voucher_id BIGINT PRIMARY KEY REFERENCES core_subscriptions.reset_vouchers(id),
    result JSONB NOT NULL CHECK(jsonb_typeof(result)='object'),
    created_at BIGINT NOT NULL CHECK(created_at>0)
);
CREATE TABLE core_subscriptions.reset_previews (
    token TEXT PRIMARY KEY CHECK(token ~ '^[a-f0-9]{64}$'),
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    mode TEXT NOT NULL CHECK(mode IN ('hard','soft')),
    targets JSONB NOT NULL CHECK(jsonb_typeof(targets)='array' AND jsonb_array_length(targets) BETWEEN 1 AND 20000),
    payload_sha256 BYTEA NOT NULL CHECK(octet_length(payload_sha256)=32),
    voucher_expires_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL CHECK(created_at>0),
    expires_at BIGINT NOT NULL CHECK(expires_at=created_at+600),
    CHECK((mode='hard' AND voucher_expires_at=0) OR
          (mode='soft' AND voucher_expires_at>created_at AND voucher_expires_at<=253402300799))
);
CREATE TABLE core_subscriptions.reset_operations (
    operation_id TEXT PRIMARY KEY CHECK(length(operation_id) BETWEEN 1 AND 64),
    preview_token TEXT NOT NULL UNIQUE REFERENCES core_subscriptions.reset_previews(token),
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    result JSONB NOT NULL CHECK(jsonb_typeof(result)='object'),
    created_at BIGINT NOT NULL CHECK(created_at>0)
);
CREATE TABLE core_subscriptions.reset_events (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    operation_id TEXT NOT NULL CHECK(length(operation_id) BETWEEN 1 AND 200),
    mode TEXT NOT NULL CHECK(mode IN ('hard','voucher_redeem')),
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    plan_id BIGINT NOT NULL REFERENCES core_subscriptions.plans(id),
    subscription_id BIGINT NOT NULL REFERENCES core_charging.subscriptions(id),
    voucher_id BIGINT REFERENCES core_subscriptions.reset_vouchers(id),
    window_start BIGINT NOT NULL,
    window_end BIGINT NOT NULL CHECK(window_end>window_start),
    previous_reset_id BIGINT REFERENCES core_subscriptions.reset_events(id),
    before_limit BIGINT NOT NULL CHECK(before_limit>=0),
    before_unlimited BOOLEAN NOT NULL,
    before_used BIGINT NOT NULL CHECK(before_used>=0),
    after_limit BIGINT NOT NULL CHECK(after_limit>=0),
    after_unlimited BOOLEAN NOT NULL,
    restored_quota BIGINT NOT NULL CHECK(restored_quota>=0),
    created_at BIGINT NOT NULL CHECK(created_at>0),
    UNIQUE(operation_id,subscription_id),
    UNIQUE NULLS NOT DISTINCT(subscription_id,window_start,previous_reset_id),
    CHECK((mode='hard' AND voucher_id IS NULL) OR
          (mode='voucher_redeem' AND voucher_id IS NOT NULL AND actor_user_id=user_id))
);
CREATE INDEX reset_events_window ON core_subscriptions.reset_events(subscription_id,window_start,id DESC);
-- Generation zero needs no row. Only requests created after a reset acquire a
-- new immutable binding, so predecessor request rows stay byte-for-byte intact.
CREATE TABLE core_charging.subscription_request_resets (
    request_id TEXT PRIMARY KEY REFERENCES core_charging.requests(id),
    reset_event_id BIGINT NOT NULL REFERENCES core_subscriptions.reset_events(id)
);
CREATE INDEX subscription_request_reset_event ON core_charging.subscription_request_resets(reset_event_id);

CREATE FUNCTION core_charging.subscription_plan(p_id BIGINT)
RETURNS TABLE(user_id BIGINT,plan_id BIGINT) LANGUAGE sql STABLE STRICT
SET search_path=pg_catalog AS $$
    WITH evidence AS (
        SELECT p.actor_user_id AS user_id,q.plan_id
        FROM core_subscriptions.purchases p JOIN core_subscriptions.quotes q ON q.id=p.quote_id
        WHERE p.subscription_id=p_id
        UNION
        SELECT o.actor_user_id,o.plan_id FROM core_subscriptions.external_orders o
        JOIN (
            SELECT order_id,subscription_id FROM core_subscriptions.external_grants
            UNION ALL SELECT order_id,subscription_id FROM core_subscriptions.stripe_cycles
            UNION ALL SELECT order_id,subscription_id FROM core_subscriptions.pancake_cycles
            UNION ALL SELECT order_id,subscription_id FROM core_subscriptions.creem_cycles
        ) g ON g.order_id=o.id WHERE g.subscription_id=p_id
        UNION
        SELECT b.user_id,b.plan_id FROM core_subscriptions.reset_imported_grant_plans b
        WHERE b.subscription_id=p_id
    ), unique_evidence AS (
        SELECT e.*,count(*) OVER() AS evidence_count FROM evidence e
    )
    SELECT e.user_id,e.plan_id FROM unique_evidence e
    JOIN core_identity.users u ON u.id=e.user_id
    JOIN core_charging.subscriptions s ON s.id=p_id AND s.account_id=u.personal_account_id
    WHERE e.evidence_count=1
$$;

-- Preserve the original calendar calculation as the underlying window. Manual
-- reset events replace only the current window's basis and consumption epoch.
CREATE FUNCTION core_charging.subscription_base_window(p_id BIGINT,p_at BIGINT)
RETURNS TABLE(start_at BIGINT,end_at BIGINT,limit_credits BIGINT,unlimited BOOLEAN)
LANGUAGE sql STABLE STRICT SET search_path=pg_catalog AS $$
    SELECT CASE WHEN o.subscription_id IS NOT NULL AND p_at < o.end_at THEN o.start_at ELSE w.start_at END,
           least(CASE WHEN o.subscription_id IS NOT NULL AND p_at < o.end_at THEN o.end_at ELSE w.end_at END,s.ends_at),
           CASE WHEN o.subscription_id IS NOT NULL AND p_at >= o.end_at AND s.reset_basis='explicit'
                THEN s.reset_limit_credits ELSE s.limit_credits END,
           CASE WHEN o.subscription_id IS NOT NULL AND p_at >= o.end_at AND s.reset_basis='explicit'
                THEN s.reset_unlimited ELSE s.unlimited END
    FROM core_charging.subscriptions s
    LEFT JOIN core_charging.subscription_opening_consumption o ON o.subscription_id=s.id
    CROSS JOIN LATERAL core_charging.period_window(
        CASE WHEN s.period='grant' THEN 'custom' ELSE s.period END,
        CASE WHEN s.period='custom' AND o.subscription_id IS NOT NULL AND p_at >= o.end_at
             THEN o.end_at ELSE s.anchor END,
        CASE WHEN s.period='grant' THEN s.ends_at-s.anchor ELSE s.seconds END,p_at) w
    WHERE s.id=p_id AND p_at>=s.anchor AND p_at<s.ends_at
      AND (o.subscription_id IS NULL OR p_at>=o.start_at)
      AND (s.reset_basis='same' OR o.subscription_id IS NOT NULL)
$$;
CREATE OR REPLACE FUNCTION core_charging.subscription_window(p_id BIGINT,p_at BIGINT)
RETURNS TABLE(start_at BIGINT,end_at BIGINT,limit_credits BIGINT,unlimited BOOLEAN)
LANGUAGE sql STABLE STRICT SET search_path=pg_catalog AS $$
    SELECT w.start_at,w.end_at,coalesce(e.after_limit,w.limit_credits),coalesce(e.after_unlimited,w.unlimited)
    FROM core_charging.subscription_base_window(p_id,p_at) w
    LEFT JOIN LATERAL (
        SELECT r.after_limit,r.after_unlimited FROM core_subscriptions.reset_events r
        WHERE r.subscription_id=p_id AND r.window_start=w.start_at ORDER BY r.id DESC LIMIT 1
    ) e ON true
$$;
CREATE FUNCTION core_charging.subscription_consumption(p_id BIGINT,p_start BIGINT,p_exclude TEXT)
RETURNS TABLE(used_credits NUMERIC,reserved_credits NUMERIC,reset_event_id BIGINT)
LANGUAGE sql STABLE SET search_path=pg_catalog AS $$
    WITH generation AS (
        SELECT (SELECT id FROM core_subscriptions.reset_events
            WHERE subscription_id=p_id AND window_start=p_start ORDER BY id DESC LIMIT 1) AS id
    )
    SELECT coalesce((SELECT sum(CASE WHEN r.state='settled' THEN r.settled-r.refunded ELSE 0 END)
              FROM core_charging.requests r LEFT JOIN core_charging.subscription_request_resets b ON b.request_id=r.id
              WHERE r.source_subscription_id=p_id AND r.source_start=p_start
                AND r.id IS DISTINCT FROM p_exclude AND b.reset_event_id IS NOT DISTINCT FROM g.id),0)
           + CASE WHEN g.id IS NULL THEN coalesce((SELECT consumed_credits
              FROM core_charging.subscription_opening_consumption WHERE subscription_id=p_id AND start_at=p_start),0) ELSE 0 END,
           coalesce((SELECT sum(r.reserved) FROM core_charging.requests r
              LEFT JOIN core_charging.subscription_request_resets b ON b.request_id=r.id
              WHERE r.source_subscription_id=p_id AND r.source_start=p_start
                AND r.id IS DISTINCT FROM p_exclude AND r.state IN ('reserved','streaming','reconcile')
                AND b.reset_event_id IS NOT DISTINCT FROM g.id),0),g.id
    FROM generation g
$$;
CREATE FUNCTION core_charging.bind_subscription_reset() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE event_id BIGINT;
BEGIN
    IF NEW.source_subscription_id IS NOT NULL THEN
        PERFORM 1 FROM core_charging.subscriptions WHERE id=NEW.source_subscription_id FOR SHARE;
        SELECT id INTO event_id FROM core_subscriptions.reset_events
        WHERE subscription_id=NEW.source_subscription_id AND window_start=NEW.source_start
        ORDER BY id DESC LIMIT 1;
        IF event_id IS NOT NULL THEN
            INSERT INTO core_charging.subscription_request_resets VALUES(NEW.id,event_id);
        END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER request_reset_generation AFTER INSERT ON core_charging.requests
FOR EACH ROW EXECUTE FUNCTION core_charging.bind_subscription_reset();
CREATE FUNCTION core_charging.guard_subscription_reset_binding() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF NOT EXISTS(SELECT FROM core_charging.requests r JOIN core_subscriptions.reset_events e
        ON e.subscription_id=r.source_subscription_id AND e.window_start=r.source_start
        WHERE r.id=NEW.request_id AND e.id=NEW.reset_event_id
          AND r.state='reserved' AND e.created_at<=r.authorized_at
          AND e.id=(SELECT max(id) FROM core_subscriptions.reset_events
                    WHERE subscription_id=e.subscription_id AND window_start=e.window_start)) THEN
        RAISE EXCEPTION 'request reset generation mismatch' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER request_reset_binding BEFORE INSERT ON core_charging.subscription_request_resets
FOR EACH ROW EXECUTE FUNCTION core_charging.guard_subscription_reset_binding();
CREATE FUNCTION core_subscriptions.guard_reset_event() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE s core_charging.subscriptions%ROWTYPE; w RECORD; u RECORD; p RECORD; t BIGINT; expected BIGINT;
BEGIN
    SELECT * INTO STRICT s FROM core_charging.subscriptions WHERE id=NEW.subscription_id FOR UPDATE;
    t:=floor(extract(epoch FROM clock_timestamp()))::bigint;
    SELECT * INTO w FROM core_charging.subscription_window(s.id,t);
    SELECT * INTO u FROM core_charging.subscription_consumption(s.id,w.start_at,NULL);
    SELECT * INTO p FROM core_charging.subscription_plan(s.id);
    IF NOT s.active OR w.start_at IS NULL OR p.plan_id IS NULL
       OR (p.user_id,p.plan_id) IS DISTINCT FROM (NEW.user_id,NEW.plan_id)
       OR (w.start_at,w.end_at,w.limit_credits,w.unlimited,u.used_credits,u.reset_event_id)
            IS DISTINCT FROM (NEW.window_start,NEW.window_end,NEW.before_limit,NEW.before_unlimited,NEW.before_used::numeric,NEW.previous_reset_id)
       OR EXISTS(SELECT FROM core_charging.requests WHERE source_subscription_id=s.id AND state IN ('reserved','streaming','reconcile'))
       OR NEW.created_at>t OR NEW.created_at<w.start_at THEN
        RAISE EXCEPTION 'subscription reset changed or has inflight requests' USING ERRCODE='23514';
    END IF;
    IF (NEW.after_limit,NEW.after_unlimited) IS DISTINCT FROM
       (CASE WHEN s.reset_basis='explicit' THEN s.reset_limit_credits ELSE w.limit_credits END,
        CASE WHEN s.reset_basis='explicit' THEN s.reset_unlimited ELSE w.unlimited END) THEN
        RAISE EXCEPTION 'subscription reset basis mismatch' USING ERRCODE='23514';
    END IF;
    expected:=CASE WHEN s.reset_basis='explicit' THEN greatest(0::numeric,
        NEW.after_limit::numeric-greatest(0::numeric,NEW.before_limit::numeric-NEW.before_used::numeric))::bigint
        ELSE NEW.before_used END;
    IF NEW.restored_quota<>expected OR (NEW.voucher_id IS NOT NULL AND NOT EXISTS(
        SELECT FROM core_subscriptions.reset_vouchers v WHERE v.id=NEW.voucher_id
        AND v.user_id=NEW.user_id AND v.plan_id=NEW.plan_id AND v.expires_at>t)) THEN
        RAISE EXCEPTION 'subscription reset voucher or amount mismatch' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER reset_event_basis BEFORE INSERT ON core_subscriptions.reset_events
FOR EACH ROW EXECUTE FUNCTION core_subscriptions.guard_reset_event();
CREATE FUNCTION core_subscriptions.check_reset_redemption() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE v core_subscriptions.reset_vouchers%ROWTYPE; n BIGINT; amount NUMERIC;
BEGIN
    SELECT * INTO STRICT v FROM core_subscriptions.reset_vouchers WHERE id=NEW.voucher_id;
    SELECT count(*),coalesce(sum(restored_quota),0) INTO n,amount FROM core_subscriptions.reset_events
    WHERE voucher_id=v.id AND mode='voucher_redeem' AND user_id=v.user_id AND plan_id=v.plan_id;
    IF n=0 OR NEW.result->>'reset_count' IS DISTINCT FROM n::text
       OR NEW.result->>'restored_quota' IS DISTINCT FROM amount::text
       OR NEW.result->>'plan_id' IS DISTINCT FROM v.plan_id::text THEN
        RAISE EXCEPTION 'reset redemption requires original events' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER reset_redemption_bound AFTER INSERT ON core_subscriptions.reset_voucher_redemptions
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_subscriptions.check_reset_redemption();
CREATE FUNCTION core_subscriptions.guard_reset_voucher() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF NEW.code_digest IS NOT NULL AND NOT EXISTS(SELECT FROM core_subscriptions.reset_codes c
        WHERE c.digest=NEW.code_digest AND c.plan_id=NEW.plan_id AND c.enabled
        AND c.issuer_user_id=NEW.created_by AND c.voucher_expires_at=NEW.expires_at
        AND (c.expires_at=0 OR c.expires_at>=NEW.created_at) AND NEW.expires_at>NEW.created_at) THEN
        RAISE EXCEPTION 'reset voucher code mismatch' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER reset_voucher_source BEFORE INSERT ON core_subscriptions.reset_vouchers
FOR EACH ROW EXECUTE FUNCTION core_subscriptions.guard_reset_voucher();

DO $$ DECLARE name TEXT; BEGIN
    FOREACH name IN ARRAY ARRAY['reset_codes','reset_imported_grant_plans','reset_vouchers',
        'reset_voucher_redemptions','reset_previews','reset_operations','reset_events'] LOOP
        EXECUTE format('CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON core_subscriptions.%I FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable()',name||'_immutable',name);
        EXECUTE format('CREATE TRIGGER %I BEFORE TRUNCATE ON core_subscriptions.%I FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable()',name||'_no_truncate',name);
        EXECUTE format('REVOKE ALL ON core_subscriptions.%I FROM PUBLIC',name);
    END LOOP;
END $$;
CREATE TRIGGER subscription_request_resets_immutable BEFORE UPDATE OR DELETE ON core_charging.subscription_request_resets
FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER subscription_request_resets_no_truncate BEFORE TRUNCATE ON core_charging.subscription_request_resets
FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TABLE core_meta.subscription_reset_upgrades (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
    request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
    preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER subscription_reset_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.subscription_reset_upgrades
FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER subscription_reset_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.subscription_reset_upgrades
FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_charging.subscription_request_resets,core_meta.subscription_reset_upgrades FROM PUBLIC;
REVOKE ALL ON FUNCTION core_charging.subscription_plan(BIGINT),
    core_charging.subscription_base_window(BIGINT,BIGINT),
    core_charging.subscription_consumption(BIGINT,BIGINT,TEXT),
    core_charging.bind_subscription_reset(),core_charging.guard_subscription_reset_binding(),
    core_subscriptions.guard_reset_event(),core_subscriptions.check_reset_redemption(),
    core_subscriptions.guard_reset_voucher() FROM PUBLIC;

-- Keep the existing source-bound history guard on the exact current assembly.
CREATE OR REPLACE FUNCTION core_history.guard_connection_snapshot() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE d JSONB:=NEW.document_utf8::jsonb; m JSONB:=NEW.mapping_utf8::jsonb;
 row_ JSONB; user_ JSONB; source_id_ BIGINT; nonce_ TEXT; expected_ BIGINT;
BEGIN
 IF current_user<>pg_get_userbyid((SELECT relowner FROM pg_class WHERE oid='core_history.connection_snapshot'::regclass))
 OR current_setting('transaction_isolation')<>'serializable' THEN
  RAISE EXCEPTION 'connection import requires serializable offline owner' USING ERRCODE='42501';
 END IF;
 IF NEW.target_database<>current_database()
 OR NEW.target_database_oid<>(SELECT oid FROM pg_database WHERE datname=current_database())
 OR NEW.catalog_sha256<>(SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton)
 OR (SELECT count(*) FROM core_meta.component_contract)<>79 THEN
  RAISE EXCEPTION 'connection target contract mismatch';
 END IF;
 SELECT substring(c FROM length('lmm.migration_instance_nonce=')+1) INTO nonce_
 FROM pg_db_role_setting s CROSS JOIN LATERAL unnest(s.setconfig) c
 WHERE s.setdatabase=NEW.target_database_oid AND s.setrole=0 AND c LIKE 'lmm.migration_instance_nonce=%';
 IF nonce_ IS DISTINCT FROM NEW.target_nonce THEN RAISE EXCEPTION 'connection target nonce mismatch'; END IF;
 IF NOT EXISTS(SELECT FROM core_billing.opening_import_runs r WHERE r.migration_id=NEW.opening_migration_id
 AND r.source_archive_sha256=NEW.source_archive_sha256 AND r.mapping_sha256=NEW.opening_mapping_sha256)
 OR NOT EXISTS(SELECT FROM core_migration.import_runs i WHERE i.migration_id=NEW.identity_migration_id
 AND i.source_archive_sha256=NEW.source_archive_sha256 AND i.manifest->>'scope'='legacy-cutover-identity.v1'
 AND i.manifest->>'source_archive_sha256'=NEW.source_archive_sha256) THEN
  RAISE EXCEPTION 'connection original migration mismatch';
 END IF;
 IF NOT coalesce(d->>'contract'='legacy.connection-source.v1'
 AND NEW.metadata->>'contract'='legacy.connection-export-binding.v1'
 AND NEW.metadata->>'source_archive_sha256'=NEW.source_archive_sha256
 AND NEW.metadata->>'export_file_sha256'=NEW.export_file_sha256
 AND NEW.metadata->>'source_schema'=d->>'source_schema'
 AND (NEW.metadata->>'snapshot_observed_at')::bigint=NEW.snapshot_observed_at
 AND NEW.snapshot_observed_at<=(d->>'observed_at')::bigint
 AND length(NEW.metadata->>'source_snapshot_id') BETWEEN 1 AND 200
 AND d->>'coverage'='all_retained_rows_in_frozen_snapshot_including_soft_deleted_and_revoked'
 AND NEW.metadata->>'coverage'=d->>'coverage'
 AND m->>'contract'='legacy.connection-mapping.v1',false) THEN
  RAISE EXCEPTION 'connection source coverage/binding mismatch';
 END IF;
 FOREACH row_ IN ARRAY ARRAY[d->'users',d->'tokens',d->'oauth_server_grants',d->'developer_access_requests',m->'users',m->'omitted_users'] LOOP
  IF jsonb_typeof(row_) IS DISTINCT FROM 'array' THEN RAISE EXCEPTION 'connection source arrays required'; END IF;
 END LOOP;
 IF (d->'counts'->>'users')::bigint IS DISTINCT FROM jsonb_array_length(d->'users')::bigint
 OR (d->'counts'->>'tokens')::bigint IS DISTINCT FROM jsonb_array_length(d->'tokens')::bigint
 OR (d->'counts'->>'oauth_server_grants')::bigint IS DISTINCT FROM jsonb_array_length(d->'oauth_server_grants')::bigint
 OR (d->'counts'->>'developer_access_requests')::bigint IS DISTINCT FROM jsonb_array_length(d->'developer_access_requests')::bigint
 OR jsonb_array_length(d->'users')<>jsonb_array_length(m->'users')+jsonb_array_length(m->'omitted_users') THEN
  RAISE EXCEPTION 'connection retained cohort/count mismatch';
 END IF;
 IF EXISTS(SELECT FROM jsonb_array_elements(d->'users') u GROUP BY u->>'id' HAVING count(*)<>1)
 OR EXISTS(SELECT FROM (SELECT v->>'source_user_id' id FROM jsonb_array_elements(m->'users') v
 UNION ALL SELECT v->>'source_user_id' FROM jsonb_array_elements(m->'omitted_users') v) ids GROUP BY id HAVING count(*)<>1)
 OR EXISTS(SELECT FROM jsonb_array_elements(m->'users') v GROUP BY v->>'user_id' HAVING count(*)<>1) THEN
  RAISE EXCEPTION 'connection duplicate mapping';
 END IF;
 FOR user_ IN SELECT value FROM jsonb_array_elements(d->'users') LOOP
  source_id_:=(user_->>'id')::bigint;
  SELECT v INTO row_ FROM jsonb_array_elements(m->'users') v WHERE (v->>'source_user_id')::bigint=source_id_;
  IF row_ IS NULL THEN
   IF NOT EXISTS(SELECT FROM jsonb_array_elements(m->'omitted_users') v
     WHERE (v->>'source_user_id')::bigint=source_id_ AND length(btrim(v->>'reason')) BETWEEN 1 AND 500) THEN
    RAISE EXCEPTION 'connection source user missing from mapping';
   END IF;
  ELSE
   IF row_->>'opening_entity_key' NOT IN ('user:'||source_id_::text,'legacy.users/'||source_id_::text)
   OR NOT EXISTS(SELECT FROM core_identity.users u
    JOIN core_identity.accounts a ON a.id=u.personal_account_id
    JOIN core_billing.opening_imports o ON o.account_id=a.id
    JOIN core_migration.import_runs i ON i.migration_id=NEW.identity_migration_id
    WHERE u.id=(row_->>'user_id')::bigint AND a.id=(row_->>'account_id')::bigint
    AND o.migration_id=NEW.opening_migration_id AND o.source_entity_key=row_->>'opening_entity_key'
    AND i.manifest->'registration_times'->(a.id::text)=to_jsonb(coalesce((user_->>'created_at')::bigint,0))
    AND floor(extract(epoch FROM a.created_at))::bigint=coalesce((user_->>'created_at')::bigint,0)) THEN
    RAISE EXCEPTION 'connection personal opening/identity mapping mismatch';
   END IF;
  END IF;
 END LOOP;
 SELECT coalesce(max(id),0) INTO expected_ FROM core_identity.credentials;
 IF NEW.native_credential_floor<>expected_ THEN RAISE EXCEPTION 'connection credential boundary changed'; END IF;
 -- The supported cutover importer preserves token IDs. Check every existing
 -- api_key against its original row and mapped owner, including managed/revoked.
 -- Native keys must be created AFTER this import, never guessed from row status.
 IF EXISTS(SELECT FROM core_identity.credentials c WHERE c.kind='api_key' AND NOT EXISTS(
  SELECT FROM jsonb_array_elements(d->'tokens') t JOIN jsonb_array_elements(m->'users') v
  ON (v->>'source_user_id')::bigint=(t->>'user_id')::bigint
  WHERE (t->>'id')::bigint=c.id AND (v->>'user_id')::bigint=c.user_id
  AND (v->>'account_id')::bigint=c.owner_account_id
  AND coalesce((t->>'created_time')::bigint,0)=floor(extract(epoch FROM c.created_at))::bigint)) THEN
  RAISE EXCEPTION 'connection unsupported or native credential before source boundary';
 END IF;
 RETURN NEW;
END $$;

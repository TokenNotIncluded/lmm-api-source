-- Core59: explicit offline installation, after legacy-payments. No wallet money.
CREATE TABLE core_subscriptions.external_prices (
 plan_id BIGINT NOT NULL,
 plan_revision BIGINT NOT NULL,
 channel_id BIGINT NOT NULL CHECK(channel_id>0),
 currency TEXT NOT NULL CHECK(currency IN ('CNY','LDC')),
 amount_minor BIGINT NOT NULL CHECK(amount_minor BETWEEN 1 AND 9000000000000),
 actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 PRIMARY KEY(plan_id,plan_revision,channel_id),
 FOREIGN KEY(plan_id,plan_revision) REFERENCES core_subscriptions.plan_revisions(plan_id,revision)
);
CREATE TABLE core_subscriptions.external_orders (
 id TEXT PRIMARY KEY CHECK(id ~ '^sub_[a-f0-9]{64}$'),
 actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 account_id BIGINT NOT NULL REFERENCES core_identity.accounts(id),
 operation_key TEXT NOT NULL CHECK(octet_length(operation_key) BETWEEN 16 AND 80),
 plan_id BIGINT NOT NULL,
 plan_revision BIGINT NOT NULL,
 channel_id BIGINT NOT NULL,
 provider TEXT NOT NULL CHECK(provider='epay'),
 merchant TEXT NOT NULL,
 environment TEXT NOT NULL CHECK(environment IN ('test','live')),
 currency TEXT NOT NULL CHECK(currency IN ('CNY','LDC')),
 amount_minor BIGINT NOT NULL CHECK(amount_minor>0),
 channel_binding JSONB NOT NULL CHECK(jsonb_typeof(channel_binding)='object'),
 quoted_at BIGINT NOT NULL,
 expires_at BIGINT NOT NULL,
 confirmed_at BIGINT,
 canceled_at BIGINT,
 UNIQUE(actor_user_id,operation_key),
 FOREIGN KEY(plan_id,plan_revision,channel_id) REFERENCES core_subscriptions.external_prices(plan_id,plan_revision,channel_id),
 CHECK(expires_at>quoted_at)
);
-- Same provider namespace as native and legacy payments, with exactly one owner.
ALTER TABLE core_payments.provider_identifiers ADD COLUMN subscription_order_id TEXT
 REFERENCES core_subscriptions.external_orders(id);
ALTER TABLE core_payments.provider_identifiers DROP CONSTRAINT provider_identifiers_check;
ALTER TABLE core_payments.provider_identifiers ADD CONSTRAINT provider_identifiers_check
 CHECK(num_nonnulls(native_intent_id,legacy_basis_id,subscription_order_id)=1);
CREATE TABLE core_subscriptions.external_proofs (
 id TEXT PRIMARY KEY CHECK(id ~ '^sproof_[a-f0-9]{64}$'),
 order_id TEXT NOT NULL UNIQUE REFERENCES core_subscriptions.external_orders(id),
 event_id TEXT NOT NULL,
 transaction_id TEXT NOT NULL,
 payload_sha256 BYTEA NOT NULL CHECK(octet_length(payload_sha256)=32),
 signed_payload BYTEA NOT NULL CHECK(octet_length(signed_payload) BETWEEN 1 AND 32768),
 signature BYTEA NOT NULL CHECK(octet_length(signature) BETWEEN 1 AND 2048),
 accepted_at BIGINT NOT NULL,
 UNIQUE(order_id,id)
);
CREATE TABLE core_subscriptions.external_grants (
 order_id TEXT PRIMARY KEY REFERENCES core_subscriptions.external_orders(id),
 proof_id TEXT NOT NULL UNIQUE,
 operation_key TEXT NOT NULL CHECK(octet_length(operation_key) BETWEEN 16 AND 80),
 subscription_id BIGINT NOT NULL UNIQUE REFERENCES core_charging.subscriptions(id),
 granted_at BIGINT NOT NULL,
 ends_at BIGINT NOT NULL CHECK(ends_at>granted_at),
 FOREIGN KEY(order_id,proof_id) REFERENCES core_subscriptions.external_proofs(order_id,id)
);
CREATE FUNCTION core_subscriptions.guard_external_order() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE p core_subscriptions.external_prices%ROWTYPE;
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable external order' USING ERRCODE='23514'; END IF;
 IF TG_OP='UPDATE' THEN
  IF (to_jsonb(NEW)-ARRAY['confirmed_at','canceled_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['confirmed_at','canceled_at'])
   OR (OLD.confirmed_at IS NOT NULL AND NEW.confirmed_at IS DISTINCT FROM OLD.confirmed_at)
   OR (OLD.canceled_at IS NOT NULL AND NEW.canceled_at IS DISTINCT FROM OLD.canceled_at)
   OR (OLD.confirmed_at IS NULL AND NEW.confirmed_at IS NOT NULL AND OLD.canceled_at IS NOT NULL) THEN
   RAISE EXCEPTION 'immutable external order binding' USING ERRCODE='23514';
  END IF;
 ELSE
  SELECT * INTO STRICT p FROM core_subscriptions.external_prices WHERE plan_id=NEW.plan_id AND plan_revision=NEW.plan_revision AND channel_id=NEW.channel_id;
  IF p.currency<>NEW.currency OR p.amount_minor<>NEW.amount_minor OR NOT EXISTS (
   SELECT FROM core_identity.users WHERE id=NEW.actor_user_id AND personal_account_id=NEW.account_id
  ) THEN RAISE EXCEPTION 'external order price or owner mismatch' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER external_order_guard BEFORE INSERT OR UPDATE OR DELETE ON core_subscriptions.external_orders
 FOR EACH ROW EXECUTE FUNCTION core_subscriptions.guard_external_order();
CREATE FUNCTION core_subscriptions.external_ends_at(terms JSONB, starts BIGINT) RETURNS BIGINT
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT CASE WHEN coalesce((terms->>'duration_months')::bigint,0)=0
 THEN starts+(terms->>'duration_seconds')::bigint
 ELSE extract(epoch FROM (
  date_trunc('month',to_timestamp(starts) AT TIME ZONE 'UTC')
  + make_interval(months=>(terms->>'duration_months')::integer)
  + ((extract(day FROM to_timestamp(starts) AT TIME ZONE 'UTC')::integer-1)*interval '1 day')
  + ((to_timestamp(starts) AT TIME ZONE 'UTC')::time- time '00:00:00')
 ) AT TIME ZONE 'UTC')::bigint END
$$;
CREATE FUNCTION core_subscriptions.check_external_grant() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE o core_subscriptions.external_orders%ROWTYPE; s core_charging.subscriptions%ROWTYPE;
 p core_subscriptions.external_proofs%ROWTYPE; terms JSONB;
BEGIN
 SELECT * INTO STRICT o FROM core_subscriptions.external_orders WHERE id=NEW.order_id;
 SELECT * INTO STRICT p FROM core_subscriptions.external_proofs WHERE id=NEW.proof_id;
 SELECT * INTO STRICT s FROM core_charging.subscriptions WHERE id=NEW.subscription_id;
 SELECT r.terms INTO STRICT terms FROM core_subscriptions.plan_revisions r WHERE r.plan_id=o.plan_id AND r.revision=o.plan_revision;
 IF o.confirmed_at IS NULL OR p.order_id<>o.id OR s.account_id<>o.account_id
  OR s.grant_reference<>'external-subscription:'||o.id OR s.anchor<>NEW.granted_at OR s.ends_at<>NEW.ends_at
  OR NEW.ends_at<>core_subscriptions.external_ends_at(terms,NEW.granted_at)
  OR s.period<>terms->>'reset_period' OR s.seconds<>(terms->>'reset_seconds')::bigint
  OR s.limit_credits<>(terms->>'limit_units')::bigint OR s.unlimited OR s.reset_basis<>'same'
  OR to_jsonb(s.models)<>terms->'models' OR to_jsonb(s.groups)<>terms->'groups'
  OR NOT EXISTS(SELECT FROM core_payments.provider_identifiers i WHERE i.provider=o.provider AND i.merchant=o.merchant AND i.environment=o.environment AND i.kind='payment' AND i.identifier=p.transaction_id AND i.subscription_order_id=o.id)
 THEN RAISE EXCEPTION 'external grant does not match original payment and plan' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER external_grant_bound AFTER INSERT ON core_subscriptions.external_grants
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION core_subscriptions.check_external_grant();
CREATE TRIGGER external_prices_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.external_prices FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER external_proofs_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.external_proofs FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER external_grants_immutable BEFORE UPDATE OR DELETE ON core_subscriptions.external_grants FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER external_prices_no_truncate BEFORE TRUNCATE ON core_subscriptions.external_prices FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER external_orders_no_truncate BEFORE TRUNCATE ON core_subscriptions.external_orders FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER external_proofs_no_truncate BEFORE TRUNCATE ON core_subscriptions.external_proofs FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER external_grants_no_truncate BEFORE TRUNCATE ON core_subscriptions.external_grants FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TABLE core_meta.subscription_epay_upgrades (
 upgrade_id TEXT PRIMARY KEY,
 request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
 request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
 preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
 preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
 successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
 recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER subscription_epay_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.subscription_epay_upgrades FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER subscription_epay_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.subscription_epay_upgrades FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_subscriptions.external_prices,core_subscriptions.external_orders,core_subscriptions.external_proofs,core_subscriptions.external_grants,core_meta.subscription_epay_upgrades FROM PUBLIC;
REVOKE ALL ON FUNCTION core_subscriptions.guard_external_order(),core_subscriptions.check_external_grant() FROM PUBLIC;

CREATE FUNCTION core_subscriptions.apply_epay_grant(p_order TEXT,p_proof TEXT,p_key TEXT) RETURNS JSONB
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE o core_subscriptions.external_orders%ROWTYPE; g core_subscriptions.external_grants%ROWTYPE;
 terms JSONB; owner_id BIGINT; starts BIGINT; ends BIGINT; sub BIGINT;
BEGIN
 IF current_setting('transaction_isolation')<>'serializable' OR p_key !~ '^[A-Za-z0-9_-]{16,80}$' OR p_key IS NULL THEN
  RAISE EXCEPTION 'serializable transaction and original key required' USING ERRCODE='22023'; END IF;
 SELECT actor_user_id INTO STRICT owner_id FROM core_subscriptions.external_orders WHERE id=p_order;
 PERFORM pg_advisory_xact_lock(hashtextextended('core-subscriptions:'||owner_id::text,0));
 SELECT * INTO STRICT o FROM core_subscriptions.external_orders WHERE id=p_order FOR UPDATE;
 SELECT * INTO g FROM core_subscriptions.external_grants WHERE order_id=p_order;
 IF FOUND THEN
  IF g.proof_id IS DISTINCT FROM p_proof OR g.operation_key IS DISTINCT FROM p_key THEN
   RAISE EXCEPTION 'original grant operation mismatch' USING ERRCODE='23505'; END IF;
  RETURN to_jsonb(g);
 END IF;
 IF o.confirmed_at IS NULL OR NOT EXISTS(SELECT FROM core_subscriptions.external_proofs WHERE order_id=p_order AND id=p_proof) THEN
  RAISE EXCEPTION 'accepted original proof required' USING ERRCODE='23514'; END IF;
 SELECT r.terms INTO STRICT terms FROM core_subscriptions.plan_revisions r WHERE r.plan_id=o.plan_id AND r.revision=o.plan_revision;
 starts:=floor(extract(epoch FROM clock_timestamp()))::bigint;
 ends:=core_subscriptions.external_ends_at(terms,starts);
 INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,models,groups)
 VALUES(o.account_id,'external-subscription:'||o.id,terms->>'reset_period',starts,(terms->>'reset_seconds')::bigint,
 ends,(terms->>'limit_units')::bigint,ARRAY(SELECT jsonb_array_elements_text(terms->'models')),ARRAY(SELECT jsonb_array_elements_text(terms->'groups')))
 RETURNING id INTO sub;
 INSERT INTO core_subscriptions.external_grants(order_id,proof_id,operation_key,subscription_id,granted_at,ends_at)
 VALUES(p_order,p_proof,p_key,sub,starts,ends) RETURNING * INTO g;
 RETURN to_jsonb(g);
END $$;
REVOKE ALL ON FUNCTION core_subscriptions.external_ends_at(JSONB,BIGINT),core_subscriptions.apply_epay_grant(TEXT,TEXT,TEXT) FROM PUBLIC;

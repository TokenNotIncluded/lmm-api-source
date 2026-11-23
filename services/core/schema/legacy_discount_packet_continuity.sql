-- Component 81: current legacy coupons and code-distributing packet rights.
-- Existing payment, promotion and packet tables remain their native authority.
-- Only the offline owner may add frozen source bases. Empty bases preserve the
-- native behavior. Claiming an imported packet distributes a code; it never
-- creates a gift journal, reset event, voucher or payment intent.

CREATE TABLE core_payments.legacy_discount_bases (
    code_id BIGINT PRIMARY KEY REFERENCES core_payments.discount_codes(id),
    import_migration_id TEXT NOT NULL REFERENCES core_migration.import_runs(migration_id)
        DEFERRABLE INITIALLY DEFERRED,
    source_namespace_sha256 BYTEA NOT NULL CHECK(octet_length(source_namespace_sha256)=32),
    source_archive_sha256 BYTEA NOT NULL CHECK(octet_length(source_archive_sha256)=32),
    source_discount_id BIGINT NOT NULL CHECK(source_discount_id>0),
    source_row_sha256 BYTEA NOT NULL CHECK(octet_length(source_row_sha256)=32),
    original_owner_user_id BIGINT REFERENCES core_identity.users(id),
    original_used_count BIGINT NOT NULL CHECK(original_used_count>=0),
    original_min_amount BIGINT NOT NULL CHECK(original_min_amount>=0),
    min_amount_unit TEXT NOT NULL CHECK(min_amount_unit IN ('ledger_quota','legacy_batch_500000')),
    UNIQUE(source_namespace_sha256,source_discount_id),
    UNIQUE(code_id,source_namespace_sha256)
);

CREATE TABLE core_payments.legacy_discount_holds (
    code_id BIGINT NOT NULL,
    source_namespace_sha256 BYTEA NOT NULL CHECK(octet_length(source_namespace_sha256)=32),
    source_order_id BIGINT NOT NULL CHECK(source_order_id>0),
    source_row_sha256 BYTEA NOT NULL CHECK(octet_length(source_row_sha256)=32),
    source_reservation_id BIGINT CHECK(source_reservation_id>0),
    reason TEXT NOT NULL CHECK(reason IN ('reserved_without_confirmed_closed_order',
        'released_order_still_payable','unreserved_order_still_payable')),
    -- A missing historical owner or native liability cannot prove closure.
    -- Such a hold stays reserved; it is never synthesized as a payment intent.
    pending_migration_id TEXT,
    pending_source_table TEXT,
    pending_source_row_key TEXT,
    CHECK(num_nonnulls(pending_migration_id,pending_source_table,pending_source_row_key) IN (0,3)),
    CHECK(pending_source_table IS NULL OR pending_source_table IN ('top_ups','legacy.top_ups')),
    FOREIGN KEY(pending_migration_id,pending_source_table,pending_source_row_key)
        REFERENCES core_billing.legacy_pending_liabilities(migration_id,source_table,source_row_key),
    FOREIGN KEY(code_id,source_namespace_sha256)
        REFERENCES core_payments.legacy_discount_bases(code_id,source_namespace_sha256),
    PRIMARY KEY(source_namespace_sha256,source_order_id)
);

CREATE TABLE core_promotions.legacy_red_packet_bases (
    packet_id BIGINT PRIMARY KEY REFERENCES core_promotions.red_packets(id),
    import_migration_id TEXT NOT NULL REFERENCES core_migration.import_runs(migration_id)
        DEFERRABLE INITIALLY DEFERRED,
    source_namespace_sha256 BYTEA NOT NULL CHECK(octet_length(source_namespace_sha256)=32),
    source_archive_sha256 BYTEA NOT NULL CHECK(octet_length(source_archive_sha256)=32),
    source_packet_id BIGINT NOT NULL CHECK(source_packet_id>0),
    source_row_sha256 BYTEA NOT NULL CHECK(octet_length(source_row_sha256)=32),
    UNIQUE(source_namespace_sha256,source_packet_id),
    UNIQUE(packet_id,source_namespace_sha256)
);

CREATE TABLE core_promotions.legacy_red_packet_items (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    packet_id BIGINT NOT NULL,
    source_namespace_sha256 BYTEA NOT NULL CHECK(octet_length(source_namespace_sha256)=32),
    source_item_id BIGINT NOT NULL CHECK(source_item_id>0),
    source_reward_id BIGINT NOT NULL CHECK(source_reward_id>0),
    source_row_sha256 BYTEA NOT NULL CHECK(octet_length(source_row_sha256)=32),
    source_reward_row_sha256 BYTEA NOT NULL CHECK(octet_length(source_reward_row_sha256)=32),
    reward_kind TEXT NOT NULL CHECK(reward_kind IN ('quota','reset_voucher','discount')),
    code_digest BYTEA NOT NULL CHECK(octet_length(code_digest)=32),
    discount_code_id BIGINT REFERENCES core_payments.legacy_discount_bases(code_id),
    CHECK((reward_kind='discount')=(discount_code_id IS NOT NULL)),
    -- The old product distributes the original bearer code, including for
    -- historical claims. Like the existing imported claim snapshot, this is
    -- private database data and may only be returned to its actual claimant.
    reward JSONB NOT NULL CHECK(jsonb_typeof(reward)='object' AND octet_length(reward::text)<=4096),
    CHECK(coalesce(jsonb_typeof(reward->'code')='string',FALSE)),
    CHECK(coalesce((reward_kind='discount' AND reward->>'item_type'='discount') OR
          (reward_kind<>'discount' AND reward->>'item_type'='redemption'),FALSE)),
    CHECK(code_digest=sha256(
        CASE WHEN reward_kind='reset_voucher' THEN
            convert_to('core.subscription-reset-code.v1','UTF8')||decode('00','hex')
        ELSE ''::bytea END || convert_to(reward->>'code','UTF8'))),
    source_admission_enabled BOOLEAN NOT NULL,
    weight INTEGER NOT NULL CHECK(weight BETWEEN 1 AND 1000000),
    created_at BIGINT NOT NULL,
    released_at BIGINT,
    FOREIGN KEY(packet_id,source_namespace_sha256)
        REFERENCES core_promotions.legacy_red_packet_bases(packet_id,source_namespace_sha256),
    FOREIGN KEY(discount_code_id,source_namespace_sha256)
        REFERENCES core_payments.legacy_discount_bases(code_id,source_namespace_sha256),
    UNIQUE(source_namespace_sha256,source_item_id)
);
CREATE UNIQUE INDEX legacy_red_packet_item_live_source ON core_promotions.legacy_red_packet_items
    (source_namespace_sha256,(CASE WHEN reward_kind='discount' THEN 'discount' ELSE 'redemption' END),source_reward_id)
    WHERE released_at IS NULL;
CREATE UNIQUE INDEX legacy_red_packet_item_live_discount ON core_promotions.legacy_red_packet_items(discount_code_id)
    WHERE discount_code_id IS NOT NULL AND released_at IS NULL;
CREATE INDEX legacy_red_packet_items_packet ON core_promotions.legacy_red_packet_items(packet_id,id);
CREATE UNIQUE INDEX legacy_red_packet_item_live_code ON core_promotions.legacy_red_packet_items(code_digest)
    WHERE reward_kind IN ('quota','reset_voucher') AND released_at IS NULL;

CREATE TABLE core_promotions.legacy_red_packet_claim_links (
    claim_id BIGINT PRIMARY KEY REFERENCES core_promotions.red_packet_claims(id),
    item_id BIGINT NOT NULL UNIQUE REFERENCES core_promotions.legacy_red_packet_items(id),
    -- Only owner import may populate these source-history columns. Runtime
    -- receives INSERT on claim_id,item_id and cannot label a new claim as old.
    source_claim_id BIGINT CHECK(source_claim_id>0),
    source_namespace_sha256 BYTEA CHECK(octet_length(source_namespace_sha256)=32),
    source_row_sha256 BYTEA CHECK(octet_length(source_row_sha256)=32),
    CHECK(num_nonnulls(source_claim_id,source_namespace_sha256,source_row_sha256) IN (0,3)),
    UNIQUE(source_namespace_sha256,source_claim_id)
);

CREATE FUNCTION core_promotions.guard_legacy_packet_claim_link() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE i core_promotions.legacy_red_packet_items%ROWTYPE;
        c core_promotions.red_packet_claims%ROWTYPE;
        expected_code TEXT;
        original_owner BIGINT;
BEGIN
    SELECT * INTO STRICT i FROM core_promotions.legacy_red_packet_items WHERE id=NEW.item_id;
    SELECT * INTO STRICT c FROM core_promotions.red_packet_claims WHERE id=NEW.claim_id;
    expected_code:=i.reward->>'code';
    IF NEW.source_claim_id IS NULL AND i.reward_kind='discount' THEN
        SELECT code INTO STRICT expected_code FROM core_payments.discount_codes WHERE id=i.discount_code_id;
    END IF;
    IF i.reward_kind='discount' THEN
        SELECT original_owner_user_id INTO STRICT original_owner
          FROM core_payments.legacy_discount_bases WHERE code_id=i.discount_code_id;
        IF original_owner IS NOT NULL AND (NEW.source_claim_id IS NULL OR original_owner<>c.user_id) THEN
            RAISE EXCEPTION 'a private source discount cannot be reassigned by a packet' USING ERRCODE='23514';
        END IF;
    END IF;
    IF c.packet_id<>i.packet_id OR c.item_id IS NOT NULL OR c.promotion_claim_id IS NOT NULL
       OR i.released_at IS NOT NULL
       OR c.reward->>'code' IS DISTINCT FROM expected_code
       OR c.reward->>'item_type' IS DISTINCT FROM i.reward->>'item_type'
       OR (NEW.source_claim_id IS NOT NULL AND NEW.source_namespace_sha256<>i.source_namespace_sha256) THEN
        RAISE EXCEPTION 'legacy packet claim must bind its distributed code without a grant' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER legacy_packet_claim_link_bound BEFORE INSERT ON core_promotions.legacy_red_packet_claim_links
    FOR EACH ROW EXECUTE FUNCTION core_promotions.guard_legacy_packet_claim_link();

CREATE FUNCTION core_promotions.guard_legacy_packet_item() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP='INSERT' THEN
        IF NEW.reward_kind='quota' AND EXISTS(SELECT FROM core_promotions.red_packet_items
            WHERE code_digest=NEW.code_digest AND released_at IS NULL) THEN
            RAISE EXCEPTION 'code is already bound to a native packet' USING ERRCODE='23514';
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP='DELETE' OR (to_jsonb(NEW)-'released_at') IS DISTINCT FROM (to_jsonb(OLD)-'released_at')
       OR OLD.released_at IS NOT NULL OR NEW.released_at IS NULL
       OR EXISTS(SELECT FROM core_promotions.legacy_red_packet_claim_links WHERE item_id=OLD.id) THEN
        RAISE EXCEPTION 'legacy packet item is immutable after claim or release' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER legacy_packet_item_identity BEFORE INSERT OR UPDATE OR DELETE ON core_promotions.legacy_red_packet_items
    FOR EACH ROW EXECUTE FUNCTION core_promotions.guard_legacy_packet_item();

CREATE FUNCTION core_promotions.guard_native_packet_continuation_binding() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF EXISTS(SELECT FROM core_promotions.legacy_red_packet_items
              WHERE reward_kind='quota' AND code_digest=NEW.code_digest AND released_at IS NULL) THEN
        RAISE EXCEPTION 'code is already bound to an imported packet' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER native_packet_continuation_binding BEFORE INSERT ON core_promotions.red_packet_items
    FOR EACH ROW EXECUTE FUNCTION core_promotions.guard_native_packet_continuation_binding();

-- A packet reader must serialize with ordinary discount checkout. Taking a
-- row lock ordinarily requires UPDATE permission; this closed helper grants
-- only the lock, with no ability to rewrite any coupon terms or source facts.
CREATE FUNCTION core_payments.lock_legacy_discount_code(code BIGINT) RETURNS BOOLEAN
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
    PERFORM d.id FROM core_payments.discount_codes d
      JOIN core_payments.legacy_discount_bases b ON b.code_id=d.id
      WHERE d.id=$1 FOR UPDATE OF d;
    RETURN FOUND;
END $$;

CREATE VIEW core_payments.legacy_discount_owners WITH(security_invoker=true) AS
    SELECT b.code_id,coalesce(b.original_owner_user_id,c.user_id) AS owner_user_id
    FROM core_payments.legacy_discount_bases b
    LEFT JOIN LATERAL (
        SELECT c.user_id FROM core_promotions.legacy_red_packet_items i
        JOIN core_promotions.legacy_red_packet_claim_links l ON l.item_id=i.id
        JOIN core_promotions.red_packet_claims c ON c.id=l.claim_id
        WHERE i.discount_code_id=b.code_id
    ) c ON TRUE;

-- A terminal provider receipt or a validated legacy posting is current state;
-- the immutable original liability's 'unknown' is only the fallback. Conflicts
-- and unbound old orders keep capacity. A posted use is not counted twice as
-- both a held slot and a consumed slot, and never becomes reusable on refund.
CREATE VIEW core_payments.legacy_discount_hold_state WITH(security_invoker=true) AS
    SELECT h.code_id,h.source_order_id,
        CASE WHEN EXISTS(SELECT FROM core_payments.legacy_discount_consumptions a
                         WHERE a.source_archive_sha256=p.source_archive_sha256
                           AND a.source_order_id=h.source_order_id
                           AND a.discount_code_id=b.source_discount_id)
               OR EXISTS(SELECT FROM core_billing.legacy_pending_liability_receipts r
                         WHERE (r.migration_id,r.source_table,r.source_row_key)=
                               (h.pending_migration_id,h.pending_source_table,h.pending_source_row_key)
                           AND r.settlement_state='terminal_paid')
             THEN 'consumed'
             WHEN current_fact.settlement_state='terminal_no_payment' THEN 'released'
             ELSE 'reserved' END AS state
    FROM core_payments.legacy_discount_holds h
    JOIN core_payments.legacy_discount_bases b ON b.code_id=h.code_id
    LEFT JOIN core_billing.legacy_pending_liabilities p
        ON (p.migration_id,p.source_table,p.source_row_key)=
           (h.pending_migration_id,h.pending_source_table,h.pending_source_row_key)
    LEFT JOIN LATERAL (
        SELECT CASE WHEN count(*)=0 THEN p.settlement_state
                    WHEN count(DISTINCT r.settlement_state)=1 THEN min(r.settlement_state)
                    ELSE 'manual_review' END AS settlement_state
        FROM core_billing.legacy_pending_liability_receipts r
        WHERE (r.migration_id,r.source_table,r.source_row_key)=
              (h.pending_migration_id,h.pending_source_table,h.pending_source_row_key)
          AND r.observed_at=(SELECT max(t.observed_at) FROM core_billing.legacy_pending_liability_receipts t
                            WHERE (t.migration_id,t.source_table,t.source_row_key)=
                                  (h.pending_migration_id,h.pending_source_table,h.pending_source_row_key))
    ) current_fact ON TRUE;

CREATE VIEW core_payments.discount_code_usage WITH(security_invoker=true) AS
    SELECT d.id AS code_id,
        coalesce(b.original_used_count,0)
          +(SELECT count(*) FROM core_payments.discount_reservations r WHERE r.code_id=d.id AND r.consumed)
          +(SELECT count(*) FROM core_payments.legacy_discount_hold_state h WHERE h.code_id=d.id AND h.state='consumed') AS used_count,
        (SELECT count(*) FROM core_payments.discount_reservations r WHERE r.code_id=d.id AND NOT r.consumed)
          +(SELECT count(*) FROM core_payments.legacy_discount_hold_state h WHERE h.code_id=d.id AND h.state='reserved') AS reserved_count
    FROM core_payments.discount_codes d LEFT JOIN core_payments.legacy_discount_bases b ON b.code_id=d.id;

DO $immutable$ DECLARE relation TEXT;
BEGIN
    FOREACH relation IN ARRAY ARRAY['core_payments.legacy_discount_bases','core_payments.legacy_discount_holds',
        'core_promotions.legacy_red_packet_bases','core_promotions.legacy_red_packet_claim_links'] LOOP
        EXECUTE format('CREATE TRIGGER legacy_continuation_immutable BEFORE UPDATE OR DELETE ON %s FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable()',relation);
    END LOOP;
    FOREACH relation IN ARRAY ARRAY['core_payments.legacy_discount_bases','core_payments.legacy_discount_holds',
        'core_promotions.legacy_red_packet_bases','core_promotions.legacy_red_packet_items','core_promotions.legacy_red_packet_claim_links'] LOOP
        EXECUTE format('CREATE TRIGGER legacy_continuation_no_truncate BEFORE TRUNCATE ON %s FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable()',relation);
    END LOOP;
END $immutable$;
REVOKE ALL ON core_payments.legacy_discount_bases,core_payments.legacy_discount_holds,
    core_promotions.legacy_red_packet_bases,core_promotions.legacy_red_packet_items,
    core_promotions.legacy_red_packet_claim_links,core_payments.legacy_discount_owners,
    core_payments.legacy_discount_hold_state,core_payments.discount_code_usage FROM PUBLIC;
REVOKE ALL ON SEQUENCE core_promotions.legacy_red_packet_items_id_seq FROM PUBLIC;
REVOKE ALL ON FUNCTION core_promotions.guard_legacy_packet_claim_link(),core_promotions.guard_legacy_packet_item(),
    core_promotions.guard_native_packet_continuation_binding(),core_payments.lock_legacy_discount_code(BIGINT) FROM PUBLIC;

CREATE TABLE core_meta.legacy_discount_packet_upgrades (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
    request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
    preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER legacy_discount_packet_upgrades_immutable BEFORE UPDATE OR DELETE
    ON core_meta.legacy_discount_packet_upgrades FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER legacy_discount_packet_upgrades_no_truncate BEFORE TRUNCATE
    ON core_meta.legacy_discount_packet_upgrades FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.legacy_discount_packet_upgrades FROM PUBLIC;

-- Preserve the original source import guard at the complete current assembly.
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
 OR (SELECT count(*) FROM core_meta.component_contract)<>81 THEN
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

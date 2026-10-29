-- Installed only in a separately administered authority database, never init-db.
CREATE SCHEMA lmm_upgrade_history;
CREATE TABLE lmm_upgrade_history.installation (
    singleton BOOLEAN PRIMARY KEY DEFAULT true CHECK(singleton),
    authority_id TEXT NOT NULL CHECK(authority_id ~ '^[0-9a-f]{64}$'),
    contract_sha256 TEXT NOT NULL CHECK(contract_sha256 ~ '^[0-9a-f]{64}$'),
    writer NAME NOT NULL
);
CREATE TABLE lmm_upgrade_history.operations (
    site_id TEXT PRIMARY KEY CHECK(length(site_id) BETWEEN 16 AND 128),
    operation_id TEXT NOT NULL CHECK(length(operation_id) BETWEEN 16 AND 128),
    binding JSONB NOT NULL,
    sequence BIGINT NOT NULL CHECK(sequence BETWEEN 1 AND 4),
    phase TEXT NOT NULL CHECK(phase IN ('closing','held','started','committed')),
    fence_sha256 TEXT CHECK(fence_sha256 ~ '^[0-9a-f]{64}$'),
    receipt_sha256 TEXT CHECK(receipt_sha256 ~ '^[0-9a-f]{64}$'),
    CHECK((sequence=1 AND phase='closing' AND fence_sha256 IS NULL AND receipt_sha256 IS NULL)
       OR (sequence=2 AND phase='held' AND fence_sha256 IS NOT NULL AND receipt_sha256 IS NULL)
       OR (sequence=3 AND phase='started' AND fence_sha256 IS NOT NULL AND receipt_sha256 IS NULL)
       OR (sequence=4 AND phase='committed' AND fence_sha256 IS NOT NULL AND receipt_sha256 IS NOT NULL))
);
CREATE FUNCTION lmm_upgrade_history.identity() RETURNS JSONB
LANGUAGE SQL SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('authority_id',i.authority_id,'contract_sha256',i.contract_sha256,
   'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
   'database',current_database(),'database_oid',(SELECT oid::bigint FROM pg_database WHERE datname=current_database()),
   'writer',i.writer::text,'owner',current_user,
   'primary',NOT pg_is_in_recovery(),'fsync',current_setting('fsync'),
   'full_page_writes',current_setting('full_page_writes'))
 FROM lmm_upgrade_history.installation i
 WHERE singleton AND i.writer=session_user
$$;
CREATE FUNCTION lmm_upgrade_history.snapshot(p_site TEXT) RETURNS JSONB
LANGUAGE SQL SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT to_jsonb(o) FROM lmm_upgrade_history.operations o
 WHERE site_id=p_site AND EXISTS(SELECT FROM lmm_upgrade_history.installation WHERE writer=session_user)
$$;
CREATE FUNCTION lmm_upgrade_history.reserve(p_binding JSONB) RETURNS JSONB
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE o lmm_upgrade_history.operations;
BEGIN
 IF NOT EXISTS(SELECT FROM lmm_upgrade_history.installation WHERE writer=session_user)
    OR current_setting('synchronous_commit')<>'on' THEN
   RAISE EXCEPTION 'UPGRADE_HISTORY_WRITER_REQUIRED';
 END IF;
 INSERT INTO lmm_upgrade_history.operations(site_id,operation_id,binding,sequence,phase)
 VALUES(p_binding->>'site_id',p_binding->>'operation_id',p_binding,1,'closing')
 ON CONFLICT(site_id) DO NOTHING;
 SELECT * INTO STRICT o FROM lmm_upgrade_history.operations WHERE site_id=p_binding->>'site_id' FOR UPDATE;
 IF o.binding<>p_binding THEN RAISE EXCEPTION 'UPGRADE_HISTORY_BINDING_CONFLICT'; END IF;
 RETURN to_jsonb(o);
END
$$;
CREATE FUNCTION lmm_upgrade_history.advance(p_binding JSONB,p_sequence BIGINT,p_fact TEXT) RETURNS JSONB
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE o lmm_upgrade_history.operations;
BEGIN
 IF NOT EXISTS(SELECT FROM lmm_upgrade_history.installation WHERE writer=session_user)
    OR current_setting('synchronous_commit')<>'on' THEN
   RAISE EXCEPTION 'UPGRADE_HISTORY_WRITER_REQUIRED';
 END IF;
 SELECT * INTO o FROM lmm_upgrade_history.operations WHERE site_id=p_binding->>'site_id' FOR UPDATE;
 IF NOT FOUND OR o.binding<>p_binding THEN RAISE EXCEPTION 'UPGRADE_HISTORY_BINDING_CONFLICT'; END IF;
 IF o.sequence<>p_sequence OR p_sequence NOT BETWEEN 1 AND 3 THEN
   RAISE EXCEPTION 'UPGRADE_HISTORY_CAS_CONFLICT';
 END IF;
 IF p_sequence IN (1,3) AND (p_fact IS NULL OR p_fact !~ '^[0-9a-f]{64}$') THEN
   RAISE EXCEPTION 'UPGRADE_HISTORY_FACT_REQUIRED';
 END IF;
 IF p_sequence=2 AND p_fact IS NOT NULL THEN RAISE EXCEPTION 'UPGRADE_HISTORY_UNEXPECTED_FACT'; END IF;
 UPDATE lmm_upgrade_history.operations SET sequence=p_sequence+1,
   phase=CASE p_sequence WHEN 1 THEN 'held' WHEN 2 THEN 'started' WHEN 3 THEN 'committed' END,
   fence_sha256=CASE WHEN p_sequence=1 THEN p_fact ELSE fence_sha256 END,
   receipt_sha256=CASE WHEN p_sequence=3 THEN p_fact ELSE NULL END
 WHERE site_id=o.site_id RETURNING * INTO o;
 RETURN to_jsonb(o);
END
$$;
REVOKE ALL ON SCHEMA lmm_upgrade_history FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA lmm_upgrade_history FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA lmm_upgrade_history FROM PUBLIC;

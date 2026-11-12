-- Extension-owned business state; no connection, FK or write into Core storage.
CREATE SCHEMA lmm_checkins;
CREATE TABLE lmm_checkins.contract(singleton boolean PRIMARY KEY CHECK(singleton),version integer NOT NULL CHECK(version=1));
INSERT INTO lmm_checkins.contract VALUES(true,1);
CREATE TABLE lmm_checkins.policy_revisions(
 revision bigint PRIMARY KEY CHECK(revision>0), policy jsonb NOT NULL CHECK(jsonb_typeof(policy)='object'),
 operation_key text NOT NULL UNIQUE CHECK(octet_length(operation_key) BETWEEN 16 AND 80),
 created_at bigint NOT NULL CHECK(created_at>=0)
);
CREATE TABLE lmm_checkins.active_policy(singleton boolean PRIMARY KEY CHECK(singleton),revision bigint NOT NULL REFERENCES lmm_checkins.policy_revisions);
CREATE TABLE lmm_checkins.intentions(
 account_id bigint NOT NULL CHECK(account_id>0),user_id bigint NOT NULL CHECK(user_id>0),day date NOT NULL,
 operation_key text NOT NULL CHECK(octet_length(operation_key) BETWEEN 16 AND 80),
 amount_units bigint NOT NULL CHECK(amount_units>=0),trust_level integer NOT NULL CHECK(trust_level BETWEEN 0 AND 6),
 policy_revision bigint NOT NULL REFERENCES lmm_checkins.policy_revisions,
 created_at bigint NOT NULL CHECK(created_at>=0),state text NOT NULL CHECK(state IN ('pending','settled')),
 record jsonb, PRIMARY KEY(account_id,day),UNIQUE(user_id,day),UNIQUE(user_id,account_id,day),
 CHECK((state='pending' AND record IS NULL) OR (state='settled' AND jsonb_typeof(record)='object'))
);
CREATE TABLE lmm_checkins.claim_commands(
 user_id bigint NOT NULL CHECK(user_id>0),operation_key text NOT NULL CHECK(octet_length(operation_key) BETWEEN 16 AND 80),
 account_id bigint NOT NULL,day date NOT NULL,result jsonb,
 PRIMARY KEY(user_id,operation_key),FOREIGN KEY(user_id,account_id,day) REFERENCES lmm_checkins.intentions(user_id,account_id,day),
 CHECK(result IS NULL OR jsonb_typeof(result)='object')
);
CREATE FUNCTION lmm_checkins.immutable() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$ BEGIN RAISE EXCEPTION 'immutable checkin record' USING ERRCODE='23514'; END $$;
CREATE TRIGGER policy_immutable BEFORE UPDATE OR DELETE ON lmm_checkins.policy_revisions FOR EACH ROW EXECUTE FUNCTION lmm_checkins.immutable();
CREATE TRIGGER policy_no_truncate BEFORE TRUNCATE ON lmm_checkins.policy_revisions FOR EACH STATEMENT EXECUTE FUNCTION lmm_checkins.immutable();
CREATE FUNCTION lmm_checkins.guard_intention() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='INSERT' THEN
  IF NEW.state<>'pending' OR NEW.record IS NOT NULL THEN RAISE EXCEPTION 'invalid initial checkin intention' USING ERRCODE='23514'; END IF; RETURN NEW;
 END IF;
 IF OLD.state='settled' OR (to_jsonb(NEW)-'state'-'record') IS DISTINCT FROM (to_jsonb(OLD)-'state'-'record') OR NEW.state<>'settled' THEN
  RAISE EXCEPTION 'immutable checkin intention' USING ERRCODE='23514';
 END IF;
 IF (NEW.record->>'checkin_date') IS DISTINCT FROM NEW.day::text
 OR (NEW.record->>'quota_awarded') IS DISTINCT FROM NEW.amount_units::text
 OR (NEW.record->>'policy_revision')::bigint IS DISTINCT FROM NEW.policy_revision
 OR (NEW.record->>'trust_level')::integer IS DISTINCT FROM NEW.trust_level
 OR ((NEW.amount_units=0) <> (NEW.record->>'journal_id' IS NULL)) THEN
  RAISE EXCEPTION 'checkin receipt mismatch' USING ERRCODE='23514';
 END IF; RETURN NEW;
END $$;
CREATE TRIGGER intention_transition BEFORE INSERT OR UPDATE ON lmm_checkins.intentions FOR EACH ROW EXECUTE FUNCTION lmm_checkins.guard_intention();
CREATE TRIGGER intention_no_delete BEFORE DELETE ON lmm_checkins.intentions FOR EACH ROW EXECUTE FUNCTION lmm_checkins.immutable();
CREATE TRIGGER intention_no_truncate BEFORE TRUNCATE ON lmm_checkins.intentions FOR EACH STATEMENT EXECUTE FUNCTION lmm_checkins.immutable();
CREATE FUNCTION lmm_checkins.guard_command() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN IF TG_OP='INSERT' THEN IF NEW.result IS NOT NULL THEN RAISE EXCEPTION 'invalid initial checkin command' USING ERRCODE='23514'; END IF; RETURN NEW; END IF;
IF OLD.result IS NOT NULL OR (to_jsonb(NEW)-'result') IS DISTINCT FROM (to_jsonb(OLD)-'result') OR NEW.result IS NULL THEN
 RAISE EXCEPTION 'immutable checkin command' USING ERRCODE='23514'; END IF;
 IF NOT EXISTS(SELECT FROM lmm_checkins.intentions i WHERE i.user_id=NEW.user_id AND i.account_id=NEW.account_id AND i.day=NEW.day AND i.state='settled' AND NEW.result->'record'=i.record AND NEW.result->'already_claimed'=to_jsonb(NEW.operation_key<>i.operation_key)) THEN
  RAISE EXCEPTION 'checkin command receipt mismatch' USING ERRCODE='23514';
 END IF; RETURN NEW; END $$;
CREATE TRIGGER command_transition BEFORE INSERT OR UPDATE ON lmm_checkins.claim_commands FOR EACH ROW EXECUTE FUNCTION lmm_checkins.guard_command();
CREATE TRIGGER command_no_delete BEFORE DELETE ON lmm_checkins.claim_commands FOR EACH ROW EXECUTE FUNCTION lmm_checkins.immutable();
CREATE TRIGGER command_no_truncate BEFORE TRUNCATE ON lmm_checkins.claim_commands FOR EACH STATEMENT EXECUTE FUNCTION lmm_checkins.immutable();
REVOKE ALL ON SCHEMA lmm_checkins FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA lmm_checkins FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA lmm_checkins FROM PUBLIC;

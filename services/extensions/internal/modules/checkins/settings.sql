-- Runtime changes only this policy through a bounded owner function.
-- Core independently authorizes every monetary posting and its daily ceiling.
CREATE FUNCTION lmm_checkins.save_settings(
 expected_revision bigint, operation text, enabled boolean, minimum text, maximum text
) RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE
 current_revision bigint := 0;
 prior_revision bigint;
 prior jsonb;
 next_policy jsonb;
BEGIN
 IF expected_revision IS NULL OR expected_revision < 0 OR expected_revision >= 9223372036854775807
 OR operation IS NULL OR octet_length(operation) NOT BETWEEN 16 AND 80
 OR operation !~ '^admin-[1-9][0-9]*-[A-Za-z0-9_-]{16,40}$'
 OR enabled IS NULL OR minimum IS NULL OR maximum IS NULL
 OR minimum !~ '^(0|[1-9][0-9]{0,18})$' OR maximum !~ '^(0|[1-9][0-9]{0,18})$'
 OR minimum::numeric > 9223372036854775807 OR maximum::numeric > 9223372036854775807
 OR minimum::numeric > maximum::numeric THEN
  RAISE EXCEPTION 'invalid checkin settings' USING ERRCODE='22023';
 END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('lmm-checkins-owner',0));
 SELECT revision,policy INTO prior_revision,prior FROM lmm_checkins.policy_revisions WHERE operation_key=operation;
 IF FOUND THEN
  IF prior_revision<>expected_revision+1 OR prior->'enabled'<>to_jsonb(enabled)
  OR prior->>'min_quota'<>minimum OR prior->>'max_quota'<>maximum THEN
   RAISE EXCEPTION 'checkin settings operation conflict' USING ERRCODE='40001';
  END IF;
  RETURN prior_revision;
 END IF;
 SELECT p.revision,p.policy INTO current_revision,next_policy
 FROM lmm_checkins.active_policy a JOIN lmm_checkins.policy_revisions p ON p.revision=a.revision WHERE a.singleton;
 IF NOT FOUND THEN
  current_revision := 0;
  next_policy := jsonb_build_object(
   'calendar_offset_seconds',28800,'credits_per_usd',500000,
   'source_revision','cc52aa195c6f2a01d703a1a407be73c31b89867d',
   'source_options_sha256','','history_seed_id',''
  );
 END IF;
 IF current_revision<>expected_revision THEN
  RAISE EXCEPTION 'checkin settings revision conflict' USING ERRCODE='40001';
 END IF;
 next_policy := (next_policy - 'level_multipliers') || jsonb_build_object(
  'enabled',enabled,'min_quota',minimum,'max_quota',maximum
 );
 INSERT INTO lmm_checkins.policy_revisions(revision,policy,operation_key,created_at)
 VALUES(current_revision+1,next_policy,operation,floor(extract(epoch FROM transaction_timestamp()))::bigint);
 INSERT INTO lmm_checkins.active_policy(singleton,revision) VALUES(true,current_revision+1)
 ON CONFLICT(singleton) DO UPDATE SET revision=excluded.revision;
 RETURN current_revision+1;
END $$;
REVOKE ALL ON FUNCTION lmm_checkins.save_settings(bigint,text,boolean,text,text) FROM PUBLIC;

-- Component 54. Existing pending contracts only; no signup or historical replay.
-- Raw source rows retain their original units; reviewed credit counters are separate.
CREATE SCHEMA core_referrals;
CREATE TABLE core_referrals.source (
 archive_sha256 TEXT PRIMARY KEY CHECK(archive_sha256 ~ '^[a-f0-9]{64}$'),
 migration_id TEXT NOT NULL REFERENCES core_billing.opening_import_runs(migration_id),
 document JSONB NOT NULL,
 review JSONB NOT NULL,
 imported_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE core_referrals.bindings (
 user_id BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
 -- Original legacy binding may name an omitted/deleted user; never executable authority.
 inviter_id BIGINT,
 legacy_first_top_up BIGINT NOT NULL,
 stripe_customer TEXT NOT NULL,
 prior_cash BOOLEAN NOT NULL,
 archive_sha256 TEXT NOT NULL REFERENCES core_referrals.source(archive_sha256),
 source_user_id BIGINT NOT NULL
);
CREATE TABLE core_referrals.contracts (
 id BIGINT PRIMARY KEY,
 invitee BIGINT NOT NULL UNIQUE REFERENCES core_referrals.bindings(user_id),
 inviter BIGINT NOT NULL REFERENCES core_identity.users(id),
 policy JSONB NOT NULL,
 -- Explicit reviewed conversion to current credit_500k_usd; no implicit scaling.
 registration_credit BIGINT NOT NULL CHECK(registration_credit>=0),
 reward_credit BIGINT NOT NULL CHECK(reward_credit>=0),
 max_reward_credit BIGINT NOT NULL CHECK(max_reward_credit>=0),
 min_top_up_credit BIGINT NOT NULL CHECK(min_top_up_credit>=0),
 max_penalty_credit BIGINT NOT NULL CHECK(max_penalty_credit>=0),
 penalty_percent INT NOT NULL CHECK(penalty_percent BETWEEN 0 AND 100),
 stage TEXT NOT NULL CHECK(stage IN ('awaiting_first_top_up','first_top_up_completed','first_top_up_ineligible')),
 status TEXT NOT NULL CHECK(status IN ('earned','revoked')),
 reason TEXT NOT NULL,
 quota BIGINT NOT NULL CHECK(quota>=0 AND quota<=9007199254740991),
 revoked BIGINT NOT NULL DEFAULT 0 CHECK(revoked>=0),
 penalty BIGINT NOT NULL DEFAULT 0 CHECK(penalty>=0),
 revision BIGINT NOT NULL DEFAULT 0,
 first_source TEXT UNIQUE,
 paid_micros BIGINT,
 refunded_micros BIGINT NOT NULL DEFAULT 0 CHECK(refunded_micros>=0),
 archive_sha256 TEXT NOT NULL REFERENCES core_referrals.source(archive_sha256)
);
CREATE TABLE core_referrals.history (
 inviter BIGINT PRIMARY KEY REFERENCES core_identity.users(id),
 granted BIGINT NOT NULL CHECK(granted>=0 AND granted<=9007199254740991)
);
CREATE TABLE core_referrals.events (
 id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 reward_id BIGINT NOT NULL REFERENCES core_referrals.contracts(id),
 kind TEXT NOT NULL,
 delta BIGINT NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE core_referrals.moderation (
 request_id TEXT PRIMARY KEY CHECK(length(request_id) BETWEEN 1 AND 100),
 actor BIGINT NOT NULL REFERENCES core_identity.users(id),
 user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
 action TEXT NOT NULL CHECK(action IN ('ban_abuse','restore_referral')),
 reason TEXT NOT NULL,
 evidence TEXT NOT NULL CHECK(length(btrim(evidence)) BETWEEN 1 AND 1000),
 penalize BOOLEAN NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- No runtime table writes, including contract import. All effects use bounded functions.
REVOKE ALL ON SCHEMA core_referrals FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_referrals FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_referrals FROM PUBLIC;
CREATE TRIGGER referral_source_immutable BEFORE UPDATE OR DELETE ON core_referrals.source
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER referral_binding_immutable BEFORE UPDATE OR DELETE ON core_referrals.bindings
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER referral_events_immutable BEFORE UPDATE OR DELETE ON core_referrals.events
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER referral_moderation_immutable BEFORE UPDATE OR DELETE ON core_referrals.moderation
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();

-- Private ledger source. Amount/account cannot be chosen by a runtime caller.
CREATE FUNCTION core_referrals.delta(p_reward BIGINT,p_kind TEXT,p_delta BIGINT) RETURNS VOID
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE r core_referrals.contracts%ROWTYPE; a BIGINT; c BIGINT; j BIGINT; k TEXT; result JSONB; ev BIGINT;
BEGIN
 IF p_delta=0 THEN RETURN; END IF;
 SELECT * INTO STRICT r FROM core_referrals.contracts WHERE id=p_reward FOR UPDATE;
 SELECT personal_account_id INTO STRICT a FROM core_identity.users WHERE id=r.inviter;
 PERFORM core_billing.open_ledger_wallet(a);
 SELECT id INTO STRICT a FROM core_billing.ledger_accounts WHERE owner_account_id=a AND bucket='affiliate';
 SELECT id INTO STRICT c FROM core_billing.ledger_accounts WHERE bucket='clearing';
 PERFORM ledger_account_id FROM core_billing.balance_state WHERE ledger_account_id IN (a,c)
 ORDER BY ledger_account_id FOR NO KEY UPDATE;
 INSERT INTO core_referrals.events(reward_id,kind,delta) VALUES(r.id,p_kind,p_delta) RETURNING id INTO ev;
 k:='referral-'||ev::text;
 INSERT INTO core_billing.ledger_journals(scope,operation_key,kind,payer_id,payee_id,amount_units,reason)
 VALUES('core-legacy-referral',k,CASE WHEN p_delta>0 THEN 'affiliate_award' ELSE 'affiliate_clawback' END,
 CASE WHEN p_delta>0 THEN c ELSE a END,CASE WHEN p_delta>0 THEN a ELSE c END,abs(p_delta),p_kind) RETURNING id INTO j;
 INSERT INTO core_billing.ledger_entries(journal_id,ledger_account_id,delta_units)
 SELECT j,x.id,x.units FROM (VALUES(a,p_delta),(c,-p_delta)) x(id,units) ORDER BY x.id;
 SELECT jsonb_build_object('status','posted','journal_id',j,'entries',jsonb_agg(jsonb_build_object(
 'ledger_account_id',ledger_account_id,'delta_units',delta_units,'balance_after_units',balance_after_units,
 'balance_revision',balance_revision) ORDER BY ledger_account_id)) INTO result FROM core_billing.ledger_entries WHERE journal_id=j;
 INSERT INTO core_billing.ledger_operations(scope,operation_key,request,result)
 VALUES('core-legacy-referral',k,jsonb_build_object('reward',r.id,'kind',p_kind,'delta',p_delta),result);
 -- Event's journal must be known on insertion; insert a replacement is not allowed.
 -- journal linkage is maintained by a separate source index below.
 INSERT INTO core_referrals.event_journals VALUES(ev,j);
END $$;
CREATE TABLE core_referrals.event_journals (
 event_id BIGINT PRIMARY KEY REFERENCES core_referrals.events(id),
 journal_id BIGINT NOT NULL UNIQUE REFERENCES core_billing.ledger_journals(id)
);
CREATE TRIGGER referral_event_journals_immutable BEFORE UPDATE OR DELETE ON core_referrals.event_journals
 FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();

CREATE FUNCTION core_referrals.promotion_email(p_email TEXT) RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT NOT (lower(btrim(p_email)) ~ '@(10minutemail[.]com|disposablemail[.]com|emailondeck[.]com|fakeinbox[.]com|getnada[.]com|guerrillamail[.]com|maildrop[.]cc|mailinator[.]com|sharklasers[.]com|tempmail[.]com|temp-mail[.]org|yopmail[.]com)$')
$$;

-- Only called after Core has credited a provider-verified payment, in its transaction.
CREATE FUNCTION core_referrals.settle(p_user BIGINT,p_source TEXT,p_provider TEXT,p_currency TEXT,p_cash BIGINT,p_credit BIGINT)
RETURNS VOID LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE r core_referrals.contracts%ROWTYPE; binding core_referrals.bindings%ROWTYPE;
 i core_identity.users%ROWTYPE; v core_identity.users%ROWTYPE;
 ie TEXT; ve TEXT; vs TEXT; floor_text TEXT; floor_micros NUMERIC; amount BIGINT; balance BIGINT; hist BIGINT;
 eligible BOOLEAN;
BEGIN
 IF p_cash<=0 OR p_credit<=0 OR p_provider NOT IN ('stripe','creem','waffo','waffo_pancake','epay')
 OR (p_provider='epay' AND p_currency<>'CNY') THEN RETURN; END IF;
 SELECT * INTO binding FROM core_referrals.bindings WHERE user_id=p_user;
 IF NOT FOUND THEN RETURN; END IF;
 SELECT * INTO r FROM core_referrals.contracts WHERE invitee=p_user FOR UPDATE;
 IF NOT FOUND OR r.first_source IS NOT NULL THEN RETURN; END IF;
 -- Marker precedes every qualification check and persists for low/ineligible payments.
 UPDATE core_referrals.contracts SET first_source=p_source,paid_micros=p_cash,stage='first_top_up_ineligible' WHERE id=r.id;
 IF binding.legacy_first_top_up<>0 OR binding.prior_cash OR r.stage<>'awaiting_first_top_up' THEN RETURN; END IF;
 -- Serialize all contracts of one inviter and read live identity, never imported status.
 PERFORM id FROM core_identity.users WHERE id IN (r.invitee,r.inviter) ORDER BY id FOR UPDATE;
 SELECT * INTO STRICT i FROM core_identity.users WHERE id=r.invitee;
 SELECT * INTO STRICT v FROM core_identity.users WHERE id=r.inviter;
 SELECT lower(btrim(email)) INTO ie FROM core_identity.user_profiles WHERE user_id=i.id;
 SELECT lower(btrim(email)) INTO ve FROM core_identity.user_profiles WHERE user_id=v.id;
 SELECT stripe_customer INTO vs FROM core_referrals.bindings WHERE user_id=v.id;
 eligible:=i.id<>v.id AND i.active AND v.active AND r.status='earned'
 AND core_referrals.promotion_email(COALESCE(ie,'')) AND core_referrals.promotion_email(COALESCE(ve,''))
 AND (COALESCE(ie,'')='' OR ie IS DISTINCT FROM ve)
 AND (binding.stripe_customer='' OR binding.stripe_customer IS DISTINCT FROM vs)
 AND EXISTS(SELECT FROM core_console.options WHERE key='payment_setting.compliance_confirmed' AND value='true')
 AND EXISTS(SELECT FROM core_console.options WHERE key='payment_setting.compliance_terms_version' AND value='v1')
 AND r.reward_credit>0 AND p_credit>=r.min_top_up_credit;
 floor_text:=r.policy->'min_top_up_amounts'->>p_currency;
 IF floor_text IS NULL OR floor_text !~ '^[0-9]{1,10}(\.[0-9]{1,6})?$' THEN RETURN; END IF;
 floor_micros:=floor_text::numeric*1000000;
 IF NOT COALESCE(eligible,FALSE) OR floor_micros<=0 OR p_cash<floor_micros THEN RETURN; END IF;
 PERFORM core_billing.open_ledger_wallet(v.personal_account_id);
 SELECT b.balance_units INTO STRICT balance FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a
 ON a.id=b.ledger_account_id WHERE a.owner_account_id=v.personal_account_id AND a.bucket='affiliate' FOR NO KEY UPDATE OF b;
 SELECT granted INTO STRICT hist FROM core_referrals.history WHERE inviter=v.id FOR UPDATE;
 amount:=least(r.reward_credit,9007199254740991-r.quota,9007199254740991-greatest(balance,0),9007199254740991-hist);
 IF r.max_reward_credit>0 THEN amount:=least(amount,greatest(0,r.max_reward_credit-r.quota)); END IF;
 IF amount<=0 THEN RETURN; END IF;
 PERFORM core_referrals.delta(r.id,'first_top_up',amount);
 UPDATE core_referrals.contracts SET quota=quota+amount,stage='first_top_up_completed' WHERE id=r.id;
 UPDATE core_referrals.history SET granted=granted+amount WHERE inviter=v.id;
END $$;

CREATE FUNCTION core_referrals.revoke(p_reward BIGINT,p_reason TEXT,p_penalize BOOLEAN) RETURNS VOID
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE r core_referrals.contracts%ROWTYPE; computed_penalty BIGINT;
BEGIN
 SELECT * INTO STRICT r FROM core_referrals.contracts WHERE id=p_reward FOR UPDATE;
 IF r.status<>'earned' THEN RETURN; END IF;
 computed_penalty:=CASE WHEN p_penalize THEN r.quota/100*r.penalty_percent+r.quota%100*r.penalty_percent/100 ELSE 0 END;
 IF r.max_penalty_credit>0 THEN computed_penalty:=least(computed_penalty,r.max_penalty_credit); END IF;
 PERFORM core_referrals.delta(r.id,'clawback',-r.quota);
 PERFORM core_referrals.delta(r.id,'penalty',-computed_penalty);
 UPDATE core_referrals.contracts SET revision=revision+1,status='revoked',reason=p_reason,revoked=quota,penalty=computed_penalty WHERE id=r.id;
END $$;

CREATE FUNCTION core_referrals.native_payment() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE cash_eligible BOOLEAN;
BEGIN
 IF OLD.credit_journal_id IS NULL AND NEW.credit_journal_id IS NOT NULL THEN
  IF NOT EXISTS(SELECT FROM core_payments.accepted_proofs p
   JOIN core_billing.ledger_journals j ON j.id=NEW.credit_journal_id
   JOIN core_billing.ledger_accounts a ON a.id=j.payee_id
   WHERE p.intent_id=NEW.id AND p.channel_id=NEW.channel_id
   AND p.verified_facts->>'kind'='Credit' AND p.verified_facts->>'transaction'=NEW.provider_transaction
   AND (p.verified_facts->>'amount_minor')::bigint=NEW.amount_minor
   AND j.kind='credit' AND j.scope='payments' AND j.operation_key LIKE NEW.id::text||':credit:%'
   AND j.actor_user_id=NEW.created_by_user_id AND j.amount_units=NEW.credit_units
   AND a.owner_account_id=NEW.account_id AND a.bucket='wallet'
   AND j.payment_reference=NEW.provider||':'||encode(sha256(convert_to(
      array_to_json(ARRAY[NEW.provider,NEW.merchant,NEW.environment,NEW.provider_transaction])::text,'UTF8')),'hex')) THEN
   RAISE EXCEPTION 'referral payment lacks matching Core proof and credit' USING ERRCODE='23514';
  END IF;
  SELECT COALESCE(bool_or(COALESCE((verified_facts->>'referral_cash_eligible')::boolean,NEW.provider<>'epay')),FALSE)
   INTO cash_eligible FROM core_payments.accepted_proofs WHERE intent_id=NEW.id
    AND verified_facts->>'kind'='Credit' AND verified_facts->>'transaction'=NEW.provider_transaction;
  IF cash_eligible AND EXISTS(SELECT FROM core_identity.users WHERE id=NEW.created_by_user_id AND personal_account_id=NEW.account_id) THEN
   PERFORM core_referrals.settle(NEW.created_by_user_id,'native:'||NEW.id,NEW.provider,NEW.currency,
    NEW.amount_minor*10000,NEW.credit_units);
  END IF;
 END IF;
 IF NEW.refunded_minor>OLD.refunded_minor AND NEW.credit_journal_id IS NOT NULL THEN
  IF (SELECT COALESCE(sum(amount_units),0) FROM core_billing.ledger_journals
    WHERE kind='refund_topup' AND scope='payments' AND parent_journal_id=NEW.credit_journal_id)
    <> NEW.refunded_minor::numeric*NEW.credits_per_minor THEN
   RAISE EXCEPTION 'referral refund lacks original ledger compensation' USING ERRCODE='23514';
  END IF;
  PERFORM core_referrals.refund('native:'||NEW.id,NEW.refunded_minor*10000);
 END IF;
 RETURN NEW;
END $$;

CREATE FUNCTION core_referrals.refund(p_source TEXT,p_cumulative BIGINT) RETURNS VOID
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE r core_referrals.contracts%ROWTYPE;
BEGIN
 SELECT * INTO r FROM core_referrals.contracts WHERE first_source=p_source FOR UPDATE;
 IF NOT FOUND THEN RETURN; END IF;
 IF p_cumulative<r.refunded_micros OR p_cumulative>r.paid_micros THEN
 RAISE EXCEPTION 'referral refund cumulative mismatch' USING ERRCODE='23514'; END IF;
 UPDATE core_referrals.contracts SET refunded_micros=p_cumulative WHERE id=r.id;
 IF p_cumulative<r.paid_micros THEN RETURN; END IF;
 PERFORM core_referrals.revoke(r.id,'refund',FALSE);
 UPDATE core_referrals.contracts SET reason='refund' WHERE id=r.id;
END $$;
CREATE TRIGGER referral_native_payment AFTER UPDATE OF credit_journal_id,refunded_minor ON core_payments.intents
 FOR EACH ROW EXECUTE FUNCTION core_referrals.native_payment();

CREATE FUNCTION core_referrals.legacy_payment() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE b core_payments.legacy_bases%ROWTYPE;
BEGIN
 IF NEW.journal_id IS NULL THEN RETURN NEW; END IF;
 SELECT * INTO STRICT b FROM core_payments.legacy_bases WHERE id=NEW.basis_id;
 PERFORM core_referrals.settle(b.user_id,'legacy:'||b.id,b.provider,b.currency,b.amount_micros,b.credit_units);
 RETURN NEW;
END $$;
CREATE TRIGGER referral_legacy_payment AFTER INSERT ON core_payments.legacy_applications
 FOR EACH ROW EXECUTE FUNCTION core_referrals.legacy_payment();

-- Explicit confirmed-abuse boundary, never ordinary disable/enable. Owner only
-- until a browser-confirmed Core moderation caller is independently accepted.
CREATE FUNCTION core_referrals.moderate(p_request TEXT,p_actor BIGINT,p_user BIGINT,p_action TEXT,p_reason TEXT,p_evidence TEXT,p_penalize BOOLEAN)
RETURNS VOID LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE prior core_referrals.moderation%ROWTYPE; r core_referrals.contracts%ROWTYPE; actor_role TEXT; target_role TEXT;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended('referral-moderate:'||p_request,0));
 SELECT * INTO prior FROM core_referrals.moderation WHERE request_id=p_request;
 IF FOUND THEN
  IF (prior.actor,prior.user_id,prior.action,prior.reason,prior.evidence,prior.penalize)
    IS DISTINCT FROM (p_actor,p_user,p_action,p_reason,p_evidence,p_penalize) THEN
   RAISE EXCEPTION 'referral moderation retry conflict' USING ERRCODE='P1001'; END IF;
  RETURN;
 END IF;
 PERFORM id FROM core_identity.users WHERE id IN(p_actor,p_user) ORDER BY id FOR UPDATE;
 SELECT platform_role INTO actor_role FROM core_identity.users WHERE id=p_actor AND active;
 SELECT platform_role INTO target_role FROM core_identity.users WHERE id=p_user;
 IF NOT COALESCE(actor_role IN ('admin','superadmin') AND
 (target_role='user' OR (actor_role='superadmin' AND target_role='admin')),FALSE) THEN
 RAISE EXCEPTION 'referral moderation permission denied' USING ERRCODE='42501'; END IF;
 IF NOT ((p_action='ban_abuse' AND p_reason IN ('abuse','bulk_registration')) OR
 (p_action='restore_referral' AND p_reason='mistaken_ban' AND NOT p_penalize)) THEN
 RAISE EXCEPTION 'invalid referral moderation decision' USING ERRCODE='23514'; END IF;
 INSERT INTO core_referrals.moderation(request_id,actor,user_id,action,reason,evidence,penalize)
 VALUES(p_request,p_actor,p_user,p_action,p_reason,p_evidence,p_penalize);
 SELECT * INTO r FROM core_referrals.contracts WHERE invitee=p_user FOR UPDATE;
 IF FOUND THEN
  IF p_action='ban_abuse' THEN PERFORM core_referrals.revoke(r.id,p_reason,p_penalize);
  ELSIF r.status<>'earned' THEN
   IF r.reason<>'refund' THEN PERFORM core_referrals.delta(r.id,'restore_reward',r.revoked); END IF;
   PERFORM core_referrals.delta(r.id,'restore_penalty',r.penalty);
   UPDATE core_referrals.contracts SET penalty=0,status=CASE WHEN reason='refund' THEN status ELSE 'earned' END,
    revoked=CASE WHEN reason='refund' THEN revoked ELSE 0 END,reason=CASE WHEN reason='refund' THEN reason ELSE '' END WHERE id=r.id;
  END IF;
 END IF;
 UPDATE core_identity.users SET active=(p_action='restore_referral'),auth_version=auth_version+1
 WHERE id=p_user AND active IS DISTINCT FROM (p_action='restore_referral');
END $$;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_referrals FROM PUBLIC;

CREATE OR REPLACE FUNCTION core_referrals.native_payment() RETURNS TRIGGER
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
    <> div(NEW.refunded_minor::numeric * NEW.credit_units * 2 + NEW.amount_minor, NEW.amount_minor::numeric * 2) THEN
   RAISE EXCEPTION 'referral refund lacks original ledger compensation' USING ERRCODE='23514';
  END IF;
  PERFORM core_referrals.refund('native:'||NEW.id,NEW.refunded_minor*10000);
 END IF;
 RETURN NEW;
END $$;

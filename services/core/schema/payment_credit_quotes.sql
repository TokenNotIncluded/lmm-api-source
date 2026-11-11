-- Freeze purchased credits independently from rounded fiat settlement.
ALTER TABLE core_payments.intents ADD COLUMN credit_quote JSONB;
ALTER TABLE core_payments.intents
    DROP CONSTRAINT intents_credits_per_minor_check,
    DROP CONSTRAINT intents_check2,
    ADD CONSTRAINT intents_credits_per_minor_check CHECK (credits_per_minor >= 0),
    ADD CONSTRAINT intents_frozen_credit_quote CHECK (COALESCE(
      CASE WHEN credit_quote IS NULL THEN
        credits_per_minor > 0 AND credit_units::numeric = amount_minor::numeric * credits_per_minor
      ELSE
        credits_per_minor = 0 AND provider = 'waffo_pancake' AND credit_units >= amount_minor
        AND currency IN ('USD','CNY')
        AND jsonb_typeof(credit_quote) = 'object'
        AND credit_quote->>'kind' = 'credit_quantity_v1'
        AND credit_quote - ARRAY['kind','settlement_per_usd_numerator','settlement_per_usd_denominator',
            'discount_numerator','discount_denominator'] = '{}'::jsonb
        AND jsonb_typeof(credit_quote->'settlement_per_usd_numerator') = 'number'
        AND (credit_quote->>'settlement_per_usd_numerator')::numeric BETWEEN 1 AND 1000000000
        AND trunc((credit_quote->>'settlement_per_usd_numerator')::numeric) = (credit_quote->>'settlement_per_usd_numerator')::numeric
        AND jsonb_typeof(credit_quote->'settlement_per_usd_denominator') = 'number'
        AND (credit_quote->>'settlement_per_usd_denominator')::numeric BETWEEN 1 AND 1000000000
        AND trunc((credit_quote->>'settlement_per_usd_denominator')::numeric) = (credit_quote->>'settlement_per_usd_denominator')::numeric
        AND jsonb_typeof(credit_quote->'discount_numerator') = 'number'
        AND (credit_quote->>'discount_numerator')::numeric BETWEEN 1 AND 1000000000
        AND trunc((credit_quote->>'discount_numerator')::numeric) = (credit_quote->>'discount_numerator')::numeric
        AND jsonb_typeof(credit_quote->'discount_denominator') = 'number'
        AND (credit_quote->>'discount_denominator')::numeric BETWEEN 1 AND 1000000000
        AND trunc((credit_quote->>'discount_denominator')::numeric) = (credit_quote->>'discount_denominator')::numeric
        AND (credit_quote->>'discount_numerator')::numeric <= (credit_quote->>'discount_denominator')::numeric
        AND (currency <> 'USD' OR (credit_quote->>'settlement_per_usd_numerator')::numeric
            = (credit_quote->>'settlement_per_usd_denominator')::numeric)
        AND amount_minor::numeric = div(
            credit_units::numeric * (credit_quote->>'settlement_per_usd_numerator')::numeric
                * (credit_quote->>'discount_numerator')::numeric * 2
                + 5000 * (credit_quote->>'settlement_per_usd_denominator')::numeric
                    * (credit_quote->>'discount_denominator')::numeric,
            NULLIF(10000 * (credit_quote->>'settlement_per_usd_denominator')::numeric
                * (credit_quote->>'discount_denominator')::numeric, 0))
      END, FALSE));

CREATE OR REPLACE FUNCTION core_payments.protect_intent() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
    IF TG_OP='DELETE' OR (NEW.id,NEW.account_id,NEW.created_by_user_id,NEW.channel_id,
        NEW.provider,NEW.merchant,NEW.environment,NEW.currency,NEW.amount_minor,
        NEW.credit_units,NEW.credits_per_minor,NEW.credit_quote,NEW.prepare_key,NEW.prepare_hash,NEW.created_at)
        IS DISTINCT FROM (OLD.id,OLD.account_id,OLD.created_by_user_id,OLD.channel_id,
        OLD.provider,OLD.merchant,OLD.environment,OLD.currency,OLD.amount_minor,
        OLD.credit_units,OLD.credits_per_minor,OLD.credit_quote,OLD.prepare_key,OLD.prepare_hash,OLD.created_at)
        OR (OLD.intent_wire IS NOT NULL AND NEW.intent_wire IS DISTINCT FROM OLD.intent_wire)
        OR (OLD.credit_journal_id IS NOT NULL AND NEW.credit_journal_id IS DISTINCT FROM OLD.credit_journal_id)
        OR (OLD.provider_transaction IS NOT NULL AND NEW.provider_transaction IS DISTINCT FROM OLD.provider_transaction)
        OR (OLD.refund_fenced AND NOT NEW.refund_fenced)
        OR NEW.refunded_minor<OLD.refunded_minor THEN
        RAISE EXCEPTION 'immutable payment identity or decreasing refund fence' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;


-- Reviewed legacy display counters, not wallet credits or settlement receipts.
CREATE TABLE core_channels.opening_sources (
    source_key TEXT PRIMARY KEY CHECK(source_key ~ '^[0-9a-f]{64}$'),
    archive_sha256 TEXT NOT NULL CHECK(archive_sha256 ~ '^[0-9a-f]{64}$'),
    export_sha256 TEXT NOT NULL CHECK(export_sha256 ~ '^[0-9a-f]{64}$'),
    mapping_sha256 TEXT NOT NULL CHECK(mapping_sha256 ~ '^[0-9a-f]{64}$'),
    unit_evidence_sha256 TEXT NOT NULL CHECK(unit_evidence_sha256 ~ '^[0-9a-f]{64}$'),
    source_revision TEXT NOT NULL CHECK(length(source_revision) BETWEEN 1 AND 128),
    observed_at BIGINT CHECK(observed_at BETWEEN 1 AND 253402300799),
    native_overlap TEXT NOT NULL CHECK(native_overlap IN ('disjoint','included_at_seal')),
    native_request_ids TEXT[] NOT NULL,
    native_seal_sha256 TEXT NOT NULL CHECK(native_seal_sha256 ~ '^[0-9a-f]{64}$'),
    rows_sha256 TEXT NOT NULL CHECK(rows_sha256 ~ '^[0-9a-f]{64}$'),
    plan_sha256 TEXT NOT NULL UNIQUE CHECK(plan_sha256 ~ '^[0-9a-f]{64}$'),
    captured_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- One reviewed source namespace; channel numbers from another archive may be reused.
CREATE UNIQUE INDEX channel_opening_namespace ON core_channels.opening_sources ((true));

CREATE TABLE core_channels.opening_baselines (
    channel_id BIGINT PRIMARY KEY REFERENCES core_channels.identities(id),
    source_key TEXT NOT NULL REFERENCES core_channels.opening_sources(source_key),
    source_channel_id BIGINT NOT NULL CHECK(source_channel_id BETWEEN 1 AND 9007199254740991),
    raw_used_quota BIGINT NOT NULL,
    credits_per_usd BIGINT NOT NULL CHECK(credits_per_usd BETWEEN 1 AND 9007199254740991),
    offset_numerator NUMERIC(78,0) NOT NULL CHECK(offset_numerator > '-Infinity'::numeric AND offset_numerator < 'Infinity'::numeric),
    offset_denominator NUMERIC(78,0) NOT NULL CHECK(offset_denominator > 0 AND offset_denominator < 'Infinity'::numeric),
    historical_coverage BOOLEAN NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('imported','copied')),
    parent_channel_id BIGINT REFERENCES core_channels.identities(id),
    captured_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CHECK((kind='imported' AND parent_channel_id IS NULL) OR (kind='copied' AND parent_channel_id IS NOT NULL))
);
-- Runtime may preserve a reviewed parent, never invent imported source facts.
CREATE FUNCTION core_channels.check_opening_copy() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE parent core_channels.opening_baselines;
BEGIN
    IF current_user=(SELECT pg_get_userbyid(relowner) FROM pg_class WHERE oid=TG_RELID) THEN
        RETURN NEW;
    END IF;
    SELECT * INTO parent FROM core_channels.opening_baselines WHERE channel_id=NEW.parent_channel_id;
    IF NEW.kind<>'copied' OR parent.channel_id IS NULL
       OR (NEW.source_key,NEW.source_channel_id,NEW.raw_used_quota,NEW.credits_per_usd,NEW.offset_numerator,NEW.offset_denominator)
          IS DISTINCT FROM
          (parent.source_key,parent.source_channel_id,parent.raw_used_quota,parent.credits_per_usd,parent.offset_numerator,parent.offset_denominator)
       OR (NEW.historical_coverage AND NOT parent.historical_coverage) THEN
        RAISE EXCEPTION 'channel opening copies must preserve reviewed source facts';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION core_channels.check_opening_copy() FROM PUBLIC;
CREATE TRIGGER channel_opening_copy_source
BEFORE INSERT ON core_channels.opening_baselines
FOR EACH ROW EXECUTE FUNCTION core_channels.check_opening_copy();
CREATE TRIGGER channel_opening_source_immutable
BEFORE UPDATE OR DELETE ON core_channels.opening_sources
FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER channel_opening_source_no_truncate
BEFORE TRUNCATE ON core_channels.opening_sources
FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER channel_opening_baseline_immutable
BEFORE UPDATE OR DELETE ON core_channels.opening_baselines
FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER channel_opening_baseline_no_truncate
BEFORE TRUNCATE ON core_channels.opening_baselines
FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();

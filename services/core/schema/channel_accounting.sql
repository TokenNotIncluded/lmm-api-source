-- Display-only channel continuity facts. Never a second ledger or a startup migration.
-- Owner-installed publication fence without replacing any frozen function.
-- Publications write prices -> config -> active. The five-argument wrapper's
-- earlier singleton FOR UPDATE takes ROW SHARE, compatible with these SHARE
-- table locks. Do not add a singleton row lock here: it would invert that
-- wrapper's row-before-price ordering. Tables fence writes even without rows.
-- The authorized caller retains its existing SET LOCAL lock_timeout='500ms'.
CREATE FUNCTION public.routing_control_balance_lock() RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
    LOCK TABLE public.routing_price_versions IN SHARE MODE;
    LOCK TABLE public.routing_config_versions IN SHARE MODE;
    LOCK TABLE public.routing_active_release IN SHARE MODE;
END;
$$;
REVOKE ALL ON FUNCTION public.routing_control_balance_lock() FROM PUBLIC;

CREATE SCHEMA core_channels;
CREATE TABLE core_channels.identities (
    id BIGINT PRIMARY KEY CHECK(id BETWEEN 1 AND 9007199254740991),
    origin TEXT NOT NULL CHECK(origin IN ('bootstrap','native','copy','migrated')),
    history_ambiguous BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    retired_at TIMESTAMPTZ
);
CREATE TABLE core_channels.counter_baselines (
    channel_id BIGINT PRIMARY KEY REFERENCES core_channels.identities(id),
    gross_credits NUMERIC(38,0) NOT NULL DEFAULT 0 CHECK(gross_credits>=0 AND gross_credits<'Infinity'::numeric),
    refunded_credits NUMERIC(38,0) NOT NULL DEFAULT 0 CHECK(refunded_credits>=0 AND refunded_credits<=gross_credits),
    historical_coverage BOOLEAN NOT NULL DEFAULT false,
    source_channel_id BIGINT REFERENCES core_channels.identities(id),
    source_routing_version BIGINT CHECK(source_routing_version>0),
    captured_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE core_channels.balance_observations (
    channel_id BIGINT PRIMARY KEY REFERENCES core_channels.identities(id),
    balance DOUBLE PRECISION CHECK(balance>'-Infinity'::float8 AND balance<'Infinity'::float8),
    unit_label TEXT CHECK(length(unit_label)<=32),
    observed_at TIMESTAMPTZ,
    kind TEXT NOT NULL DEFAULT 'unknown' CHECK(kind IN ('unknown','provider','copied','reset')),
    credential_id BIGINT CHECK(credential_id>0),
    credential_revision BIGINT CHECK(credential_revision>0),
    source_channel_id BIGINT REFERENCES core_channels.identities(id),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CHECK((credential_id IS NULL)=(credential_revision IS NULL)),
    CHECK(kind<>'unknown' OR balance IS NULL),
    CHECK(kind<>'provider' OR (balance IS NOT NULL AND observed_at IS NOT NULL
        AND credential_id IS NOT NULL AND credential_revision IS NOT NULL)),
    CHECK(kind<>'reset' OR (balance IS NOT NULL AND balance=0))
);
-- Rust validates Unicode control characters in unit_label at the write boundary.
-- All historical documents count, including unpublished versions and deleted IDs.
WITH history AS MATERIALIZED (
    SELECT v.version,(u->>'id')::bigint AS id
    FROM public.routing_config_versions v
    CROSS JOIN LATERAL jsonb_array_elements(coalesce(v.document->'upstreams','[]'::jsonb)) u
), audit_ids AS MATERIALIZED (
    SELECT (target#>>'{change,id}')::bigint AS id,action,created_at
    FROM core_identity.audit
    WHERE action IN ('channel.create','channel.copy','channel.delete')
      AND target#>>'{change,id}' IS NOT NULL
), native_contracts AS MATERIALIZED (
    SELECT e.detail,r.authorized_at FROM core_charging.usage_events e
    JOIN core_charging.requests r ON r.id=e.request_id
    WHERE event_id='reservation-contract:v1'
      AND detail->>'kind'='model_reservation_contract' AND detail->>'version'='1'
), native_raw_ids(raw_id,seen_epoch) AS (
    SELECT detail#>>'{binding,upstream_id}',authorized_at FROM native_contracts
    UNION ALL
    SELECT fallback#>>'{binding,upstream_id}',authorized_at FROM native_contracts
    CROSS JOIN LATERAL jsonb_array_elements(
        CASE WHEN jsonb_typeof(detail->'fallbacks')='array' THEN detail->'fallbacks' ELSE '[]'::jsonb END) fallback
    UNION ALL
    SELECT price#>>'{image,request,upstream_id}',authorized_at FROM core_charging.requests
    UNION ALL
    SELECT price#>>'{image,rule,upstream_id}',authorized_at FROM core_charging.requests
    UNION ALL
    SELECT price#>>'{image,request,upstream_id}',created_at FROM core_image.operations
    UNION ALL
    SELECT price#>>'{image,rule,upstream_id}',created_at FROM core_image.operations
), native_ids AS (
    -- Only reserve exact persisted numeric IDs, never legacy rule-name guesses.
    -- This is continuity evidence, not validation/repair of a usage receipt.
    SELECT raw_id::bigint AS id,max(seen_epoch) AS last_seen_epoch FROM native_raw_ids
    WHERE CASE WHEN raw_id ~ '^[1-9][0-9]{0,15}$'
        THEN raw_id::numeric<=9007199254740991 ELSE false END
    GROUP BY raw_id::bigint
), ids AS (
    SELECT id FROM history UNION SELECT id FROM audit_ids UNION SELECT id FROM native_ids
), active AS MATERIALIZED (
    SELECT h.id FROM history h JOIN public.routing_active_release a ON a.config_version=h.version
), spans AS (
    SELECT id,min(version) AS first_version,max(version) AS last_version FROM history GROUP BY id
)
INSERT INTO core_channels.identities(id,origin,history_ambiguous,retired_at)
SELECT i.id,'migrated',
    EXISTS(SELECT FROM spans s JOIN public.routing_config_versions v
        ON v.version>s.first_version AND v.version<s.last_version
        WHERE s.id=i.id AND NOT EXISTS(SELECT FROM history h WHERE h.id=i.id AND h.version=v.version))
    OR (EXISTS(SELECT FROM audit_ids d WHERE d.id=i.id AND d.action='channel.delete')
        AND EXISTS(SELECT FROM active a WHERE a.id=i.id)),
    CASE WHEN EXISTS(SELECT FROM audit_ids d WHERE d.id=i.id AND d.action='channel.delete')
        OR (NOT EXISTS(SELECT FROM active a WHERE a.id=i.id)
            AND EXISTS(SELECT FROM public.routing_active_release a
                JOIN public.routing_config_versions v ON v.version=a.config_version
                WHERE EXISTS(SELECT FROM spans s WHERE s.id=i.id AND s.last_version<v.version)
                   OR EXISTS(SELECT FROM native_ids n WHERE n.id=i.id
                       AND n.last_seen_epoch<=extract(epoch FROM v.created_at))
                   OR EXISTS(SELECT FROM audit_ids c WHERE c.id=i.id
                       AND c.action IN ('channel.create','channel.copy') AND c.created_at<=v.created_at)))
        THEN clock_timestamp() ELSE NULL END
FROM ids i;
-- An absent publication is not evidence that a bootstrap channel was retired.
-- Receipt-only IDs stay reserved with unknown lineage; only persisted active
-- channels receive display baseline/observation rows during this offline seed.
INSERT INTO core_channels.counter_baselines(channel_id)
SELECT i.id FROM core_channels.identities i
WHERE i.retired_at IS NULL AND EXISTS(
    SELECT FROM public.routing_active_release a
    JOIN public.routing_config_versions v ON v.version=a.config_version
    CROSS JOIN LATERAL jsonb_array_elements(coalesce(v.document->'upstreams','[]'::jsonb)) u
    WHERE (u->>'id')::bigint=i.id);
INSERT INTO core_channels.balance_observations(channel_id)
SELECT channel_id FROM core_channels.counter_baselines;
-- Immutable, owner-only receipt for the exact 42 -> 43 offline procedure.
CREATE TABLE core_meta.channel_accounting_upgrades (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
    request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
    preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER channel_accounting_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.channel_accounting_upgrades
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER channel_accounting_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.channel_accounting_upgrades
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.channel_accounting_upgrades FROM PUBLIC;
REVOKE ALL ON SCHEMA core_channels FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_channels FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_channels FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_channels FROM PUBLIC;

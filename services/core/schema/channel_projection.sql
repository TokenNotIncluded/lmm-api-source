-- Offline component44: derived Native channel usage only, never a money ledger.
-- Exact fields shared with the typed live builder and canonical owner backfill.
-- Channel zero denotes unknown attribution; historical IDs need no registry FK.
CREATE TABLE core_channels.usage_contributions (
    request_id TEXT NOT NULL REFERENCES core_charging.requests(id),
    channel_id BIGINT NOT NULL CHECK(channel_id BETWEEN 0 AND 9007199254740991),
    authorized_at BIGINT NOT NULL CHECK(authorized_at>=0),
    gross_credits NUMERIC(38,0) NOT NULL DEFAULT 0 CHECK(gross_credits>=0 AND gross_credits<'Infinity'::numeric),
    refunded_credits NUMERIC(38,0) NOT NULL DEFAULT 0 CHECK(refunded_credits>=0 AND refunded_credits<=gross_credits AND refunded_credits<'Infinity'::numeric),
    settled_requests BIGINT NOT NULL CHECK(settled_requests IN (0,1)),
    pending_requests BIGINT NOT NULL CHECK(pending_requests IN (0,1)),
    unknown_requests BIGINT NOT NULL CHECK(unknown_requests IN (0,1)),
    PRIMARY KEY(request_id,channel_id),
    CHECK(settled_requests+pending_requests+unknown_requests=1),
    CHECK(settled_requests=1 OR (gross_credits=0 AND refunded_credits=0))
);
CREATE INDEX channel_usage_contributions_time ON core_channels.usage_contributions(channel_id,authorized_at,request_id);
CREATE TABLE core_channels.usage_rollups (
    channel_id BIGINT NOT NULL CHECK(channel_id BETWEEN 0 AND 9007199254740991),
    day BIGINT NOT NULL CHECK(day>=-1),
    bucket SMALLINT NOT NULL CHECK(bucket BETWEEN 0 AND 63),
    gross_credits NUMERIC(38,0) NOT NULL DEFAULT 0 CHECK(gross_credits>=0 AND gross_credits<'Infinity'::numeric),
    refunded_credits NUMERIC(38,0) NOT NULL DEFAULT 0 CHECK(refunded_credits>=0 AND refunded_credits<=gross_credits AND refunded_credits<'Infinity'::numeric),
    settled_requests BIGINT NOT NULL DEFAULT 0 CHECK(settled_requests>=0),
    pending_requests BIGINT NOT NULL DEFAULT 0 CHECK(pending_requests>=0),
    unknown_requests BIGINT NOT NULL DEFAULT 0 CHECK(unknown_requests>=0),
    PRIMARY KEY(channel_id,day,bucket)
);
REVOKE ALL ON core_channels.usage_contributions,core_channels.usage_rollups FROM PUBLIC;
CREATE TABLE core_channels.projection_state (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
    ready BOOLEAN NOT NULL
);
CREATE TABLE core_channels.projection_dirty (
    request_id TEXT PRIMARY KEY REFERENCES core_charging.requests(id),
    authorized_at BIGINT NOT NULL
);
CREATE INDEX channel_projection_dirty_authorized ON core_channels.projection_dirty(authorized_at,request_id);
-- A fresh initializer has no requests. Existing-site upgrades remain unavailable
-- until the owner backfill finishes under the same migration transaction/fence.
INSERT INTO core_channels.projection_state(singleton,ready)
SELECT true,NOT EXISTS(SELECT FROM core_charging.requests);
INSERT INTO core_channels.projection_dirty(request_id,authorized_at)
SELECT id,authorized_at FROM core_charging.requests;

CREATE FUNCTION core_channels.mark_projection_dirty() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE rid TEXT; at BIGINT; old_relevant BOOLEAN := false;
BEGIN
    IF TG_TABLE_NAME='requests' THEN
        IF TG_OP='UPDATE' THEN
            IF (CASE WHEN NEW.state IN ('reserved','streaming','reconcile') THEN 'held' ELSE NEW.state END)
                IS NOT DISTINCT FROM
               (CASE WHEN OLD.state IN ('reserved','streaming','reconcile') THEN 'held' ELSE OLD.state END)
               AND (NEW.actor_user_id,NEW.key_id,NEW.oauth_source,NEW.assistant_grant_id,
                    NEW.assistant_sponsor_account_id,NEW.image_operation_id,NEW.owner_account_id,
                    NEW.payer_account_id,NEW.fingerprint,NEW.spec_digest,NEW.price,
                    NEW.source_subscription_id,NEW.source_start,NEW.source_end,NEW.authorized_at,
                    NEW.reserved,NEW.settled,NEW.refunded,NEW.outcome)
                   IS NOT DISTINCT FROM
                   (OLD.actor_user_id,OLD.key_id,OLD.oauth_source,OLD.assistant_grant_id,
                    OLD.assistant_sponsor_account_id,OLD.image_operation_id,OLD.owner_account_id,
                    OLD.payer_account_id,OLD.fingerprint,OLD.spec_digest,OLD.price,
                    OLD.source_subscription_id,OLD.source_start,OLD.source_end,OLD.authorized_at,
                    OLD.reserved,OLD.settled,OLD.refunded,OLD.outcome)
               AND ((NEW.state IN ('reserved','streaming','reconcile')
                     AND OLD.state IN ('reserved','streaming','reconcile'))
                    OR (NEW.input_tokens,NEW.cached_tokens,NEW.output_tokens,NEW.image_usage)
                       IS NOT DISTINCT FROM
                       (OLD.input_tokens,OLD.cached_tokens,OLD.output_tokens,OLD.image_usage)) THEN
                RETURN NEW;
            END IF;
        END IF;
        rid:=NEW.id; at:=NEW.authorized_at;
    ELSE
        rid:=CASE WHEN TG_OP='DELETE' THEN OLD.request_id ELSE NEW.request_id END;
        IF TG_TABLE_NAME='usage_events' THEN
            IF TG_OP IN ('UPDATE','DELETE') THEN
                old_relevant:=OLD.event_id IN ('reservation-contract:v1','reservation-admission:v1','final')
                    OR ((OLD.event_id LIKE 'route-dispatch:%' OR OLD.event_id LIKE 'route-failure:%')
                        AND EXISTS(SELECT FROM core_charging.requests r WHERE r.id=OLD.request_id
                            AND r.state IN ('settled','released')));
            END IF;
            IF TG_OP='DELETE' THEN
                IF NOT old_relevant THEN RETURN NULL; END IF;
            ELSIF NOT old_relevant AND NEW.event_id NOT IN ('reservation-contract:v1','reservation-admission:v1','final') THEN
                -- Ordinary held dispatch/failure cannot change the pending set.
                -- Only terminal late attempts can change attribution/validation.
                IF NOT ((NEW.event_id LIKE 'route-dispatch:%' OR NEW.event_id LIKE 'route-failure:%')
                    AND EXISTS(SELECT FROM core_charging.requests r WHERE r.id=rid
                        AND r.state IN ('settled','released'))) THEN
                    RETURN NULL;
                END IF;
            END IF;
        END IF;
        IF TG_OP='UPDATE' AND OLD.request_id IS DISTINCT FROM NEW.request_id THEN
            INSERT INTO core_channels.projection_dirty(request_id,authorized_at)
                SELECT id,authorized_at FROM core_charging.requests WHERE id=OLD.request_id
                ON CONFLICT(request_id) DO UPDATE SET authorized_at=EXCLUDED.authorized_at;
        END IF;
        SELECT r.authorized_at INTO STRICT at FROM core_charging.requests r WHERE r.id=rid;
    END IF;
    INSERT INTO core_channels.projection_dirty(request_id,authorized_at) VALUES(rid,at)
        ON CONFLICT(request_id) DO UPDATE SET authorized_at=EXCLUDED.authorized_at;
    RETURN NULL;
END;
$$;
REVOKE ALL ON FUNCTION core_channels.mark_projection_dirty() FROM PUBLIC;
CREATE TRIGGER channel_projection_request_dirty AFTER INSERT OR UPDATE ON core_charging.requests
    FOR EACH ROW EXECUTE FUNCTION core_channels.mark_projection_dirty();
CREATE TRIGGER channel_projection_event_dirty AFTER INSERT ON core_charging.usage_events
    FOR EACH ROW WHEN (NEW.event_id IN ('reservation-contract:v1','reservation-admission:v1','final')
        OR NEW.event_id LIKE 'route-dispatch:%' OR NEW.event_id LIKE 'route-failure:%')
    EXECUTE FUNCTION core_channels.mark_projection_dirty();
CREATE TRIGGER channel_projection_event_update_dirty AFTER UPDATE ON core_charging.usage_events
    FOR EACH ROW WHEN (NEW.event_id IN ('reservation-contract:v1','reservation-admission:v1','final')
        OR NEW.event_id LIKE 'route-dispatch:%' OR NEW.event_id LIKE 'route-failure:%'
        OR OLD.event_id IN ('reservation-contract:v1','reservation-admission:v1','final')
        OR OLD.event_id LIKE 'route-dispatch:%' OR OLD.event_id LIKE 'route-failure:%')
    EXECUTE FUNCTION core_channels.mark_projection_dirty();
CREATE TRIGGER channel_projection_event_delete_dirty AFTER DELETE ON core_charging.usage_events
    FOR EACH ROW WHEN (OLD.event_id IN ('reservation-contract:v1','reservation-admission:v1','final')
        OR OLD.event_id LIKE 'route-dispatch:%' OR OLD.event_id LIKE 'route-failure:%')
    EXECUTE FUNCTION core_channels.mark_projection_dirty();
CREATE TRIGGER channel_projection_command_dirty AFTER INSERT OR UPDATE OR DELETE ON core_charging.ledger_commands
    FOR EACH ROW EXECUTE FUNCTION core_channels.mark_projection_dirty();
CREATE TRIGGER channel_projection_refund_dirty AFTER INSERT OR UPDATE OR DELETE ON core_charging.refunds
    FOR EACH ROW EXECUTE FUNCTION core_channels.mark_projection_dirty();

CREATE TABLE core_meta.channel_projection_upgrades (
    upgrade_id TEXT PRIMARY KEY,
    request_sha256 BYTEA NOT NULL CHECK(octet_length(request_sha256)=32),
    request JSONB NOT NULL CHECK(jsonb_typeof(request)='object'),
    preserved_data_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_data_sha256)=32),
    preserved_permissions_sha256 BYTEA NOT NULL CHECK(octet_length(preserved_permissions_sha256)=32),
    successor_catalog_sha256 BYTEA NOT NULL CHECK(octet_length(successor_catalog_sha256)=32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER channel_projection_upgrades_immutable BEFORE UPDATE OR DELETE ON core_meta.channel_projection_upgrades
    FOR EACH ROW EXECUTE FUNCTION core_billing.ledger_immutable();
CREATE TRIGGER channel_projection_upgrades_no_truncate BEFORE TRUNCATE ON core_meta.channel_projection_upgrades
    FOR EACH STATEMENT EXECUTE FUNCTION core_billing.ledger_immutable();
REVOKE ALL ON core_meta.channel_projection_upgrades FROM PUBLIC;
REVOKE ALL ON core_channels.projection_state,core_channels.projection_dirty FROM PUBLIC;

-- STAGED ONLY: requires the existing email challenge delta in an isolated DB.
-- No canonical schema, component version, upgrade receipt or runtime grants change.
CREATE TABLE core_identity.email_delivery_outbox (
    id TEXT PRIMARY KEY CHECK (id ~ '^[a-f0-9]{32}$'),
    challenge_id TEXT NOT NULL REFERENCES core_identity.email_challenges(id),
    kind TEXT NOT NULL CHECK (kind IN ('challenge','security_notice')),
    recipient_slot TEXT NOT NULL DEFAULT 'current'
        CHECK (recipient_slot IN ('current','previous')),
    CHECK (kind='security_notice' OR recipient_slot='current'),
    key_id TEXT NOT NULL CHECK (key_id ~ '^[A-Za-z0-9_-]{1,64}$'),
    payload BYTEA CHECK (octet_length(payload) BETWEEN 30 AND 2077),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','leased','accepted_by_smtp','rejected','unknown','canceled')),
    error_class TEXT CHECK (error_class IN ('smtp_accepted','smtp_temporary','smtp_permanent',
        'smtp_uncertain','lease_expired','invalidated','expired','payload_unavailable')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    lease_token TEXT CHECK (lease_token ~ '^[a-f0-9]{32}$'),
    lease_until TIMESTAMPTZ,
    smtp_started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    CHECK (expires_at > created_at AND expires_at <= created_at + interval '24 hours'),
    CHECK ((status = 'leased') = (lease_token IS NOT NULL AND lease_until IS NOT NULL)),
    CHECK (status = 'leased' OR (lease_token IS NULL AND lease_until IS NULL)),
    CHECK ((status IN ('pending','leased')) = (finished_at IS NULL)),
    CHECK (status NOT IN ('pending','leased') OR payload IS NOT NULL),
    CHECK (status NOT IN ('pending','leased') OR error_class IS NULL)
);
CREATE UNIQUE INDEX email_delivery_one_inflight
    ON core_identity.email_delivery_outbox(challenge_id,kind,recipient_slot)
    WHERE status IN ('pending','leased');
-- Security notices are not recreated after a terminal SMTP outcome.
CREATE UNIQUE INDEX email_delivery_notice_once
    ON core_identity.email_delivery_outbox(challenge_id,recipient_slot)
    WHERE kind='security_notice';
CREATE INDEX email_delivery_pending ON core_identity.email_delivery_outbox(created_at,id)
    WHERE status = 'pending';
CREATE INDEX email_delivery_retention ON core_identity.email_delivery_outbox(expires_at)
    WHERE payload IS NOT NULL;

CREATE FUNCTION core_identity.protect_email_delivery() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF (NEW.id,NEW.challenge_id,NEW.kind,NEW.recipient_slot,NEW.key_id,NEW.created_at,NEW.expires_at)
        IS DISTINCT FROM (OLD.id,OLD.challenge_id,OLD.kind,OLD.recipient_slot,OLD.key_id,OLD.created_at,OLD.expires_at)
        OR (NEW.payload IS DISTINCT FROM OLD.payload AND NEW.payload IS NOT NULL)
        OR (OLD.smtp_started_at IS NOT NULL AND NEW.smtp_started_at IS DISTINCT FROM OLD.smtp_started_at)
        OR (OLD.status NOT IN ('pending','leased') AND
            (NEW.status,NEW.error_class,NEW.lease_token,NEW.lease_until,NEW.finished_at)
            IS DISTINCT FROM
            (OLD.status,OLD.error_class,OLD.lease_token,OLD.lease_until,OLD.finished_at))
        OR (OLD.status = 'leased' AND NEW.status = 'pending')
        OR (OLD.status = 'leased' AND NEW.status = 'leased' AND NEW.lease_token <> OLD.lease_token)
    THEN
        RAISE EXCEPTION 'invalid mail transition' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER email_delivery_guard BEFORE UPDATE ON core_identity.email_delivery_outbox
    FOR EACH ROW EXECUTE FUNCTION core_identity.protect_email_delivery();

-- The challenge owner already holds its lock. Keep the same challenge -> mail
-- lock order in enqueue, prepare, finish and maintenance.
CREATE FUNCTION core_identity.invalidate_email_delivery() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF NEW.state <> 'active' THEN
        UPDATE core_identity.email_delivery_outbox SET
            payload = NULL,
            status = CASE WHEN status IN ('pending','leased')
                THEN CASE WHEN smtp_started_at IS NULL THEN 'canceled' ELSE 'unknown' END
                ELSE status END,
            error_class = CASE WHEN status IN ('pending','leased') THEN 'invalidated' ELSE error_class END,
            finished_at = CASE WHEN status IN ('pending','leased') THEN clock_timestamp() ELSE finished_at END,
            lease_token = NULL, lease_until = NULL
        WHERE challenge_id = NEW.id AND kind = 'challenge' AND payload IS NOT NULL;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER email_delivery_challenge_end AFTER UPDATE OF state ON core_identity.email_challenges
    FOR EACH ROW EXECUTE FUNCTION core_identity.invalidate_email_delivery();
REVOKE ALL ON core_identity.email_delivery_outbox FROM PUBLIC;
REVOKE ALL ON FUNCTION core_identity.protect_email_delivery(),
    core_identity.invalidate_email_delivery() FROM PUBLIC;

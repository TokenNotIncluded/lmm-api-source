-- Fresh-install model request admission. Runtime receives narrow function rights.
CREATE SCHEMA core_request_limits;

CREATE TABLE core_request_limits.policy (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    window_seconds INTEGER NOT NULL DEFAULT 60 CHECK (window_seconds BETWEEN 1 AND 86400),
    user_limit INTEGER CHECK (user_limit BETWEEN 1 AND 100000000),
    group_limits JSONB NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(group_limits) = 'object'),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
INSERT INTO core_request_limits.policy(singleton) VALUES (TRUE);

CREATE TABLE core_request_limits.auto_policy (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    groups TEXT[] NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
INSERT INTO core_request_limits.auto_policy(singleton) VALUES (TRUE);

CREATE TABLE core_request_limits.buckets (
    scope SMALLINT NOT NULL CHECK (scope IN (1, 2)),
    subject TEXT NOT NULL CHECK (length(subject) BETWEEN 1 AND 64),
    window_seconds INTEGER NOT NULL CHECK (window_seconds BETWEEN 1 AND 86400),
    window_started BIGINT NOT NULL CHECK (window_started >= 0),
    used INTEGER NOT NULL DEFAULT 0 CHECK (used BETWEEN 0 AND 100000000),
    PRIMARY KEY (scope, subject, window_seconds, window_started)
);
CREATE INDEX request_limit_bucket_expiry ON core_request_limits.buckets
    ((window_started + window_seconds));

CREATE TABLE core_request_limits.admissions (
    scope SMALLINT NOT NULL,
    subject TEXT NOT NULL,
    window_seconds INTEGER NOT NULL,
    window_started BIGINT NOT NULL,
    user_id BIGINT NOT NULL CHECK (user_id > 0),
    request_id TEXT NOT NULL CHECK (length(request_id) BETWEEN 1 AND 128),
    PRIMARY KEY (scope, subject, window_seconds, window_started, user_id, request_id),
    FOREIGN KEY (scope, subject, window_seconds, window_started)
        REFERENCES core_request_limits.buckets ON DELETE CASCADE
);

CREATE TABLE core_request_limits.policy_events (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor_user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    kind TEXT NOT NULL CHECK (kind IN ('request_limits', 'auto_groups')),
    revision BIGINT NOT NULL CHECK (revision > 0),
    document JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE FUNCTION core_request_limits.write_policy(
    p_revision BIGINT, p_enabled BOOLEAN, p_window INTEGER, p_user_limit INTEGER,
    p_group_limits JSONB, p_actor BIGINT
) RETURNS BIGINT LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE result BIGINT;
BEGIN
    IF p_revision IS NULL OR p_revision < 1 OR p_enabled IS NULL
        OR p_window IS NULL OR p_window NOT BETWEEN 1 AND 86400
        OR (p_user_limit IS NOT NULL AND p_user_limit NOT BETWEEN 1 AND 100000000)
        OR p_group_limits IS NULL OR jsonb_typeof(p_group_limits) <> 'object'
        OR p_actor IS NULL OR p_actor <= 0 THEN
        RAISE EXCEPTION 'invalid request limit policy' USING ERRCODE = '22023';
    END IF;
    IF (SELECT count(*) FROM jsonb_each(p_group_limits)) > 1024
        OR EXISTS (SELECT FROM jsonb_each(p_group_limits) AS entry
            WHERE length(entry.key) NOT BETWEEN 1 AND 64
                OR jsonb_typeof(entry.value) <> 'number'
                OR entry.value::text !~ '^[1-9][0-9]{0,8}$'
                OR (entry.value::text)::numeric > 100000000)
        OR (p_enabled AND p_user_limit IS NULL AND p_group_limits = '{}') THEN
        RAISE EXCEPTION 'invalid request limit policy' USING ERRCODE = '22023';
    END IF;
    UPDATE core_request_limits.policy SET revision = revision + 1,
        enabled = p_enabled, window_seconds = p_window, user_limit = p_user_limit,
        group_limits = p_group_limits, updated_at = clock_timestamp()
        WHERE singleton AND revision = p_revision RETURNING revision INTO result;
    IF result IS NOT NULL THEN
        INSERT INTO core_request_limits.policy_events(actor_user_id, kind, revision, document)
        VALUES (p_actor, 'request_limits', result, jsonb_build_object(
            'enabled', p_enabled, 'window_seconds', p_window,
            'user_limit', p_user_limit, 'group_limits', p_group_limits));
    END IF;
    RETURN result;
END
$$;

CREATE FUNCTION core_request_limits.write_auto_groups(
    p_revision BIGINT, p_groups TEXT[], p_actor BIGINT
) RETURNS BIGINT LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE result BIGINT;
BEGIN
    IF p_revision IS NULL OR p_revision < 1 OR p_groups IS NULL
        OR cardinality(p_groups) > 1024 OR p_actor IS NULL OR p_actor <= 0
        OR EXISTS (SELECT FROM unnest(p_groups) AS name
            WHERE name IS NULL OR length(name) NOT BETWEEN 1 AND 64)
        OR (SELECT count(DISTINCT name) FROM unnest(p_groups) AS name) <> cardinality(p_groups) THEN
        RAISE EXCEPTION 'invalid Auto group order' USING ERRCODE = '22023';
    END IF;
    UPDATE core_request_limits.auto_policy SET revision = revision + 1,
        groups = p_groups, updated_at = clock_timestamp()
        WHERE singleton AND revision = p_revision RETURNING revision INTO result;
    IF result IS NOT NULL THEN
        INSERT INTO core_request_limits.policy_events(actor_user_id, kind, revision, document)
        VALUES (p_actor, 'auto_groups', result, jsonb_build_object('groups', p_groups));
    END IF;
    RETURN result;
END
$$;

-- Both scopes are checked before either is consumed. Entity locks are acquired
-- before reading the DB clock, so a waiter crossing a boundary uses the new window.
CREATE FUNCTION core_request_limits.admit(p_user BIGINT, p_group TEXT, p_request TEXT)
RETURNS TABLE(allowed BOOLEAN, retry_after_seconds BIGINT)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    config core_request_limits.policy%ROWTYPE;
    group_cap INTEGER;
    now_seconds BIGINT;
    started BIGINT;
    user_used INTEGER;
    group_used INTEGER;
    user_new BOOLEAN := FALSE;
    group_new BOOLEAN := FALSE;
BEGIN
    IF p_user IS NULL OR p_user <= 0 OR p_group IS NULL
        OR length(p_group) NOT BETWEEN 1 AND 64 OR p_request IS NULL
        OR length(p_request) NOT BETWEEN 1 AND 128 THEN
        RAISE EXCEPTION 'invalid request admission' USING ERRCODE = '22023';
    END IF;
    SELECT * INTO STRICT config FROM core_request_limits.policy WHERE singleton FOR SHARE;
    IF NOT config.enabled THEN
        RETURN QUERY SELECT TRUE, 0::BIGINT;
        RETURN;
    END IF;
    group_cap := (config.group_limits ->> p_group)::INTEGER;
    IF config.user_limit IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(1667520001, hashtext(p_user::TEXT));
    END IF;
    IF group_cap IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(1667520002, hashtext(p_group));
    END IF;
    now_seconds := floor(extract(epoch FROM clock_timestamp()))::BIGINT;
    started := now_seconds - (now_seconds % config.window_seconds);

    IF config.user_limit IS NOT NULL THEN
        INSERT INTO core_request_limits.buckets(scope, subject, window_seconds, window_started)
            VALUES (1, p_user::TEXT, config.window_seconds, started) ON CONFLICT DO NOTHING;
        SELECT used INTO user_used FROM core_request_limits.buckets
            WHERE scope = 1 AND subject = p_user::TEXT
                AND window_seconds = config.window_seconds AND window_started = started;
        user_new := NOT EXISTS (SELECT FROM core_request_limits.admissions
            WHERE scope = 1 AND subject = p_user::TEXT AND user_id = p_user
                AND window_seconds = config.window_seconds AND window_started = started
                AND request_id = p_request);
    END IF;
    IF group_cap IS NOT NULL THEN
        INSERT INTO core_request_limits.buckets(scope, subject, window_seconds, window_started)
            VALUES (2, p_group, config.window_seconds, started) ON CONFLICT DO NOTHING;
        SELECT used INTO group_used FROM core_request_limits.buckets
            WHERE scope = 2 AND subject = p_group
                AND window_seconds = config.window_seconds AND window_started = started;
        group_new := NOT EXISTS (SELECT FROM core_request_limits.admissions
            WHERE scope = 2 AND subject = p_group AND user_id = p_user
                AND window_seconds = config.window_seconds AND window_started = started
                AND request_id = p_request);
    END IF;
    IF (user_new AND user_used >= config.user_limit)
        OR (group_new AND group_used >= group_cap) THEN
        RETURN QUERY SELECT FALSE, started + config.window_seconds - now_seconds;
        RETURN;
    END IF;
    IF user_new THEN
        UPDATE core_request_limits.buckets SET used = used + 1
            WHERE scope = 1 AND subject = p_user::TEXT
                AND window_seconds = config.window_seconds AND window_started = started;
        INSERT INTO core_request_limits.admissions VALUES
            (1, p_user::TEXT, config.window_seconds, started, p_user, p_request);
    END IF;
    IF group_new THEN
        UPDATE core_request_limits.buckets SET used = used + 1
            WHERE scope = 2 AND subject = p_group
                AND window_seconds = config.window_seconds AND window_started = started;
        INSERT INTO core_request_limits.admissions VALUES
            (2, p_group, config.window_seconds, started, p_user, p_request);
    END IF;
    DELETE FROM core_request_limits.buckets WHERE (scope, subject, window_seconds, window_started) IN (
        SELECT scope, subject, window_seconds, window_started FROM core_request_limits.buckets
        WHERE window_started + window_seconds <= now_seconds
        ORDER BY window_started + window_seconds LIMIT 100 FOR UPDATE SKIP LOCKED
    );
    RETURN QUERY SELECT TRUE, 0::BIGINT;
END
$$;

REVOKE ALL ON SCHEMA core_request_limits FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_request_limits FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_request_limits FROM PUBLIC;
REVOKE ALL ON FUNCTION core_request_limits.admit(BIGINT, TEXT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION core_request_limits.write_policy(BIGINT, BOOLEAN, INTEGER, INTEGER, JSONB, BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION core_request_limits.write_auto_groups(BIGINT, TEXT[], BIGINT) FROM PUBLIC;

-- Fresh-install console compatibility facts, not a runtime migration.
-- Announcement semantics originate in main 7be6ee81 model/mandatory_announcement.go.
CREATE SCHEMA core_console;
CREATE TABLE core_console.options (
    key TEXT PRIMARY KEY CHECK (length(key) BETWEEN 1 AND 128),
    value TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);
CREATE TABLE core_console.announcement_reads (
    user_id BIGINT NOT NULL REFERENCES core_identity.users(id),
    announcement_id BIGINT NOT NULL CHECK (announcement_id > 0),
    revision TEXT NOT NULL CHECK (revision ~ '^[a-f0-9]{64}$'),
    read_at BIGINT NOT NULL DEFAULT floor(extract(epoch FROM statement_timestamp()))::BIGINT
        CHECK (read_at > 0),
    PRIMARY KEY (user_id, announcement_id, revision)
);
CREATE TABLE core_console.initialization (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    version TEXT NOT NULL CHECK (length(version) BETWEEN 1 AND 128),
    initialized_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
);
-- Created by the same installation owner as options. Runtime receives only
-- EXECUTE; these functions cannot write any other option or relation.
CREATE FUNCTION core_console.lock_public_options() RETURNS VOID
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    LOCK TABLE core_console.options IN SHARE ROW EXCLUSIVE MODE;
END
$$;
CREATE FUNCTION core_console.write_public_option(p_key TEXT, p_value TEXT) RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE result BIGINT;
BEGIN
    IF p_key IS NULL OR p_key NOT IN ('Notice', 'Announcements', 'console_setting.announcements') THEN
        RAISE EXCEPTION 'unsupported public option' USING ERRCODE = '22023';
    END IF;
    PERFORM core_console.lock_public_options();
    INSERT INTO core_console.options AS o(key, value) VALUES (p_key, p_value)
    ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value,
        revision = o.revision + 1, updated_at = statement_timestamp()
    RETURNING revision INTO result;
    RETURN result;
END
$$;
REVOKE ALL ON FUNCTION core_console.lock_public_options() FROM PUBLIC;
REVOKE ALL ON FUNCTION core_console.write_public_option(TEXT, TEXT) FROM PUBLIC;
REVOKE ALL ON SCHEMA core_console FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_console FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_console FROM PUBLIC;

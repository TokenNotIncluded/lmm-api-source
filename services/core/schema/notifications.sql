-- Fresh-install notification storage. The ledger remains read-only here.
CREATE SCHEMA core_notifications;
CREATE TABLE core_notifications.preferences (
    user_id bigint PRIMARY KEY REFERENCES core_identity.users(id),
    enabled boolean NOT NULL DEFAULT true,
    low_balance_units bigint NOT NULL DEFAULT 0 CHECK(low_balance_units BETWEEN 0 AND 9007199254740991),
    below_threshold boolean NOT NULL DEFAULT false,
    revision bigint NOT NULL DEFAULT 1 CHECK(revision>0),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE core_notifications.inbox (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id bigint NOT NULL REFERENCES core_identity.users(id),
    source_key text NOT NULL CHECK(length(source_key) BETWEEN 1 AND 128),
    category text NOT NULL CHECK(category IN ('account_action','security_incident','moderation','human_support','open_source_bounty_review','open_source_bounty')),
    kind text NOT NULL CHECK(length(kind) BETWEEN 1 AND 64),
    title text NOT NULL CHECK(length(title) BETWEEN 1 AND 200),
    summary text NOT NULL CHECK(length(summary)<=2000),
    details jsonb NOT NULL DEFAULT '{}' CHECK(jsonb_typeof(details)='object' AND octet_length(details::text)<=16384),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    read_at timestamptz,
    UNIQUE(user_id,source_key)
);
CREATE INDEX inbox_user_page ON core_notifications.inbox(user_id,id DESC);
CREATE INDEX inbox_user_unread ON core_notifications.inbox(user_id,category) WHERE read_at IS NULL;

-- Only a committed routing publication creates public price events. Drafts,
-- failed CAS operations and aborted transactions cannot announce a price.
CREATE TABLE core_notifications.price_events (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    release_version bigint NOT NULL REFERENCES public.routing_config_versions(version),
    price_version bigint NOT NULL REFERENCES public.routing_price_versions(version),
    model text NOT NULL,
    group_name text NOT NULL,
    speed text NOT NULL,
    previous_price jsonb,
    current_price jsonb,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(release_version,model,group_name,speed)
);
CREATE INDEX price_events_group_page ON core_notifications.price_events(group_name,id DESC);
CREATE FUNCTION core_notifications.capture_price_publication() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE old_price bigint; new_price bigint;
BEGIN
    SELECT price_version INTO old_price FROM public.routing_config_versions WHERE version=OLD.config_version;
    SELECT price_version INTO new_price FROM public.routing_config_versions WHERE version=NEW.config_version;
    IF old_price IS NOT DISTINCT FROM new_price THEN RETURN NEW; END IF;
    INSERT INTO core_notifications.price_events(release_version,price_version,model,group_name,speed,previous_price,current_price)
    WITH previous AS (SELECT r->>'model' AS model,r->>'group' AS group_name,r->>'speed' AS speed,r-ARRAY['id','model','group','speed'] AS price
           FROM public.routing_price_versions v CROSS JOIN LATERAL jsonb_array_elements(v.document->'rules') r WHERE v.version=old_price),
         current AS (SELECT r->>'model' AS model,r->>'group' AS group_name,r->>'speed' AS speed,r-ARRAY['id','model','group','speed'] AS price
           FROM public.routing_price_versions v CROSS JOIN LATERAL jsonb_array_elements(v.document->'rules') r WHERE v.version=new_price),
         changes AS (
           SELECT coalesce(n.model,o.model) AS model,coalesce(n.group_name,o.group_name) AS group_name,coalesce(n.speed,o.speed) AS speed,
           o.price AS previous_price,n.price AS current_price
           FROM previous o FULL JOIN current n USING(model,group_name,speed)
         )
    SELECT NEW.config_version,new_price,model,group_name,speed,previous_price,current_price FROM changes
    WHERE previous_price IS DISTINCT FROM current_price ORDER BY group_name,model,speed;
    RETURN NEW;
END;
$$;
CREATE TRIGGER notification_price_publication AFTER UPDATE ON public.routing_active_release
FOR EACH ROW EXECUTE FUNCTION core_notifications.capture_price_publication();

-- Security changes appear in the affected account's inbox in the same
-- transaction as their immutable audit. No audit payload or secret is copied.
CREATE FUNCTION core_notifications.capture_account_audit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE affected bigint; headline text;
BEGIN
    headline := CASE NEW.action
      WHEN 'admin.user.disable' THEN 'Account suspended'
      WHEN 'admin.user.enable' THEN 'Account restored'
      WHEN 'admin.user.credentials.revoke' THEN 'Account credentials revoked'
      WHEN 'admin.user.mfa.reset' THEN 'Two-factor authentication reset'
      ELSE NULL END;
    IF headline IS NULL OR coalesce(NEW.target->>'user_id','') !~ '^[1-9][0-9]{0,17}$' THEN RETURN NEW; END IF;
    affected := (NEW.target->>'user_id')::bigint;
    IF NOT EXISTS(SELECT 1 FROM core_identity.users WHERE id=affected) THEN RETURN NEW; END IF;
    INSERT INTO core_notifications.inbox(user_id,source_key,category,kind,title,summary)
      VALUES(affected,'identity-audit:'||NEW.id::text,'security_incident',NEW.action,headline,'An administrator changed your account security. Contact support if you did not expect this change.')
      ON CONFLICT(user_id,source_key) DO NOTHING;
    RETURN NEW;
END;
$$;
CREATE TRIGGER notification_account_audit AFTER INSERT ON core_identity.audit
FOR EACH ROW EXECUTE FUNCTION core_notifications.capture_account_audit();

REVOKE ALL ON FUNCTION core_notifications.capture_price_publication(),core_notifications.capture_account_audit() FROM PUBLIC;
REVOKE ALL ON SCHEMA core_notifications FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA core_notifications FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA core_notifications FROM PUBLIC;

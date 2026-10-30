-- Channel control adds a SECURITY DEFINER entry under pg_catalog only.
-- Existing offline routing helpers were invoker functions with unqualified
-- table references. Rebind those four equivalent bodies here, in this new
-- component, so the immutable original routing component remains unchanged.
-- This is an explicit owner-run schema upgrade, never normal startup DDL.
CREATE OR REPLACE FUNCTION public.routing_guard_projection() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE
    v bigint;
    is_sealed boolean;
BEGIN
    IF TG_OP = 'UPDATE' AND NEW.version <> OLD.version THEN
        RAISE EXCEPTION 'routing version is immutable' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN v := OLD.version; ELSE v := NEW.version; END IF;
    IF TG_ARGV[0] = 'price' THEN
        SELECT sealed INTO is_sealed FROM public.routing_price_versions WHERE version = v FOR SHARE;
    ELSE
        SELECT sealed INTO is_sealed FROM public.routing_config_versions WHERE version = v FOR SHARE;
    END IF;
    IF is_sealed IS DISTINCT FROM false THEN
        RAISE EXCEPTION 'routing snapshot is sealed' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN RETURN OLD; ELSE RETURN NEW; END IF;
END;
$$;

CREATE OR REPLACE FUNCTION public.routing_guard_active() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE next_price bigint; old_price bigint; ready boolean;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'routing active slot cannot be removed' USING ERRCODE = '23514';
    END IF;
    IF NEW.config_version IS NULL OR (OLD.config_version IS NOT NULL AND NEW.config_version <= OLD.config_version) THEN
        RAISE EXCEPTION 'routing version must increase' USING ERRCODE = '40001';
    END IF;
    SELECT c.price_version, c.sealed AND p.sealed INTO next_price, ready
        FROM public.routing_config_versions c JOIN public.routing_price_versions p ON p.version = c.price_version
        WHERE c.version = NEW.config_version;
    SELECT price_version INTO old_price FROM public.routing_config_versions WHERE version = OLD.config_version;
    IF ready IS DISTINCT FROM true OR next_price < old_price THEN
        RAISE EXCEPTION 'routing release is not ready' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION public.routing_stage_prices(p_document jsonb) RETURNS void LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE v bigint := (p_document->>'version')::bigint; existing jsonb; inserted bigint;
BEGIN
    INSERT INTO public.routing_price_versions(version, document) VALUES (v, p_document)
        ON CONFLICT (version) DO NOTHING RETURNING version INTO inserted;
    IF inserted IS NULL THEN
        SELECT document INTO existing FROM public.routing_price_versions WHERE version = v AND sealed;
        IF existing IS DISTINCT FROM p_document THEN
            RAISE EXCEPTION 'price version already exists' USING ERRCODE = '23505';
        END IF;
        RETURN;
    END IF;
    INSERT INTO public.routing_price_rules
        SELECT v, (r->>'id')::bigint, r->>'model', r->>'group', r->>'speed',
            (r->'rates'->>'input')::bigint, (r->'rates'->>'output')::bigint,
            (r->'rates'->>'cache_read')::bigint, (r->'rates'->>'cache_write')::bigint,
            (r->'rates'->>'request')::bigint,
            (r->>'group_multiplier_ppm')::bigint, (r->>'speed_multiplier_ppm')::bigint
        FROM jsonb_array_elements(p_document->'rules') r;
    UPDATE public.routing_price_versions SET sealed = true WHERE version = v;
END;
$$;

CREATE OR REPLACE FUNCTION public.routing_publish_config(
    p_document jsonb, p_price bigint, p_expected_config bigint, p_expected_price bigint
) RETURNS void LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE
    v bigint := (p_document->>'version')::bigint;
    old_config bigint;
    old_price bigint;
BEGIN
    SELECT config_version INTO old_config FROM public.routing_active_release WHERE singleton FOR UPDATE;
    SELECT price_version INTO old_price FROM public.routing_config_versions WHERE version = old_config;
    IF old_config IS DISTINCT FROM p_expected_config OR old_price IS DISTINCT FROM p_expected_price
        OR (old_config IS NOT NULL AND v <= old_config) THEN
        RAISE EXCEPTION 'routing publication conflict' USING ERRCODE = '40001';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM public.routing_price_versions WHERE version = p_price AND sealed)
        OR (old_price IS NOT NULL AND p_price < old_price) THEN
        RAISE EXCEPTION 'routing price version is unavailable' USING ERRCODE = '23514';
    END IF;
    INSERT INTO public.routing_config_versions(version, price_version, document) VALUES (v, p_price, p_document);
    INSERT INTO public.routing_groups
        SELECT v, g->>'name', (g->>'enabled')::boolean FROM jsonb_array_elements(p_document->'groups') g;
    INSERT INTO public.routing_upstreams
        SELECT v, (u->>'id')::bigint, u->>'endpoint', u->>'protocol',
            (u->'credential'->>'id')::bigint, (u->'credential'->>'revision')::bigint,
            (u->>'enabled')::boolean FROM jsonb_array_elements(p_document->'upstreams') u;
    INSERT INTO public.routing_routes
        SELECT v, (r->>'id')::bigint, r->>'group', r->>'model', (r->>'enabled')::boolean,
            ARRAY(SELECT jsonb_array_elements_text(r->'speeds')),
            (r->'failover'->>'max_attempts')::smallint,
            ARRAY(SELECT jsonb_array_elements_text(r->'failover'->'retry_on')),
            (r->'failover'->>'allow_replay_after_send')::boolean
        FROM jsonb_array_elements(p_document->'routes') r;
    INSERT INTO public.routing_targets
        SELECT v, (t->>'id')::bigint, (r->>'id')::bigint, (t->>'upstream_id')::bigint,
            t->>'upstream_model', (t->>'enabled')::boolean, (t->>'priority')::integer,
            (t->>'weight')::bigint, ARRAY(SELECT jsonb_array_elements_text(t->'speeds')),
            t->'payload', (t->>'timeout_ms')::integer
        FROM jsonb_array_elements(p_document->'routes') r,
            LATERAL jsonb_array_elements(r->'targets') t;
    INSERT INTO public.routing_aliases
        SELECT v, a->>'group', a->>'alias', a->>'model'
        FROM jsonb_array_elements(p_document->'aliases') a;
    UPDATE public.routing_config_versions SET sealed = true WHERE version = v;
    UPDATE public.routing_active_release SET config_version = v WHERE singleton;
END;
$$;

-- Installed only by the offline schema owner. Runtime receives EXECUTE on these
-- two functions, never DML on routing artifacts or access to credential files.
CREATE FUNCTION public.routing_control_read() RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT jsonb_build_object('contract_version',1,'routing',c.document,'pricing',p.document)
 FROM public.routing_active_release a
 JOIN public.routing_config_versions c ON c.version=a.config_version AND c.sealed
 JOIN public.routing_price_versions p ON p.version=c.price_version AND p.sealed
 WHERE a.singleton
$$;

CREATE FUNCTION public.routing_control_publish(
 p_routing jsonb,p_prices jsonb,p_expected_config bigint,p_expected_price bigint
) RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 -- The Rust writer validates the document against loaded credential/provider
 -- bindings and retains Session/policy locks and the audit in this transaction.
 -- References identify immutable host-loaded secrets; they contain no material.
 INSERT INTO public.routing_credential_refs(id,revision)
 SELECT DISTINCT (u->'credential'->>'id')::bigint,(u->'credential'->>'revision')::bigint
 FROM jsonb_array_elements(p_routing->'upstreams') u ON CONFLICT DO NOTHING;
 PERFORM public.routing_stage_prices(p_prices);
 PERFORM public.routing_publish_config(p_routing,(p_prices->>'version')::bigint,
                                      p_expected_config,p_expected_price);
END;
$$;
REVOKE ALL ON FUNCTION public.routing_control_read() FROM PUBLIC;
REVOKE ALL ON FUNCTION public.routing_control_publish(jsonb,jsonb,bigint,bigint) FROM PUBLIC;

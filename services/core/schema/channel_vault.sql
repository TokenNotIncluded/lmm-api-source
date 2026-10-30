-- Explicit second owner-run component. No startup DDL, no plaintext secrets.
ALTER TABLE public.routing_upstreams DROP CONSTRAINT routing_upstreams_protocol_check;
ALTER TABLE public.routing_upstreams ADD CONSTRAINT routing_upstreams_protocol_check CHECK(protocol IN ('openai_chat','openai_responses','anthropic_messages','gemini','system_one','openai_embeddings','jina_rerank','deepseek_search','deepseek_fetch'));
ALTER TABLE public.routing_price_rules DROP CONSTRAINT routing_price_rules_group_multiplier_ppm_check;
ALTER TABLE public.routing_price_rules ADD CONSTRAINT routing_price_rules_group_multiplier_ppm_check CHECK(group_multiplier_ppm BETWEEN 0 AND 1000000000);
CREATE TABLE public.channel_release_bindings (
 config_version bigint PRIMARY KEY REFERENCES public.routing_config_versions(version),
 document jsonb NOT NULL CHECK(jsonb_typeof(document)='object' AND octet_length(document::text)<=16777216)
);
CREATE TABLE public.channel_secret_revisions (
 upstream_id bigint NOT NULL CHECK(upstream_id>0),
 id bigint NOT NULL,
 revision bigint NOT NULL,
 label text NOT NULL CHECK(length(label) BETWEEN 1 AND 128),
 ciphertext bytea NOT NULL CHECK(octet_length(ciphertext) BETWEEN 29 AND 4124),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 removed_at timestamptz,
 PRIMARY KEY(id,revision),
 FOREIGN KEY(id,revision) REFERENCES public.routing_credential_refs(id,revision)
);
CREATE FUNCTION public.routing_control_publish(
 p_routing jsonb,p_prices jsonb,p_bindings jsonb,p_expected_config bigint,p_expected_price bigint
) RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 PERFORM singleton FROM public.routing_active_release WHERE singleton FOR UPDATE;
 IF EXISTS(SELECT 1 FROM jsonb_array_elements(p_routing->'upstreams') u JOIN public.channel_secret_revisions s ON s.id=(u->'credential'->>'id')::bigint AND s.revision=(u->'credential'->>'revision')::bigint WHERE s.removed_at IS NOT NULL) THEN
  RAISE EXCEPTION 'credential is retired' USING ERRCODE='40001';
 END IF;
 PERFORM public.routing_control_publish(p_routing,p_prices,p_expected_config,p_expected_price);
 INSERT INTO public.channel_release_bindings(config_version,document) VALUES((p_routing->>'version')::bigint,p_bindings);
END;
$$;
CREATE FUNCTION public.routing_control_bindings(p_version bigint) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT document FROM public.channel_release_bindings WHERE config_version=p_version
$$;
CREATE FUNCTION public.routing_control_secret_read(p_id bigint,p_revision bigint) RETURNS bytea
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT ciphertext FROM public.channel_secret_revisions WHERE id=p_id AND revision=p_revision AND removed_at IS NULL
$$;
CREATE FUNCTION public.routing_control_secret_create(p_upstream bigint,p_id bigint,p_revision bigint,p_label text,p_ciphertext bytea) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 INSERT INTO public.routing_credential_refs(id,revision) VALUES(p_id,p_revision);
 INSERT INTO public.channel_secret_revisions(upstream_id,id,revision,label,ciphertext) VALUES(p_upstream,p_id,p_revision,p_label,p_ciphertext);
END;
$$;
CREATE FUNCTION public.routing_control_secret_list(p_upstream bigint) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT coalesce(jsonb_agg(jsonb_build_object('reference',jsonb_build_object('id',id,'revision',revision),'label',label,'created_at',created_at) ORDER BY created_at,id),'[]'::jsonb)
 FROM public.channel_secret_revisions WHERE upstream_id=p_upstream AND removed_at IS NULL
$$;
CREATE FUNCTION public.routing_control_secret_remove(p_upstream bigint,p_id bigint,p_revision bigint) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE changed boolean;
BEGIN
 -- Serialize retirement against activation under the release pointer lock.
 PERFORM singleton FROM public.routing_active_release WHERE singleton FOR UPDATE;
 IF EXISTS(SELECT 1 FROM public.routing_upstreams u JOIN public.routing_active_release a ON a.config_version=u.version WHERE u.credential_id=p_id AND u.credential_revision=p_revision) THEN
  RAISE EXCEPTION 'credential is active' USING ERRCODE='40001';
 END IF;
 UPDATE public.channel_secret_revisions SET removed_at=clock_timestamp() WHERE upstream_id=p_upstream AND id=p_id AND revision=p_revision AND removed_at IS NULL;
 changed:=FOUND;
 RETURN changed;
END;
$$;
REVOKE ALL ON FUNCTION public.routing_control_publish(jsonb,jsonb,jsonb,bigint,bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.routing_control_bindings(bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.routing_control_secret_read(bigint,bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.routing_control_secret_create(bigint,bigint,bigint,text,bytea) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.routing_control_secret_list(bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.routing_control_secret_remove(bigint,bigint,bigint) FROM PUBLIC;

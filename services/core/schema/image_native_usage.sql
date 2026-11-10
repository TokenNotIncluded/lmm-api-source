-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Component 65: additive native image receipt support. Owner install/upgrade only.
-- No new payer, ledger authority, table, redispatch path or historical SQL edit.

CREATE FUNCTION core_image.native_counter(value JSONB, maximum NUMERIC) RETURNS BOOLEAN
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
BEGIN
 IF jsonb_typeof(value) IS DISTINCT FROM 'number'
    OR value::text !~ '^(0|[1-9][0-9]{0,19})$' THEN RETURN FALSE; END IF;
 RETURN value::text::numeric <= maximum;
END $$;

-- Private normalized receipt. Null cache counters mean UNKNOWN, never zero.
CREATE FUNCTION core_image.valid_native_tokens(value JSONB) RETURNS BOOLEAN
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
DECLARE name TEXT; i NUMERIC; t NUMERIC; m NUMERIC; o NUMERIC;
 c NUMERIC; ct NUMERIC; cm NUMERIC;
BEGIN
 IF jsonb_typeof(value) IS DISTINCT FROM 'object' OR octet_length(value::text)>4096
    OR NOT value ?& ARRAY['input','input_text','input_image','cached','cached_text','cached_image','output','output_image','output_image_source']
    OR value-ARRAY['input','input_text','input_image','cached','cached_text','cached_image','output','output_image','output_image_source']<>'{}'::jsonb THEN RETURN FALSE; END IF;
 FOREACH name IN ARRAY ARRAY['input','input_text','input_image','output','output_image'] LOOP
  IF core_image.native_counter(value->name,9223372036854775807) IS NOT TRUE THEN RETURN FALSE; END IF;
 END LOOP;
 FOREACH name IN ARRAY ARRAY['cached','cached_text','cached_image'] LOOP
  IF value->name<>'null'::jsonb AND core_image.native_counter(value->name,9223372036854775807) IS NOT TRUE THEN RETURN FALSE; END IF;
 END LOOP;
 IF jsonb_typeof(value->'output_image_source') IS DISTINCT FROM 'string'
    OR value->>'output_image_source' NOT IN ('reported_details','images_endpoint_total') THEN RETURN FALSE; END IF;
 i:=(value->>'input')::numeric; t:=(value->>'input_text')::numeric;
 m:=(value->>'input_image')::numeric; o:=(value->>'output')::numeric;
 IF t+m<>i OR (value->>'output_image')::numeric>o
    OR (value->>'output_image_source'='images_endpoint_total' AND (value->>'output_image')::numeric<>o) THEN RETURN FALSE; END IF;
 c:=(value->>'cached')::numeric; ct:=(value->>'cached_text')::numeric; cm:=(value->>'cached_image')::numeric;
 IF c>i OR ct>t OR cm>m THEN RETURN FALSE; END IF;
 IF ct IS NULL AND cm IS NULL THEN RETURN TRUE; END IF;
 RETURN (c IS NOT NULL AND ct IS NOT NULL AND cm IS NOT NULL AND ct+cm=c) IS TRUE;
END $$;

-- Raw native Images usage, not Chat usage. The Images endpoint's output total
-- can be interpreted as image tokens only when no explicit breakdown exists.
CREATE FUNCTION core_image.valid_native_usage(value JSONB) RETURNS BOOLEAN
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
DECLARE details JSONB; cached_details JSONB; output_details JSONB;
 c JSONB := 'null'::jsonb; ct JSONB := 'null'::jsonb; cm JSONB := 'null'::jsonb;
 oi JSONB; origin TEXT; name TEXT;
BEGIN
 IF jsonb_typeof(value) IS DISTINCT FROM 'object' OR octet_length(value::text)>8192
    OR NOT value ?& ARRAY['input_tokens','output_tokens','total_tokens','input_tokens_details']
    OR value-ARRAY['input_tokens','output_tokens','total_tokens','input_tokens_details','output_tokens_details']<>'{}'::jsonb THEN RETURN FALSE; END IF;
 FOREACH name IN ARRAY ARRAY['input_tokens','output_tokens'] LOOP
  IF core_image.native_counter(value->name,9223372036854775807) IS NOT TRUE THEN RETURN FALSE; END IF;
 END LOOP;
 IF core_image.native_counter(value->'total_tokens',18446744073709551615) IS NOT TRUE THEN RETURN FALSE; END IF;
 IF (value->>'total_tokens')::numeric<>(value->>'input_tokens')::numeric+(value->>'output_tokens')::numeric THEN RETURN FALSE; END IF;
 details:=value->'input_tokens_details';
 IF jsonb_typeof(details) IS DISTINCT FROM 'object'
    OR NOT details ?& ARRAY['text_tokens','image_tokens']
    OR details-ARRAY['text_tokens','image_tokens','cached_tokens','cached_tokens_details']<>'{}'::jsonb THEN RETURN FALSE; END IF;
 FOREACH name IN ARRAY ARRAY['text_tokens','image_tokens'] LOOP
  IF core_image.native_counter(details->name,9223372036854775807) IS NOT TRUE THEN RETURN FALSE; END IF;
 END LOOP;
 IF details ? 'cached_tokens' THEN
  c:=details->'cached_tokens';
  IF core_image.native_counter(c,9223372036854775807) IS NOT TRUE THEN RETURN FALSE; END IF;
 END IF;
 IF details ? 'cached_tokens_details' THEN
  cached_details:=details->'cached_tokens_details';
  IF jsonb_typeof(cached_details) IS DISTINCT FROM 'object'
     OR NOT cached_details ?& ARRAY['text_tokens','image_tokens']
     OR cached_details-ARRAY['text_tokens','image_tokens','audio_tokens']<>'{}'::jsonb THEN RETURN FALSE; END IF;
  ct:=cached_details->'text_tokens'; cm:=cached_details->'image_tokens';
  IF core_image.native_counter(ct,9223372036854775807) IS NOT TRUE
     OR core_image.native_counter(cm,9223372036854775807) IS NOT TRUE THEN RETURN FALSE; END IF;
  IF cached_details ? 'audio_tokens' AND
     core_image.native_counter(cached_details->'audio_tokens',0) IS NOT TRUE THEN RETURN FALSE; END IF;
 ELSIF c='0'::jsonb THEN ct:='0'::jsonb; cm:='0'::jsonb;
 END IF;
 IF value ? 'output_tokens_details' THEN
  output_details:=value->'output_tokens_details';
  IF jsonb_typeof(output_details) IS DISTINCT FROM 'object'
     OR NOT output_details ?& ARRAY['text_tokens','image_tokens']
     OR output_details-ARRAY['text_tokens','image_tokens']<>'{}'::jsonb THEN RETURN FALSE; END IF;
  IF core_image.native_counter(output_details->'text_tokens',9223372036854775807) IS NOT TRUE
     OR core_image.native_counter(output_details->'image_tokens',9223372036854775807) IS NOT TRUE THEN RETURN FALSE; END IF;
  IF (output_details->>'text_tokens')::numeric+(output_details->>'image_tokens')::numeric<>(value->>'output_tokens')::numeric THEN RETURN FALSE; END IF;
  oi:=output_details->'image_tokens'; origin:='reported_details';
 ELSE oi:=value->'output_tokens'; origin:='images_endpoint_total'; END IF;
 RETURN core_image.valid_native_tokens(jsonb_build_object(
  'input',value->'input_tokens','input_text',details->'text_tokens','input_image',details->'image_tokens',
  'cached',c,'cached_text',ct,'cached_image',cm,'output',value->'output_tokens',
  'output_image',oi,'output_image_source',origin));
END $$;

CREATE FUNCTION core_image.valid_image_usage(value JSONB) RETURNS BOOLEAN
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
BEGIN
 IF jsonb_typeof(value) IS DISTINCT FROM 'object' OR octet_length(value::text)>8192
    OR NOT value ?& ARRAY['output_images','provider_receipt_sha256']
    OR value-ARRAY['output_images','provider_receipt_sha256','native_tokens']<>'{}'::jsonb THEN RETURN FALSE; END IF;
 IF core_image.native_counter(value->'output_images',128) IS NOT TRUE THEN RETURN FALSE; END IF;
 IF (value->>'output_images')::integer<1
    OR jsonb_typeof(value->'provider_receipt_sha256') IS DISTINCT FROM 'array' THEN RETURN FALSE; END IF;
 IF jsonb_array_length(value->'provider_receipt_sha256')<>32
    OR jsonb_path_exists(value->'provider_receipt_sha256','$[*] ? (@.type() != "number" || @ < 0 || @ > 255 || @ % 1 != 0)')
    OR NOT jsonb_path_exists(value->'provider_receipt_sha256','$[*] ? (@ > 0)') THEN RETURN FALSE; END IF;
 IF value ? 'native_tokens' THEN RETURN core_image.valid_native_tokens(value->'native_tokens') IS TRUE; END IF;
 RETURN TRUE;
END $$;

CREATE OR REPLACE FUNCTION core_image.valid_output(value JSONB) RETURNS BOOLEAN
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
DECLARE item JSONB; name TEXT;
BEGIN
 IF jsonb_typeof(value) IS DISTINCT FROM 'object'
    OR value-ARRAY['created','data','usage','background','output_format','quality','size']<>'{}'::jsonb
    OR jsonb_typeof(value->'data') IS DISTINCT FROM 'array'
    OR octet_length(value::text)>33624064 THEN RETURN FALSE; END IF;
 IF jsonb_array_length(value->'data') NOT BETWEEN 1 AND 128 THEN RETURN FALSE; END IF;
 -- Retain the historical created-number shape without tightening old rows.
 IF value ? 'created' AND (jsonb_typeof(value->'created') IS DISTINCT FROM 'number'
    OR (value->>'created') !~ '^(0|[1-9][0-9]{0,19})$') THEN RETURN FALSE; END IF;
 IF value ? 'usage' AND core_image.valid_native_usage(value->'usage') IS NOT TRUE THEN RETURN FALSE; END IF;
 FOREACH name IN ARRAY ARRAY['background','output_format','quality','size'] LOOP
  IF value ? name AND (jsonb_typeof(value->name) IS DISTINCT FROM 'string'
     OR octet_length(value->>name) NOT BETWEEN 1 AND 512) THEN RETURN FALSE; END IF;
 END LOOP;
 FOR item IN SELECT e.value FROM jsonb_array_elements(value->'data') AS e(value) LOOP
  IF jsonb_typeof(item) IS DISTINCT FROM 'object'
     OR item-ARRAY['b64_json','url','revised_prompt']<>'{}'::jsonb
     OR (item ? 'b64_json')=(item ? 'url') THEN RETURN FALSE; END IF;
  IF item ? 'b64_json' AND (jsonb_typeof(item->'b64_json') IS DISTINCT FROM 'string'
     OR octet_length(item->>'b64_json') NOT BETWEEN 1 AND 33554432) THEN RETURN FALSE; END IF;
  IF item ? 'url' AND (jsonb_typeof(item->'url') IS DISTINCT FROM 'string'
     OR octet_length(item->>'url') NOT BETWEEN 1 AND 8192
     OR (item->>'url') !~ '^https://[^[:space:][:cntrl:]]+$') THEN RETURN FALSE; END IF;
  IF item ? 'revised_prompt' AND (jsonb_typeof(item->'revised_prompt') IS DISTINCT FROM 'string'
     OR octet_length(item->>'revised_prompt')>65536) THEN RETURN FALSE; END IF;
 END LOOP;
 RETURN TRUE;
END $$;

-- Keep the old surrounding image/ledger constraints and deferred binding guard.
ALTER TABLE core_image.operations DROP CONSTRAINT operations_image_usage_check;
ALTER TABLE core_image.operations ADD CONSTRAINT operations_image_usage_check
 CHECK(image_usage IS NULL OR core_image.valid_image_usage(image_usage) IS TRUE);

ALTER TABLE core_charging.requests DROP CONSTRAINT requests_check2;
ALTER TABLE core_charging.requests ADD CONSTRAINT requests_check2
 CHECK(image_usage IS NULL OR ((core_image.valid_image_usage(image_usage)
   AND jsonb_typeof(price->'image')='object'
   AND input_tokens=0 AND cached_tokens=0 AND output_tokens=0) IS TRUE));

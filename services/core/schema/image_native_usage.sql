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


-- Same bounded tariffs as billing/image_price/native_tokens.rs. Pure arithmetic
-- over the frozen rule, never an asserted reservation or a current model price.
-- discount SQL NULL means the original undiscounted quote.
CREATE FUNCTION core_image.native_reservation(image_price JSONB, discount JSONB) RETURNS BIGINT
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog AS $$
DECLARE rule JSONB; basis JSONB; tariff JSONB; mode TEXT; value JSONB;
 name TEXT; part TEXT; expr TEXT; compact TEXT; body TEXT; term TEXT;
 variable TEXT; coefficient TEXT; coefficient_digits TEXT; index INTEGER;
 rates NUMERIC[]:=ARRAY[NULL::numeric,NULL,NULL,NULL,NULL];
 ratio_names TEXT[]; ratio_values JSONB[]; ratio_nums NUMERIC[]:=ARRAY[]::numeric[];
 ratio_dens NUMERIC[]:=ARRAY[]::numeric[]; i INTEGER;
 max_input NUMERIC; max_output NUMERIC; raw_num NUMERIC; raw_den NUMERIC;
 numerator NUMERIC; denominator NUMERIC; original NUMERIC;
 discount_num NUMERIC:=1; discount_den NUMERIC:=1; rounded NUMERIC;
BEGIN
 IF jsonb_typeof(image_price) IS DISTINCT FROM 'object' THEN
  RAISE EXCEPTION 'invalid native image price' USING ERRCODE='23514';
 END IF;
 rule:=image_price->'rule'; basis:=rule->'count_basis';
 IF jsonb_typeof(rule) IS DISTINCT FROM 'object'
    OR jsonb_typeof(basis) IS DISTINCT FROM 'object'
    OR NOT ((basis ? 'native_tokens')<>(basis ? 'legacy_token_ratio'))
    OR basis-ARRAY['native_tokens','legacy_token_ratio']<>'{}'::jsonb THEN
  RAISE EXCEPTION 'unsupported native image tariff' USING ERRCODE='23514';
 END IF;
 mode:=CASE WHEN basis ? 'native_tokens' THEN 'native_tokens' ELSE 'legacy_token_ratio' END;
 tariff:=basis->mode;
 IF jsonb_typeof(tariff) IS DISTINCT FROM 'object' OR octet_length(tariff::text)>8192
    OR jsonb_typeof(tariff->'billing_price_key') IS DISTINCT FROM 'string'
    OR octet_length(tariff->>'billing_price_key') NOT BETWEEN 1 AND 512
    OR tariff->>'billing_price_key' ~ '[[:cntrl:]]'
    OR core_image.native_counter(tariff->'credits_per_usd',500000) IS NOT TRUE
    OR (tariff->>'credits_per_usd')::numeric<>500000 THEN
  RAISE EXCEPTION 'invalid native image tariff fields' USING ERRCODE='23514';
 END IF;
 FOREACH name IN ARRAY ARRAY['max_input_tokens','max_output_tokens'] LOOP
  IF core_image.native_counter(tariff->name,1000000000) IS NOT TRUE
     OR (tariff->>name)::numeric=0 THEN
   RAISE EXCEPTION 'invalid native image token bound' USING ERRCODE='23514';
  END IF;
 END LOOP;
 max_input:=(tariff->>'max_input_tokens')::numeric;
 max_output:=(tariff->>'max_output_tokens')::numeric;
 -- Keep the unused fixed-price placeholder separate from a token tariff.
 value:=rule->'usd_micros_per_image';
 IF jsonb_typeof(value) IS DISTINCT FROM 'object'
    OR NOT value ?& ARRAY['numerator','denominator']
    OR value-ARRAY['numerator','denominator']<>'{}'::jsonb
    OR core_image.native_counter(value->'numerator',0) IS NOT TRUE
    OR core_image.native_counter(value->'denominator',18446744073709551615) IS NOT TRUE
    OR (value->>'denominator')::numeric=0 THEN
  RAISE EXCEPTION 'mixed fixed and native image tariff' USING ERRCODE='23514';
 END IF;
 ratio_names:=ARRAY['group_multiplier','usd_multiplier'];
 ratio_values:=ARRAY[rule->'group_multiplier',tariff->'usd_multiplier'];
 IF mode='native_tokens' THEN
  IF NOT tariff ?& ARRAY['billing_price_key','source_expression','source_sha256','currency_basis','credits_per_usd','usd_multiplier','max_input_tokens','max_output_tokens']
     OR tariff-ARRAY['billing_price_key','source_expression','source_sha256','currency_basis','credits_per_usd','usd_multiplier','max_input_tokens','max_output_tokens']<>'{}'::jsonb
     OR tariff->>'currency_basis' IS DISTINCT FROM 'legacy_pricing_unit'
     OR jsonb_typeof(tariff->'source_expression') IS DISTINCT FROM 'string'
     OR jsonb_typeof(tariff->'source_sha256') IS DISTINCT FROM 'string' THEN
   RAISE EXCEPTION 'invalid native expression contract' USING ERRCODE='23514';
  END IF;
  expr:=tariff->>'source_expression';
  IF octet_length(expr)>4096
     OR tariff->>'source_sha256' IS DISTINCT FROM encode(sha256(convert_to(expr,'UTF8')),'hex')
     OR expr ~ E'[A-Za-z0-9_.][ \\t\\n\\f\\r]+[A-Za-z0-9_.]'
     OR substring(expr FROM '"[^"]*"') IS DISTINCT FROM '"standard"' THEN
   RAISE EXCEPTION 'invalid native expression source' USING ERRCODE='23514';
  END IF;
  compact:=translate(expr,E' \t\n\f\r','');
  IF left(compact,length('(tier("standard",'))<>'(tier("standard",'
     OR right(compact,length('))*(1)'))<>'))*(1)' THEN
   RAISE EXCEPTION 'unsupported native expression' USING ERRCODE='23514';
  END IF;
  body:=substring(compact FROM length('(tier("standard",')+1
     FOR length(compact)-length('(tier("standard",')-length('))*(1)'));
  FOREACH term IN ARRAY string_to_array(body,'+') LOOP
   variable:=split_part(term,'*',1); coefficient:=split_part(term,'*',2);
   index:=array_position(ARRAY['p','cr_text','img','cr_img','img_o'],variable);
   IF index IS NULL OR cardinality(string_to_array(term,'*'))<>2
      OR length(coefficient)>20 OR coefficient !~ '^[0-9]+([.][0-9]{0,9})?$' THEN
    RAISE EXCEPTION 'unsupported native expression term' USING ERRCODE='23514';
   END IF;
   coefficient_digits:=replace(coefficient,'.','');
   IF rates[index] IS NOT NULL OR coefficient_digits::numeric>18446744073709551615 THEN
    RAISE EXCEPTION 'invalid native expression coefficient' USING ERRCODE='23514';
   END IF;
   -- Every accepted coefficient has <=9 fractional digits: this scale is exact.
   rates[index]:=coefficient::numeric*1000000000;
  END LOOP;
  IF array_position(rates,NULL::numeric) IS NOT NULL THEN
   RAISE EXCEPTION 'missing native expression rate' USING ERRCODE='23514';
  END IF;
  raw_num:=greatest(rates[1],rates[2],rates[3],rates[4])*max_input+rates[5]*max_output;
  raw_den:=1000000000*1000000::numeric;
 ELSE
  IF NOT tariff ?& ARRAY['billing_price_key','model_ratio','completion_ratio','cache_ratio','image_ratio','legacy_quota_per_unit','credits_per_usd','usd_multiplier','max_input_tokens','max_output_tokens']
     OR tariff-ARRAY['billing_price_key','model_ratio','completion_ratio','cache_ratio','image_ratio','legacy_quota_per_unit','credits_per_usd','usd_multiplier','max_input_tokens','max_output_tokens']<>'{}'::jsonb
     OR core_image.native_counter(tariff->'legacy_quota_per_unit',500000) IS NOT TRUE
     OR (tariff->>'legacy_quota_per_unit')::numeric<>500000 THEN
   RAISE EXCEPTION 'invalid legacy image ratio contract' USING ERRCODE='23514';
  END IF;
  ratio_names:=ratio_names||ARRAY['model_ratio','completion_ratio','cache_ratio','image_ratio'];
  ratio_values:=ratio_values||ARRAY[tariff->'model_ratio',tariff->'completion_ratio',tariff->'cache_ratio',tariff->'image_ratio'];
 END IF;
 FOR i IN 1..cardinality(ratio_names) LOOP
  value:=ratio_values[i];
  IF jsonb_typeof(value) IS DISTINCT FROM 'object'
     OR NOT value ?& ARRAY['numerator','denominator']
     OR value-ARRAY['numerator','denominator']<>'{}'::jsonb THEN
   RAISE EXCEPTION 'invalid native image ratio %',ratio_names[i] USING ERRCODE='23514';
  END IF;
  FOREACH part IN ARRAY ARRAY['numerator','denominator'] LOOP
   IF core_image.native_counter(value->part,18446744073709551615) IS NOT TRUE
      OR (part='denominator' AND (value->>part)::numeric=0) THEN
    RAISE EXCEPTION 'invalid native image ratio integer' USING ERRCODE='23514';
   END IF;
  END LOOP;
  ratio_nums:=array_append(ratio_nums,(value->>'numerator')::numeric);
  ratio_dens:=array_append(ratio_dens,(value->>'denominator')::numeric);
 END LOOP;
 IF mode='legacy_token_ratio' THEN
  -- Cache and image counters overlap in this historical path; reserve max(1,c+i).
  raw_num:=greatest(ratio_dens[5]*ratio_dens[6],ratio_nums[5]*ratio_dens[6]+ratio_nums[6]*ratio_dens[5])
     *max_input*ratio_dens[4]+ratio_nums[4]*max_output*ratio_dens[5]*ratio_dens[6];
  raw_den:=ratio_dens[5]*ratio_dens[6]*ratio_dens[4]*500000;
  raw_num:=raw_num*ratio_nums[3]; raw_den:=raw_den*ratio_dens[3];
 END IF;
 numerator:=raw_num*500000*ratio_nums[1]*ratio_nums[2];
 denominator:=raw_den*ratio_dens[1]*ratio_dens[2];
 original:=div(numerator,denominator)+CASE WHEN mod(numerator,denominator)<>0 THEN 1 ELSE 0 END;
 IF original>9223372036854775807 THEN
  RAISE EXCEPTION 'native image original reservation overflow' USING ERRCODE='23514';
 END IF;
 IF discount IS NOT NULL THEN
  IF jsonb_typeof(discount) IS DISTINCT FROM 'object'
     OR NOT discount ?& ARRAY['numerator','denominator']
     OR discount-ARRAY['numerator','denominator']<>'{}'::jsonb
     OR core_image.native_counter(discount->'numerator',1000000000000000000) IS NOT TRUE
     OR core_image.native_counter(discount->'denominator',1000000000000000000) IS NOT TRUE
     OR (discount->>'numerator')::numeric=0
     OR (discount->>'denominator')::numeric=0
     OR (discount->>'numerator')::numeric>(discount->>'denominator')::numeric THEN
   RAISE EXCEPTION 'invalid native image trust discount' USING ERRCODE='23514';
  END IF;
  discount_num:=(discount->>'numerator')::numeric;
  discount_den:=(discount->>'denominator')::numeric;
 END IF;
 -- Discount the complete rational before rounding; never ceil(base)*discount.
 numerator:=numerator*discount_num; denominator:=denominator*discount_den;
 rounded:=div(numerator,denominator)+CASE WHEN mod(numerator,denominator)<>0 THEN 1 ELSE 0 END;
 IF rounded>9223372036854775807 THEN
  RAISE EXCEPTION 'native image discounted reservation overflow' USING ERRCODE='23514';
 END IF;
 RETURN rounded::bigint;
END $$;

REVOKE ALL ON FUNCTION core_image.native_reservation(jsonb,jsonb) FROM PUBLIC;

-- Keep all original state, owner, receipt, charge and settlement bindings.
CREATE OR REPLACE FUNCTION core_image.check_charge_atomic() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE operation_id TEXT; operation RECORD; charge RECORD;
 image_price JSONB; ratio JSONB; ratio_name TEXT; part TEXT; raw_value TEXT;
 requested_n NUMERIC; maximum_n NUMERIC; max_budget NUMERIC;
 usd_num NUMERIC; usd_den NUMERIC; group_num NUMERIC; group_den NUMERIC;
 numerators NUMERIC[]; denominators NUMERIC[];
 gcd_a NUMERIC; gcd_b NUMERIC; remainder NUMERIC;
 base_numerator NUMERIC; base_denominator NUMERIC; base_credits NUMERIC;
 discounted_numerator NUMERIC; discounted_denominator NUMERIC;
 discounted_credits NUMERIC; trusted_reservation BIGINT; factor NUMERIC;
BEGIN
 IF TG_TABLE_SCHEMA='core_charging' THEN
  IF NEW.image_operation_id IS NULL THEN RETURN NULL; END IF;
  operation_id:=NEW.image_operation_id;
 ELSE operation_id:=NEW.id;
 END IF;
 SELECT o.* INTO operation FROM core_image.operations o WHERE o.id=operation_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'missing image operation source' USING ERRCODE='23514'; END IF;
 -- The old no-trust path is unchanged. New frozen evidence must also preserve
 -- the original Rust image quote's count, budget and base-overflow checks.
 -- Run before the no-charge return: previews must not persist an invalid base.
 IF operation.price ? 'user_trust' THEN
  image_price:=operation.price->'image';
  IF jsonb_typeof(image_price->'rule') IS DISTINCT FROM 'object'
     OR jsonb_typeof(image_price->'request') IS DISTINCT FROM 'object' THEN
   RAISE EXCEPTION 'invalid trusted image tariff shape' USING ERRCODE='23514';
  END IF;
  FOREACH ratio_name IN ARRAY ARRAY['usd_micros_per_image','group_multiplier'] LOOP
   ratio:=image_price#>ARRAY['rule',ratio_name];
   IF jsonb_typeof(ratio) IS DISTINCT FROM 'object' THEN
    RAISE EXCEPTION 'invalid trusted image ratio shape' USING ERRCODE='23514';
   END IF;
   IF NOT (ratio ?& ARRAY['numerator','denominator'])
      OR ratio-ARRAY['numerator','denominator']<>'{}'::jsonb THEN
    RAISE EXCEPTION 'invalid trusted image ratio fields' USING ERRCODE='23514';
   END IF;
   FOREACH part IN ARRAY ARRAY['numerator','denominator'] LOOP
    raw_value:=ratio->>part;
    IF jsonb_typeof(ratio->part) IS DISTINCT FROM 'number'
       OR raw_value !~ '^(0|[1-9][0-9]{0,19})$' THEN
     RAISE EXCEPTION 'invalid trusted image ratio integer' USING ERRCODE='23514';
    END IF;
    IF raw_value::numeric>18446744073709551615
       OR (part='denominator' AND raw_value::numeric=0) THEN
     RAISE EXCEPTION 'trusted image ratio outside u64 domain' USING ERRCODE='23514';
    END IF;
   END LOOP;
  END LOOP;
  IF jsonb_typeof(image_price#>'{request,n}') IS DISTINCT FROM 'number'
     OR image_price#>>'{request,n}' !~ '^[1-9][0-9]{0,2}$'
     OR jsonb_typeof(image_price#>'{rule,max_n}') IS DISTINCT FROM 'number'
     OR image_price#>>'{rule,max_n}' !~ '^[1-9][0-9]{0,2}$'
     OR jsonb_typeof(image_price#>'{rule,max_budget}') IS DISTINCT FROM 'number'
     OR image_price#>>'{rule,max_budget}' !~ '^(0|[1-9][0-9]{0,18})$' THEN
   RAISE EXCEPTION 'invalid trusted image count or budget integer' USING ERRCODE='23514';
  END IF;
  requested_n:=(image_price#>>'{request,n}')::numeric;
  maximum_n:=(image_price#>>'{rule,max_n}')::numeric;
  max_budget:=(image_price#>>'{rule,max_budget}')::numeric;
  IF requested_n>maximum_n OR maximum_n>128 OR max_budget>9223372036854775807 THEN
   RAISE EXCEPTION 'trusted image count or budget outside domain' USING ERRCODE='23514';
  END IF;
  usd_num:=(image_price#>>'{rule,usd_micros_per_image,numerator}')::numeric;
  usd_den:=(image_price#>>'{rule,usd_micros_per_image,denominator}')::numeric;
  group_num:=(image_price#>>'{rule,group_multiplier,numerator}')::numeric;
  group_den:=(image_price#>>'{rule,group_multiplier,denominator}')::numeric;
  IF jsonb_typeof(image_price#>'{rule,count_basis}')='object' THEN
   base_credits:=core_image.native_reservation(image_price,NULL);
   IF base_credits>max_budget OR base_credits IS DISTINCT FROM
       (image_price->>'reservation_credits')::numeric THEN
    RAISE EXCEPTION 'trusted native image original reservation or budget mismatch' USING ERRCODE='23514';
   END IF;
   trusted_reservation:=core_image.native_reservation(image_price,operation.price#>'{user_trust,discount}');
  ELSE
  -- Rust validates both the requested quote and the maximum configured count.
  -- Euclidean cancellation precedes each checked u128 base multiplication.
  FOR count_index IN 1..2 LOOP
   numerators:=ARRAY[usd_num,group_num,
       CASE WHEN count_index=1 THEN requested_n ELSE maximum_n END,500000::numeric];
   denominators:=ARRAY[usd_den,group_den,1000000::numeric];
   FOR numerator_index IN 1..4 LOOP
    FOR denominator_index IN 1..3 LOOP
     gcd_a:=numerators[numerator_index];
     gcd_b:=denominators[denominator_index];
     WHILE gcd_b<>0 LOOP
      remainder:=mod(gcd_a,gcd_b); gcd_a:=gcd_b; gcd_b:=remainder;
     END LOOP;
     numerators[numerator_index]:=div(numerators[numerator_index],gcd_a);
     denominators[denominator_index]:=div(denominators[denominator_index],gcd_a);
    END LOOP;
   END LOOP;
   base_numerator:=1; base_denominator:=1;
   FOREACH factor IN ARRAY numerators LOOP
    base_numerator:=base_numerator*factor;
    IF base_numerator>340282366920938463463374607431768211455 THEN
     RAISE EXCEPTION 'trusted image original numerator overflow' USING ERRCODE='23514';
    END IF;
   END LOOP;
   FOREACH factor IN ARRAY denominators LOOP
    base_denominator:=base_denominator*factor;
    IF base_denominator>340282366920938463463374607431768211455 THEN
     RAISE EXCEPTION 'trusted image original denominator overflow' USING ERRCODE='23514';
    END IF;
   END LOOP;
   base_credits:=div(base_numerator,base_denominator)
       + CASE WHEN mod(base_numerator,base_denominator)<>0 THEN 1 ELSE 0 END;
   IF base_credits>9223372036854775807 OR base_credits>max_budget
      OR (count_index=1 AND base_credits IS DISTINCT FROM
          (image_price->>'reservation_credits')::numeric) THEN
    RAISE EXCEPTION 'trusted image original reservation or budget mismatch' USING ERRCODE='23514';
   END IF;
  END LOOP;
  -- Multiply the trust factor into the complete exact amount. Never discount
  -- the already-rounded persisted base reservation, and never use numeric ceil
  -- on division whose scale may round away a nonzero remainder.
  discounted_numerator:=usd_num*group_num*requested_n*500000
      *(operation.price#>>'{user_trust,discount,numerator}')::numeric;
  discounted_denominator:=usd_den*group_den*1000000
      *(operation.price#>>'{user_trust,discount,denominator}')::numeric;
  discounted_credits:=div(discounted_numerator,discounted_denominator)
      + CASE WHEN mod(discounted_numerator,discounted_denominator)<>0 THEN 1 ELSE 0 END;
  IF discounted_credits>9223372036854775807 THEN
   RAISE EXCEPTION 'trusted image discounted reservation overflow' USING ERRCODE='23514';
  END IF;
  trusted_reservation:=discounted_credits::bigint;
  END IF;
 END IF;

 SELECT q.* INTO charge FROM core_charging.requests q WHERE q.image_operation_id=operation_id;
 IF NOT FOUND THEN
  IF operation.state NOT IN ('previewed','confirmed') OR operation.charge_id IS NOT NULL THEN
   RAISE EXCEPTION 'missing image charge' USING ERRCODE='23514';
  END IF;
  RETURN NULL;
 END IF;
 IF charge.id<>operation.id OR charge.image_operation_id<>operation.id
    OR charge.actor_user_id<>operation.actor_user_id
    OR charge.owner_account_id<>operation.owner_account_id
    OR (operation.source_kind='drawing' AND (charge.key_id IS NOT NULL OR charge.payer_account_id<>operation.owner_account_id))
    OR (operation.source_kind='api_key' AND charge.key_id IS DISTINCT FROM operation.key_id)
    OR charge.oauth_source IS NOT NULL
    OR charge.assistant_grant_id IS NOT NULL OR charge.assistant_sponsor_account_id IS NOT NULL
    OR charge.fingerprint IS DISTINCT FROM operation.fingerprint OR charge.price IS DISTINCT FROM operation.price
    OR charge.reserved IS DISTINCT FROM (CASE WHEN operation.price ? 'user_trust' THEN trusted_reservation
       ELSE (operation.price#>>'{image,reservation_credits}')::bigint END)
    OR charge.input_tokens<>0 OR charge.cached_tokens<>0 OR charge.output_tokens<>0 THEN
  RAISE EXCEPTION 'image charge source mismatch' USING ERRCODE='23514';
 END IF;
 IF (operation.state='confirmed' AND (charge.state NOT IN ('reserved','released') OR operation.charge_id IS NOT NULL))
    OR (operation.state='dispatching' AND (charge.state NOT IN ('streaming','reconcile') OR operation.charge_id IS DISTINCT FROM charge.id))
    OR (operation.state='billing_pending' AND (charge.state NOT IN ('streaming','reconcile','settled') OR operation.charge_id IS DISTINCT FROM charge.id))
    OR operation.state='previewed'
    OR (operation.state='completed' AND charge.state<>'settled') THEN
  RAISE EXCEPTION 'image operation and charge states must commit together' USING ERRCODE='23514';
 END IF;
 IF charge.image_usage IS NOT NULL AND charge.image_usage IS DISTINCT FROM operation.image_usage THEN
  RAISE EXCEPTION 'image usage evidence mismatch' USING ERRCODE='23514';
 END IF;
 IF charge.state='settled' AND (operation.state NOT IN ('billing_pending','completed')
    OR operation.output IS NULL OR operation.image_usage IS DISTINCT FROM charge.image_usage
    OR charge.outcome IS DISTINCT FROM 'completed') THEN
  RAISE EXCEPTION 'image settlement requires durable original output receipt' USING ERRCODE='23514';
 END IF;
 IF operation.state='completed' AND (operation.charge_id IS DISTINCT FROM charge.id
    OR operation.settled_credits IS DISTINCT FROM charge.settled
    OR operation.image_usage IS DISTINCT FROM charge.image_usage
    OR operation.output IS NULL OR charge.outcome IS DISTINCT FROM 'completed') THEN
  RAISE EXCEPTION 'image output and completed settlement must commit together' USING ERRCODE='23514';
 END IF;
 RETURN NULL;
END $$;

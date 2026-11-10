-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Run after the actual identity/charging/image base DDL and component65.
-- JSON/constraint/EXECUTE regression only; not a Core funds/HTTP acceptance.
BEGIN;
CREATE ROLE image_native_fixture_runtime NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_image FROM PUBLIC;
GRANT USAGE ON SCHEMA core_image TO image_native_fixture_runtime;
GRANT EXECUTE ON FUNCTION core_image.native_counter(jsonb,numeric),
 core_image.valid_native_tokens(jsonb),core_image.valid_native_usage(jsonb),
 core_image.valid_image_usage(jsonb),core_image.valid_output(jsonb)
 TO image_native_fixture_runtime;

-- Reuse the exact real-table CHECK definitions, without claiming that isolated
-- JSON probes exercise unrelated FK, identity, ledger or deferred funds guards.
CREATE TEMP TABLE image_native_operation_probe(image_usage JSONB,output JSONB);
CREATE TEMP TABLE image_native_charge_probe(image_usage JSONB,price JSONB,
 input_tokens BIGINT DEFAULT 0,cached_tokens BIGINT DEFAULT 0,output_tokens BIGINT DEFAULT 0);
DO $$
DECLARE definition TEXT;
BEGIN
 SELECT pg_get_constraintdef(oid) INTO STRICT definition FROM pg_constraint
 WHERE conrelid='core_image.operations'::regclass AND conname='operations_image_usage_check';
 EXECUTE 'ALTER TABLE image_native_operation_probe ADD CONSTRAINT original_image_usage '||definition;
 SELECT pg_get_constraintdef(oid) INTO STRICT definition FROM pg_constraint
 WHERE conrelid='core_image.operations'::regclass AND conname='operations_output_check';
 EXECUTE 'ALTER TABLE image_native_operation_probe ADD CONSTRAINT original_output '||definition;
 SELECT pg_get_constraintdef(oid) INTO STRICT definition FROM pg_constraint
 WHERE conrelid='core_charging.requests'::regclass AND conname='requests_check2';
 EXECUTE 'ALTER TABLE image_native_charge_probe ADD CONSTRAINT original_image_usage '||definition;
END $$;
GRANT INSERT,SELECT ON image_native_operation_probe,image_native_charge_probe TO image_native_fixture_runtime;

DO $$
DECLARE raw JSONB := '{"created":1,"data":[{"b64_json":"AQID"}],"background":"opaque","output_format":"png","quality":"high","size":"1024x1024","usage":{"input_tokens":100,"output_tokens":10,"total_tokens":110,"input_tokens_details":{"cached_tokens":40,"text_tokens":40,"image_tokens":60,"cached_tokens_details":{"text_tokens":10,"image_tokens":30,"audio_tokens":0}}}}';
 tokens JSONB := '{"input":100,"input_text":40,"input_image":60,"cached":40,"cached_text":10,"cached_image":30,"output":10,"output_image":10,"output_image_source":"images_endpoint_total"}';
 receipt JSONB; legacy JSONB; changed JSONB; signature TEXT;
BEGIN
 SELECT jsonb_build_object('output_images',1,'provider_receipt_sha256',jsonb_agg(1)) INTO legacy FROM generate_series(1,32);
 receipt:=legacy||jsonb_build_object('native_tokens',tokens);
 IF core_image.valid_output(raw) IS NOT TRUE OR core_image.valid_image_usage(receipt) IS NOT TRUE
    OR core_image.valid_image_usage(legacy) IS NOT TRUE
    OR core_image.valid_output('{"created":1,"data":[{"b64_json":"AQID"}]}') IS NOT TRUE THEN
  RAISE EXCEPTION 'native or legacy valid receipt rejected'; END IF;
 INSERT INTO image_native_operation_probe(image_usage,output) VALUES(legacy,NULL),(receipt,raw);
 INSERT INTO image_native_charge_probe(image_usage,price) VALUES(legacy,'{"image":{}}'),(receipt,'{"image":{}}');
 -- Positive aggregate without modality classification is valid UNKNOWN evidence.
 changed:=jsonb_set(jsonb_set(tokens,'{cached_text}','null'),'{cached_image}','null');
 IF core_image.valid_native_tokens(changed) IS NOT TRUE OR core_image.valid_native_usage(raw#>'{usage}' #- '{input_tokens_details,cached_tokens_details}') IS NOT TRUE THEN
  RAISE EXCEPTION 'unknown cache was discarded'; END IF;
 IF core_image.valid_native_usage((raw->'usage') #- '{input_tokens_details,cached_tokens}' #- '{input_tokens_details,cached_tokens_details}') IS NOT TRUE THEN
  RAISE EXCEPTION 'missing cache aggregate must stay representable as unknown'; END IF;
 IF core_image.valid_native_usage(jsonb_set((raw->'usage') #- '{input_tokens_details,cached_tokens_details}','{input_tokens_details,cached_tokens}','0')) IS NOT TRUE THEN
  RAISE EXCEPTION 'reported zero aggregate rejected'; END IF;
 changed:=jsonb_set(raw,'{usage,output_tokens_details}','{"text_tokens":4,"image_tokens":6}');
 IF core_image.valid_output(changed) IS NOT TRUE THEN RAISE EXCEPTION 'explicit output details rejected'; END IF;
 FOREACH changed IN ARRAY ARRAY[
  jsonb_set(raw,'{usage,total_tokens}','111'),
  jsonb_set(raw,'{usage,input_tokens}','-1'),
  jsonb_set(raw,'{usage,input_tokens}','"100"'),
  jsonb_set(raw,'{usage,input_tokens}','100.5'),
  jsonb_set(raw,'{usage,total_tokens}','18446744073709551616'),
  jsonb_set(raw,'{usage,input_tokens_details,cached_tokens_details,image_tokens}','61'),
  jsonb_set(raw,'{usage,input_tokens_details,cached_tokens_details,audio_tokens}','1'),
  raw||'{"payer_account_id":7}',
  jsonb_set(raw,'{usage}',(raw->'usage')||'{"cost":0}'),
  jsonb_set(raw,'{quality}',to_jsonb(repeat('x',513))),
  jsonb_set(raw,'{data}','[]')
 ] LOOP
  IF core_image.valid_output(changed) IS NOT FALSE THEN RAISE EXCEPTION 'invalid output accepted: %',changed; END IF;
 END LOOP;
 FOREACH changed IN ARRAY ARRAY[
  jsonb_set(receipt,'{output_images}','99999999999999999999'),
  jsonb_set(receipt,'{native_tokens,input_text}','41'),
  jsonb_set(receipt,'{native_tokens,cached_text}','11'),
  jsonb_set(receipt,'{native_tokens,output_image_source}','"chat_guess"'),
  jsonb_set(receipt,'{native_tokens,output_image}','9'),
  jsonb_set(receipt,'{native_tokens}','null'),
  receipt||'{"credits":0}',
  jsonb_set(receipt,'{provider_receipt_sha256}','[0]')
 ] LOOP
  IF core_image.valid_image_usage(changed) IS NOT FALSE THEN RAISE EXCEPTION 'invalid receipt accepted: %',changed; END IF;
  BEGIN INSERT INTO image_native_operation_probe(image_usage) VALUES(changed); RAISE EXCEPTION 'operation CHECK bypassed';
  EXCEPTION WHEN check_violation THEN NULL; END;
  BEGIN INSERT INTO image_native_charge_probe(image_usage,price) VALUES(changed,'{"image":{}}'); RAISE EXCEPTION 'charge CHECK bypassed';
  EXCEPTION WHEN check_violation THEN NULL; END;
 END LOOP;
 BEGIN INSERT INTO image_native_charge_probe(image_usage,price,input_tokens) VALUES(receipt,'{"image":{}}',1); RAISE EXCEPTION 'token exclusivity bypassed';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN INSERT INTO image_native_charge_probe(image_usage,price) VALUES(receipt,'{}'); RAISE EXCEPTION 'missing image authority accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 -- Each required helper is exercised through its real invoker call chain.
 FOREACH signature IN ARRAY ARRAY['core_image.native_counter(jsonb,numeric)',
  'core_image.valid_native_tokens(jsonb)','core_image.valid_native_usage(jsonb)',
  'core_image.valid_image_usage(jsonb)'] LOOP
  EXECUTE 'REVOKE EXECUTE ON FUNCTION '||signature||' FROM image_native_fixture_runtime';
  BEGIN
   SET LOCAL ROLE image_native_fixture_runtime;
   INSERT INTO image_native_operation_probe(image_usage,output) VALUES(receipt,raw);
   INSERT INTO image_native_charge_probe(image_usage,price) VALUES(receipt,'{"image":{}}');
   PERFORM core_image.valid_image_usage(receipt); PERFORM core_image.valid_output(raw);
   RAISE EXCEPTION 'missing function privilege not enforced: %',signature;
  EXCEPTION WHEN insufficient_privilege THEN NULL; END;
  RESET ROLE;
  EXECUTE 'GRANT EXECUTE ON FUNCTION '||signature||' TO image_native_fixture_runtime';
 END LOOP;
 SET LOCAL ROLE image_native_fixture_runtime;
 INSERT INTO image_native_operation_probe(image_usage,output) VALUES(receipt,raw);
 INSERT INTO image_native_charge_probe(image_usage,price) VALUES(receipt,'{"image":{}}');
 IF core_image.valid_image_usage(receipt) IS NOT TRUE OR core_image.valid_output(raw) IS NOT TRUE THEN
  RAISE EXCEPTION 'restored runtime function grants failed'; END IF;
 RESET ROLE;
END $$;
SELECT 'image_native_usage: legacy/native/unknown/invalid/both_checks/four_privileges PASS' AS result;
ROLLBACK;

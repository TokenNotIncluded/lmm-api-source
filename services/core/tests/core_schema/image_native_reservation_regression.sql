-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Actual helper and deferred operation/charge guards; isolated synthetic prices.
-- Run after identity, charging, image, image_trust_extension, image_native_usage.
-- This is not HTTP, wallet-ledger or independent funds acceptance.
BEGIN;
CREATE ROLE image_reservation_runtime NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
-- Match the runtime grant contract: otherwise revoked role EXECUTE is still
-- inherited from CREATE FUNCTION's default PUBLIC grant.
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA core_image FROM PUBLIC;
GRANT USAGE ON SCHEMA core_identity,core_image,core_charging TO image_reservation_runtime;
GRANT SELECT ON core_identity.credentials,core_image.operations,core_charging.requests TO image_reservation_runtime;
GRANT INSERT,UPDATE ON core_image.operations TO image_reservation_runtime;
GRANT INSERT ON core_charging.requests TO image_reservation_runtime;
GRANT EXECUTE ON FUNCTION core_image.native_counter(jsonb,numeric),core_image.native_reservation(jsonb,jsonb),
 core_image.valid_output(jsonb),core_image.valid_native_tokens(jsonb),
 core_image.valid_native_usage(jsonb),core_image.valid_image_usage(jsonb)
 TO image_reservation_runtime;
CREATE TEMP TABLE reservation_identity(account_id BIGINT,user_id BIGINT,credential_id BIGINT);
GRANT SELECT ON reservation_identity TO image_reservation_runtime;
DO $$
DECLARE account_id BIGINT; user_id BIGINT; credential_id BIGINT;
BEGIN
 INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id INTO account_id;
 INSERT INTO core_identity.users(personal_account_id) VALUES(account_id) RETURNING id INTO user_id;
 INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at)
 VALUES(decode(repeat('42',32),'hex'),'session',user_id,1,account_id,clock_timestamp()+interval '1 day')
 RETURNING id INTO credential_id;
 INSERT INTO reservation_identity VALUES(account_id,user_id,credential_id);
END $$;
CREATE TEMP SEQUENCE reservation_case;
GRANT USAGE ON reservation_case TO image_reservation_runtime;
CREATE FUNCTION pg_temp.test_image_price(mode TEXT) RETURNS JSONB LANGUAGE plpgsql AS $$
DECLARE one JSONB := '{"numerator":1,"denominator":1}'; tariff JSONB; basis JSONB;
 expr TEXT := '(tier("standard", p * 5 + cr_text * 1.25 + img * 8 + cr_img * 2 + img_o * 30)) * (1)';
 amount BIGINT;
BEGIN
 IF mode='native_tokens' THEN
  tariff:=jsonb_build_object('billing_price_key','gpt-image-2','source_expression',expr,
   'source_sha256',encode(sha256(convert_to(expr,'UTF8')),'hex'),'currency_basis','legacy_pricing_unit',
   'credits_per_usd',500000,'usd_multiplier',one,'max_input_tokens',100,'max_output_tokens',100);
  basis:=jsonb_build_object(mode,tariff); amount:=1900;
 ELSIF mode='legacy_token_ratio' THEN
  tariff:=jsonb_build_object('billing_price_key','google/gemini-3.1-flash-image',
   'model_ratio','{"numerator":1,"denominator":4}'::jsonb,'completion_ratio','{"numerator":6,"denominator":1}'::jsonb,
   'cache_ratio','{"numerator":1,"denominator":10}'::jsonb,'image_ratio',one,
   'legacy_quota_per_unit',500000,'credits_per_usd',500000,'usd_multiplier',one,
   'max_input_tokens',100,'max_output_tokens',100);
  basis:=jsonb_build_object(mode,tariff); amount:=178;
 ELSE basis:=to_jsonb('returned_payloads'::text); amount:=1500000;
 END IF;
 RETURN jsonb_build_object('revision',1,'reservation_credits',amount,
  'rule',jsonb_build_object('upstream_id',1,'vendor','openai','billing_group','default',
   'upstream_model','fixture','semantic_model','fixture','endpoint','generations','size','','quality','',
   'usd_micros_per_image',jsonb_build_object('numerator',CASE WHEN mode='fixed' THEN 1000000 ELSE 0 END,'denominator',1),
   'group_multiplier',one,'max_n',3,'count_basis',basis,'max_budget',2000000,'source_revision','fixture'),
  'request',jsonb_build_object('upstream_id',1,'vendor','openai','billing_group','default',
   'upstream_model','fixture','semantic_model','fixture','endpoint','generations','size','','quality','','n',3));
END $$;

-- Every call inserts real operation rows and forces the real deferred trigger.
-- Optional reserved creates the original confirmed charge in the same tx.
CREATE FUNCTION pg_temp.test_image_trigger(image_price JSONB, held BIGINT) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE actor RECORD; price JSONB; ident TEXT; digest BYTEA:=decode(repeat('01',32),'hex');
 now_seconds BIGINT:=floor(extract(epoch FROM clock_timestamp()))::bigint;
BEGIN
 SELECT * INTO STRICT actor FROM reservation_identity;
 ident:='image:'||lpad(nextval('reservation_case')::text,64,'0');
 price:=jsonb_build_object('version','fixture','model','fixture','group','default',
  'input_per_million',0,'cached_per_million',0,'output_per_million',0,'image',image_price,
  'user_trust',jsonb_build_object('user_id',actor.user_id,'policy_revision',1,'basis_revision',0,
   'effective_level',0,'level_source','automatic','discount','{"numerator":9,"denominator":10}'::jsonb,
   'evaluated_at',now_seconds,'evidence_digest',to_jsonb(array_fill(1,ARRAY[32]))));
 SET LOCAL ROLE image_reservation_runtime;
 IF current_user<>'image_reservation_runtime' THEN RAISE EXCEPTION 'fixture role switch failed'; END IF;
 INSERT INTO core_image.operations(id,session_id,actor_user_id,owner_account_id,auth_version,context,input,fingerprint,price,created_at,expires_at)
 VALUES(ident,actor.credential_id,actor.user_id,actor.account_id,1,
  jsonb_build_object('call_id',ident,'dispatch_id',ident,'tool_id','fixture','version_id','fixture','grant_id','fixture','input_digest',repeat('01',32)),
  '{}',digest,price,now_seconds,now_seconds+600);
 SET CONSTRAINTS ALL IMMEDIATE;
 SET CONSTRAINTS ALL DEFERRED;
 IF held IS NOT NULL THEN
  UPDATE core_image.operations SET state='confirmed',confirmed_at=now_seconds,
   confirmation_digest=digest,confirmation_sid='fixture' WHERE id=ident;
  INSERT INTO core_charging.requests(id,actor_user_id,image_operation_id,owner_account_id,payer_account_id,
   fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved)
  VALUES(ident,actor.user_id,ident,actor.account_id,actor.account_id,digest,digest,digest,price,
   now_seconds,now_seconds+60,now_seconds+300,'reserved',held);
  SET CONSTRAINTS ALL IMMEDIATE;
  SET CONSTRAINTS ALL DEFERRED;
 END IF;
 RESET ROLE;
END $$;

DO $$
DECLARE native JSONB:=pg_temp.test_image_price('native_tokens'); legacy JSONB:=pg_temp.test_image_price('legacy_token_ratio');
 fixed JSONB:=pg_temp.test_image_price('fixed'); changed JSONB; tariff JSONB; expr TEXT;
 discount JSONB:='{"numerator":9,"denominator":10}'; mode TEXT; whitespace TEXT;
BEGIN
 IF EXISTS(SELECT 1 FROM pg_proc p CROSS JOIN LATERAL aclexplode(coalesce(p.proacl,acldefault('f',p.proowner))) a
  WHERE p.oid='core_image.native_reservation(jsonb,jsonb)'::regprocedure
   AND a.grantee=0 AND a.privilege_type='EXECUTE') THEN
  RAISE EXCEPTION 'native helper must not be executable by PUBLIC'; END IF;
 IF core_image.native_reservation(native,NULL)<>1900 OR core_image.native_reservation(native,discount)<>1710
    OR core_image.native_reservation(legacy,NULL)<>178 OR core_image.native_reservation(legacy,discount)<>160 THEN
  RAISE EXCEPTION 'exact original/discounted native or legacy reservation mismatch'; END IF;
 -- A forged asserted reservation never affects the arithmetic.
 IF core_image.native_reservation(jsonb_set(native,'{reservation_credits}','1'),NULL)<>1900 THEN
  RAISE EXCEPTION 'helper trusted asserted reservation'; END IF;
 PERFORM pg_temp.test_image_trigger(native,1710);
 PERFORM pg_temp.test_image_trigger(legacy,160);
 PERFORM pg_temp.test_image_trigger(fixed,1350000);
 -- Bounds cover the entire operation: n=1 and n=3 have the same native hold.
 PERFORM pg_temp.test_image_trigger(jsonb_set(native,'{request,n}','1'),1710);
 PERFORM pg_temp.test_image_trigger(jsonb_set(jsonb_set(native,'{rule,max_n}','128'),'{request,n}','128'),1710);
 FOREACH whitespace IN ARRAY ARRAY[E' ',E'\t',E'\n',E'\r',E'\f'] LOOP
  expr:=replace(native#>>'{rule,count_basis,native_tokens,source_expression}',' ',whitespace);
  tariff:=jsonb_set(jsonb_set(native#>'{rule,count_basis,native_tokens}','{source_expression}',to_jsonb(expr)),
   '{source_sha256}',to_jsonb(encode(sha256(convert_to(expr,'UTF8')),'hex')));
  changed:=jsonb_set(native,'{rule,count_basis,native_tokens}',tariff);
  IF core_image.native_reservation(changed,discount)<>1710 THEN RAISE EXCEPTION 'valid ASCII whitespace rejected'; END IF;
 END LOOP;
 -- Exact 9-decimal coefficient and trailing decimal point are accepted by Rust.
 expr:='(tier("standard",p*0.000000001+cr_text*0.+img*0+cr_img*0+img_o*0))*(1)';
 tariff:=jsonb_set(jsonb_set(native#>'{rule,count_basis,native_tokens}','{source_expression}',to_jsonb(expr)),
  '{source_sha256}',to_jsonb(encode(sha256(convert_to(expr,'UTF8')),'hex')));
 IF core_image.native_reservation(jsonb_set(native,'{rule,count_basis,native_tokens}',tariff),discount)<>1 THEN
  RAISE EXCEPTION 'positive subcredit must ceil to one'; END IF;
 FOREACH expr IN ARRAY ARRAY[
  '(tier("standard",p*5+cr_text*1+img*8+cr_img*2+p*30))*(1)',
  '(tier("standard",p*5+cr_text*1+img*8+cr_img*2))*(1)',
  '(tier("standard",p*5+cr_text*1+img*8+cr_img*2+unknown*30))*(1)',
  '(tier("standard",p*0.0000000001+cr_text*1+img*8+cr_img*2+img_o*30))*(1)',
  '(tier("standard",p*5+cr_text*1+img*8+cr_img*2+img_o*30))*(2)',
  '(tier(" standard",p*5+cr_text*1+img*8+cr_img*2+img_o*30))*(1)',
  '(tier("standard",p*5+cr_ text*1+img*8+cr_img*2+img_o*30))*(1)',
  '(tier("standard",p*5+cr_text*1+img*8+cr_img*2+img_o*-30))*(1)',
  '(tier("standard",p*5+cr_text*1+img*8+cr_img*2+img_o*18446744073709551616))*(1)'
 ] LOOP
  tariff:=jsonb_set(jsonb_set(native#>'{rule,count_basis,native_tokens}','{source_expression}',to_jsonb(expr)),
   '{source_sha256}',to_jsonb(encode(sha256(convert_to(expr,'UTF8')),'hex')));
  BEGIN PERFORM core_image.native_reservation(jsonb_set(native,'{rule,count_basis,native_tokens}',tariff),discount);
   RAISE EXCEPTION 'unsupported expression accepted: %',expr;
  EXCEPTION WHEN check_violation THEN NULL; END;
 END LOOP;
 FOREACH changed IN ARRAY ARRAY[
  native #- '{rule,count_basis,native_tokens,max_input_tokens}',
  jsonb_set(native,'{rule,count_basis,native_tokens,max_input_tokens}','-1'),
  jsonb_set(native,'{rule,count_basis,native_tokens,max_output_tokens}','1000000001'),
  jsonb_set(native,'{rule,count_basis,native_tokens,usd_multiplier,denominator}','0'),
  jsonb_set(native,'{rule,count_basis,native_tokens,source_sha256}',to_jsonb(repeat('0',64))),
  jsonb_set(native,'{rule,count_basis,native_tokens,usd_multiplier,numerator}','18446744073709551616'),
  jsonb_set(native,'{rule,count_basis,native_tokens,usd_multiplier,numerator}','18446744073709551615'),
  jsonb_set(native,'{rule,usd_micros_per_image,numerator}','1'),
  jsonb_set(native,'{rule,count_basis}',(native#>'{rule,count_basis}')||(legacy#>'{rule,count_basis}')),
  jsonb_set(legacy,'{rule,count_basis,legacy_token_ratio,model_ratio,denominator}','0'),
  jsonb_set(legacy,'{rule,count_basis,legacy_token_ratio,legacy_quota_per_unit}','1')
 ] LOOP
  BEGIN PERFORM core_image.native_reservation(changed,discount); RAISE EXCEPTION 'bad tariff accepted';
  EXCEPTION WHEN check_violation THEN NULL; END;
 END LOOP;
 FOREACH changed IN ARRAY ARRAY[
  jsonb_set(native,'{reservation_credits}','1899'),
  jsonb_set(native,'{rule,max_budget}','1899'),
  jsonb_set(native,'{request,n}','4'),
  jsonb_set(native,'{request,n}','0'),
  jsonb_set(native,'{rule,max_n}','129'),
  jsonb_set(legacy,'{reservation_credits}','177'),
  jsonb_set(legacy,'{rule,max_budget}','177'),
  jsonb_set(fixed,'{reservation_credits}','1499999'),
  jsonb_set(fixed,'{rule,max_budget}','1499999'),
  jsonb_set(fixed,'{rule,max_n}','128')
 ] LOOP
  BEGIN PERFORM pg_temp.test_image_trigger(changed,NULL); RAISE EXCEPTION 'trigger accepted forged base/count/budget';
  EXCEPTION WHEN check_violation THEN NULL; END;
 END LOOP;
 BEGIN PERFORM pg_temp.test_image_trigger(native,1709); RAISE EXCEPTION 'wrong native hold accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN PERFORM pg_temp.test_image_trigger(legacy,161); RAISE EXCEPTION 'rounded base then discount accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 REVOKE EXECUTE ON FUNCTION core_image.native_reservation(jsonb,jsonb) FROM image_reservation_runtime;
 BEGIN PERFORM pg_temp.test_image_trigger(native,NULL); RAISE EXCEPTION 'missing helper privilege accepted';
 EXCEPTION WHEN insufficient_privilege THEN NULL; END;
 GRANT EXECUTE ON FUNCTION core_image.native_reservation(jsonb,jsonb) TO image_reservation_runtime;
 PERFORM pg_temp.test_image_trigger(native,1710);
 REVOKE EXECUTE ON FUNCTION core_image.native_counter(jsonb,numeric) FROM image_reservation_runtime;
 IF has_function_privilege('image_reservation_runtime','core_image.native_counter(jsonb,numeric)','EXECUTE') THEN
  RAISE EXCEPTION 'fixture counter EXECUTE still inherited'; END IF;
 BEGIN PERFORM pg_temp.test_image_trigger(native,NULL); RAISE EXCEPTION 'nested helper privilege accepted';
 EXCEPTION WHEN insufficient_privilege THEN NULL; END;
 GRANT EXECUTE ON FUNCTION core_image.native_counter(jsonb,numeric) TO image_reservation_runtime;
 PERFORM pg_temp.test_image_trigger(legacy,160);
END $$;
SELECT 'native_reservation: native5rate/legacy/fixed/whitespace/grammar/bounds/trust/deferred/ACL PASS' AS result;
ROLLBACK;

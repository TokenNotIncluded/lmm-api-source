-- Offline owner migration. Supply the previously active SidebarModulesAdmin
-- JSON via psql -v sidebar_value=... (use '{}' for a verified unset option).
\set ON_ERROR_STOP on
BEGIN;
LOCK TABLE lmm_community.contract IN EXCLUSIVE MODE;
DO $$ BEGIN
 IF (SELECT version FROM lmm_community.contract WHERE singleton) IS DISTINCT FROM 3 THEN
  RAISE EXCEPTION 'community version 3 required';
 END IF;
END $$;
ALTER TABLE lmm_community.navigation
 ADD COLUMN sidebar_value text NOT NULL DEFAULT '{}' CHECK(octet_length(sidebar_value)<=65536 AND jsonb_typeof(sidebar_value::jsonb)='object'),
 ADD COLUMN sidebar_revision bigint NOT NULL DEFAULT 1 CHECK(sidebar_revision>0),
 ADD COLUMN sidebar_updated_by bigint CHECK(sidebar_updated_by>0),
 ADD COLUMN sidebar_updated_at bigint NOT NULL DEFAULT 0 CHECK(sidebar_updated_at>=0);
UPDATE lmm_community.navigation SET sidebar_value=:'sidebar_value' WHERE singleton;
DO $$
DECLARE
 config jsonb;
BEGIN
 SELECT sidebar_value::jsonb INTO STRICT config FROM lmm_community.navigation WHERE singleton;
 IF EXISTS (SELECT FROM jsonb_each(config) s WHERE jsonb_typeof(s.value)<>'object') THEN
  RAISE EXCEPTION 'invalid legacy sidebar section';
 END IF;
 IF EXISTS (
  SELECT FROM jsonb_each(config) s CROSS JOIN LATERAL jsonb_each(s.value) f
  WHERE NOT (
   jsonb_typeof(f.value)='boolean'
   OR (jsonb_typeof(f.value)='number' AND f.value IN ('0'::jsonb,'1'::jsonb))
   OR (jsonb_typeof(f.value)='string' AND lower(regexp_replace(f.value #>> '{}', '^[[:space:]]+|[[:space:]]+$', '', 'g')) IN ('0','1','true','false'))
  )
 ) THEN
  RAISE EXCEPTION 'invalid legacy sidebar flag';
 END IF;
END $$;
UPDATE lmm_community.contract SET version=4 WHERE singleton;
COMMIT;

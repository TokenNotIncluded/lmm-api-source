-- Offline owner-only migration in the isolated community database.
-- Pass the frozen original HeaderNavModules value from the Core options export
-- using psql -v navigation_value=... . The value is required, even for '{}'.
-- Never join the Core database or grant its role to an extension.
\set ON_ERROR_STOP on
BEGIN;
LOCK TABLE lmm_community.contract IN EXCLUSIVE MODE;
DO $$ BEGIN
 IF (SELECT version FROM lmm_community.contract WHERE singleton) IS DISTINCT FROM 2 THEN
  RAISE EXCEPTION 'community version 2 required';
 END IF;
END $$;
CREATE TABLE lmm_community.navigation (
 singleton boolean PRIMARY KEY DEFAULT true CHECK(singleton),
 value text NOT NULL DEFAULT '{}' CHECK(octet_length(value)<=65536 AND jsonb_typeof(value::jsonb)='object'),
 revision bigint NOT NULL DEFAULT 1 CHECK(revision>0),
 updated_by bigint CHECK(updated_by>0),
 updated_at bigint NOT NULL DEFAULT 0 CHECK(updated_at>=0)
);
INSERT INTO lmm_community.navigation(singleton,value) VALUES(true,:'navigation_value');
REVOKE ALL ON lmm_community.navigation FROM PUBLIC;
UPDATE lmm_community.contract SET version=3 WHERE singleton;
COMMIT;

-- Owner-installed public GitHub configuration and last complete imported snapshot.
CREATE TABLE lmm_operations.script_repository (
 singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
 revision bigint NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
 repository_url text NOT NULL DEFAULT '' CHECK (octet_length(repository_url)<=256),
 branch text NOT NULL DEFAULT 'main' CHECK (octet_length(branch) BETWEEN 1 AND 200),
 scripts jsonb CHECK (jsonb_typeof(scripts)='object' AND octet_length(scripts::text)<=12582912),
 snapshot_repository_url text NOT NULL DEFAULT '' CHECK (octet_length(snapshot_repository_url)<=256),
 snapshot_branch text NOT NULL DEFAULT '' CHECK (octet_length(snapshot_branch)<=200),
 snapshot_pulled_at timestamptz,
 updated_by bigint CHECK (updated_by BETWEEN 1 AND 9007199254740991),
 CHECK ((scripts IS NULL AND snapshot_repository_url='' AND snapshot_branch='' AND snapshot_pulled_at IS NULL)
 OR (scripts IS NOT NULL AND snapshot_repository_url<>'' AND snapshot_branch<>'' AND snapshot_pulled_at IS NOT NULL))
);
INSERT INTO lmm_operations.script_repository(singleton) VALUES (true);
REVOKE ALL ON lmm_operations.script_repository FROM PUBLIC;

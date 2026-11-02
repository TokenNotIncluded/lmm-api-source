-- Read-only component46 authority check, included inside the offline grant transaction.
-- The component uses the existing NOLOGIN lmm_core_owner; no new owner/runtime membership.
SELECT EXISTS(SELECT FROM pg_roles WHERE rolname='lmm_core_owner'
  AND NOT (rolcanlogin OR rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
AND (SELECT count(*)=2 AND bool_and(c.relowner='lmm_core_owner'::regrole AND c.relkind='r')
  FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
  WHERE (n.nspname,c.relname) IN (('core_channels','catalog_observations'),('core_meta','channel_observations_upgrades')))
AND NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
  CROSS JOIN LATERAL aclexplode(c.relacl) a
  WHERE n.nspname='core_meta' AND c.relname='channel_observations_upgrades' AND a.grantee<>c.relowner)
AND NOT EXISTS(SELECT FROM pg_attribute col JOIN pg_class c ON c.oid=col.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace
  CROSS JOIN LATERAL aclexplode(col.attacl) a
  WHERE n.nspname='core_meta' AND c.relname='channel_observations_upgrades' AND a.grantee<>c.relowner)
AS channel_observations_owner_safe \gset
\if :channel_observations_owner_safe
\else
DO $$ BEGIN RAISE EXCEPTION 'Component46 requires the restricted installation owner and private upgrade receipt'; END $$;
\endif

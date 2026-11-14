-- Revokes only component70 privileges. It never drops lineage or changes grants/data.
\set ON_ERROR_STOP on
REVOKE ALL ON core_oauth.managed_authorization_bindings,core_oauth.managed_family_bindings,
 core_oauth.managed_refresh_lineage,core_oauth.managed_refresh_responses FROM :"runtime_role";
REVOKE ALL ON FUNCTION core_oauth.cleanup_managed_refresh_responses(integer) FROM :"runtime_role";
REVOKE ALL ON core_meta.managed_applications_upgrades FROM :"runtime_role";

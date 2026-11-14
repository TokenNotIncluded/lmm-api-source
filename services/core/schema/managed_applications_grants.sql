-- Run after the unchanged historical runtime grants, with an explicit runtime_role.
GRANT SELECT, INSERT ON core_oauth.managed_authorization_bindings,
 core_oauth.managed_family_bindings,core_oauth.managed_refresh_lineage,
 core_oauth.managed_refresh_responses TO :"runtime_role";
REVOKE UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN ON
 core_oauth.managed_authorization_bindings,core_oauth.managed_family_bindings,
 core_oauth.managed_refresh_lineage,core_oauth.managed_refresh_responses FROM :"runtime_role";
REVOKE ALL ON FUNCTION core_oauth.cleanup_managed_refresh_responses(integer) FROM :"runtime_role";
REVOKE ALL ON core_meta.managed_applications_upgrades FROM :"runtime_role";

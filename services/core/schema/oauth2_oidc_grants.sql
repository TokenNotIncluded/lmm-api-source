-- Apply with the existing explicit owner installation/runtime-role procedure.
GRANT USAGE ON SCHEMA core_oidc TO :"runtime_role";
GRANT SELECT,INSERT ON core_oidc.authorization_requests,core_oidc.families,core_oidc.tokens TO :"runtime_role";

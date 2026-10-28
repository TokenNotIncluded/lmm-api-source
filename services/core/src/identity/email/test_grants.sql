-- Test-only role. This is not a replacement for grant-core-runtime.sql.
GRANT USAGE ON SCHEMA core_identity TO email_runtime;
GRANT SELECT ON core_identity.accounts,core_identity.users,core_identity.user_profiles,
 core_identity.password_logins,core_identity.credentials,core_identity.web_sessions,
 core_identity.session_authentication,core_identity.mfa_factors,core_identity.email_addresses
 TO email_runtime;
-- PostgreSQL row locks require UPDATE privilege, including FOR SHARE.
GRANT UPDATE(id) ON core_identity.accounts,core_identity.users,core_identity.credentials TO email_runtime;
GRANT UPDATE(user_id) ON core_identity.password_logins,core_identity.mfa_factors,
 core_identity.session_authentication,core_identity.email_addresses TO email_runtime;
GRANT UPDATE(sid) ON core_identity.web_sessions TO email_runtime;
GRANT UPDATE(display_name,email,email_version,email_login_allowed) ON core_identity.user_profiles TO email_runtime;
GRANT SELECT,INSERT ON core_identity.email_challenges,core_identity.email_rate_buckets,
 core_identity.email_rate_events TO email_runtime;
GRANT UPDATE(state,attempts,consumed_at,result_user_id) ON core_identity.email_challenges TO email_runtime;
GRANT UPDATE(bucket) ON core_identity.email_rate_buckets TO email_runtime;
GRANT INSERT ON core_identity.audit TO email_runtime;
GRANT USAGE ON SEQUENCE core_identity.audit_id_seq TO email_runtime;

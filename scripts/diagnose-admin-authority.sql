-- Read-only administrator admission facts. Never print credential digests,
-- passwords, MFA secrets or policy payloads. Run against the explicitly selected
-- database; this script does not connect to or modify a production instance.
\set ON_ERROR_STOP on
\if :{?user_id}
\else
\set user_id 1
\endif
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT u.id,u.platform_role,u.active AS user_active,a.active AS account_active,
       u.auth_version,p.password_login_allowed,p.second_factor_required,
       COALESCE(f.enabled,false) AS factor_enabled,f.revision AS factor_revision,
       EXISTS(SELECT FROM core_identity.privileged_import_facts i WHERE i.user_id=u.id)
         AS imported_privileged
FROM core_identity.users u
JOIN core_identity.accounts a ON a.id=u.personal_account_id
LEFT JOIN core_identity.user_profiles p ON p.user_id=u.id
LEFT JOIN core_identity.mfa_factors f ON f.user_id=u.id
WHERE u.id=:'user_id'::bigint;

SELECT p.user_id,p.revision,p.contract,p.verified_complete,
       octet_length(p.sha256)=32 AS digest_length_valid,
       sha256(p.canonical_payload)=p.sha256 AS payload_digest_matches,
       (SELECT count(*) FROM core_identity.permission_policy_rows r WHERE r.user_id=p.user_id)
         AS policy_row_count
FROM core_identity.permission_policies p WHERE p.user_id=:'user_id'::bigint;
SELECT action,effect FROM core_identity.permission_policy_rows
WHERE user_id=:'user_id'::bigint ORDER BY action;

SELECT c.id AS credential_id,c.kind,c.user_version,
       c.user_version=u.auth_version AS generation_matches,
       c.owner_account_id=u.personal_account_id AS owns_personal_account,
       c.revoked_at IS NULL AS credential_not_revoked,
       c.expires_at>clock_timestamp() AS credential_not_expired,
       s.user_id AS proof_user_id,s.user_version AS proof_user_version,
       s.factor_level,s.factor_revision,s.method,
       s.mfa_verified_at IS NOT NULL AS has_mfa_timestamp,
       w.current_credential_id=c.id AS current_browser_credential,
       w.revoked_at IS NULL AND w.expires_at>clock_timestamp()
         AND w.user_version=u.auth_version AS browser_family_live
FROM core_identity.credentials c
JOIN core_identity.users u ON u.id=c.user_id
LEFT JOIN core_identity.session_authentication s ON s.credential_id=c.id
LEFT JOIN core_identity.web_sessions w ON w.current_credential_id=c.id
WHERE c.user_id=:'user_id'::bigint AND c.kind='session'
ORDER BY c.id DESC LIMIT 20;
ROLLBACK;

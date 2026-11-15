-- Check-in rewards use browser identity/account-security grants as prerequisites.
-- The owner installs component 75 before these optional runtime grants.
GRANT USAGE ON SCHEMA core_checkin_challenges TO :"runtime_role";
GRANT SELECT,INSERT,DELETE ON core_checkin_challenges.challenges TO :"runtime_role";
GRANT UPDATE(attempts,verified_at,consumed_at) ON core_checkin_challenges.challenges TO :"runtime_role";
GRANT SELECT,INSERT ON core_checkin_challenges.receipts TO :"runtime_role";

-- Immutable source authority is installed only by the fenced owner procedure.
GRANT SELECT ON core_gift.legacy_installations,core_gift.legacy_bindings,
 core_gift.legacy_decisions,core_gift.legacy_risk_memories,core_gift.legacy_imports TO :"runtime_role";
GRANT SELECT,INSERT ON core_gift.legacy_profiles,core_gift.legacy_fingerprints,core_gift.legacy_claims TO :"runtime_role";
GRANT UPDATE(identity_hash,network_hash,observed_at) ON core_gift.legacy_profiles TO :"runtime_role";
GRANT DELETE ON core_gift.legacy_fingerprints TO :"runtime_role";
GRANT USAGE ON SEQUENCE core_gift.legacy_fingerprints_id_seq TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_gift.legacy_oauth_subject(bigint,text) TO :"runtime_role";

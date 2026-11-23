"""Synthetic pure regression; no database, real source rows or approvals."""
from copy import deepcopy
import hashlib
import os
from pathlib import Path
import unittest

import current_key_source_partition as S

SUPPORT = Path(os.environ.get("LMM_PARTITION_TEST_SUPPORT", "/tmp/lmm-source-fa57/source/scripts"))
P, R, C = S.load_helpers(SUPPORT)
F = R.F
H = lambda value: hashlib.sha256(value.encode()).hexdigest()


def token(ident, key, **changes):
    row = dict(id=ident, user_id=1, key=key, status=1, name="synthetic",
        created_time=1, accessed_time=1, expired_time=-1, remain_quota=-15,
        unlimited_quota=True, one_time_reveal=True, account_balance_read=False,
        model_limits_enabled=False, model_limits="", allow_ips=None, used_quota=70,
        group="paid", cross_group_retry=False, auto_groups="", oauth_managed=False,
        creation_source="manual", deleted_at=None)
    row.update(changes)
    return row


def fixture():
    gid = "g" * 43
    parent = dict(format_version=1, source_revision=P.LEGACY_REVISION,
        source_archive_sha256=H("archive"), source_schema_sha256=H("schema"),
        source_export_sha256=H("export"), source_projection_sha256="",
        snapshot_at=10, tokens=[
            token(1, H("ordinary secret")),
            token(2, "oauth_managed_" + H(gid + "\0paid"), oauth_managed=True),
            token(3, H("assistant internal secret"), creation_source="assistant_runtime")],
        users=[dict(id=1, status=1, deleted_at=None, auth_version=1, group="paid", billing_preference="wallet_first")],
        policy=dict(auto_groups=[], max_token_auto_groups=1, user_model_rate_limit_enabled=False,
            source_options_sha256=H("options"), group_authority=dict(user_usable_groups={"paid": "Paid"},
                special_usable_groups={}, group_ratios={"paid": 1}, group_group_ratios={})))
    provenance = {k: parent[k] for k in ("source_archive_sha256", "source_export_sha256",
        "source_schema_sha256", "snapshot_at", "source_revision")}
    provenance.update(scope="legacy-api-key-source-projection", source_row_counts={"users": 1, "tokens": 3, "options": 4})
    parent["source_projection_sha256"] = P.sha256(provenance)
    grant = dict(id=gid, user_id=1, client_id="lmm-opencode", issuer="https://synthetic.invalid",
        resource="https://synthetic.invalid/api/oauth2", scope="models:invoke group:cGFpZA",
        created_at_ms=1000, absolute_expires_at_ms=1000000, revoked_at_ms=0,
        redirect_uri="http://127.0.0.1:2345/oauth/lmm/callback",
        binding_method="", binding_thumbprint="", revocation_reason="", lock_version=1)
    member = dict(digest=H("retained refresh digest"), family_id=gid, issuer=grant["issuer"],
        kind="refresh", scope=grant["scope"], created_at_ms=1000, expires_at_ms=200000, used_at_ms=0)
    obs = dict(observed_at="1970-01-01T00:01:40+00:00", oauth_server_grants=[grant],
        oauth_server_tokens=[member], oauth_server_codes=[], oauth_server_authorizations=[],
        oauth_server_mcp_clients=[], o_auth_billing_bindings=[dict(grant_id=gid, token_id=2, user_id=1, group="paid")],
        managed_tokens=[{**{k: v for k, v in r.items() if k != "key"}, "key_sha256": H(r["key"])}
                        for r in parent["tokens"][1:]])
    return parent, provenance, obs


def bind(parent, provenance, obs):
    context = S.parent_context(P, F, parent, provenance, obs["oauth_server_tokens"],
        {"synthetic": H("not an actual source file")}, origin="SYNTHETIC_PURE_TEST")
    acquisition = dict(scope="SYNTHETIC_PURE_TEST", source_observation_sha256=F.digest(obs),
        observed_at=obs["observed_at"], source_production_fenced=False)
    return context, acquisition


def build(parent, provenance, obs, resolutions=()):
    context, acquisition = bind(parent, provenance, obs)
    return S.assemble(P, F, C, parent, provenance, context, obs, acquisition, resolutions)


class PartitionTests(unittest.TestCase):
    def setUp(self):
        self.parent, self.provenance, self.obs = fixture()
        self.context, self.acquisition = bind(self.parent, self.provenance, self.obs)
        self.material, self.view, self.archive = build(self.parent, self.provenance, self.obs)

    def validate(self):
        return S.validate_saved(P, F, C, self.parent, self.provenance, self.context,
            self.obs, self.acquisition, self.material, self.view, self.archive)

    def test_exact_source_partition_preserves_negative_unlimited(self):
        self.validate()
        self.assertEqual([r["id"] for r in self.view["tokens"]], [1])
        self.assertEqual([r["id"] for r in self.archive["rows"]], [2, 3])
        self.assertEqual(self.view["tokens"][0]["remain_quota"], -15)
        self.assertEqual(self.view["tokens"][0]["used_quota"], 70)
        self.assertEqual(self.material["status"], "BLOCKED_OAUTH_CONTINUATION_REQUIRED")
        self.assertFalse(self.material["apply_supported"])
        self.assertEqual(self.material["credential_ids_allocated"], 0)

    def test_wrong_parent_rejected(self):
        self.parent["tokens"][0]["remain_quota"] = 5
        with self.assertRaisesRegex(S.Error, "verified_parent_changed"):
            self.validate()

    def test_wrong_parent_provenance_rejected(self):
        self.provenance["source_export_sha256"] = H("another export")
        with self.assertRaisesRegex(S.Error, "verified_parent_changed"):
            self.validate()

    def test_parent_count_lie_rejected(self):
        self.provenance["source_row_counts"]["tokens"] = 2
        self.parent["source_projection_sha256"] = P.sha256(self.provenance)
        with self.assertRaisesRegex(S.Error, "parent_coverage_mismatch"):
            bind(self.parent, self.provenance, self.obs)

    def test_missing_ordinary_row_rejected(self):
        self.view["tokens"].clear()
        with self.assertRaisesRegex(S.Error, "saved_partition_not_exact_rebuild"):
            self.validate()

    def test_overlap_rejected(self):
        self.view["tokens"].append(deepcopy(self.archive["rows"][0]))
        with self.assertRaisesRegex(S.Error, "saved_partition_not_exact_rebuild"):
            self.validate()

    def test_ordinary_row_mutation_rejected(self):
        self.view["tokens"][0]["unlimited_quota"] = False
        with self.assertRaisesRegex(S.Error, "saved_partition_not_exact_rebuild"):
            self.validate()

    def test_internal_archive_row_mutation_rejected(self):
        self.archive["rows"][0]["status"] = 2
        with self.assertRaisesRegex(S.Error, "saved_partition_not_exact_rebuild"):
            self.validate()

    def test_fake_reuse_of_full_parent_projection_digest_rejected(self):
        self.view["source_projection_sha256"] = self.parent["source_projection_sha256"]
        with self.assertRaisesRegex(S.Error, "saved_partition_not_exact_rebuild"):
            self.validate()

    def test_unresolved_live_refresh_cannot_be_marked_ready(self):
        self.material["status"] = "PARTITION_VALID_PREVIEW_ONLY"
        self.material["oauth"]["unresolved"].clear()
        with self.assertRaisesRegex(S.Error, "saved_partition_not_exact_rebuild"):
            self.validate()

    def test_arbitrary_receipt_hash_is_not_continuation(self):
        with self.assertRaisesRegex(S.Error, "oauth_continuation_contract_not_implemented"):
            build(self.parent, self.provenance, self.obs,
                  [{"kind": "native_receipt", "sha256": H("arbitrary"), "approved": True}])

    def test_active_family_cannot_claim_source_terminal(self):
        with self.assertRaisesRegex(S.Error, "oauth_terminal_evidence_not_actual"):
            build(self.parent, self.provenance, self.obs, [{
                "kind": "source_terminal_authority", "source_grant_id": "g" * 43,
                "reason": "family_revoked", "source_observation_sha256": F.digest(self.obs)}])

    def test_consumed_refresh_does_not_hide_live_descendant(self):
        self.obs["oauth_server_tokens"][0]["used_at_ms"] = 50000
        child = deepcopy(self.obs["oauth_server_tokens"][0])
        child.update(digest=H("child"), used_at_ms=0, created_at_ms=50000, expires_at_ms=300000)
        self.obs["oauth_server_tokens"].append(child)
        material, _, _ = build(self.parent, self.provenance, self.obs)
        self.assertEqual(len(material["oauth"]["unresolved"]), 1)
        self.assertEqual(material["oauth"]["unresolved"][0]["live_unused_refresh_count"], 1)

    def test_consumed_refresh_does_not_hide_live_access(self):
        self.obs["oauth_server_tokens"][0]["used_at_ms"] = 50000
        access = deepcopy(self.obs["oauth_server_tokens"][0])
        access.update(digest=H("access"), kind="access", used_at_ms=0)
        self.obs["oauth_server_tokens"].append(access)
        material, _, _ = build(self.parent, self.provenance, self.obs)
        self.assertEqual(material["oauth"]["unresolved"][0]["live_access_count"], 1)

    def test_missing_member_against_complete_canonical_set_rejected(self):
        self.obs["oauth_server_tokens"].clear()
        self.acquisition["source_observation_sha256"] = F.digest(self.obs)
        with self.assertRaisesRegex(S.Error, "oauth_full_canonical_member_set_changed"):
            self.validate()

    def test_family_without_billing_is_still_examined(self):
        self.obs["oauth_server_grants"][0]["revoked_at_ms"] = 90000
        g = deepcopy(self.obs["oauth_server_grants"][0])
        g.update(id="h" * 43, revoked_at_ms=0)
        t = deepcopy(self.obs["oauth_server_tokens"][0])
        t.update(family_id=g["id"], digest=H("no billing family member"))
        self.obs["oauth_server_grants"].append(g)
        self.obs["oauth_server_tokens"].append(t)
        material, _, _ = build(self.parent, self.provenance, self.obs)
        self.assertEqual(material["oauth"]["unresolved"][0]["source_grant_id"], g["id"])

    def test_expiry_equality_is_terminal_without_approval(self):
        self.obs["oauth_server_tokens"][0]["expires_at_ms"] = 100000
        material, _, _ = build(self.parent, self.provenance, self.obs)
        self.assertEqual(material["status"], "PARTITION_VALID_PREVIEW_ONLY")
        self.assertEqual(material["oauth"]["terminal_facts"][0]["reason"], "no_live_members_or_codes")
        self.assertFalse(material["execution_authorized"])
        self.assertFalse(material["apply_supported"])

    def test_actual_revoked_family_resolves_only_its_scope(self):
        self.obs["oauth_server_grants"][0]["revoked_at_ms"] = 90000
        preliminary, _, _ = build(self.parent, self.provenance, self.obs)
        evidence = preliminary["oauth"]["terminal_facts"]
        material, _, _ = build(self.parent, self.provenance, self.obs, evidence)
        self.assertEqual(material["status"], "PARTITION_VALID_PREVIEW_ONLY")
        self.assertEqual(material["oauth"]["explicit_terminal_evidence"], evidence)
        self.assertFalse(material["source_review_generated"])
        self.assertEqual(material["mode"], "PREVIEW_ONLY")

    def test_expired_family_preserves_live_member_rows_as_archive(self):
        self.obs["oauth_server_grants"][0]["absolute_expires_at_ms"] = 100000
        material, _, _ = build(self.parent, self.provenance, self.obs)
        self.assertEqual(material["oauth"]["terminal_facts"][0]["reason"], "family_expired")
        self.assertEqual(len(self.obs["oauth_server_tokens"]), 1)

    def test_duplicate_terminal_evidence_rejected(self):
        self.obs["oauth_server_grants"][0]["revoked_at_ms"] = 90000
        preliminary, _, _ = build(self.parent, self.provenance, self.obs)
        r = preliminary["oauth"]["terminal_facts"][0]
        with self.assertRaisesRegex(S.Error, "oauth_terminal_evidence_not_actual"):
            build(self.parent, self.provenance, self.obs, [r, r])

    def test_unknown_member_kind_rejected(self):
        self.obs["oauth_server_tokens"][0]["kind"] = "approved"
        with self.assertRaisesRegex(S.Error, "oauth_member_kind_unknown"):
            build(self.parent, self.provenance, self.obs)

    def test_orphan_member_rejected(self):
        self.obs["oauth_server_tokens"][0]["family_id"] = "missing"
        with self.assertRaisesRegex(S.Error, "oauth_member_family_missing"):
            build(self.parent, self.provenance, self.obs)

    def test_internal_binding_cannot_be_silently_changed(self):
        self.obs["o_auth_billing_bindings"][0]["group"] = "FREE"
        with self.assertRaisesRegex(S.Error, "internal_billing_binding_missing"):
            build(self.parent, self.provenance, self.obs)

    def test_preview_is_deterministic_and_does_not_mutate_source(self):
        before = deepcopy((self.parent, self.provenance, self.obs))
        one = build(self.parent, self.provenance, self.obs)
        two = build(self.parent, self.provenance, self.obs)
        self.assertEqual(one, two)
        self.assertEqual((self.parent, self.provenance, self.obs), before)


if __name__ == "__main__":
    unittest.main()

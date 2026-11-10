"""Synthetic only: no human approval, activation, database or production read."""
from copy import deepcopy
import hashlib
import unittest

import legacy_admin_activation_lineage as L


def fixture():
    mapping = {"source_user_id": 37, "target_user_id": 37, "target_personal_account_id": 8}
    map_hash = hashlib.sha256((L.canonical(mapping) + "\n").encode()).hexdigest()
    a = {name: hashlib.sha256(("synthetic:" + name).encode()).hexdigest() for name in L.FACT_HASHES}
    a.update(approval_id="1" * 64, approver="2" * 64, expiry_unix=1700000000,
        migration_id="synthetic-identity-import", user_id=37, personal_account_id=8,
        database_oid=13, source_enabled=True, source_deleted=False, password_verifier_recognized=True,
        password_login_enabled=True, second_factor_required=False, role="superadmin", auth_version=6,
        policy_revision=1, policy={k: "deny" for k in L.ACTIONS})
    a["archive_sha256"] = "a" * 64
    a["import_run"] = hashlib.sha256(a["migration_id"].encode()).hexdigest()
    a["system_id"] = (123).to_bytes(8, "big").hex() + "00" * 24
    a["policy_sha256"] = hashlib.sha256(L.policy_bytes(a)).hexdigest()
    a["password_hash_sha256"] = hashlib.sha256(b"synthetic-bcrypt-verifier").hexdigest()
    a["login_name_sha256"] = hashlib.sha256(b"legacy-37").hexdigest()
    f = {k: a[k] for k in (*L.FACT_HASHES, "user_id", "migration_id", "database_oid")}
    f.update({k: a[v] for k, v in L.FACT_TO_APPROVAL.items()})
    payload = L.approval_hash(a)
    current = {"user_id": 37, "personal_account_id": 8, "platform_role": "superadmin", "active": True,
        "auth_version": 7, "account_kind": "personal", "account_active": True, "password_login_allowed": True,
        "second_factor_required": False, "password_hash_sha256": a["password_hash_sha256"], "login_name_sha256": a["login_name_sha256"]}
    e = {"facts": f, "receipts": [{"approval_id": a["approval_id"], "payload_sha256": payload, "user_id": 37, "expected_auth_version": 6}],
        "audits": [{"id": 19, "actor_user_id": 37, "action": "privileged_activation", "target": {"approval_id": a["approval_id"], "auth_version": 7}}],
        "policy": {"user_id": 37, "revision": 1, "contract": L.CONTRACT, "sha256": a["policy_sha256"],
                   "canonical_payload": L.policy_bytes(a).hex(), "verified_complete": True},
        "policy_rows": [{"user_id": 37, "action": k, "effect": "deny"} for k in sorted(L.ACTIONS)],
        "current": current, "instance": {k: a[k] for k in ("system_id", "database_oid", "instance_nonce")},
        "identity_import": {"migration_id": a["migration_id"], "source_archive_sha256": a["archive_sha256"],
            "mapping_sha256": a["a2_semantic_sha256"], "manifest": {"scope": "identity-only",
                "mapping_file_sha256": a["a2_file_sha256"], "source_target_id_map_sha256": map_hash}}}
    b = {**mapping, "source_target_id_map_sha256": map_hash,
         "approved_apply_request": {"confirmation_sha256": payload, "approval": a}, "native_evidence": e}
    instance = {"system_identifier": "123", "database_oid": 13, "server_instance_nonce": a["instance_nonce"]}
    return b, mapping, instance


def check(b):
    _, mapping, _ = fixture()
    return L.validate_bindings([b], fixture()[0]["approved_apply_request"]["approval"]["archive_sha256"],
        [mapping], b["source_target_id_map_sha256"])


class LineageTests(unittest.TestCase):
    def test_consumed_source6_derives_only_current7(self):
        b, _, instance = fixture()
        self.assertEqual(list(check(b)), [37])
        observed = {k: b["native_evidence"]["current"][k] for k in ("user_id", "personal_account_id", "active", "auth_version")}
        source = {"auth_version": 6, "status": 1, "deleted_at": None}
        self.assertEqual(L.expected_version(source, observed, b), 7)
        self.assertEqual(source["auth_version"], 6)
        L.bind_instance([b], instance, b["native_evidence"]["identity_import"])
        # Receipt consumption may be verified after approval expiry; it does not
        # grant permission to apply an expired unconsumed candidate now.

    def test_native_format_fixed_vector(self):
        a = fixture()[0]["approved_apply_request"]["approval"]
        self.assertEqual(a["policy_sha256"], "9f7fc2baee5ef2872fbbc7bcf0fb84f6fc6961061602d5c104463f2b4fba5d9d")
        self.assertEqual(L.approval_hash(a), "74d7f23cf035829600a04582dbfb6aa4a2460b005d5af80e20a5f33e36d1a2ed")

    def test_missing_duplicate_or_unconsumed_evidence_rejects(self):
        for field in ("receipts", "audits", "policy_rows"):
            for replacement in ([], None):
                b, _, _ = fixture()
                b["native_evidence"][field] = replacement
                with self.subTest(field=field, replacement=replacement), self.assertRaises(L.LineageError):
                    check(b)
        for field in ("receipts", "audits"):
            b, _, _ = fixture()
            b["native_evidence"][field] *= 2
            with self.assertRaises(L.LineageError):
                check(b)

    def test_tampering_each_authority_binding_rejects(self):
        cases = [("facts", "source_auth_version", 7), ("facts", "source_enabled", False),
            ("facts", "source_deleted", True), ("facts", "source_role", "user"),
            ("facts", "a2_sql_sha256", "3" * 64), ("facts", "import_run", "4" * 64),
            ("current", "auth_version", 6), ("current", "auth_version", 8),
            ("current", "platform_role", "admin"), ("current", "account_active", False),
            ("current", "active", 1), ("current", "second_factor_required", True),
            ("current", "personal_account_id", 37), ("policy", "verified_complete", 1),
            ("policy", "canonical_payload", "00"), ("instance", "database_oid", 14),
            ("identity_import", "mapping_sha256", "5" * 64)]
        for section, field, value in cases:
            b, _, _ = fixture()
            b["native_evidence"][section][field] = value
            with self.subTest(section=section, field=field), self.assertRaises(L.LineageError):
                check(b)
        for field, value in (("approver", "6" * 64), ("expiry_unix", 1800000000),
                             ("role", "user"), ("auth_version", 7), ("source_deleted", True)):
            b, _, _ = fixture()
            b["approved_apply_request"]["approval"][field] = value
            with self.subTest(approval=field), self.assertRaises(L.LineageError):
                check(b)

    def test_wrong_source_map_archive_and_instance_reject(self):
        b, mapping, instance = fixture()
        for changes in ({"target_personal_account_id": 37}, {"source_user_id": 8}):
            bad = {**mapping, **changes}
            with self.assertRaises(L.LineageError):
                L.validate_bindings([b], b["approved_apply_request"]["approval"]["archive_sha256"], [bad], b["source_target_id_map_sha256"])
        with self.assertRaises(L.LineageError):
            L.validate_bindings([b], "7" * 64, [mapping], b["source_target_id_map_sha256"])
        for field, value in (("system_identifier", "124"), ("database_oid", 14), ("server_instance_nonce", "8" * 64)):
            with self.subTest(field=field), self.assertRaises(L.LineageError):
                L.bind_instance([b], {**instance, field: value}, b["native_evidence"]["identity_import"])

    def test_sql_rechecks_real_native_rows_without_authority_writes(self):
        sql = L.guard_sql([fixture()[0]])
        for relation in ("privileged_import_facts", "admin_activation_receipts", "audit", "permission_policies", "permission_policy_rows"):
            self.assertIn("core_identity." + relation, sql)
        self.assertIn("FOR SHARE OF u,a,p,l", sql)
        self.assertIn("pg_control_system()", sql)
        self.assertIn("IS DISTINCT FROM", sql)
        self.assertNotIn("UPDATE ", sql)
        self.assertNotIn("INSERT ", sql)


if __name__ == "__main__":
    unittest.main()

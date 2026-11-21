"""Invented source records only; never a financial approval or a DB test."""
from copy import deepcopy
import hashlib
import unittest

import migrate_current_quota_redemptions as M
import test_migrate_current_runtime_keys as KT


def source_row(sid=1, **changes):
    return {"id": sid, "key": format(sid, "032x"), "user_id": 37,
        "status": 1, "deleted_at": None, "reward_type": "quota", "quota": 31,
        "created_time": 100, "expired_time": 0, "name": "Synthetic legacy code",
        "reset_plan_id": 0, "reset_voucher_expires_at": 0, "used_user_id": 0,
        "redeemed_time": 0, **changes}


def fixture(rows=None, *, native_changes=None, migration="synthetic-current-quota"):
    _, target = KT.inputs([])
    mapping = target["source_target_id_map"]
    contract = KT.synthetic_contract()
    snap = KT.snapshot(target, contract)
    names = set(M.CHANGED) | {"core_promotions.claims"}
    snap["state"]["inventory"]["tables"] = sorted(set(snap["state"]["inventory"]["tables"]) | names)
    snap["state"]["tables"].update({t: {"rows": 0, "sha256": "1" * 64} for t in names})
    native = {"instance": snap["instance"], "prepared_transactions": 0, "imports": [],
              "identity_authority": [], "ledger": {}, **{t: [] for t in names}}
    if native_changes:
        native.update(native_changes)
    facts = M.source_facts(rows or [source_row()], mapping, 1000)
    m = M.prepare_mechanics(facts, mapping, snap, contract, KT.NAMESPACE, "a" * 64,
                           migration, ["synthetic_runtime"], native)
    return m, native


class SourceContractTests(unittest.TestCase):
    def test_same_expiry_second_remains_live_and_next_second_expires(self):
        _, target = KT.inputs([])
        mapping = target["source_target_id_map"]
        r = source_row(expired_time=1000)
        f = M.source_facts([r], mapping, 1000)
        self.assertEqual(f["codes"], 1)
        self.assertEqual(f["records"][0]["native_ends_at"], 1001)
        self.assertEqual(M.source_facts([r], mapping, 1001)["codes"], 0)

    def test_disabled_deleted_used_and_expired_codes_never_activate(self):
        rows = [source_row(1), source_row(2, status=2), source_row(3, status=3),
                source_row(4, deleted_at="2026-11-01T00:00:00Z"), source_row(5, expired_time=999)]
        m, _ = fixture(rows)
        self.assertEqual([r["source_redemption_id"] for r in m["plan"]["records"]], [1])
        self.assertEqual(m["source_facts"]["omitted"], {"nonredeemable_source_code": 4})

    def test_blank_quota_default_and_separate_reset_purpose_preserved(self):
        m, _ = fixture([source_row(1, reward_type=""), source_row(2, reward_type="reset_voucher", quota=999999)])
        self.assertEqual(m["source_facts"]["future_liability_units"], 31)
        self.assertEqual(m["source_facts"]["omitted"], {"separate_reset_voucher_right": 1})
        self.assertEqual(m["source_facts"]["reset_vouchers_imported"], 0)

    def test_source_creator_is_mapped_not_assumed_equal_to_native_user(self):
        _, t = KT.inputs([])
        mapping = [{**t["source_target_id_map"][0], "target_user_id": 802}]
        f = M.source_facts([source_row()], mapping, 1000)
        p = M.plan_records(f, KT.NAMESPACE)
        self.assertEqual(p["issues"][0]["actor_user_id"], 802)
        self.assertEqual(p["records"][0]["source_creator_id"], 37)

    def test_raw_code_is_never_copied_into_material_or_sql(self):
        r = source_row(key="a56f3327a11d190bed701c329f0a9cc4")
        m, _ = fixture([r])
        self.assertNotIn(r["key"], M.canonical(m))
        self.assertNotIn(r["key"], M.synthetic_sql(m, "a" * 64))
        self.assertEqual(m["plan"]["codes"][0]["digest"], hashlib.sha256(r["key"].encode()).hexdigest())
        self.assertEqual(m["source_facts"]["immediate_credit_units"], 0)

    def test_unrepresentable_expiry_and_unknown_reward_or_status_reject(self):
        for change in ({"expired_time": M.MAX_TIME}, {"reward_type": "new_unsupported_right"},
                       {"status": 4}, {"quota": -1}, {"quota": True}, {"created_time": 1001},
                       {"key": " " + "a" * 32}, {"user_id": 9999}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                fixture([source_row(**change)])

    def test_duplicate_source_identity_or_code_never_collapses(self):
        for rows in ([source_row(), source_row()], [source_row(1), source_row(2, key=format(1, "032x"))]):
            with self.subTest(rows=len(rows)), self.assertRaises(ValueError):
                fixture(rows)


class PlanTests(unittest.TestCase):
    def test_every_live_code_has_exact_reserved_amount_and_one_identity(self):
        rows = [source_row(i, quota=20 if i % 2 else 30) for i in range(1, 208)]
        m, _ = fixture(rows)
        p = m["plan"]
        self.assertEqual(len(p["codes"]), len(rows))
        self.assertEqual(sum(x["quantity"] for x in p["issues"]), len(rows))
        self.assertEqual(sum(x["reserved_units"] for x in p["campaigns"]), sum(x["quota"] for x in rows))
        self.assertTrue(all(1 <= x["quantity"] <= 100 for x in p["issues"]))
        self.assertTrue(all(x["spent_units"] == x["claims"] == 0 for x in p["campaigns"]))
        self.assertEqual(len({x["digest"] for x in p["codes"]}), len(rows))
        self.assertEqual(len({(x["issue_id"], x["position"]) for x in p["codes"]}), len(rows))

    def test_large_awards_split_campaigns_without_clamping_any_right(self):
        m, _ = fixture([source_row(i, quota=M.MAX_UNITS) for i in range(1, 4)])
        self.assertEqual(len(m["plan"]["campaigns"]), 3)
        self.assertEqual(m["source_facts"]["future_liability_units"], 3 * M.MAX_UNITS)
        for c in m["plan"]["campaigns"]:
            self.assertEqual(c["budget_units"], M.MAX_UNITS)
            self.assertEqual(c["reserved_units"], M.MAX_UNITS)
            self.assertEqual(c["max_claims_per_user"], 1)

    def test_more_than_user_limit_splits_without_restricting_total_redemption(self):
        m, _ = fixture([source_row(i, quota=1) for i in range(1, 10002)])
        self.assertEqual(sorted(c["max_claims_per_user"] for c in m["plan"]["campaigns"]), [1, 10000])
        self.assertEqual(sum(c["reserved_claims"] for c in m["plan"]["campaigns"]), 10001)

    def test_batch_name_and_archive_do_not_change_native_stable_identities(self):
        m, _ = fixture(migration="batch-one")
        other, _ = fixture(migration="batch-two")
        other["source_archive_sha256"] = "b" * 64
        self.assertEqual(m["plan"], other["plan"])
        self.assertNotEqual(M.plan_records(m["source_facts"], {**KT.NAMESPACE, "database": "another_source"}), m["plan"])

    def test_source_identity_cannot_reenter_under_another_snapshot_or_code(self):
        m, native = fixture()
        native["imports"] = [{"manifest": {"scope": M.SCOPE,
            "source_namespace_sha256": m["source_namespace_sha256"], "mapping": m["plan"]["records"]}}]
        new = M.source_facts([source_row(key="f" * 32)], m["identity_map"], 1000)
        with self.assertRaisesRegex(ValueError, "source_identity_already_imported"):
            M.check_conflicts(M.plan_records(new, KT.NAMESPACE), KT.NAMESPACE, native)

    def test_native_same_digest_collision_rejected_without_overwrite(self):
        m, native = fixture()
        native["core_promotions.codes"] = deepcopy(m["plan"]["codes"])
        with self.assertRaisesRegex(ValueError, "codes_collision"):
            M.check_conflicts(m["plan"], KT.NAMESPACE, native)

    def test_material_budget_expiry_identity_and_reservation_tamper_rejected(self):
        m, _ = fixture()
        for field, value in (("reserved_units", 0), ("max_claims_per_user", 0), ("ends_at", 1000)):
            bad = deepcopy(m)
            bad["plan"]["campaigns"][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                M.validate_material(bad)
        bad = deepcopy(m)
        bad["source_facts"]["records"][0]["native_ends_at"] -= 1
        with self.assertRaises(ValueError):
            M.validate_material(bad)

    def test_only_expected_promotion_rows_are_exempted_from_full_state_comparison(self):
        m, _ = fixture()
        # Constructing both real queries verifies the frozen shared hash-query
        # shape without mocking production guards. Actual preservation is
        # checked against live PostgreSQL by the separate regression driver.
        before = M.state_sql(m)
        after = M.state_sql(m, preserve=True)
        self.assertNotEqual(before, after)
        self.assertEqual(m["before_state"], m["snapshot"]["state"])
        self.assertEqual(m["before_state"]["sequences"], m["snapshot"]["state"]["sequences"])


if __name__ == "__main__":
    unittest.main()

"""Synthetic continuation inputs preserve native users and independent accounts."""
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import financial_import as F
import legacy_cutover_import as I
import legacy_cutover_keys as K
import legacy_cutover_money as M
import legacy_cutover_history as H


ARCHIVE = "a" * 64
OBSERVED = 1000


def identity_files(directory):
    users = [{"id": uid, "username": f"synthetic-{uid}", "password": "$2b$10$" + "." * 53,
              "email": "", "display_name": "Synthetic", "setting": "{}", "role": 1,
              "status": 1, "auth_version": 1, "deleted_at": None, "trust_level_override": 0,
              "console_activated_at": 0} for uid in (2, 3)]
    plan = F.IDENTITY.map_identity(users, [], source_sha256=ARCHIVE)
    raw = F.IDENTITY.id_map_bytes(plan["records"])
    manifest = {"format_version": 2, "scope": "identity-only", "source_archive_sha256": ARCHIVE,
                "mapping_sha256": plan["mapping_sha256"], "source_target_id_map_sha256": F.sha(raw.encode()),
                "identity_contract": F.IDENTITY.identity_contract()}
    map_path, manifest_path = directory / "map.jsonl", directory / "manifest.json"
    for path, text in ((map_path, raw), (manifest_path, F.canonical(manifest))):
        path.write_text(text)
        path.chmod(0o600)
    binding = I.load_identity_binding(map_path, manifest_path, ARCHIVE, [2, 3])
    return binding, map_path, manifest_path, manifest


def token(tid=40, uid=2):
    return {"id": tid, "user_id": uid, "key": str(tid) * 20, "deleted_at": None,
            "oauth_managed": False, "creation_source": "synthetic", "status": 1,
            "expired_time": -1, "name": "Synthetic", "group": "default", "auto_groups": "[]",
            "cross_group_retry": False, "model_limits": "", "model_limits_enabled": False,
            "allow_ips": "", "one_time_reveal": True, "used_quota": 4, "remain_quota": 6,
            "unlimited_quota": False, "created_time": 10, "accessed_time": 20,
            "account_balance_read": False}


def source_data(binding):
    return {"users": [{"id": uid, "role": 100 if uid == 2 else 1, "deleted_at": None,
                      "setting": "{}", "used_quota": 4, "quota": aid * 100,
                      "aff_quota": 0, "aff_history": 0, "request_count": 0}
                     for uid, aid in binding["accounts"].items()],
            "tokens": [token(), token(41, 3)], "options": {"UserUsableGroups": '{"default":1}'},
            "identity_binding": binding, "opening_binding": {"run": {
                "migration_id": "formal-opening", "mapping_sha256": "b" * 64,
                "source_archive_sha256": ARCHIVE, "observed_at": OBSERVED}}}


def subscription_data(binding):
    data = source_data(binding)
    data.update(subscription_plans=[{
        "id": 9, "title": "Synthetic", "duration_unit": "day", "duration_value": 30,
        "quota_reset_period": "never", "quota_reset_custom_seconds": 0,
        "price_amount": "2", "currency": "USD", "total_amount": 100,
        "max_purchase_per_user": 1, "enabled": True, "archived_at": 0,
        "updated_at": 10, "created_at": 10}], user_subscriptions=[{
            "id": 8, "user_id": 2, "plan_id": 9, "start_time": 10, "end_time": 2000,
            "amount_total": 100, "amount_used": 20, "status": "active",
            "reset_amount": None, "renewal_amount": None}], subscription_orders=[])
    return data


class MappingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="cutover-map-unit-", dir=F.IDENTITY.BACKUP_ROOT)
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.binding, self.map_path, self.manifest_path, self.manifest = identity_files(self.directory)

    def test_formal_map_handles_account_user_id_collision(self):
        self.assertEqual(self.binding["accounts"], {2: 1, 3: 2})
        self.assertEqual(K.personal_account({"identity_binding": self.binding}, 2), 1)
        with self.assertRaisesRegex(SystemExit, "mapping_missing"):
            K.personal_account({}, 2)
        with self.assertRaisesRegex(SystemExit, "mapping_missing"):
            K.personal_account({"identity_binding": self.binding}, 1)

    def test_missing_conflicting_stale_and_changed_user_maps_are_rejected(self):
        original = self.map_path.read_text()
        cases = [original.splitlines()[0] + "\n", original + original.splitlines()[0] + "\n"]
        rows = [json.loads(x) for x in original.splitlines()]
        rows[1]["target_personal_account_id"] = 1
        cases.append("".join(F.canonical(r) + "\n" for r in rows))
        rows = [json.loads(x) for x in original.splitlines()]
        rows[0]["target_user_id"] = 3
        cases.append("".join(F.canonical(r) + "\n" for r in rows))
        for raw in cases:
            self.map_path.write_text(raw)
            with self.subTest(raw=raw), self.assertRaises((F.Error, SystemExit)):
                I.load_identity_binding(self.map_path, self.manifest_path, ARCHIVE, [2, 3])
        self.map_path.write_text(original + "\n")
        with self.assertRaisesRegex(F.Error, "file_hash_mismatch"):
            I.load_identity_binding(self.map_path, self.manifest_path, ARCHIVE, [2, 3])
        self.map_path.write_text(original)
        with self.assertRaisesRegex(SystemExit, "coverage_mismatch"):
            I.load_identity_binding(self.map_path, self.manifest_path, ARCHIVE, [2])
        with self.assertRaisesRegex(F.Error, "source_mismatch"):
            I.load_identity_binding(self.map_path, self.manifest_path, "c" * 64, [2, 3])

    def test_keys_map_owner_funding_and_alias_but_keep_native_issuer(self):
        data = source_data(self.binding)
        args = SimpleNamespace(output=self.directory, observed_at=OBSERVED,
                               api_key_encryption_key_file=None, archive_sha256=ARCHIVE,
                               migration_id="keys")
        with patch.object(K, "target_info", return_value={"versions": {"2": 1, "3": 1},
                  "nonce": "d" * 64, "database": "synthetic", "database_oid": 10}):
            sql, _ = K.plan_keys(data, args, None)
        text = "\n".join(sql)
        self.assertIn("'api_key',2,1,1,", text)
        self.assertIn("VALUES(40,0,1)", text)
        self.assertIn("VALUES('key',40,2,'lifetime'", text)
        receipt = json.loads(K._pending["keys"][K.RECEIPT_FILE])
        self.assertEqual([(r["target_user_id"], r["target_account_id"]) for r in receipt["records"]], [(2, 1), (3, 2)])
        K._pending.pop("keys")

    def test_subscriptions_plans_pending_only_defers_all_grants(self):
        data = subscription_data(self.binding)
        full, full_stats = M.plan_subscriptions(data, ARCHIVE, "subs", OBSERVED)
        self.assertEqual((full, full_stats),
                         M.plan_subscriptions(data, ARCHIVE, "subs", OBSERVED, plans_pending_only=False))
        planned, stats = M.plan_subscriptions(data, ARCHIVE, "subs", OBSERVED, plans_pending_only=True)
        self.assertEqual(planned, [line for line in full if not line.startswith("INSERT INTO core_charging.")])
        self.assertEqual(stats["plans"], full_stats["plans"])
        self.assertEqual(stats["user_subscriptions_deferred_to_formal_opening"], 1)
        self.assertNotIn("user_subscriptions", stats)
        self.assertNotIn("core_charging.subscriptions", M.SUBSCRIPTION_PLANS_PENDING_GUARD)
        self.assertIn("legacy-sub:%", M.GUARDS["subscriptions"])

    def test_subscriptions_map_account_keep_plan_actor_user(self):
        sql, _ = M.plan_subscriptions(subscription_data(self.binding), ARCHIVE, "subs", OBSERVED)
        text = "\n".join(sql)
        self.assertIn("VALUES(1,'legacy-sub:subs:8'", text)
        self.assertIn("actor_user_id", text)
        self.assertIn(",2,10)", text)
        broken = subscription_data(self.binding)
        broken["identity_binding"] = {"accounts": {3: 2}}
        with self.assertRaisesRegex(SystemExit, "mapping_missing"):
            M.plan_subscriptions(broken, ARCHIVE, "subs", OBSERVED)

    def test_wallet_maps_payer_and_uses_committed_financial_run(self):
        data = source_data(self.binding)
        data.update(wallet_transfers=[{"id": 7, "sender_id": 2, "quota": 10, "status": "pending",
            "token": "f" * 64, "request_key": "synthetic_request_1", "created_at": 10}], top_ups_payable=[])
        sql, _ = M.plan_wallet(data, ARCHIVE, "wallet-phase", OBSERVED)
        text = "\n".join(sql)
        self.assertIn("VALUES(7,2,1,", text)
        self.assertIn("owner_account_id=1 AND bucket='wallet'", text)
        self.assertIn("'formal-opening'", text)
        self.assertIn('"actor_user_id":2', text)
        self.assertIn('"account_id":1', text)
        self.assertIn("b" * 64, text)

    def test_escrow_maps_owner_recipient_fee_without_changing_source_users(self):
        data = source_data(self.binding)
        data.update(bounty_projects=[{"id": 7, "owner_user_id": 2, "escrow_quota": 50,
                                      "status": "published"}], bounty_disputes=[],
                    bounty_challenges=[{"id": 8, "project_id": 7, "participant_user_id": 3,
                        "paid_at": 0, "status": "accepted", "rejected_at": 0, "reward_quota": 20}])
        sql, _ = M.plan_bounties(data, ARCHIVE, "bounty-phase", OBSERVED)
        text = "\n".join(sql)
        for fragment in ('"owner_user_id":2', '"payer_account_id":1',
                         '"participant_user_id":3', '"recipient_account_id":2',
                         '"migration_id":"formal-opening"', "VALUES(TRUE,0,1)"):
            self.assertIn(fragment, text)

    def test_history_refuses_wrong_native_map_and_cross_owned_key_before_source_read(self):
        args = SimpleNamespace(source_schema="synthetic", archive_sha256=ARCHIVE,
                               identity_binding=self.binding)
        owners = [{"id": 2, "personal_account_id": 1}, {"id": 3, "personal_account_id": 2}]
        with patch.object(H, "query", side_effect=[owners, [{"id": 40, "user_id": 2, "owner_account_id": 2}]]):
            with self.assertRaisesRegex(SystemExit, "credential_account_map_mismatch"):
                H.plan_history(args, object(), object())
        with patch.object(H, "query", return_value=[{"id": 2, "personal_account_id": 2}]):
            with self.assertRaisesRegex(SystemExit, "identity_account_map_mismatch"):
                H.plan_history(args, object(), object())


if __name__ == "__main__":
    unittest.main()

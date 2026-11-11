"""Opt-in real PG evidence for synthetic UTC subscription migration only.

Uses the existing owned, network-disabled Docker fixture. No source connection,
real credentials, application/worker or existing database is accepted.
"""
import copy
import json
import os
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import financial_import as F
import legacy_subscription_opening as S
import legacy_pancake_subscriptions as W
import test_financial_import_pg as P
from test_legacy_subscription_opening import fixture


@unittest.skipUnless(os.environ.get("LMM_FINANCIAL_PG_TEST") == "1", "isolated PostgreSQL test not enabled")
class SubscriptionPostgreSQLTests(P.PostgreSQLTests):
    def setUp(self):
        super().setUp()
        changes = {}
        if self._testMethodName in ("test_sub_expired_overflow_preserves_policy_without_spendable_grants",
                                   "test_sub_originals_archive_policy_without_prices_or_orders"):
            changes = {"period": "never", "status": "expired", "allow_wallet_overflow": True,
                       "source_timezone": None, "reset_amount": None, "renewal_amount": None}
        candidate, review = fixture(**changes)
        snapshot = json.loads(self.sql(F.snapshot_sql(self.inventory, "synthetic-subscription")).stdout)
        candidate["before_state"] = snapshot["state"]
        candidate["target"].update({key:snapshot[key] for key in
            ("instance","catalog_fingerprint_sha256","ownership_sha256","component_fingerprints","trust_policy_seed")})
        candidate["native_users"] = snapshot["native_users"]
        candidate["identity_import"] = snapshot["import_runs"][0]
        self.sub = S.prepare_material(candidate, review, migration_id="synthetic-subscription")
        self.sql_batch = S.build_apply_sql(self.sub)

    def test_sub_expired_overflow_preserves_policy_without_spendable_grants(self):
        before = self.full_state()
        self.sql(self.sql_batch)
        after = self.full_state()
        for table, state in before["tables"].items():
            if table not in (*S.CHANGED_TABLES, "core_migration.import_runs"):
                self.assertEqual(after["tables"][table], state, table)
        cutoff = self.sub["financial_attestation"]["observation"]["observed_at"]
        result = json.loads(self.sql("SELECT json_build_object("
            "'active',(SELECT active FROM core_charging.subscriptions),"
            "'used',(SELECT consumed_credits FROM core_charging.subscription_opening_consumption),"
            "'windows',(SELECT count(*) FROM core_charging.subscriptions s CROSS JOIN LATERAL "
            f"core_charging.subscription_window(s.id,{cutoff}) w),"
            "'source_overflow',(SELECT manifest->'material'->'financial_attestation'->'source_plan'"
            "->'subscription_entitlements'->0->'allow_wallet_overflow' FROM core_migration.import_runs "
            "WHERE migration_id='synthetic-subscription'),"
            "'source_timezone',(SELECT manifest->'material'->'financial_attestation'->'source_plan'"
            "->'source_timezone' FROM core_migration.import_runs WHERE migration_id='synthetic-subscription'))").stdout)
        self.assertEqual(result, {"active": False, "used": 400, "windows": 0,
                                  "source_overflow": True, "source_timezone": None})
        self.sql(self.sql_batch)
        self.assertEqual(after, self.full_state())

    def test_sub_originals_archive_policy_without_prices_or_orders(self):
        self.sql(self.sql_batch)
        attestation = copy.deepcopy(self.sub["financial_attestation"])
        snapshot = json.loads(self.sql(F.snapshot_sql(self.inventory, "synthetic-originals")).stdout)
        attestation["before_state"] = snapshot["state"]
        entitlement = attestation["source_plan"]["subscription_entitlements"][0]
        sid = entitlement["source_subscription_id"]
        reference = "legacy-sub:synthetic-subscription:" + str(sid)
        row = {"id": 101, "user_id": entitlement["source_user_id"],
            "plan_id": entitlement["source_plan_id"], "user_subscription_id": sid,
            "trade_no": "WAFFO_PANCAKE_SUB-synthetic-history", "status": "success",
            "payment_provider": "waffo_pancake", "expected_amount_micros": 1230000,
            "settlement_currency": "USD", "provider_product_id": "synthetic-product",
            "provider_store_id": "synthetic-store", "provider_subscription_id": "",
            "created_at": entitlement["start_time"], "plan_snapshot": {
                "allow_wallet_overflow": True, "waffo_pancake_product_type": "one_time"}}
        binding = {"source_order_id": row["id"], "source_row_sha256": F.digest(row),
            "account_id": entitlement["account_id"], "snapshot_field": "plan_snapshot",
            "disposition": "retain-unknown", "projection": "historical-only",
            "opening_grant_reference": reference, "last_paid_period_end": entitlement["end_time"]}
        pending = {**row, "id": 102, "trade_no": "WAFFO_PANCAKE_SUB-synthetic-pending",
                   "status": "pending", "user_subscription_id": 0}
        missing = {**pending, "id": 103, "trade_no": "WAFFO_PANCAKE_SUB-synthetic-missing",
                   "plan_snapshot": ""}
        archive = attestation["source_archive_sha256"]
        source = {"source_archive_sha256": archive, "orders": [row, pending, missing]}
        review = {"source_archive_sha256": archive, "decision": "approve-original-subscription-bindings",
            "orders": [binding, *[{"source_order_id": value["id"], "source_row_sha256": F.digest(value),
                "account_id": entitlement["account_id"], "snapshot_field": "plan_snapshot",
                "disposition": "retain-unknown"} for value in (pending, missing)]]}
        material = W.prepare_material(attestation, source, review, migration_id="synthetic-originals")
        sql = W.build_apply_sql(material)
        self.rejected_unchanged(sql.replace(reference, "legacy-sub:missing:" + str(sid)),
                                "original_opening_receipt_mismatch")
        before = self.full_state()
        self.sql(sql)
        after = self.full_state()
        for table, state in before["tables"].items():
            if table not in ("core_subscriptions.commands", "core_migration.import_runs"):
                self.assertEqual(after["tables"][table], state, table)
        records = self.query("SELECT json_agg(json_build_array(result->>'state',result->>'source_status',"
            "result->>'projection') ORDER BY (result->>'source_order_id')::bigint) "
            "FROM core_subscriptions.commands WHERE request->>'kind'='legacy_subscription_unknown_v1'")
        self.assertEqual(records, [["UNKNOWN", "success", "historical-only"],
                                  ["UNKNOWN", "pending", None], ["UNKNOWN", "pending", None]])
        self.sql(sql)
        self.assertEqual(after, self.full_state())

    def test_sub_exact_reset_usage_and_strict_replay(self):
        before = self.full_state()
        result=json.loads(self.sql(self.sql_batch).stdout)
        self.assertEqual(result["status"],"COMMITTED")
        after=self.full_state()
        self.assertEqual(json.loads(self.sql(self.sql_batch).stdout),result)
        self.assertEqual(after,self.full_state())
        for table in before["tables"]:
            if table not in set(S.CHANGED_TABLES)|{"core_migration.import_runs"}:
                self.assertEqual(before["tables"][table],after["tables"][table],table)
        actual=self.query("SELECT jsonb_build_object('current',s.limit_credits,'used',o.consumed_credits,'future',s.reset_limit_credits,'models',s.models,'groups',s.groups) FROM core_charging.subscriptions s JOIN core_charging.subscription_opening_consumption o ON o.subscription_id=s.id")
        self.assertEqual(actual,{"current":1000,"used":400,"future":2000,
            "models":["synthetic-model"],"groups":["synthetic-group"]})
        for at,limit in ((1706745600,1000),(1709251200,2000)):
            actual=self.query(f"SELECT jsonb_build_object('limit',limit_credits) FROM core_charging.subscription_window(1,{at})")
            self.assertEqual(actual,{"limit":limit})

    def test_sub_conflicts_and_post_application_mutation_refuse_replay(self):
        self.sql(self.sql_batch)
        changed=copy.deepcopy(self.sub)
        changed["migration_id"]="another-subscription"
        self.rejected_unchanged(S.build_apply_sql(changed),"subscription_target_state_changed")
        self.sql("BEGIN ISOLATION LEVEL SERIALIZABLE; UPDATE core_charging.subscriptions SET active=false; COMMIT;")
        self.rejected_unchanged(self.sql_batch,"subscription_replay_conflict")

    def test_sub_partial_failure_rolls_back_tables_and_sequence(self):
        bad=self.sql_batch.replace("  SET CONSTRAINTS ALL IMMEDIATE;", "  PERFORM 1/0;\n  SET CONSTRAINTS ALL IMMEDIATE;")
        self.rejected_unchanged(bad,"division by zero")
        self.assertEqual(self.query("SELECT count(*)::int FROM core_charging.subscriptions"),0)

    def test_sub_unknown_ack_resolution_requires_an_exact_receipt(self):
        name="lmm-subscription-"+"a"*32
        resolver=S.build_resolve_sql(self.sub,name)
        before=self.full_state()
        self.assertEqual(json.loads(self.sql(resolver).stdout)["status"],"UNKNOWN")
        self.assertEqual(before,self.full_state())
        self.sql(self.sql_batch)
        committed=self.full_state()
        self.assertEqual(json.loads(self.sql(resolver).stdout)["status"],"COMMITTED")
        self.assertEqual(committed,self.full_state())
        self.sql("BEGIN ISOLATION LEVEL SERIALIZABLE; UPDATE core_charging.subscriptions SET active=false; COMMIT;")
        self.rejected_unchanged(resolver,"subscription_resolution_conflict")

    def test_sub_bad_target_and_calendar_boundary_are_atomic(self):
        bad=copy.deepcopy(self.sub)
        bad["financial_attestation"]["target"]["instance"]["server_instance_nonce"]="wrong"
        self.rejected_unchanged(S.build_apply_sql(bad),"financial_target_instance_mismatch")
        # Tamper SQL after generation to exercise the actual DB calendar trigger.
        bad=self.sql_batch.replace("(p->>'opening_end')::bigint,", "(p->>'opening_end')::bigint-1,",1)
        self.rejected_unchanged(bad,"opening subscription reset mismatch")


def load_tests(loader, tests, pattern):
    # The shared parent owns the existing wallet tests. Run only new subscription
    # cases here; do not count inherited tests twice or run two extra fixtures.
    return unittest.TestSuite(SubscriptionPostgreSQLTests(name) for name in
        loader.getTestCaseNames(SubscriptionPostgreSQLTests) if name.startswith("test_sub_"))


if __name__ == "__main__":
    unittest.main()

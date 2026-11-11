"""Opt-in local synthetic PG coverage of mapped continuation transactions.

The target uses the current assembled schema fixture. Source payloads are
synthetic, with an account/user numeric collision. No production connection,
binary build, economic approval file or real-money operation is involved.
"""
import contextlib
import io
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import financial_import as F
import legacy_cutover_import as I
import legacy_cutover_keys as K
import legacy_cutover_money as M
from test_financial_import_pg import reviewed_fixture_ddl
from test_legacy_cutover_mapping import ARCHIVE, OBSERVED, identity_files, source_data, subscription_data


@unittest.skipUnless(os.environ.get("LMM_CUTOVER_MAPPING_PG") == "1", "local synthetic PG opt-in")
class MappingPostgreSQLTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = Path(tempfile.mkdtemp(prefix="cutover-map-pg-", dir=F.IDENTITY.BACKUP_ROOT))
        cls.socket = cls.root / "socket"
        cls.socket.mkdir(mode=0o700)
        cls.port = random.randrange(20000, 50000)
        cls.datadir = cls.root / "pgdata"
        cls.commands = cls.root / "fixture-commands.log"
        cls.environment = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        cls.environment.update(PGHOST=str(cls.socket), PGPORT=str(cls.port), PGDATABASE="postgres")
        cls.run_command(["initdb", "-D", str(cls.datadir), "-A", "trust", "--no-locale", "-E", "UTF8"])
        cls.run_command(["pg_ctl", "-D", str(cls.datadir), "-l", str(cls.root / "postgres.log"),
                         "-w", "start", "-o", f"-k {cls.socket} -h '' -p {cls.port} -c max_connections=12"])
        cls.addClassCleanup(cls.stop)
        print("MAPPING_PG_EVIDENCE=" + str(cls.root), flush=True)
        cls.binding, cls.map_path, cls.manifest_path, cls.manifest = identity_files(cls.root)
        cls.psql("CREATE ROLE cutover_owner NOLOGIN; CREATE ROLE cutover_installer LOGIN; "
                 "GRANT cutover_owner TO cutover_installer; CREATE ROLE cutover_runtime LOGIN;", admin=True)
        cls.psql("CREATE DATABASE cutover_template OWNER cutover_owner", admin=True)
        cls.psql(reviewed_fixture_ddl(F.IDENTITY.current_contract()), database="cutover_template")
        sql = []
        for uid, aid in cls.binding["accounts"].items():
            sql.append("INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE "
                       f"VALUES({aid},'personal'); INSERT INTO core_identity.users(id,personal_account_id) "
                       f"VALUES({uid},{aid});")
        sql.append("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) "
                   f"VALUES('synthetic-identity',{I.lit(ARCHIVE)},{I.lit(cls.manifest['mapping_sha256'])},"
                   f"{I.jlit(cls.manifest)});")
        sql.append("BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;")
        # Exercise actual opening functions with a distinct reviewed-plan hash.
        # These are synthetic fixture postings, not an import approval.
        for uid, aid in cls.binding["accounts"].items():
            base = {"migration_id": "formal-opening", "source_entity_key": f"legacy.users/{uid}",
                    "source_archive_sha256": ARCHIVE, "mapping_sha256": "b" * 64,
                    "source_row_sha256": "c" * 64, "observed_at": OBSERVED, "account_id": aid}
            sql.append("SELECT core_billing.import_opening_balance(" + I.jlit({**base,
                "available_units": aid * 100, "reserved_units": 0, "affiliate_units": 0}) + ");")
            sql.append("SELECT core_billing.import_account_opening_counters(" + I.jlit({**base,
                "user_id": uid, "used_quota": 4, "request_count": 0, "aff_history": 0}) + ");")
        sql.append("COMMIT;")
        cls.psql("\n".join(sql), database="cutover_template")
        cls.serial = 0

    @classmethod
    def run_command(cls, command):
        result = subprocess.run(command, env=cls.environment, text=True, capture_output=True)
        with cls.commands.open("a") as log:
            log.write(result.stdout + result.stderr)
        if result.returncode:
            raise AssertionError(result.stderr[-3000:])
        return result

    @classmethod
    def stop(cls):
        cls.run_command(["pg_ctl", "-D", str(cls.datadir), "-m", "fast", "-w", "stop"])

    @classmethod
    def psql(cls, sql, *, database="postgres", admin=False, check=True):
        env = {**cls.environment, "PGDATABASE": database}
        if not admin:
            env.update(PGUSER="cutover_installer", PGOPTIONS="-c role=cutover_owner")
        result = subprocess.run(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
                                input=sql, env=env, text=True, capture_output=True)
        with cls.commands.open("a") as log:
            log.write(result.stdout + result.stderr)
        if check and result.returncode:
            raise AssertionError(result.stderr[-3000:])
        return result

    def setUp(self):
        type(self).serial += 1
        self.database = "cutover_case_" + str(self.serial)
        self.psql(f"CREATE DATABASE {self.database} TEMPLATE cutover_template OWNER cutover_owner", admin=True)
        self.psql(f"ALTER DATABASE {self.database} SET lmm.migration_instance_nonce='{ 'd' * 64}'", admin=True)
        self.addCleanup(self.psql, f"DROP DATABASE {self.database}", admin=True)
        self.target = I.Db(self.socket, self.port, self.database, "cutover_installer",
                           role="cutover_owner", readonly=True)

    def query(self, sql):
        return json.loads(self.psql(sql, database=self.database).stdout)

    def state(self):
        return self.query("SELECT json_build_object('credentials',(SELECT coalesce(json_agg(to_jsonb(c)),'[]') "
            "FROM core_identity.credentials c),'budgets',(SELECT coalesce(json_agg(to_jsonb(b)),'[]') "
            "FROM core_charging.budgets b),'funding',(SELECT coalesce(json_agg(to_jsonb(f)),'[]') "
            "FROM core_identity.key_funding_rules f),'subscriptions',(SELECT coalesce(json_agg(to_jsonb(s)),'[]') "
            "FROM core_charging.subscriptions s),'balances',(SELECT json_agg(to_jsonb(b) ORDER BY ledger_account_id) "
            "FROM core_billing.balance_state b),'seq',(SELECT json_build_object('last_value',last_value,"
            "'is_called',is_called) FROM core_identity.credentials_id_seq))")

    def invoke(self, phase, data, *, apply=True, extra=()):
        out = self.root / (self.database + "-" + phase + "-" + str(len(list(self.root.glob(self.database + '-*')))))
        argv = ["legacy_cutover_import.py", "--socket", str(self.socket), "--port", str(self.port),
                "--source-db", "synthetic-source", "--source-user", "synthetic-reader",
                "--source-schema", "lmm_prod_20260802", "--target-db", self.database,
                "--target-login", "cutover_installer", "--target-owner", "cutover_owner",
                "--archive-sha256", ARCHIVE, "--observed-at", str(OBSERVED), "--migration-id", phase + "-phase",
                "--identity-map", str(self.map_path), "--identity-manifest", str(self.manifest_path),
                "--phase", phase, "--output", str(out), *extra]
        if apply:
            argv.append("--apply")
        # Only the source-reader boundary is substituted by synthetic payloads.
        # Argument parsing, target reads, guards, generated SQL and apply are real.
        with patch.object(sys, "argv", argv), patch.object(I, "load_source", return_value=data), \
             patch.object(M, "load_source", return_value=data), contextlib.redirect_stdout(io.StringIO()):
            I.main()
        return out

    def test_normal_keys_entry_reads_back_mapped_owner_payer_and_rejects_replay(self):
        before = self.state()
        out = self.invoke("keys", source_data(self.binding))
        receipt = json.loads((out / K.RECEIPT_FILE).read_text())
        self.assertEqual([(r["target_user_id"], r["target_account_id"]) for r in receipt["records"]], [(2, 1), (3, 2)])
        report = json.loads((out / "keys-report.json").read_text())
        self.assertEqual(report["reconciliation"]["mismatches"], {})
        rows = self.query("SELECT json_agg(json_build_array(c.user_id,c.owner_account_id,u.personal_account_id,"
            "f.payer_account_id) ORDER BY c.id) FROM core_identity.credentials c JOIN core_identity.users u "
            "ON u.id=c.user_id JOIN core_identity.key_funding_rules f ON f.credential_id=c.id")
        self.assertEqual(rows, [[2, 1, 1, 1], [3, 2, 2, 2]])
        after = self.state()
        self.assertEqual(before["balances"], after["balances"])
        with self.assertRaisesRegex(SystemExit, "psql_failed"):
            self.invoke("keys", source_data(self.binding))
        self.assertEqual(after, self.state())

    def test_subscriptions_entry_maps_actual_account_and_keeps_opening_unchanged(self):
        before = self.state()
        self.invoke("subscriptions", subscription_data(self.binding))
        rows = self.query("SELECT json_agg(json_build_array(account_id,limit_credits,"
            "(SELECT consumed_credits FROM core_charging.subscription_opening_consumption o "
            "WHERE o.subscription_id=s.id))) FROM core_charging.subscriptions s")
        self.assertEqual(rows, [[1, 100, 20]])
        self.assertEqual(before["balances"], self.state()["balances"])
        after = self.state()
        with self.assertRaisesRegex(SystemExit, "psql_failed"):
            self.invoke("subscriptions", subscription_data(self.binding))
        self.assertEqual(after, self.state())

    def test_subscriptions_plans_pending_only_preserves_existing_formal_grants(self):
        data = subscription_data(self.binding)
        data["subscription_plans"][0]["allow_balance_pay"] = False
        # Existing grants are outside the new mode's ownership. Formal writer
        # receipt generation remains covered by the subscription-opening PG suite.
        grants, _ = M.plan_subscriptions(data, ARCHIVE, "formal-subscription", OBSERVED)
        self.psql("BEGIN ISOLATION LEVEL SERIALIZABLE;\n"
                  + "\n".join(line for line in grants if line.startswith("INSERT INTO core_charging."))
                  + "\nCOMMIT;",
                  database=self.database)
        before = self.state()
        data["subscription_orders"] = [{"id": 31, "user_id": 2, "plan_id": 9,
            "status": "pending", "trade_no": "synthetic_pending_subscription_31",
            "money": "2", "expected_amount_micros": 2000000,
            "settlement_currency": "USD", "plan_currency": "USD"}]
        out = self.invoke("subscriptions", data, extra=("--subscriptions-plans-pending-only",))
        self.assertIs(self.query("SELECT terms->'allow_balance_pay' FROM core_subscriptions.plan_revisions"), False)
        after = self.state()
        self.assertEqual(before["subscriptions"], after["subscriptions"])
        self.assertEqual(before["balances"], after["balances"])
        self.assertEqual(self.query("SELECT json_build_object('plans',(SELECT count(*) FROM core_subscriptions.plans),"
            "'usage',(SELECT count(*) FROM core_charging.subscription_opening_consumption),"
            "'pending',(SELECT count(*) FROM core_billing.legacy_pending_liabilities WHERE source_table='subscription_orders'))"),
            {"plans": 1, "usage": 1, "pending": 1})
        report = json.loads((out / "subscriptions-report.json").read_text())
        self.assertEqual(report["subscription_grants"], "deferred-to-formal-subscription-opening")
        with self.assertRaisesRegex(SystemExit, "psql_failed"):
            self.invoke("subscriptions", data, extra=("--subscriptions-plans-pending-only",))
        self.assertEqual(after, self.state())

    def test_native_map_change_after_planning_rolls_back_prior_write_and_sequence(self):
        I.verify_identity_binding(self.target, self.binding)
        guard = I.identity_guard_sql(self.binding)
        # Existing users cannot be reassigned by Core's own immutable identity
        # guard. A concurrently added valid user still changes the bound cohort.
        self.psql("INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES(4,'personal'); "
                  "INSERT INTO core_identity.users(id,personal_account_id) VALUES(9,4)", database=self.database)
        before = self.state()
        result = self.psql("BEGIN ISOLATION LEVEL SERIALIZABLE; "
            "INSERT INTO core_console.options(key,value) VALUES('synthetic-rollback','x'); " + guard + " COMMIT;",
            database=self.database, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("target_identity_account_map_mismatch", result.stderr)
        self.assertEqual(self.query("SELECT count(*)::int FROM core_console.options"), 0)
        self.assertEqual(before, self.state())

    def test_pending_wallet_transfer_uses_mapped_payer_and_preserves_available_balance(self):
        data = source_data(self.binding)
        data.update(wallet_transfers=[{"id": 7, "sender_id": 2, "quota": 10, "status": "pending",
            "token": "f" * 64, "request_key": "synthetic_request_1", "created_at": 10}], top_ups_payable=[])
        self.invoke("wallet", data)
        self.assertEqual(self.query("SELECT json_build_array(owner_user,payer_account,state) "
                                   "FROM core_wallet.transfers WHERE id=7"), [2, 1, "pending"])
        self.assertEqual(self.query("SELECT balance_units::int FROM core_billing.balance_state s "
            "JOIN core_billing.ledger_accounts a ON a.id=s.ledger_account_id "
            "WHERE a.owner_account_id=1 AND a.bucket='wallet'"), 100)

    def test_history_entry_keeps_user_key_and_mapped_account_together_without_posting_usage(self):
        data = source_data(self.binding)
        self.invoke("keys", data)
        before = self.state()
        logs = [{"id": i, "user_id": uid, "token_id": tid, "created_at": 20,
                 "type": 2, "quota": 4} for i, uid, tid in ((1, 2, 40), (2, 3, 41))]

        def synthetic_read(sql):
            if "SELECT row_to_json(l)::text FROM logs" in sql:
                return "\n".join(json.dumps(row) for row in logs)
            if "FROM users ORDER BY id" in sql:
                return json.dumps(data["users"])
            if "FROM tokens ORDER BY id" in sql:
                return json.dumps(data["tokens"])
            if "count(*) AS n FROM perf_metrics" in sql:
                return '[{"n":0}]'
            if any(table in sql for table in ("FROM wallet_credit_rebases", "FROM violation_fee_records",
                                              "FROM quota_data", "FROM subscription_orders", "FROM top_ups")):
                return "[]"
            raise AssertionError("unhandled synthetic history query")

        original_run = I.Db.run

        def run(db, sql):
            if db.env["PGDATABASE"] == "synthetic-source":
                return synthetic_read(sql)
            return original_run(db, sql)

        with patch.object(I.Db, "run", new=run):
            self.invoke("history", data)
        self.assertEqual(self.query("SELECT json_agg(json_build_array(user_id,owner_account_id,token_id,"
            "pricing_credits_per_usd) ORDER BY source_log_id) FROM core_history.log_records"),
            [[2, 1, 40, None], [3, 2, 41, None]])
        self.assertEqual(self.query("SELECT count(*)::int FROM core_history.scope_receipts"), 4)
        self.assertEqual(self.query("SELECT count(*)::int FROM core_charging.requests"), 0)
        self.assertEqual(before, self.state())

    def test_foreign_manifest_and_missing_map_never_write(self):
        before = self.state()
        manifest = json.loads(self.manifest_path.read_text())
        self.manifest_path.write_text(F.canonical({**manifest, "mapping_sha256": "e" * 64}))
        try:
            with self.assertRaisesRegex(SystemExit, "target_identity_import_run_mismatch"):
                self.invoke("keys", source_data(self.binding))
        finally:
            self.manifest_path.write_text(F.canonical(manifest))
        self.assertEqual(before, self.state())
        raw = self.map_path.read_bytes()
        self.map_path.unlink()
        try:
            with self.assertRaises((F.Error, FileNotFoundError)):
                self.invoke("keys", source_data(self.binding))
        finally:
            self.map_path.write_bytes(raw)
            self.map_path.chmod(0o600)
        self.assertEqual(before, self.state())

    def test_opening_binding_uses_actual_run_and_rejects_changed_binding(self):
        data = source_data(self.binding)
        M.bind_opening(data, self.target, ARCHIVE, OBSERVED)
        self.assertEqual(M.opening_mapping(data), "b" * 64)
        self.assertNotEqual(M.opening_mapping(data), self.manifest["mapping_sha256"])
        self.psql("BEGIN ISOLATION LEVEL SERIALIZABLE; " + I.identity_guard_sql(self.binding) +
                  M.opening_guard_sql(data) + " COMMIT;", database=self.database)
        data["opening_binding"]["run"]["mapping_sha256"] = "f" * 64
        before = self.state()
        result = self.psql("BEGIN ISOLATION LEVEL SERIALIZABLE; " + M.opening_guard_sql(data) + " COMMIT;",
                           database=self.database, check=False)
        self.assertIn("committed_opening_binding_changed", result.stderr)
        self.assertEqual(before, self.state())


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Execute the site's actual credit projection under restricted PG privileges."""
import copy
import json
import os
from pathlib import Path
import re
import unittest

import legacy_cutover_history as H
import test_legacy_history_refresh_pg as PG


@unittest.skipUnless(os.environ.get("LMM_HISTORY_REFRESH_PG_TEST") == "1",
                     "isolated PostgreSQL test not enabled")
class SiteFinanceHistoryIdentityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        PG.PostgreSQLRefreshTests.setUpClass()
        cls.pg = PG.PostgreSQLRefreshTests
        cls.pg.sql("CREATE ROLE history_finance_runtime NOLOGIN;", "postgres")
        source = (Path(__file__).resolve().parent.parent /
                  "services/core/src/history_read/site_statistics.rs").read_text()
        cls.history_only = (Path(__file__).resolve().parent.parent /
                            "services/core/src/history_read/history_only_identities.sql").read_text()
        cls.query = cls.history_only + re.search(r'r#"(, users AS .*?)"#,',
                                                 source, re.DOTALL).group(1)
        cls.query = cls.query.replace(
            "SELECT floor(extract(epoch FROM statement_timestamp()))::bigint AS as_of,",
            "SELECT (SELECT count(*) FROM history_only) AS excluded,"
            " floor(extract(epoch FROM statement_timestamp()))::bigint AS as_of,", 1)

    @classmethod
    def tearDownClass(cls):
        cls.pg.tearDownClass()

    def setUp(self):
        self.pg.sql("DROP DATABASE IF EXISTS synthetic_history;", "postgres")
        self.pg.sql("CREATE DATABASE synthetic_history;", "postgres")
        self.pg.sql("""
CREATE SCHEMA core_identity;
CREATE TABLE core_identity.accounts(id bigint PRIMARY KEY,kind text,service_level int,active bool);
CREATE TABLE core_identity.users(id bigint PRIMARY KEY,personal_account_id bigint,platform_role text,active bool,auth_version bigint);
CREATE TABLE core_identity.user_profiles(user_id bigint PRIMARY KEY,password_login_allowed bool,email_login_allowed bool,legacy_bcrypt_allowed bool,second_factor_required bool,email text);
CREATE TABLE core_identity.credentials(id bigint PRIMARY KEY,user_id bigint,owner_account_id bigint);
CREATE TABLE core_identity.password_logins(user_id bigint);
CREATE TABLE core_identity.oauth_identities(user_id bigint);
CREATE TABLE core_identity.web_sessions(user_id bigint);
CREATE TABLE core_identity.mfa_factors(user_id bigint);
CREATE TABLE core_identity.email_addresses(user_id bigint);
CREATE TABLE core_identity.audit(actor_user_id bigint,action text,target jsonb);
CREATE SCHEMA core_abuse;
CREATE TABLE core_abuse.registration_observations(user_id bigint PRIMARY KEY,source_ip inet);
CREATE SCHEMA core_passkeys;
CREATE TABLE core_passkeys.credentials(user_id bigint);
CREATE SCHEMA core_billing;
CREATE TABLE core_billing.account_opening_counters(account_id bigint,user_id bigint,used_quota bigint,
  migration_id text,source_entity_key text,source_row_sha256 text,source_archive_sha256 text,mapping_sha256 text,observed_at bigint);
CREATE TABLE core_billing.opening_imports(account_id bigint,migration_id text,source_entity_key text,source_row_sha256 text,request jsonb);
CREATE TABLE core_billing.opening_import_runs(migration_id text,source_archive_sha256 text,mapping_sha256 text,observed_at bigint);
CREATE TABLE core_billing.ledger_accounts(id bigint PRIMARY KEY,owner_account_id bigint,bucket text);
CREATE TABLE core_billing.balance_state(ledger_account_id bigint,balance_units bigint);
CREATE SCHEMA core_charging;
CREATE TABLE core_charging.requests(actor_user_id bigint,owner_account_id bigint,payer_account_id bigint,state text,settled bigint,refunded bigint);
CREATE SCHEMA core_payments;
CREATE TABLE core_payments.intents(account_id bigint,created_by_user_id bigint);
CREATE SCHEMA core_migration;
CREATE TABLE core_migration.import_runs(migration_id text,source_archive_sha256 text,mapping_sha256 text,manifest jsonb);
""")
        repo = Path(__file__).resolve().parent.parent
        self.pg.sql((repo / "services/core/schema/history.sql").read_text())
        self.pg.sql("""
CREATE TABLE core_history.finance_users(user_id bigint,account_id bigint,normalized_used_credits bigint);
INSERT INTO core_identity.accounts VALUES(1,'personal',0,true),(2,'personal',0,false);
INSERT INTO core_identity.users VALUES(71,1,'user',true,1),(73,2,'user',false,1);
INSERT INTO core_identity.user_profiles VALUES(71,true,false,true,false,''),(73,false,false,false,false,'');
INSERT INTO core_identity.audit VALUES(71,'user.register','{"user_id":71}');
INSERT INTO core_billing.ledger_accounts VALUES(1,1,'wallet'),(2,1,'reserved'),(3,1,'debt'),(4,1,'affiliate');
INSERT INTO core_billing.balance_state VALUES(1,100),(2,9),(3,-5),(4,13);
INSERT INTO core_charging.requests VALUES(71,1,1,'settled',40,5);
INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete)
VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),'legacy.logs:legacy_source',1700000040,true);
INSERT INTO core_history.scope_receipts VALUES(1,73,2,NULL,repeat('a',64),repeat('b',64),repeat('c',64),true);
GRANT USAGE ON SCHEMA core_identity,core_passkeys,core_billing,core_charging,core_payments,core_history,core_migration TO history_finance_runtime;
GRANT SELECT ON ALL TABLES IN SCHEMA core_identity,core_passkeys,core_billing,core_charging,core_payments,core_history TO history_finance_runtime;
GRANT USAGE ON SCHEMA core_abuse TO history_finance_runtime;
GRANT SELECT(user_id) ON core_abuse.registration_observations TO history_finance_runtime;
GRANT SELECT(migration_id,manifest) ON core_migration.import_runs TO history_finance_runtime;
""")
        oid = int(self.pg.sql("SELECT oid FROM pg_database WHERE datname=current_database()").stdout)
        mapping = [{"source_user_id": 73, "target_user_id": 73, "target_personal_account_id": 2}]
        self.manifest = {
            "format_version": 1, "scope": "history-only-identity-supplement.v1",
            "mapping": mapping, "mapping_sha256": H.sha(mapping),
            "source_snapshot": {"sha256": "a" * 64, "metadata": {
                "schema": "legacy_source", "observed_at": 1700000040}},
            "source_user_facts": [{"facts": {"id": 73}, "source_record_sha256": "d" * 64}],
            "target": {"instance": {"database": "synthetic_history", "database_oid": oid},
                       "before_identity_state": {"users": {"max_id": 71}, "accounts": {"max_id": 1}}},
            "target_policy": {"accounts_active": False, "users_active": False,
                              "platform_role": "user", "auth_version": 1,
                              "credentials_imported": False, "wallet_opening_imported": False},
        }

    def read(self, manifest=None, change=""):
        receipt = ""
        if manifest is not None:
            receipt = ("INSERT INTO core_migration.import_runs VALUES('synthetic-history-only',"
                       "repeat('a',64),repeat('b',64)," + H.jlit(manifest) + ");")
        # Fixture writes precede SET ROLE. The projection itself runs with exactly
        # the runtime's two readable import-receipt columns and no write grant.
        result = self.pg.sql("BEGIN;" + receipt + change + "SET LOCAL ROLE history_finance_runtime;"
                             "SELECT row_to_json(q) FROM (" + self.query + ") q;ROLLBACK;")
        return json.loads(result.stdout)

    def test_admin_origin_is_page_scoped_and_returns_only_attested_ids(self):
        # Fifteen bound placeholders and one unrelated active user share this list.
        for user_id in range(74, 88):
            self.manifest["mapping"].append({"source_user_id": user_id, "target_user_id": user_id,
                                            "target_personal_account_id": user_id - 71})
            self.manifest["source_user_facts"].append(
                {"facts": {"id": user_id}, "source_record_sha256": "d" * 64})
        self.manifest["mapping_sha256"] = H.sha(self.manifest["mapping"])
        self.pg.sql("""
INSERT INTO core_identity.accounts SELECT i-71,'personal',0,false FROM generate_series(74,87) i;
INSERT INTO core_identity.users SELECT i,i-71,'user',false,1 FROM generate_series(74,87) i;
INSERT INTO core_identity.user_profiles SELECT i,false,false,false,false,'' FROM generate_series(74,87) i;
INSERT INTO core_history.scope_receipts
SELECT 1,i,i-71,NULL,repeat('a',64),repeat('b',64),repeat('c',64),true FROM generate_series(74,87) i;
""" + "INSERT INTO core_migration.import_runs VALUES('synthetic-history-only',"
                    "repeat('a',64),repeat('b',64)," + H.jlit(self.manifest) + ");")
        origin_reader = (Path(__file__).resolve().parent.parent /
                         "services/core/src/identity/admin_users/identity_origin.rs").read_text()
        suffix = re.search(r'"( SELECT DISTINCT id::text FROM history_only [^"]+)"', origin_reader).group(1)
        prepare = ("BEGIN READ ONLY;SET LOCAL ROLE history_finance_runtime;"
                   "PREPARE listed_history_only(bigint[]) AS " + self.history_only + suffix + ";")
        for page, expected in [("71", []), ("71,73,99,87", ["73", "87"])]:
            with self.subTest(page=page):
                read = prepare + "EXECUTE listed_history_only(ARRAY[" + page + "]::bigint[]);ROLLBACK;"
                self.assertEqual(sorted(self.pg.sql(read).stdout.split()), expected)

        page = ",".join(map(str, range(73, 88)))
        explain = (prepare + "EXPLAIN (ANALYZE, FORMAT JSON) EXECUTE listed_history_only(ARRAY[" +
                   page + "]::bigint[]);ROLLBACK;")
        plan = json.loads(self.pg.sql(explain).stdout)[0]["Plan"]
        self.assertEqual(plan["Actual Rows"], 15)
        nodes = [plan]
        expansions = []
        while nodes:
            node = nodes.pop()
            nodes.extend(node.get("Plans", []))
            if node.get("Function Name") == "jsonb_array_elements":
                expansions.append(node["Actual Loops"])
        # The attestation digest, mapping and source-fact arrays each expand once per receipt.
        self.assertEqual(sorted(expansions), [1, 1, 1])

    def test_attested_history_identity_preserves_finance_and_runtime_permissions(self):
        unbound = self.read()
        self.assertEqual((unbound["excluded"], unbound["missing"]), (0, 1))
        bound = self.read(self.manifest)
        self.assertEqual((bound["excluded"], bound["missing"], bound["invalid_wallets"]), (1, 0, 0))
        self.assertEqual((bound["used"], bound["balance"]), ("35", "95"))
        self.assertEqual(self.pg.sql(
            "SELECT has_column_privilege('history_finance_runtime','core_migration.import_runs',"
            "'source_archive_sha256','SELECT'),"
            "has_column_privilege('history_finance_runtime','core_migration.import_runs',"
            "'mapping_sha256','SELECT'),"
            "has_table_privilege('history_finance_runtime','core_migration.import_runs','INSERT');"
        ).stdout.strip(), "f|f|f")
        self.assertEqual(self.pg.sql("SELECT count(*) FROM core_migration.import_runs").stdout.strip(), "0")

    def test_verified_wallet_without_historical_usage_baseline(self):
        imported = """
INSERT INTO core_billing.opening_import_runs VALUES('opening','archive','mapping',42);
INSERT INTO core_billing.opening_imports VALUES(1,'opening','source','row');
INSERT INTO core_billing.account_opening_counters VALUES(1,71,22,'opening','source','row','archive','mapping',42);
"""
        wallet = self.read(self.manifest, imported)
        self.assertEqual((wallet["missing"], wallet["missing_usage"], wallet["invalid_wallets"], wallet["balance"]),
                         (0, 1, 0, "95"))
        broken = self.read(self.manifest, imported + "UPDATE core_billing.opening_imports SET source_row_sha256='wrong';")
        self.assertEqual(broken["missing"], 1)
        incomplete = self.read(self.manifest, imported + "DELETE FROM core_billing.balance_state WHERE ledger_account_id=1;")
        self.assertEqual(incomplete["invalid_wallets"], 1)

    def test_partial_wallet_receipt_without_usage_counters(self):
        from migrate_legacy_available_wallet import POLICY, SCOPE
        opening = {"account_id": 1, "source_entity_key": "legacy.users/12", "source_row_sha256": "row",
                   "migration_id": "partial", "source_archive_sha256": "archive", "mapping_sha256": "mapping",
                   "observed_at": 42, "available_units": -20, "reserved_units": 0, "affiliate_units": 0}
        position = {"source_user_id": 12, "target_user_id": 71, "account_id": 1,
                    "available_units": -20, "source_row_sha256": "row"}
        material = {"scope": SCOPE, "policy": POLICY, "migration_id": "partial", "source_archive_sha256": "archive",
                    "mapping_sha256": "mapping", "observed_at": 42, "positions": [position]}
        receipt = {"format_version": 1, "scope": SCOPE, "partial": True, "full_financial_import_complete": False,
                   "material": material}
        sql = ("INSERT INTO core_billing.opening_import_runs VALUES('partial','archive','mapping',42);"
               "INSERT INTO core_billing.opening_imports VALUES(1,'partial','legacy.users/12','row',"
               + H.jlit(opening) + ");")
        def read():
            return self.read(self.manifest, sql + "INSERT INTO core_migration.import_runs VALUES"
                             "('partial','archive','mapping'," + H.jlit(receipt) + ");")
        verified = read()
        # Current wallet+debt, not the immutable opening amount. Lifetime usage stays unknown.
        self.assertEqual((verified["missing"], verified["missing_usage"], verified["invalid_wallets"],
                          verified["balance"]), (0, 1, 0, "95"))
        for key, wrong in (("account_id", 9), ("target_user_id", 9), ("source_row_sha256", "wrong"),
                           ("available_units", 0), ("source_user_id", 99)):
            with self.subTest(key=key):
                original = position[key]
                position[key] = wrong
                self.assertEqual(read()["missing"], 1)
                position[key] = original

    def test_unbound_inactive_identity_and_tampered_receipts_keep_completeness_guard(self):
        mutations = [
            lambda m: m.update(mapping_sha256="e" * 64),
            lambda m: m["mapping"][0].update(target_personal_account_id=9),
            lambda m: m["source_snapshot"].update(sha256="f" * 64),
            lambda m: m["source_snapshot"]["metadata"].update(observed_at=1700000041),
            lambda m: m["target"]["instance"].update(database_oid=1),
            lambda m: m.update(source_user_facts=[]),
            lambda m: m["target_policy"].update(wallet_opening_imported=True),
        ]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                manifest = copy.deepcopy(self.manifest)
                mutate(manifest)
                result = self.read(manifest)
                self.assertEqual((result["excluded"], result["missing"]), (0, 1))
        for change in [
            "UPDATE core_history.scope_receipts SET complete=false WHERE user_id=73;",
            "UPDATE core_history.scope_receipts SET mapping_sha256=repeat('f',64) WHERE user_id=73;",
            "UPDATE core_history.sources SET complete=false;",
            "UPDATE core_identity.users SET active=true WHERE id=73;",
            "UPDATE core_identity.accounts SET active=true WHERE id=2;",
            "UPDATE core_identity.users SET platform_role='admin' WHERE id=73;",
        ]:
            with self.subTest(change=change):
                result = self.read(self.manifest, change)
                self.assertEqual((result["excluded"], result["missing"]), (0, 1))

    def test_native_social_creation_preserves_financial_coverage(self):
        # First login and later sign-ins share an audit action. Only account
        # creation writes the immutable per-user registration observation.
        login = """
INSERT INTO core_identity.accounts VALUES(3,'personal',0,true);
INSERT INTO core_identity.users VALUES(74,3,'user',true,1);
INSERT INTO core_identity.oauth_identities VALUES(74);
INSERT INTO core_identity.audit VALUES(74,'session.google_browser_login','{"user_id":74}');
"""
        observation = "INSERT INTO core_abuse.registration_observations(user_id) VALUES(74);"
        keyed = self.read(self.manifest, login)
        self.assertEqual((keyed["excluded"], keyed["missing"]), (1, 1))
        native = self.read(self.manifest, login + observation)
        self.assertEqual((native["excluded"], native["missing"], native["used"], native["balance"]),
                         (1, 0, "35", "95"))
        imported = self.read(self.manifest, login + observation +
                             "INSERT INTO core_billing.opening_imports VALUES(3);")
        self.assertEqual(imported["missing"], 1)
        contradicted = self.read(self.manifest,
            "INSERT INTO core_abuse.registration_observations(user_id) VALUES(73);")
        self.assertEqual((contradicted["excluded"], contradicted["missing"]), (0, 1))
        self.assertEqual(self.pg.sql(
            "SELECT has_column_privilege('history_finance_runtime',"
            "'core_abuse.registration_observations','source_ip','SELECT');"
        ).stdout.strip(), "f")

    def test_two_bound_identities_use_the_exact_mapping_digest(self):
        manifest = copy.deepcopy(self.manifest)
        manifest["mapping"].append({"source_user_id": 74, "target_user_id": 74,
                                    "target_personal_account_id": 3})
        manifest["mapping_sha256"] = H.sha(manifest["mapping"])
        manifest["source_user_facts"].append(
            {"facts": {"id": 74}, "source_record_sha256": "e" * 64})
        change = """
INSERT INTO core_identity.accounts VALUES(3,'personal',0,false);
INSERT INTO core_identity.users VALUES(74,3,'user',false,1);
INSERT INTO core_identity.user_profiles VALUES(74,false,false,false,false,'');
INSERT INTO core_history.scope_receipts VALUES(1,74,3,NULL,repeat('a',64),repeat('b',64),repeat('e',64),true);
"""
        result = self.read(manifest, change)
        self.assertEqual((result["excluded"], result["missing"], result["balance"]), (2, 0, "95"))
        manifest["mapping"].reverse()
        stale_digest = self.read(manifest, change)
        self.assertEqual((stale_digest["excluded"], stale_digest["missing"]), (0, 2))
        manifest["mapping_sha256"] = H.sha(manifest["mapping"])
        reversed_valid = self.read(manifest, change)
        self.assertEqual((reversed_valid["excluded"], reversed_valid["missing"]), (2, 0))

    def test_invalid_mapping_shapes_fail_closed_without_query_errors(self):
        for mapping in [None, {}, "invalid", [None], ["invalid"], [3],
                        [{**self.manifest["mapping"][0], "unexpected": 1}],
                        [self.manifest["mapping"][0]] * 2,
                        [{"source_user_id": -73, "target_user_id": 73, "target_personal_account_id": 2}]]:
            with self.subTest(mapping=mapping):
                manifest = copy.deepcopy(self.manifest)
                manifest["mapping"] = mapping
                manifest["mapping_sha256"] = H.sha(mapping)
                result = self.read(manifest)
                self.assertEqual((result["excluded"], result["missing"]), (0, 1))

    def test_authentication_and_financial_activity_disable_the_exception(self):
        changes = [
            "UPDATE core_identity.user_profiles SET password_login_allowed=true WHERE user_id=73;",
            "UPDATE core_identity.user_profiles SET email_login_allowed=true WHERE user_id=73;",
            "UPDATE core_identity.user_profiles SET legacy_bcrypt_allowed=true WHERE user_id=73;",
            "INSERT INTO core_identity.credentials VALUES(9,73,2);",
            "INSERT INTO core_identity.password_logins VALUES(73);",
            "INSERT INTO core_identity.oauth_identities VALUES(73);",
            "INSERT INTO core_identity.web_sessions VALUES(73);",
            "INSERT INTO core_identity.mfa_factors VALUES(73);",
            "INSERT INTO core_identity.email_addresses VALUES(73);",
            "INSERT INTO core_passkeys.credentials VALUES(73);",
            "INSERT INTO core_billing.opening_imports VALUES(2);",
            "INSERT INTO core_billing.ledger_accounts VALUES(9,2,'wallet');",
            "INSERT INTO core_charging.requests VALUES(73,2,1,'settled',7,0);",
            "INSERT INTO core_payments.intents VALUES(2,73);",
            "INSERT INTO core_history.finance_users VALUES(73,2,0);",
        ]
        for change in changes:
            with self.subTest(change=change):
                result = self.read(self.manifest, change)
                self.assertEqual((result["excluded"], result["missing"]), (0, 1))
        registered = self.read(self.manifest,
            """INSERT INTO core_identity.audit VALUES(73,'user.register','{"user_id":73}');""")
        self.assertEqual(registered["excluded"], 0)

    def test_unrelated_inactive_user_is_still_a_financial_gap(self):
        result = self.read(self.manifest, """
INSERT INTO core_identity.accounts VALUES(3,'personal',0,false);
INSERT INTO core_identity.users VALUES(74,3,'user',false,1);
""")
        self.assertEqual((result["excluded"], result["missing"]), (1, 1))


if __name__ == "__main__":
    unittest.main()

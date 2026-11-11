"""Synthetic producer execution, invoked by the canonical freshinstall Go PG test."""
import base64
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from cryptography.hazmat.primitives.ciphers.aead import AESGCM
import financial_import as F
import legacy_extensions_import as IMPORT
import legacy_cutover_import as CUTOVER
from test_financial_import_pg import reviewed_fixture_ddl
from test_legacy_cutover_mapping import ARCHIVE, identity_files


@unittest.skipUnless(os.environ.get("LMM_COMMERCE_PRODUCER_FIXTURE_DIR"), "requires canonical freshinstall PG fixture")
class ProducerPostgresTests(unittest.TestCase):
    def test_actual_producer_transactions(self):
        root = Path(os.environ["LMM_COMMERCE_PRODUCER_FIXTURE_DIR"])
        socket = os.environ["LMM_COMMERCE_PRODUCER_SOCKET"]
        port = int(os.environ["LMM_COMMERCE_PRODUCER_PORT"])
        admin = lambda database: IMPORT.Db(socket, port, database, "fixture_admin", role=None, readonly=False)
        binding, map_path, manifest_path, manifest = identity_files(root)
        self.assertEqual(binding["accounts"], {2: 1, 3: 2})
        control = admin("postgres")
        control.run("CREATE ROLE producer_core_owner NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; "
                    "CREATE ROLE producer_core_installer LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic-core'; "
                    "GRANT producer_core_owner TO producer_core_installer WITH INHERIT FALSE,SET TRUE; "
                    "ALTER DATABASE lmm_core_synthetic_test OWNER TO producer_core_owner; "
                    "GRANT CONNECT ON DATABASE lmm_core_synthetic_test TO producer_core_installer;")
        hba = Path(control.scalar("SHOW hba_file"))
        hba.write_text("local all all trust\nhost all all 127.0.0.1/32 scram-sha-256\nhost all all ::1/128 scram-sha-256\n")
        control.run("SELECT pg_reload_conf()")
        endpoints = {}
        def endpoint(component, database, owner, login, password):
            pgpass = root / (component + ".pgpass")
            pgpass.write_text(f"127.0.0.1:{port}:{database}:{login}:{password}\n")
            pgpass.chmod(0o600)
            connection = {"transport": "loopback-tcp", "host": "127.0.0.1", "port": port,
                "database": database, "role": owner, "login_role": login, "set_role": owner, "pgpass_file": str(pgpass)}
            db = CUTOVER.Db(Path(socket), port, database, login, role=owner, readonly=False, connection=connection)
            actual = json.loads(db.run(F.IDENTITY.IDENTITY_SQL))
            receipt = {"source_archive_sha256": ARCHIVE, "connection": connection, "instance_identity": actual}
            path = root / (component + "-endpoint.json")
            path.write_text(json.dumps(receipt))
            path.chmod(0o600)
            endpoints[component] = path
            self.assertTrue(actual["tcp_connection"])
            self.assertFalse(actual["session_role_privileged"])
            self.assertFalse(actual["effective_role_privileged"])
            self.assertEqual(actual["session_user"], login)
            return IMPORT.Db(socket, port, database, login, role=owner, readonly=False, connection=connection, endpoint=receipt)
        core = endpoint("core", "lmm_core_synthetic_test", "producer_core_owner", "producer_core_installer", "synthetic-core")
        core.run(reviewed_fixture_ddl(F.IDENTITY.current_contract()))
        for uid, aid in binding["accounts"].items():
            core.run(f"INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES({aid},'personal');"
                     f"INSERT INTO core_identity.users(id,personal_account_id) VALUES({uid},{aid});")
        core.run("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) "
                 f"VALUES('synthetic-identity',{IMPORT.lit(ARCHIVE)},{IMPORT.lit(manifest['mapping_sha256'])},{IMPORT.jlit(manifest)});")
        for component in ("store", "support"):
            endpoint(component, "lmm_" + component + "_prod", "commerce_" + component + "_owner",
                     "commerce_" + component + "_installer", "synthetic-only")
        # A separate offline source instance and genuine SELECT-only reader.
        source_root = Path(tempfile.mkdtemp(prefix="source-", dir=root))
        source_socket_temporary = tempfile.TemporaryDirectory(prefix="lmm-producer-source-")
        self.addCleanup(source_socket_temporary.cleanup)
        source_socket = Path(source_socket_temporary.name)
        source_port = 38539
        subprocess.run(["initdb", "-D", str(source_root / "pgdata"), "-A", "trust", "-U", "fixture_admin", "--no-locale", "-E", "UTF8"], check=True, capture_output=True)
        subprocess.run(["pg_ctl", "-D", str(source_root / "pgdata"), "-l", str(source_root / "postgres.log"), "-o",
            f"-h '' -p {source_port} -k {source_socket} -c shared_buffers=16MB -c max_connections=12", "-w", "start"], check=True, capture_output=True)
        self.addCleanup(lambda: subprocess.run(["pg_ctl", "-D", str(source_root / "pgdata"), "-m", "fast", "-w", "stop"], check=True, capture_output=True))
        source = IMPORT.Db(str(source_socket), source_port, "postgres", "fixture_admin", role=None, readonly=False)
        source.run("CREATE DATABASE producer_source")
        source = IMPORT.Db(str(source_socket), source_port, "producer_source", "fixture_admin", role=None, readonly=False)
        source.run("CREATE SCHEMA synthetic; CREATE ROLE producer_source_reader LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;")
        secret = "synthetic-source-key-only-" + "s" * 32
        def encrypt(kind, ident, plaintext):
            key = hashlib.sha256(f"merchant-store:{kind}:{ident}:{secret}".encode()).digest()
            nonce = hashlib.sha256(ident.encode()).digest()[:12]
            return "v1:" + base64.urlsafe_b64encode(nonce + AESGCM(key).encrypt(nonce, plaintext.encode(), None)).decode().rstrip("=")
        secret_path, stock_key_path = root / "source-key", root / "stock-key"
        secret_path.write_text(secret)
        stock_key_path.write_bytes(b"t" * 32)
        secret_path.chmod(0o600)
        stock_key_path.chmod(0o600)
        product_id, variant_id = "00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000002"
        product = {
            "id": product_id, "seller_id": 3, "title": "Synthetic product", "description": "",
            "image_urls": "[]", "contact": "", "links": "[]", "test_mode": False, "visibility": "public",
            "purchase_login_required": True, "max_quantity_per_order": 10, "max_quantity_per_buyer": 10,
            "template": "stock-items", "delivery_strategy": "sequential", "payment_methods": "[]",
            "pickup_login_required": True, "pickup_code_required": False, "email_pickup_link": False,
            "status": "published", "review_note": "", "reviewed_by": 0, "reviewed_at": 1,
            "created_at": 1, "updated_at": 1, "sale_limit": None, "category_id": "", "price_quota": 100,
        }
        orders, refunds, items, stocks = [], [], [], []
        for index, role in enumerate(("buyer", "seller"), 1):
            order_id, refund_id = str(index) * 64, str(index + 2) * 64
            stock_id = f"00000000-0000-0000-0000-{index + 2:012d}"
            orders.append({"id": order_id, "buyer_id": 2, "seller_id": 3, "product_id": product_id,
                "variant_id": variant_id, "variant_name": "Default", "product_title": "Original title",
                "price_quota": 100, "unit_price_quota": 100, "quantity": 1, "payment_method": "balance",
                "amount_minor": 0, "status": "refunded", "delivery_template": "stock-items", "claimed_at": 2,
                "created_at": 1, "paid_at": 1, "pickup_code_required": False, "seller_terms_version": "",
                "promotion_code": "", "input_digest": "synthetic-input"})
            refunds.append({"id": refund_id, "order_id": order_id, "request_key": f"refund-{index}",
                "input_digest": "", "mode": "full", "quantity": 1, "principal_quota": 100, "amount_minor": 0,
                "status": "completed", "requested_role": role, "requested_by": 2 if role == "buyer" else 3,
                "reason": "synthetic refund", "created_at": 3, "decision_by": 3, "decision_reason": "",
                "decided_at": 4, "completed_at": 5})
            items.append({"id": f"source-refund-item-{index}", "refund_id": refund_id, "order_id": order_id,
                          "stock_id": stock_id, "created_at": 5})
            stocks.append({"id": stock_id, "product_id": product_id, "variant_id": variant_id,
                "ciphertext": encrypt("stock", product_id + ":" + stock_id, f"synthetic unit {index}"), "state": "refunded",
                "order_id": order_id, "position": index, "created_at": 1})
        conversation = {"id": "source-conversation", "buyer_id": 2, "seller_id": 3, "product_id": product_id,
            "order_id": orders[0]["id"], "created_at": 1, "updated_at": 3, "status": "open",
            "buyer_read_id": 100, "seller_read_id": 200}
        messages = [{"id": index * 100, "conversation_id": conversation["id"], "sender_id": uid,
            "request_key": f"message-{index}", "created_at": index + 1,
            "ciphertext": encrypt("support-message", f"{conversation['id']}:message-{index}:{uid}", text)}
            for index, uid, text in ((1, 2, "synthetic buyer"), (2, 3, "synthetic seller"))]
        rows = {"users": [{"id": 2}, {"id": 3}], "merchant_store_products": [product],
            "merchant_store_catalogue_metadata": [{"product_id": product_id, "default_variant_id": variant_id}],
            "merchant_store_orders": orders, "merchant_store_refunds": refunds, "merchant_store_refund_items": items,
            "merchant_store_stocks": stocks, "merchant_store_support_conversations": [conversation],
            "merchant_store_support_messages": messages}
        empty = ("variants", "fixed_contents", "order_fixed_deliveries", "seller_terms", "terms_acceptances",
                 "promotions", "product_likes", "product_traffic_days", "product_traffic_receipts", "events",
                 "transfers", "gateways", "configs", "email_deliveries", "customers")
        for suffix in empty:
            rows.setdefault("merchant_store_" + suffix, [])
        # Synthetic source DTO columns only; native target schemas come exclusively from RunCLI.
        for table, data in rows.items():
            fields = set().union(*(row.keys() for row in data)) if data else {"id", "created_at", "product_id", "variant_id", "seller_id", "accepted_at", "order_id"}
            definitions = []
            for field in sorted(fields):
                value = next((row[field] for row in data if row.get(field) is not None), None)
                kind = "boolean" if type(value) is bool else "bigint" if type(value) is int else "text"
                definitions.append(field + " " + kind)
            source.run(f"CREATE TABLE synthetic.{table}(" + ",".join(definitions) + ")")
            for row in data:
                source.run(f"INSERT INTO synthetic.{table}(" + ",".join(row) + ") VALUES(" + ",".join(IMPORT.lit(v) for v in row.values()) + ")")
        for component in ("store", "support"):
            db = admin("lmm_" + component + "_prod")
            tables = db.scalar(f"SELECT string_agg(quote_ident(schemaname)||'.'||quote_ident(tablename),',') FROM pg_tables WHERE schemaname='lmm_{component}'")
            db.run("TRUNCATE " + tables)
        source.run("GRANT CONNECT ON DATABASE producer_source TO producer_source_reader; GRANT USAGE ON SCHEMA synthetic TO producer_source_reader; GRANT SELECT ON ALL TABLES IN SCHEMA synthetic TO producer_source_reader")
        source_before = {table: source.scalar(f"SELECT md5(coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]')::text) FROM synthetic.{table} t") for table in rows}
        base = [sys.executable, str(Path(IMPORT.__file__)), "--socket", str(source_socket), "--port", str(source_port),
            "--source-db", "producer_source", "--source-user", "producer_source_reader", "--schema", "synthetic",
            "--identity-receipt", str(endpoints["core"]),
            "--identity-map", str(map_path), "--identity-manifest", str(manifest_path), "--archive-sha256", ARCHIVE,
            "--store-key-file", str(secret_path), "--store-stock-key-file", str(stock_key_path)]
        def invoke(component, output, apply=False):
            command = base + ["--phase", component, "--target", component + "=lmm_" + component + "_prod",
                "--target-receipt", component + "=" + str(endpoints[component]),
                "--output", str(output)]
            if apply:
                command.append("--apply")
            result = subprocess.run(command, capture_output=True, text=True)
            with (root / "producer-log.private.txt").open("a") as log:
                log.write(result.stdout + result.stderr)
            return result
        plan_dir = root / "plan"
        result = invoke("store", plan_dir)
        self.assertEqual(result.returncode, 0, result.stderr)
        original_endpoint = endpoints["store"].read_text()
        changed_endpoint = json.loads(original_endpoint)
        pgpass_path = Path(changed_endpoint["connection"]["pgpass_file"])
        password = pgpass_path.read_text()
        pgpass_path.write_text(password.replace("synthetic-only", "wrong-password"))
        result = invoke("store", root / "wrong-password", True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("password authentication failed", result.stderr)
        pgpass_path.write_text(password)
        scram_hba = hba.read_text()
        hba.write_text(scram_hba.replace("scram-sha-256", "trust"))
        control.run("SELECT pg_reload_conf(); SELECT pg_sleep(0.1)")
        result = invoke("store", root / "unexpected-trust", True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("authentication", result.stderr)
        hba.write_text(scram_hba)
        control.run("SELECT pg_reload_conf(); SELECT pg_sleep(0.1)")
        for field, wrong in (("database", "postgres"), ("role", "commerce_support_owner")):
            changed_endpoint = json.loads(original_endpoint)
            changed_endpoint["connection"][field] = wrong
            endpoints["store"].write_text(json.dumps(changed_endpoint))
            result = invoke("store", root / ("wrong-" + field), True)
            self.assertNotEqual(result.returncode, 0)
        endpoints["store"].write_text(original_endpoint)
        for field, wrong in (("database_oid", 0), ("postmaster_started_at", "2000-01-01 00:00:00+00")):
            changed_endpoint = json.loads(original_endpoint)
            changed_endpoint["instance_identity"][field] = wrong
            endpoints["store"].write_text(json.dumps(changed_endpoint))
            result = invoke("store", root / ("wrong-" + field), True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("instance_receipt_mismatch", result.stderr)
        endpoints["store"].write_text(original_endpoint)
        # The original formal file may not be silently replaced, even with the same UIDs.
        original_map = map_path.read_text()
        changed = [json.loads(line) for line in original_map.splitlines()]
        changed[0]["target_personal_account_id"] = 999
        map_path.write_text("\n".join(json.dumps(row) for row in changed) + "\n")
        result = invoke("store", root / "changed-map", True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("identity_map_file_hash_mismatch", result.stderr)
        map_path.write_text(original_map)
        self.assertEqual(admin("lmm_store_prod").scalar("SELECT count(*) FROM lmm_store.orders"), "0")
        # The real assembled Core keeps the committed identity receipt immutable.
        with self.assertRaises(SystemExit):
            core.run("UPDATE core_migration.import_runs SET mapping_sha256='" + "e" * 64 + "' WHERE migration_id='synthetic-identity'")
        self.assertEqual(core.scalar("SELECT mapping_sha256 FROM core_migration.import_runs WHERE migration_id='synthetic-identity'"), manifest["mapping_sha256"])
        # Force a real failure after all generated inserts, preserving an entirely empty target.
        store_endpoint = json.loads(endpoints["store"].read_text())
        store_db = IMPORT.Db(socket, port, "lmm_store_prod", "commerce_store_installer", role="commerce_store_owner", readonly=False,
                             connection=store_endpoint["connection"], endpoint=store_endpoint)
        failed_sql = (plan_dir / "store.sql").read_text().replace("COMMIT;", "INSERT INTO lmm_store.orders VALUES('failure','failure','[]'); COMMIT;")
        with self.assertRaises(SystemExit):
            store_db.run(failed_sql)
        self.assertEqual(admin("lmm_store_prod").scalar("SELECT count(*) FROM lmm_store.orders"), "0")
        self.assertEqual(admin("lmm_store_prod").scalar("SELECT count(*) FROM lmm_store.shops"), "0")
        expected_tables = json.loads((plan_dir / "report.json").read_text())["store"]["expect"]
        self.assertTrue(all(count == 0 for count in IMPORT.table_counts(store_db, expected_tables).values()))
        for component in ("store", "support"):
            result = invoke(component, root / (component + "-apply"), True)
            self.assertEqual(result.returncode, 0, result.stderr)
            applied_report = json.loads((root / (component + "-apply") / "report.json").read_text())[component]
            self.assertEqual(applied_report["database"], "lmm_" + component + "_prod")
            self.assertEqual(applied_report["target_instance_identity"]["session_user"], "commerce_" + component + "_installer")
            self.assertEqual(applied_report["identity_instance_identity"]["session_user"], "producer_core_installer")
            before = admin("lmm_" + component + "_prod").scalar("SELECT count(*) FROM lmm_" + component + (".orders" if component == "store" else ".messages"))
            result = invoke(component, root / (component + "-replay"), True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("target_not_empty", result.stderr)
            after = admin("lmm_" + component + "_prod").scalar("SELECT count(*) FROM lmm_" + component + (".orders" if component == "store" else ".messages"))
            self.assertEqual(before, after)
        self.assertEqual(admin("lmm_store_prod").scalar("SELECT count(*) FROM lmm_store.money_operations"), "0")
        self.assertEqual(core.scalar("SELECT count(*) FROM core_billing.ledger_journals"), "0")
        source_after = {table: source.scalar(f"SELECT md5(coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]')::text) FROM synthetic.{table} t") for table in rows}
        self.assertEqual(source_before, source_after)
        reader = IMPORT.Db(str(source_socket), source_port, "producer_source", "producer_source_reader", role=None, readonly=True)
        with self.assertRaises(SystemExit):
            reader.run("SET default_transaction_read_only=off; INSERT INTO synthetic.users VALUES(99)")
        buyer = {"kind": "personal", "id": 1}
        cid = IMPORT.go_id("conversation", 2, buyer, IMPORT.go_id("web-order-support", orders[0]["id"]))
        fixture = {"shop_id": IMPORT.go_id("legacy-web-shop-v1", {"kind": "personal", "id": 2}),
            "order_ids": [o["id"] for o in orders], "stock_ids": [s["id"] for s in stocks],
            "product_id": product_id, "conversation_id": cid,
            "source_before_sha256": hashlib.sha256(json.dumps(source_before, sort_keys=True).encode()).hexdigest(),
            "source_after_sha256": hashlib.sha256(json.dumps(source_after, sort_keys=True).encode()).hexdigest(),
            "message_ids": [f"{cid}:{n:020d}" for n in (1, 2)]}
        (root / "fixture.private.json").write_text(json.dumps(fixture))
        for path in root.rglob("*"):
            if path.is_file():
                path.chmod(0o600)


if __name__ == "__main__":
    unittest.main()

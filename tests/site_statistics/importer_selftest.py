#!/usr/bin/env python3
"""Author real-PG/full-CLI self-test; synthetic archives only, no Core build.

Run after space-first with --candidate <passed final build> --root <new short
~/Backups directory> --execute. This is developer evidence, not independent QA.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import http.client
import importlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import subprocess
import sys
import time
import traceback
from urllib.parse import urlencode

import importer_fixture as fixture

REPO = Path(__file__).resolve().parents[2]
PYTHON_FILES = ("legacy_site_statistics.py", "financial_import.py", "migrate-legacy-financial.py",
    "migrate-legacy-snapshot.py", "legacy_financial_preflight.py", "legacy_escrow_mapping.py", "legacy_bounty_basis.py")
CANDIDATE_FILES = ("services/core/src/core_schema.rs", "services/core/schema/site_statistics.sql",
                   "deployment/postgres/create-core-roles.sql", "deployment/postgres/grant-core-runtime.sql")
PG_PROGRAMS = ("postgres", "initdb", "pg_ctl", "psql", "pg_dump", "pg_restore")
GROUPS = ("fresh28-native-init", "one-real-frozen-archive", "full-cli-prepare-apply", "exact-replay-noop",
          "other-source-prepare-refusal", "other-source-apply-refusal", "completion-fault-atomic-rollback",
          "lost-ack-unknown-committed", "raw-file-change-preflight-refusal", "runtime-source-write-denied",
          "native-intent-seal-floor", "native-intent-sequence-guards", "native-request-seal-and-reader", "live-client-cache-fence")
SEAL_GROUPS = ("fresh28-native-init", "one-real-frozen-archive", "live-client-cache-fence", "native-intent-seal-floor",
               "native-request-seal-and-reader", "native-intent-sequence-guards")
TEST_GROUPS = {"base": GROUPS[:10], "seal": SEAL_GROUPS, "full": GROUPS}


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def save(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def literal(value):
    text = value if type(value) is str else json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    require("\0" not in text, "NUL in synthetic SQL")
    return "'" + text.replace("'", "''") + "'"


def identifier(value):
    require(re.fullmatch(r"[a-z_][a-z_0-9]*", value), "invalid synthetic identifier")
    return '"' + value + '"'


def clean_environment():
    env = {k: v for k, v in os.environ.items() if not k.startswith(("PG", "LMM_", "SQLX_", "CARGO_"))
           and k not in ("DATABASE_URL", "SQL_DSN", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY")}
    env.update(PATH="/usr/bin:/bin", PYTHONDONTWRITEBYTECODE="1", TZ="UTC", LC_ALL="C.UTF-8",
               XDG_RUNTIME_DIR=f"/run/user/{os.getuid()}",
               DBUS_SESSION_BUS_ADDRESS=f"unix:path=/run/user/{os.getuid()}/bus")
    return env


def verify_binding(root):
    binding = json.loads((root / "binding.json").read_text())
    for name, digest in binding["frozen_files"].items():
        require(sha(root / name) == digest, "frozen author dependency changed: " + name)
    for name, digest in binding["programs"].items():
        require(sha(name) == digest, "local PostgreSQL executable changed")
    candidate = Path(binding["candidate"])
    require(sha(candidate / "lmm-core") == binding["core_sha256"], "candidate Native changed")
    require(json.loads((candidate / "result.json").read_text()).get("status") == "passed", "candidate no longer PASS")
    return binding


class Run:
    def __init__(self, root):
        self.root = root
        self.binding = verify_binding(root)
        self.candidate = Path(self.binding["candidate"])
        self.env = clean_environment()
        self.logs = root / "logs"
        self.logs.mkdir()
        self.seq = 0
        self.pg = False
        self.socket = root / "s"
        self.socket.mkdir(mode=0o700)
        self.port = 55613
        self.selected = TEST_GROUPS[self.binding["test_selection"]]
        self.results = {name: "NOT_RUN" for name in self.selected}
        self.writer_path = root / "source/scripts/legacy_site_statistics.py"
        sys.path.insert(0, str(self.writer_path.parent))
        self.writer = importlib.import_module("legacy_site_statistics")
        self.source_dir = root / "source-material"
        self.source_dir.mkdir(mode=0o700)

    def command(self, label, command, *, data=None, env=None, expected=0, timeout=150):
        self.seq += 1
        prefix = self.logs / f"{self.seq:03}-{label}"
        args = [str(x) for x in command]
        record = {"argv": args, "started_at": time.time()}
        save(prefix.with_suffix(".command.json"), record)
        if data is not None:
            prefix.with_suffix(".stdin").write_text(data)
        try:
            result = subprocess.run(args, input=data, text=True, capture_output=True,
                                    env=env or self.env, timeout=timeout)
        except BaseException as error:
            save(prefix.with_suffix(".failure.json"), {"type": type(error).__name__})
            raise
        prefix.with_suffix(".stdout").write_text(result.stdout)
        prefix.with_suffix(".stderr").write_text(result.stderr)
        record.update(exit_code=result.returncode, finished_at=time.time())
        save(prefix.with_suffix(".command.json"), record)
        require(expected is None or result.returncode == expected,
                f"{label}: exit {result.returncode}, expected {expected}; evidence {prefix}")
        return result

    def psql(self, database, role="postgres"):
        return ["/usr/bin/psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1",
                "-h", self.socket, "-p", self.port, "-U", role, "-d", database]

    def sql(self, query, database="sf_base", *, owner=False, expected=0, label="sql"):
        role = "sf_installer" if owner else "postgres"
        if owner:
            query = "SET ROLE lmm_core_owner;\n" + query
        return self.command(label, self.psql(database, role), data=query, expected=expected).stdout.strip()

    def json_sql(self, query, database="sf_base", **kwargs):
        return json.loads(self.sql(query, database, **kwargs))

    def group(self, name, action):
        self.results[name] = "RUNNING"
        save(self.root / "results.json", self.results)
        try:
            action()
        except BaseException:
            self.results[name] = "FAIL"
            save(self.root / "results.json", self.results)
            raise
        self.results[name] = "PASS"
        save(self.root / "results.json", self.results)

    def setup(self):
        native = self.root / "lmm-core"
        shutil.copy2(self.candidate / "lmm-core", native)
        require(sha(native) == self.binding["core_sha256"], "copied Native hash mismatch")
        manifest_result = self.command("native-manifest", [native, "schema-manifest"])
        manifest = json.loads(manifest_result.stdout)
        save(self.root / "actual-native-manifest.json", manifest)
        components = manifest["components"]
        require(len(components) == 28, "expected final fresh28 Native")
        finance = [row for row in components if row["name"] == "site-finance"]
        require(len(finance) == 1 and finance[0]["raw_schema_sha256"] ==
                sha(self.root / "source/services/core/schema/site_statistics.sql"), "Native/site-finance schema mismatch")
        for name in PG_PROGRAMS:
            self.command(name + "-version", ["/usr/bin/" + name, "--version"])
        self.command("initdb", ["/usr/bin/initdb", "-D", self.root / "pg", "-U", "postgres", "--no-locale",
            "--encoding=UTF8", "--auth=trust", "--wal-segsize=1"])
        self.command("pg-start", ["/usr/bin/pg_ctl", "-D", self.root / "pg", "-l", self.root / "postgres.log", "-o",
            f"-c listen_addresses= -p {self.port} -c unix_socket_directories={self.socket} -c unix_socket_permissions=0700 "
            "-c shared_buffers=16MB -c max_connections=20 -c min_wal_size=2MB -c max_wal_size=128MB -c autovacuum=off", "-w", "start"])
        self.pg = True
        self.sql("CREATE DATABASE sf_base", "postgres")
        self.command("create-owner-roles", self.psql("sf_base") + ["-f", self.root / "source/deployment/postgres/create-core-roles.sql"])
        self.sql("ALTER DATABASE sf_base OWNER TO lmm_core_owner; CREATE ROLE sf_installer LOGIN NOINHERIT; "
            "GRANT lmm_core_owner TO sf_installer WITH INHERIT FALSE,SET TRUE; "
            "GRANT CONNECT ON DATABASE sf_base TO sf_installer; "
            "REVOKE EXECUTE ON FUNCTION pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_control_system() TO lmm_core_owner")
        url = "postgresql://sf_installer@localhost/sf_base?" + urlencode({"host": str(self.socket), "port": self.port,
            "sslmode": "disable", "options": "-c role=lmm_core_owner"})
        (self.root / "owner.url").write_text(url)
        (self.root / "owner.toml").write_text("version=1\n[database]\nurl_file=" + json.dumps(str(self.root / "owner.url")) + "\n")
        self.command("native-fresh28-init", [native, "--config", self.root / "owner.toml", "init-db"])
        require(self.sql("SELECT count(*) FROM core_meta.component_contract") == "28", "actual initialized component count")
        index = self.json_sql("SELECT jsonb_build_object('unique',i.indisunique,'valid',i.indisvalid,'definition',pg_get_indexdef(i.indexrelid)) "
            "FROM pg_index i WHERE i.indexrelid='core_history.finance_sources_one_archive'::regclass")
        require(index["unique"] and index["valid"] and "true" in index["definition"], "final single-source index absent")
        save(self.root / "installed-one-source-index.json", index)
        seal_column = self.json_sql("SELECT jsonb_build_object('not_null',a.attnotnull,'default',pg_get_expr(d.adbin,d.adrelid)) "
            "FROM pg_attribute a JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum "
            "WHERE a.attrelid='core_history.finance_sources'::regclass AND a.attname='native_intent_floor'")
        require(seal_column["not_null"] and "pg_sequence_last_value" in seal_column["default"]
                and "intents_id_seq" in seal_column["default"], "server-side Native floor default absent")
        save(self.root / "installed-source-floor.json", seal_column)
        request_column = self.json_sql("SELECT jsonb_build_object('not_null',a.attnotnull,'type',format_type(a.atttypid,a.atttypmod),"
            "'has_default',a.atthasdef) FROM pg_attribute a WHERE a.attrelid='core_history.finance_sources'::regclass "
            "AND a.attname='native_request_ids'")
        require(request_column == {"not_null": True, "type": "text[]", "has_default": False},
                "exact request seal column must be NOT NULL without a default")
        save(self.root / "installed-request-seal.json", request_column)
        self.sql("ALTER ROLE lmm_core_runtime NOLOGIN; REVOKE CONNECT ON DATABASE sf_base FROM PUBLIC; "
            "CREATE ROLE sf_source_owner NOLOGIN; CREATE ROLE sf_source_reader LOGIN; "
            "GRANT CONNECT ON DATABASE sf_base TO sf_installer;")
        self.sql("CREATE DATABASE sf_legacy OWNER sf_source_owner", "postgres")
        self.sql("CREATE DATABASE sf_frozen OWNER sf_source_owner", "postgres")
        self.sql("ALTER SCHEMA public OWNER TO sf_source_owner", "sf_legacy")

    def source(self):
        observed = int(time.time())
        self.tables = fixture.old_tables(observed)
        declarations = []
        for name, rows in self.tables.items():
            columns = sorted(rows[0]) if rows else ["id"]
            types = {}
            for key in columns:
                values = [row[key] for row in rows if row[key] is not None]
                types[key] = "bigint" if key == "id" or (values and type(values[0]) is int) else "text"
            declaration = ",".join(identifier(k) + " " + types[k] + (" PRIMARY KEY" if k == "id" else "") for k in columns)
            declarations.append(f"CREATE TABLE public.{identifier(name)} ({declaration});")
            for row in rows:
                values = ["NULL" if row[k] is None else str(row[k]) if type(row[k]) is int else literal(row[k]) for k in columns]
                declarations.append(f"INSERT INTO public.{identifier(name)} ({','.join(identifier(k) for k in columns)}) VALUES ({','.join(values)});")
        declarations += ["GRANT USAGE ON SCHEMA public TO sf_source_reader; GRANT SELECT ON ALL TABLES IN SCHEMA public TO sf_source_reader;"]
        self.sql("SET ROLE sf_source_owner;\n" + "\n".join(declarations), "sf_legacy", label="synthetic-old-source")
        self.archive = self.source_dir / "synthetic-legacy.dump"
        self.started = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        self.command("full-source-pg-dump", ["/usr/bin/pg_dump", "--no-password", "-h", self.socket, "-p", self.port,
            "-U", "postgres", "-d", "sf_legacy", "-Fc", "-f", self.archive])
        self.command("archive-list", ["/usr/bin/pg_restore", "--list", self.archive])
        self.command("actual-source-restore", ["/usr/bin/pg_restore", "--no-password", "--exit-on-error", "--clean", "--if-exists", "-h", self.socket,
            "-p", self.port, "-U", "postgres", "-d", "sf_frozen", self.archive])
        self.sql("REVOKE CONNECT ON DATABASE sf_frozen FROM PUBLIC; GRANT CONNECT ON DATABASE sf_frozen TO sf_source_reader", "sf_frozen")
        columns = self.json_sql("SELECT jsonb_agg(jsonb_build_object('schema','public','table_name',table_name,'column_name',column_name) "
            "ORDER BY table_name,ordinal_position) FROM information_schema.columns WHERE table_schema='public'", "sf_frozen")
        constraints = self.json_sql("SELECT jsonb_agg(jsonb_build_object('schema','public','table_name',c.relname,'contype',p.contype,'definition',pg_get_constraintdef(p.oid))) "
            "FROM pg_constraint p JOIN pg_class c ON c.oid=p.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND p.contype='p'", "sf_frozen")
        catalog = self.source_dir / "native-catalog-metadata.jsonl"
        catalog.write_text("\n".join(self.writer.F.canonical(v) for v in ([], [], columns, constraints, [], [], [])) + "\n")
        queries = ["BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC'; SET LOCAL row_security=off;"]
        queries.append("SELECT jsonb_build_object('kind','identity','row',(" + self.writer.I.IDENTITY_SQL.rstrip(";") + ")::jsonb "
                       "|| jsonb_build_object('schema_owner',(SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname='public')));")
        queries.append("SELECT jsonb_build_object('kind','schema','row',jsonb_build_object('schema','public','schema_owner',pg_get_userbyid(nspowner))) FROM pg_namespace WHERE nspname='public';")
        for name in sorted(self.tables):
            queries.append(f"SELECT jsonb_build_object('kind','row','table',{literal(name)},'row',to_jsonb(r)) FROM public.{identifier(name)} r ORDER BY to_jsonb(r)::text COLLATE \"C\";")
        queries.append("COMMIT;")
        raw = self.command("restored-source-readonly-export", self.psql("sf_frozen", "sf_source_reader"), data="\n".join(queries)).stdout
        self.export = self.source_dir / "source-financial-export.private.jsonl"
        self.export.write_text(raw)
        tagged = [self.writer.P.parse_json(line) for line in raw.splitlines() if line]
        instance = tagged[0]["row"]
        self.tables = {name: [] for name in self.tables}
        for row in tagged:
            if row["kind"] == "row":
                self.tables[row["table"]].append(row["row"])
        counts = {name: int(self.sql(f"SELECT count(*) FROM public.{identifier(name)}", "sf_frozen")) for name in self.tables}
        require(counts == {name: len(rows) for name, rows in self.tables.items()}, "restored source complete count mismatch")
        baseline = self.source_dir / "native-table-row-count-baseline.jsonl"
        baseline.write_text("\n".join(json.dumps({"table": name, "row_count": count}) for name, count in sorted(counts.items())) + "\n")
        self.files = {"archive": {"path": str(self.archive), "sha256": sha(self.archive)},
                      "source_export": {"path": str(self.export), "sha256": sha(self.export)}}
        receipt = {"status": "snapshot-restored-baseline-verified-frozen-and-classified", "started_utc": self.started,
            "archive": self.files["archive"], "private_metadata_files": [str(catalog), str(baseline)], "source": {"schema": "public"},
            "readonly_reader": {"database": "sf_frozen", "role": "sf_source_reader", "port": self.port},
            "source_instance_identity": {**instance, "listen_addresses": ""}}
        self.write_source("source_receipt", receipt)
        self.write_source("source_export_proof", {"source_archive_sha256": sha(self.archive), "source_export_sha256": sha(self.export),
            "metadata_hashes": {p.name: sha(p) for p in (catalog, baseline)}, "source_column_coverage_exact": True,
            "source_row_baseline_exact": True, "selected_table_counts": counts, "source_instance": instance})
        save(self.root / "archive-evidence.json", {"synthetic_only": True, "single_archive_sha256": sha(self.archive),
            "actually_dumped_and_restored": True, "source_tables": counts, "export_sha256": sha(self.export)})
        self.seed_prior_opening()

    def write_source(self, name, value):
        path = self.source_dir / (name + ".private.json")
        require(not path.exists(), "source file already exists")
        path.write_text(value if type(value) is str else self.writer.F.canonical(value) + "\n")
        self.files[name] = {"path": str(path), "sha256": sha(path)}
        return path

    def seed_prior_opening(self):
        ids = {}
        for row in self.tables["users"]:
            account = int(self.sql("INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id", owner=True))
            self.sql(f"INSERT INTO core_identity.users(id,personal_account_id,active) VALUES({row['id']},{account},{'false' if row['status'] == 2 else 'true'})", owner=True)
            ids[row["id"]] = account
        records = [{"source_user_id": uid, "target_user_id": uid, "target_personal_account_id": aid} for uid, aid in sorted(ids.items())]
        self.write_source("identity_map", "\n".join(self.writer.F.canonical(r) for r in records) + "\n")
        archive = sha(self.archive)
        manifest = {"format_version": 1, "scope": "identity-only", "source_archive_sha256": archive,
            "mapping_sha256": self.writer.F.digest(records), "source_target_id_map_sha256": self.files["identity_map"]["sha256"]}
        self.write_source("identity_manifest", manifest)
        provenance = self.writer.P.bind_identity_provenance(manifest,
            map_file_sha256=self.files["identity_map"]["sha256"], source_archive_sha256=archive)
        plan = self.writer.P.plan_financial(self.tables, ids, source_sha256=archive, identity_provenance=provenance)
        self.write_source("financial_plan", plan)
        review = {"synthetic_author_fixture": True, "decision": "approved-offline-opening",
                  "source_archive_sha256": archive, "mapping_sha256": plan["mapping_sha256"]}
        self.write_source("source_review", review)
        observed = self.writer.P.source_observation({"started_utc": self.started})
        reservations = {"source_archive_sha256": archive, "mapping_sha256": plan["mapping_sha256"],
            "reviewed_evidence_sha256": self.writer.F.sha(b"SYNTHETIC zero reservations"),
            "positions": [{"source_user_id": uid, "reserved_units": 0} for uid in ids]}
        self.write_source("reservations", reservations)
        opening = self.writer.P.build_opening_payloads(plan, migration_id="author-opening-fixture",
            observation=observed, reservation_reconciliation=reservations)
        counters = self.writer.P.build_opening_counter_payloads(plan, opening)["payloads"]
        seed = ["BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;"]
        seed.extend("SELECT core_billing.import_opening_balance(" + literal(p) + "::jsonb);" for p in opening["payloads"])
        seed.extend("SELECT core_billing.import_account_opening_counters(" + literal(c) + "::jsonb);" for c in counters)
        seed.append("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES(" +
            ",".join((literal("author-identity-fixture"), literal(archive), literal(manifest["mapping_sha256"]), literal(manifest) + "::jsonb")) + "); COMMIT;")
        self.sql("\n".join(seed), owner=True, label="prior-opening-fixture-only")
        run = self.json_sql("SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE migration_id='author-identity-fixture'")
        material = {"scope": self.writer.F.SCOPE, "migration_id": "author-opening-fixture", "source_archive_sha256": archive,
            "mapping_sha256": plan["mapping_sha256"], "source_plan": plan, "source_files": self.files,
            "identity_map_sha256": self.writer.F.digest(ids), "identity_import": run,
            "native_users": [{"id": uid, "personal_account_id": aid} for uid, aid in sorted(ids.items())],
            "observation": observed, "opening": opening["payloads"], "counters": counters, "source_review": review}
        self.opening_path = self.source_dir / "opening-material.private.json"
        self.opening_path.write_text(self.writer.F.canonical(material))
        self.writer.load_source(self.opening_path)

    def clone(self, name, template="sf_base"):
        self.sql(f"CREATE DATABASE {identifier(name)} TEMPLATE {identifier(template)} OWNER lmm_core_owner", "postgres")
        self.sql(f"REVOKE CONNECT,CREATE,TEMPORARY ON DATABASE {identifier(name)} FROM PUBLIC; GRANT CONNECT ON DATABASE {identifier(name)} TO sf_installer", name)
        directory = self.root / name
        directory.mkdir(mode=0o700)
        pgpass = directory / "pgpass.private"
        pgpass.write_text(f"{self.socket}:{self.port}:{name}:sf_installer:synthetic-unused-private-socket\n")
        connection = {"socket_directory": str(self.socket), "port": self.port, "database": name, "role": "lmm_core_owner",
                      "login_role": "sf_installer", "set_role": "lmm_core_owner", "pgpass_file": str(pgpass)}
        save(directory / "connection.private.json", connection)
        return directory

    def cli(self, directory, mode, output, *, extra=(), expected=0, env=None):
        out = directory / output
        out.mkdir(mode=0o700)
        command = [sys.executable, "-B", self.writer_path, "--mode", mode, "--output", out,
                   "--connection", directory / "connection.private.json", *extra]
        result = self.command(directory.name + "-" + output, command, expected=expected, env=env)
        return out, json.loads(result.stdout)

    def prepare(self, directory, label="plan", expected=0):
        snapshot, _ = self.cli(directory, "snapshot", label + "-snapshot", extra=("--runtime-role", "lmm_core_runtime"))
        plan, result = self.cli(directory, "prepare", label, extra=("--opening-material", self.opening_path,
            "--snapshot", snapshot / "target.private.json", "--runtime-role", "lmm_core_runtime"), expected=expected)
        return plan, result

    def approve(self, plan):
        request = json.loads((plan / "approval-request.private.json").read_text())
        require(request["decision"] == "REVIEW_REQUIRED", "CLI must not self-approve")
        request["decision"] = "approve-offline-site-finance"
        save(plan / "owner-approval.private.json", request)

    def apply(self, directory, plan, label="apply", expected=0, env=None):
        return self.cli(directory, "apply", label, extra=("--material", plan / "material.private.json",
            "--approval", plan / "owner-approval.private.json"), expected=expected, env=env)

    def capture(self, database, label):
        # Independent preservation oracle, including sequence log_cnt. This does
        # not reuse the writer's protected_state filtering or hashing helper.
        inventory = self.json_sql("SELECT jsonb_agg(jsonb_build_array(n.nspname,c.relname,c.relkind) ORDER BY n.nspname,c.relname) "
            "FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') "
            "AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p','S')", database)
        query = ["BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;"]
        for schema, table, kind in inventory:
            relation = identifier(schema) + "." + identifier(table)
            value = "jsonb_build_array(jsonb_build_object('last_value',last_value,'is_called',is_called,'log_cnt',log_cnt))" if kind == "S" else "coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text COLLATE \"C\"),'[]'::jsonb)"
            query.append(f"SELECT jsonb_build_object('relation',{literal(schema+'.'+table)},'rows',{value}) FROM {relation} t;")
        query.append("COMMIT;")
        records = [json.loads(line) for line in self.sql("\n".join(query), database, label=label + "-state").splitlines()]
        result = {r["relation"]: hashlib.sha256(json.dumps(r["rows"], sort_keys=True, ensure_ascii=False).encode()).hexdigest() for r in records}
        result["permissions"] = self.sql(self.writer.F.ownership_sql(), database)
        result["catalog"] = self.sql(self.writer.I.catalog_hash_sql(self.writer.catalog_contract()), database)
        save(self.root / (label + ".json"), result)
        return result

    @staticmethod
    def assert_preserved(before, after, *, allow_finance=False):
        allowed = {"core_history.finance_sources", "core_history.finance_users", "core_history.finance_topups"} if allow_finance else set()
        require(before.keys() == after.keys(), "protected inventory changed")
        changed = {k for k in before if before[k] != after[k]}
        require(changed <= allowed, "protected table/sequence/catalog/permission changed: " + ",".join(sorted(changed - allowed)))

    def happy(self):
        self.happy_dir = directory = self.clone("sf_happy")
        before = self.capture(directory.name, "happy-before")
        plan, status = self.prepare(directory)
        require(status["status"] == "READY", "first CLI reconcile not READY")
        fixture.assert_review(json.loads((plan / "review.private.json").read_text()))
        self.assert_preserved(before, self.capture(directory.name, "prepare-after"))
        self.approve(plan)
        _, status = self.apply(directory, plan)
        require(status["status"] == "COMMITTED", "CLI apply not committed")
        after = self.capture(directory.name, "happy-after")
        self.assert_preserved(before, after, allow_finance=True)
        self.happy_plan = plan
        require(self.json_sql("SELECT jsonb_agg(jsonb_build_array(user_id,raw_used_quota,normalized_used_credits) ORDER BY user_id) FROM core_history.finance_users", directory.name)
                == [[11, 920, 470], [12, 300, 300], [13, 0, 0]], "actual persisted normalization")
        require(self.sql("SELECT count(*) FROM core_history.finance_topups", directory.name) == "7", "all success classifications persisted")

    def noop(self):
        before = self.capture("sf_happy", "noop-before")
        plan, status = self.prepare(self.happy_dir, "noop-plan")
        require(status["status"] == "NOOP", "read-only CLI reconcile must identify exact source")
        self.approve(plan)
        _, status = self.apply(self.happy_dir, plan, "noop-apply")
        require(status["status"] == "NOOP", "identical apply must return NOOP")
        _, status = self.apply(self.happy_dir, self.happy_plan, "original-material-replay")
        require(status["status"] == "NOOP", "original approved material replay must return NOOP")
        self.assert_preserved(before, self.capture("sf_happy", "noop-after"))

    def inject_other_source(self, database):
        material = json.loads((self.happy_plan / "material.private.json").read_text())
        source = dict(material["plan"]["source"])
        source.update(source_archive_sha256=hashlib.sha256(b"SYNTHETIC foreign namespace fault").hexdigest(), user_count=0, topup_count=0)
        source.pop("source_key")
        source["source_key"] = self.writer.canonical_sha(source)
        fields = list(source)
        self.sql("INSERT INTO core_history.finance_sources (" + ",".join(fields) + ",native_request_ids) SELECT " +
                 ",".join("r." + field for field in fields) + "," + self.writer.REQUEST_IDS_SQL +
                 " FROM jsonb_populate_record(NULL::core_history.finance_sources," +
                 literal(source) + "::jsonb) AS r", database, owner=True, label="inject-foreign-namespace-fixture")
        return source["source_key"]

    def other_prepare(self):
        directory = self.clone("sf_other_prepare")
        key = self.inject_other_source(directory.name)
        before = self.capture(directory.name, "other-prepare-before")
        _, status = self.prepare(directory, expected=1)
        require(status["status"] == "REFUSED", "foreign archive prepare not refused")
        snap = json.loads((directory / "plan-snapshot/target.private.json").read_text())
        require(snap["source_namespace"]["source_keys"] == [key] and snap["source_namespace"]["source_count"] == 1,
                "snapshot hid a different source namespace")
        try:
            self.writer.reconcile(json.loads((self.happy_plan / "material.private.json").read_text())["plan"], snap)
        except self.writer.Error as error:
            require(str(error) == "target_source_key_conflict", "wrong explicit foreign namespace refusal")
        else:
            raise AssertionError("foreign source incorrectly treated as empty")
        self.assert_preserved(before, self.capture(directory.name, "other-prepare-after"))

    def other_apply(self):
        directory = self.clone("sf_other_apply")
        plan, _ = self.prepare(directory)
        self.approve(plan)
        self.inject_other_source(directory.name)
        before = self.capture(directory.name, "other-apply-before")
        out, status = self.apply(directory, plan, expected=2)
        require(status["status"] == "UNKNOWN", "post-submission failure must remain UNKNOWN")
        error = (out / "site-finance.stderr.private.txt").read_text()
        require("site_finance_source_key_conflict" in error and "duplicate key" not in error,
                "apply must reject namespace before unique violation")
        self.assert_preserved(before, self.capture(directory.name, "other-apply-after"))

    def rollback(self):
        directory = self.clone("sf_rollback")
        self.sql("CREATE FUNCTION public.author_completion_fault() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN "
            "IF (SELECT count(*) FROM core_history.finance_users)=3 AND (SELECT count(*) FROM core_history.finance_topups)=7 THEN "
            "RAISE EXCEPTION 'author_abort_after_all_leaf_inserts'; ELSE RAISE EXCEPTION 'author_wrong_insert_order'; END IF; END $$; "
            "CREATE TRIGGER author_completion_fault BEFORE INSERT ON core_history.finance_sources FOR EACH ROW EXECUTE FUNCTION public.author_completion_fault()",
            directory.name, owner=True, label="owner-only-rollback-fault-fixture")
        plan, _ = self.prepare(directory)
        self.approve(plan)
        before = self.capture(directory.name, "rollback-before")
        out, status = self.apply(directory, plan, expected=2)
        require(status["status"] == "UNKNOWN", "failed submission must not claim rollback")
        require("author_abort_after_all_leaf_inserts" in (out / "site-finance.stderr.private.txt").read_text(), "fault did not reach completion-last boundary")
        after = self.capture(directory.name, "rollback-after")
        self.assert_preserved(before, after)
        _, status = self.prepare(directory, "rollback-reconcile")
        require(status["status"] == "READY", "rolled-back first import left source or leaf residue")
        require(self.sql("SELECT (SELECT count(*) FROM core_history.finance_sources)+(SELECT count(*) FROM core_history.finance_users)+(SELECT count(*) FROM core_history.finance_topups)", directory.name) == "0", "incomplete source survived rollback")

    def unknown(self):
        directory = self.clone("sf_unknown")
        plan, _ = self.prepare(directory)
        self.approve(plan)
        shim = directory / "ack-loss-bin"
        shim.mkdir(mode=0o700)
        proxy = shim / "psql"
        proxy.write_text("#!/usr/bin/python3\nimport subprocess,sys\np=subprocess.run(['/usr/bin/psql',*sys.argv[1:]],input=sys.stdin.buffer.read(),capture_output=True)\nsys.stderr.buffer.write(p.stderr)\nif p.returncode==0:\n sys.stderr.write('AUTHOR_ACK_LOSS_AFTER_REAL_PSQL_SUCCESS\\n')\n sys.exit(42)\nsys.stdout.buffer.write(p.stdout)\nsys.exit(p.returncode)\n")
        proxy.chmod(0o700)
        save(directory / "ack-loss-binding.json", {"shim_sha256": sha(proxy), "real_psql_sha256": sha("/usr/bin/psql"), "fault": "suppress real successful acknowledgement"})
        before = self.capture(directory.name, "unknown-before")
        out, status = self.apply(directory, plan, expected=2, env={**self.env, "PATH": str(shim) + ":/usr/bin:/bin"})
        require(status["status"] == "UNKNOWN", "lost real acknowledgement must be UNKNOWN")
        require("AUTHOR_ACK_LOSS_AFTER_REAL_PSQL_SUCCESS" in (out / "site-finance.stderr.private.txt").read_text(), "transport fault not executed")
        self.assert_preserved(before, self.capture(directory.name, "unknown-after"), allow_finance=True)
        resolved, status = self.prepare(directory, "unknown-reconcile")
        require(status["status"] == "NOOP", "committed UNKNOWN source not recoverable by exact read-only reconcile")
        before_replay = self.capture(directory.name, "unknown-replay-before")
        self.approve(resolved)
        _, status = self.apply(directory, resolved, "explicit-owner-replay")
        require(status["status"] == "NOOP", "explicit replay duplicated committed UNKNOWN source")
        self.assert_preserved(before_replay, self.capture(directory.name, "unknown-replay-after"))

    def raw_change(self):
        directory = self.clone("sf_raw_change")
        plan, _ = self.prepare(directory)
        self.approve(plan)
        before = self.capture(directory.name, "raw-change-before")
        raw = self.export.read_bytes()
        try:
            self.export.write_bytes(raw + b" ")
            out, status = self.apply(directory, plan, expected=1)
            require(status["status"] == "REFUSED" and not (out / "attempt.private.json").exists(), "raw hash conflict reached submission")
        finally:
            self.export.write_bytes(raw)
        self.assert_preserved(before, self.capture(directory.name, "raw-change-after"))

    def runtime_denied(self):
        before = self.capture("sf_happy", "runtime-before")
        for name in ("sources", "users", "topups"):
            result = self.command("runtime-insert-denied-" + name, self.psql("sf_happy"),
                data=f"SET ROLE lmm_core_runtime; INSERT INTO core_history.finance_{name} DEFAULT VALUES;", expected=3)
            require("permission denied" in result.stderr, "runtime source insertion was not denied by ACL")
        self.assert_preserved(before, self.capture("sf_happy", "runtime-after"))

    def pending_intent(self, database, label, *, explicit_id=None, created_at=None):
        columns = "account_id,created_by_user_id,channel_id,provider,merchant,environment,currency,amount_minor,credit_units,credits_per_minor,prepare_key,prepare_hash"
        values = "personal_account_id,id,7,'stripe','author-synthetic','test','USD',1,5000,5000," + literal(label) + ",decode(repeat('a',64),'hex')"
        override = ""
        if explicit_id is not None:
            columns = "id," + columns
            values = str(explicit_id) + "," + values
            override = " OVERRIDING SYSTEM VALUE"
        if created_at is not None:
            require(type(created_at) is int, "synthetic intent timestamp must be integer seconds")
            columns += ",created_at"
            values += ",to_timestamp(" + str(created_at) + ")"
        return int(self.sql("INSERT INTO core_payments.intents (" + columns + ")" + override +
            " SELECT " + values + " FROM core_identity.users WHERE id=11 RETURNING id", database,
            owner=True, label="synthetic-pending-intent"))

    def seal_floor(self):
        directory = self.clone("sf_floor")
        observed = json.loads(self.opening_path.read_text())["observation"]["observed_at"]
        first = self.pending_intent(directory.name, "author-before-source-seal", created_at=observed + 10)
        require(first > 0, "preexisting Native intent absent")
        authority = self.json_sql("SELECT jsonb_build_object('role',current_user,'session_user',session_user,"
            "'role_super',(SELECT rolsuper FROM pg_roles WHERE rolname=current_user),"
            "'session_super',(SELECT rolsuper FROM pg_roles WHERE rolname=session_user),"
            "'sequence_owner',(SELECT pg_get_userbyid(relowner) FROM pg_class WHERE oid='core_payments.intents_id_seq'::regclass),"
            "'sequence_select',has_sequence_privilege(current_user,'core_payments.intents_id_seq','SELECT'),"
            "'raw_sequence_function',pg_catalog.pg_sequence_last_value('core_payments.intents_id_seq'::regclass),"
            "'stored_last_value',(SELECT last_value FROM core_payments.intents_id_seq))", directory.name, owner=True)
        require(authority["role"] == authority["sequence_owner"] == "lmm_core_owner"
                and authority["session_user"] == "sf_installer" and not authority["role_super"]
                and not authority["session_super"] and authority["sequence_select"]
                and authority["raw_sequence_function"] == authority["stored_last_value"] >= first,
                "non-superuser owner cannot read the exact Native sequence upper")
        save(directory / "sequence-authority-before-seal.json", authority)
        plan, status = self.prepare(directory)
        material = json.loads((plan / "material.private.json").read_text())
        upper = material["snapshot"]["native_intent_upper"]
        require(upper >= first and status["source_seals"] == [], "first-seal snapshot evidence incorrect")
        require(len(material["plan"]["source"]) == 12 and "native_intent_floor" not in material["plan"]["source"],
                "caller source material supplied server floor")
        self.approve(plan)
        before = self.capture(directory.name, "floor-before-seal")
        _, status = self.apply(directory, plan)
        require(status["status"] == "COMMITTED", "Native floor first import failed")
        self.assert_preserved(before, self.capture(directory.name, "floor-after-seal"), allow_finance=True)
        floor = int(self.sql("SELECT native_intent_floor FROM core_history.finance_sources", directory.name))
        require(floor == upper == authority["raw_sequence_function"], "default floor did not capture actual owner sequence upper")
        second = self.pending_intent(directory.name, "author-after-source-seal", created_at=observed)
        require(second > floor, "new Native ID did not exceed immutable seal floor")
        replay, status = self.prepare(directory, "floor-replay-plan")
        require(status["status"] == "NOOP" and status["source_seals"] ==
                [{"source_key": material["plan"]["source"]["source_key"], "native_intent_floor": floor,
                  "native_request_ids": []}], "NOOP lost seal proof")
        replay_material = json.loads((replay / "material.private.json").read_text())
        require(replay_material["plan"] == material["plan"] and status["native_intent_upper"] >= second,
                "server floor altered original source key or plan")
        self.approve(replay)
        before = self.capture(directory.name, "floor-replay-before")
        _, status = self.apply(directory, replay, "floor-replay-apply")
        require(status["status"] == "NOOP", "sealed source did not replay")
        self.assert_preserved(before, self.capture(directory.name, "floor-replay-after"))
        require(int(self.sql("SELECT native_intent_floor FROM core_history.finance_sources", directory.name)) == floor,
                "NOOP recaptured a newer floor")
        save(directory / "seal-evidence.json", {"preexisting_intent_id": first, "sealed_floor": floor,
            "later_intent_id": second, "source_key": material["plan"]["source"]["source_key"],
            "dump_start": observed, "pre_seal_created_at": observed + 10, "post_seal_created_at": observed,
            "seal_did_not_advance_sequence": True, "noop_preserved_floor": True})

    def sequence_guards(self):
        faults = [("step_first", "sf_base", "ALTER SEQUENCE core_payments.intents_id_seq INCREMENT BY 2"),
            ("step_replay", "sf_floor", "ALTER SEQUENCE core_payments.intents_id_seq INCREMENT BY 2"),
            ("cycle_first", "sf_base", "ALTER SEQUENCE core_payments.intents_id_seq CYCLE"),
            ("cycle_replay", "sf_floor", "ALTER SEQUENCE core_payments.intents_id_seq CYCLE"),
            ("manual_id", "sf_base", None), ("rewind_first", "sf_base", None), ("rewind_replay", "sf_floor", None)]
        for name, template, mutation in faults:
            directory = self.clone("sf_guard_" + name, template)
            plan, _ = self.prepare(directory)
            self.approve(plan)
            if name == "manual_id":
                self.pending_intent(directory.name, "author-manual-out-of-order", explicit_id=100)
            elif name == "rewind_first":
                self.pending_intent(directory.name, "author-sequence-before-rewind")
                mutation = "SELECT setval('core_payments.intents_id_seq',1,false)"
            elif name == "rewind_replay":
                mutation = "SELECT setval('core_payments.intents_id_seq',1,true)"
            if mutation:
                self.sql(mutation, directory.name, owner=True, label="owner-only-sequence-fault")
            before = self.capture(directory.name, name + "-before")
            _, status = self.cli(directory, "snapshot", "invalid-sequence-snapshot",
                extra=("--runtime-role", "lmm_core_runtime"), expected=1)
            require(status["status"] == "REFUSED", "unsafe sequence accepted by read-only owner guard")
            out, status = self.apply(directory, plan, expected=2)
            require(status["status"] == "UNKNOWN" and "site_finance_native_intent_sequence_invalid" in
                    (out / "site-finance.stderr.private.txt").read_text(), "apply/replay skipped current sequence guard")
            self.assert_preserved(before, self.capture(directory.name, name + "-after"))

    def live_client_fence(self):
        directory = self.clone("sf_cached_client")
        self.sql("CREATE ROLE sf_cached NOLOGIN NOINHERIT; GRANT CONNECT ON DATABASE sf_cached_client TO sf_cached", directory.name)
        self.sql("GRANT USAGE ON SCHEMA core_payments TO sf_cached; GRANT USAGE ON SEQUENCE core_payments.intents_id_seq TO sf_cached; "
                 "ALTER SEQUENCE core_payments.intents_id_seq CACHE 16", directory.name, owner=True)
        require(self.pending_intent(directory.name, "author-before-cached-allocator") == 1, "first cached sequence ID")
        plan, _ = self.prepare(directory)
        self.approve(plan)
        self.sql("ALTER ROLE sf_cached LOGIN", directory.name)
        held = None
        try:
            with (directory / "held.stdout").open("w") as output, (directory / "held.stderr").open("w") as error:
                held = subprocess.Popen([str(x) for x in self.psql(directory.name, "sf_cached")], stdin=subprocess.PIPE,
                    stdout=output, stderr=error, text=True, env=self.env)
            held.stdin.write("BEGIN; SELECT jsonb_build_object('pid',pg_backend_pid(),'allocated',nextval('core_payments.intents_id_seq'));\n\\echo author_cache_ready\n")
            held.stdin.flush()
            deadline = time.monotonic() + 10
            while "author_cache_ready" not in (directory / "held.stdout").read_text():
                require(held.poll() is None and time.monotonic() < deadline, "cached allocator did not stay alive")
                time.sleep(0.05)
            allocated = json.loads((directory / "held.stdout").read_text().splitlines()[0])
            require(allocated["allocated"] == 17, "CACHE 16 did not reserve next range")
            # NOLOGIN prevents new connections, but must not disguise this one.
            self.sql("ALTER ROLE sf_cached NOLOGIN", directory.name)
            visible = self.json_sql("SELECT jsonb_build_object('viewer',current_user,'pid',pid,'datid',datid,"
                "'backend_type',backend_type,'state',state,'read_all_stats',pg_has_role(current_user,'pg_read_all_stats','MEMBER')) "
                "FROM pg_stat_activity WHERE pid=" + str(allocated["pid"]), directory.name, owner=True)
            require(visible["viewer"] == "lmm_core_owner" and visible["pid"] == allocated["pid"]
                    and visible["datid"] is not None and visible["backend_type"] is None and not visible["read_all_stats"],
                    "ordinary owner masked-backend fixture not reproduced")
            save(directory / "ordinary-owner-visible-state.json", visible)
            before = self.capture(directory.name, "live-cache-before-denial")
            _, status = self.cli(directory, "snapshot", "live-cache-snapshot", extra=("--runtime-role", "lmm_core_runtime"), expected=1)
            require(status["status"] == "REFUSED", "live cached backend accepted by snapshot")
            out, status = self.apply(directory, plan, "live-cache-apply", expected=2)
            require(status["status"] == "UNKNOWN" and "site_finance_other_clients" in
                    (out / "site-finance.stderr.private.txt").read_text(), "live cached backend accepted by owner apply")
            self.assert_preserved(before, self.capture(directory.name, "live-cache-after-denial"))
        finally:
            if held is not None and held.poll() is None:
                held.stdin.write("ROLLBACK;\n\\q\n")
                held.stdin.flush()
                held.stdin.close()
                held.wait(timeout=5)
            self.sql("ALTER ROLE sf_cached NOLOGIN", directory.name)
        require(self.sql("SELECT pg_sequence_last_value('core_payments.intents_id_seq')", directory.name) == "32",
                "closing allocator returned cached IDs")
        self.sql("ALTER SEQUENCE core_payments.intents_id_seq CACHE 1", directory.name, owner=True)
        plan, _ = self.prepare(directory, "drained-plan")
        self.approve(plan)
        before = self.capture(directory.name, "drained-before-seal")
        _, status = self.apply(directory, plan, "drained-apply")
        require(status["status"] == "COMMITTED", "drained owner could not seal")
        self.assert_preserved(before, self.capture(directory.name, "drained-after-seal"), allow_finance=True)
        require(self.sql("SELECT native_intent_floor FROM core_history.finance_sources", directory.name) == "32", "cached upper not sealed")
        require(self.pending_intent(directory.name, "author-after-cached-session-close") == 33, "new allocator reused sealed IDs")

    def reader_admin(self, directory):
        url = "postgresql://sf_installer@localhost/" + directory.name + "?" + urlencode({
            "host": str(self.socket), "port": self.port, "sslmode": "disable", "options": "-c role=lmm_core_owner"})
        (directory / "reader-owner.url").write_text(url)
        config = directory / "reader-owner.toml"
        config.write_text("version=1\n[database]\nurl_file=" + json.dumps(str(directory / "reader-owner.url")) + "\n")
        # This is a genuine Native bootstrap/audit for a user absent from the
        # complete legacy archive. It does not use any independent QA fixture.
        issued = self.command("bootstrap-reader", [self.root / "lmm-core", "--config", config, "bootstrap-user"],
            data=json.dumps({"user_id": 17, "platform_level": 5}))
        require(self.sql("SELECT count(*) FROM core_identity.audit WHERE actor_user_id=17 AND action='user.bootstrap' "
            "AND target->>'user_id'='17'", directory.name) == "1", "genuine Native user audit absent")
        # The canonical grant script validates an actual LOGIN runtime. Drain it
        # again before any owner snapshot/apply; no runtime session starts here.
        self.sql("ALTER ROLE lmm_core_runtime LOGIN", directory.name)
        try:
            self.command("reader-runtime-grants", self.psql(directory.name) + ["-v", "enable_models=true",
                "-f", self.root / "source/deployment/postgres/grant-core-runtime.sql"])
        finally:
            self.sql("ALTER ROLE lmm_core_runtime NOLOGIN", directory.name)
        return json.loads(issued.stdout)["secret"]

    def charging_request(self, database, request_id, actor, authorized_at, *, settled=None):
        key = int(self.sql("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) "
            "SELECT sha256(convert_to(" + literal("author-key:" + database + ":" + request_id) + ",'UTF8')),'api_key',id,auth_version,"
            "personal_account_id,clock_timestamp()+interval '1 day' FROM core_identity.users WHERE id=" + str(actor) + " RETURNING id",
            database, owner=True, label="synthetic-request-key"))
        self.sql("BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on; "
            "INSERT INTO core_charging.requests(id,actor_user_id,key_id,owner_account_id,payer_account_id,"
            "fingerprint,worker_digest,spec_digest,price,authorized_at,lease_until,hard_deadline,state,reserved) "
            "SELECT " + literal(request_id) + ",id," + str(key) + ",personal_account_id,personal_account_id,"
            "decode(repeat('a',64),'hex'),decode(repeat('b',64),'hex'),decode(repeat('c',64),'hex'),'{}'::jsonb,"
            + str(authorized_at) + "," + str(authorized_at + 86400) + "," + str(authorized_at + 172800) + ","
            "'reserved',100 FROM core_identity.users WHERE id=" + str(actor) + "; COMMIT;", database, owner=True, label="synthetic-pending-request")
        if settled is not None:
            self.settle_request(database, request_id, settled)

    def settle_request(self, database, request_id, credits):
        self.sql("BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on; "
            "UPDATE core_charging.requests SET state='streaming',revision=revision+1 WHERE id=" + literal(request_id) + ";"
            "UPDATE core_charging.requests SET state='settled',settled=" + str(credits) + ",outcome='synthetic-success',revision=revision+1 WHERE id="
            + literal(request_id) + "; COMMIT;", database, owner=True, label="synthetic-request-terminal-transition")

    def reader_http(self, directory, secret, label, *, expected_status, expected_used=None):
        self.sql("ALTER ROLE lmm_core_runtime LOGIN; GRANT CONNECT ON DATABASE " + identifier(directory.name) +
            " TO lmm_core_runtime", directory.name, label="reader-role-activate")
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        url = "postgresql://lmm_core_runtime@localhost/" + directory.name + "?" + urlencode({
            "host": str(self.socket), "port": self.port, "sslmode": "disable"})
        (directory / "reader-runtime.url").write_text(url)
        config = directory / "reader-runtime.toml"
        config.write_text("version=1\n[server]\nlisten='127.0.0.1:" + str(port) + "'\nworker_threads=1\nmax_inflight=8\n"
            "drain_timeout_seconds=3\n[database]\nmax_connections=2\nurl_file=" + json.dumps(str(directory / "reader-runtime.url")) + "\n")
        command = [str(self.root / "lmm-core"), "--config", str(config), "serve"]
        save(directory / (label + "-native-command.json"), command)
        process = None
        try:
            with (directory / (label + "-native.log")).open("wb") as log:
                process = subprocess.Popen(command, env=self.env, stdout=log, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                require(process.poll() is None, "Native reader exited before listening")
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                        break
                except OSError:
                    time.sleep(0.1)
            else:
                raise AssertionError("Native reader did not listen")
            require(sha(Path(f"/proc/{process.pid}/exe")) == self.binding["core_sha256"], "actual reader MainPID is not frozen Native")
            before = self.capture(directory.name, label + "-before")
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=15)
            try:
                connection.request("GET", "/api/finance/site-statistics", headers={"Authorization": "Bearer " + secret,
                    "Origin": "http://127.0.0.1:" + str(port)})
                response = connection.getresponse()
                body = json.loads(response.read())
                status = response.status
            finally:
                connection.close()
            save(directory / (label + "-http.json"), {"status": status, "body": body, "frozen_native_sha256": self.binding["core_sha256"]})
            require(status == expected_status, label + ": unexpected Native reader status")
            if expected_status == 200:
                require(body["success"] is True and body["data"]["total_used_credits"] == expected_used,
                    label + ": exact hand-calculated Native usage mismatch")
            else:
                require(body.get("code") == "SITE_STATISTICS_SOURCE_UNAVAILABLE" and not body.get("success")
                        and "data" not in body, label + ": overlapping usage must remain unavailable")
            self.assert_preserved(before, self.capture(directory.name, label + "-after"))
        finally:
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
            self.sql("ALTER ROLE lmm_core_runtime NOLOGIN", directory.name, label="reader-role-drained")

    def request_seal(self):
        directory = self.clone("sf_requests")
        secret = self.reader_admin(directory)
        observed = json.loads(self.opening_path.read_text())["observation"]["observed_at"]
        old_native, old_historical, new_historical = "A-native/settled", "z-历史.pending", "0-post/same-second"
        self.charging_request(directory.name, old_native, 17, observed + 10, settled=13)
        self.charging_request(directory.name, old_historical, 11, observed + 10)
        sealed = [old_native, old_historical]
        plan, status = self.prepare(directory)
        require(status["native_request_ids"] == sealed, "pre-seal current request inventory incomplete")
        self.approve(plan)
        before = self.capture(directory.name, "requests-before-seal")
        _, status = self.apply(directory, plan)
        require(status["status"] == "COMMITTED", "request seal import failed")
        self.assert_preserved(before, self.capture(directory.name, "requests-after-seal"), allow_finance=True)
        require(self.json_sql("SELECT to_jsonb(native_request_ids) FROM core_history.finance_sources", directory.name) == sealed,
                "server seal did not preserve exact original request IDs")
        # Archive baseline 770, genuine Native-only user's pre-seal usage 13.
        self.reader_http(directory, secret, "native-only-sealed-usage", expected_status=200, expected_used="783")
        self.charging_request(directory.name, new_historical, 11, observed, settled=17)
        replay, status = self.prepare(directory, "requests-replay-plan")
        require(status["status"] == "NOOP" and status["source_seals"][0]["native_request_ids"] == sealed,
                "new post-seal request changed the original seal")
        self.approve(replay)
        before = self.capture(directory.name, "requests-noop-before")
        _, status = self.apply(directory, replay, "requests-noop-apply")
        require(status["status"] == "NOOP", "request seal replay did not NOOP")
        self.assert_preserved(before, self.capture(directory.name, "requests-noop-after"))
        # The new historical user's request is post-seal despite the same second.
        self.reader_http(directory, secret, "post-seal-same-second", expected_status=200, expected_used="800")
        overlap = self.clone("sf_requests_overlap", directory.name)
        self.settle_request(overlap.name, old_historical, 11)
        plan, status = self.prepare(overlap)
        require(status["status"] == "NOOP" and status["source_seals"][0]["native_request_ids"] == sealed,
                "settlement recaptured or erased a pre-seal ID")
        self.approve(plan)
        before = self.capture(overlap.name, "overlap-noop-before")
        _, status = self.apply(overlap, plan)
        require(status["status"] == "NOOP", "overlap metadata must remain immutable")
        self.assert_preserved(before, self.capture(overlap.name, "overlap-noop-after"))
        self.reader_http(overlap, secret, "pre-seal-later-settled", expected_status=503)
        save(directory / "request-seal-evidence.json", {"sealed_ids": sealed, "new_id": new_historical,
            "old_authorized_at": observed + 10, "new_authorized_at": observed,
            "native_only_actor": 17, "historical_actor": 11,
            "expected_used_before_new": "783", "expected_used_after_new": "800", "expected_overlap_http": 503,
            "no_importer_request_or_sequence_mutation": True})

    def execute(self):
        success = False
        try:
            actions = dict(zip(GROUPS, (self.setup, self.source, self.happy, self.noop, self.other_prepare,
                self.other_apply, self.rollback, self.unknown, self.raw_change, self.runtime_denied,
                self.seal_floor, self.sequence_guards, self.request_seal, self.live_client_fence)))
            for name in self.selected:
                self.group(name, actions[name])
            verify_binding(self.root)
            success = True
        except BaseException:
            (self.root / "failure.private.txt").write_text(traceback.format_exc())
        finally:
            if self.pg:
                result = self.command("pg-stop", ["/usr/bin/pg_ctl", "-D", self.root / "pg", "-m", "fast", "-w", "stop"], expected=None)
                require(result.returncode == 0, "test PG did not stop; preserve root for recovery")
            if success:
                removed = {}
                for name in ("pg", "s", "lmm-core"):
                    path = self.root / name
                    removed[name] = sum(x.stat().st_size for x in path.rglob("*") if x.is_file()) if path.is_dir() else path.stat().st_size
                    shutil.rmtree(path) if path.is_dir() else path.unlink()
                save(self.root / "cleanup.json", {"logical_bytes_removed": sum(removed.values()), "paths": removed,
                    "pg_stopped": True, "logs_and_single_source_archive_retained": True})
                for path in self.logs.glob("*-bootstrap-reader.stdout"):
                    digest = sha(path)
                    save(path, {"synthetic_one_time_credential_removed": True, "original_sha256": digest})
            save(self.root / "result.json", {"status": "PASS" if success else "FAIL", "groups": self.results,
                "scope": "author synthetic full archive + CLI + real PostgreSQL; not independent reader or upgrade acceptance",
                "test_selection": self.binding["test_selection"],
                "core_sha256": self.binding["core_sha256"], "writer_sha256": self.binding["writer_sha256"]})
        return 0 if success else 1


def worker(root):
    with Path("/tmp/lmm-delegate/heavy-build.lock").open("a") as lock:
        (root / "waiting-for-lock").write_text(str(os.getpid()))
        fcntl.flock(lock, fcntl.LOCK_EX)
        (root / "waiting-for-lock").unlink()
        memory = Path("/proc/meminfo").read_text()
        available = int(next(line.split()[1] for line in memory.splitlines() if line.startswith("MemAvailable:"))) * 1024
        group = next(line.split(":", 2)[2].strip() for line in Path("/proc/self/cgroup").read_text().splitlines() if line.startswith("0::"))
        control = Path("/sys/fs/cgroup") / group.lstrip("/")
        limits = {name: (control / name).read_text().strip() for name in ("memory.max", "memory.high", "memory.swap.max", "cpu.max")}
        save(root / "resources.json", {"mem_available_bytes": available, "cgroup": group, "limits": limits})
        require(available >= 2 * 1024**3, "2GiB memory admission failed")
        require(limits == {"memory.max": "2147483648", "memory.high": "1610612736", "memory.swap.max": "4294967296", "cpu.max": "100000 100000"}, "required resource limits missing")
        try:
            return Run(root).execute()
        finally:
            save(root / "resource-usage.json", {name: (control / name).read_text().strip() for name in
                ("memory.peak", "memory.events", "memory.swap.peak", "cpu.stat") if (control / name).exists()})


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--scope", choices=tuple(TEST_GROUPS), default="seal")
    parser.add_argument("--worker", action="store_true")
    args = parser.parse_args()
    root = args.root.absolute()
    if args.worker:
        return worker(root)
    require(args.execute and args.candidate, "explicit --execute and final candidate required")
    candidate = args.candidate.resolve()
    result = json.loads((candidate / "result.json").read_text())
    binaries = json.loads((candidate / "binaries.json").read_text())
    require(result.get("status") == "passed" and result.get("exit_code") == 0 and result.get("source_unchanged") is True,
            "final Native build must PASS before PG startup")
    require(candidate.name not in ("compile-r1", "compile-r2", "compile-r3", "compile-r4", "compile-r5"),
            "final ID-only seal-floor R6 or later required")
    expected = binaries["lmm-core"]["sha256"]
    require(sha(candidate / "lmm-core") == expected, "binaries.json/actual Native mismatch")
    pins = json.loads((candidate / "source-pins.json").read_text())
    require(all(sha(candidate / "source" / name) == digest for name, digest in pins.items()), "candidate source pins changed")
    require("native_intent_floor BIGINT" in (candidate / "source/services/core/schema/site_statistics.sql").read_text(),
            "candidate lacks the server-generated seal floor")
    require("native_request_ids TEXT[] NOT NULL" in (candidate / "source/services/core/schema/site_statistics.sql").read_text(),
            "candidate lacks the exact opaque request seal")
    require(root.parent == Path.home() / "Backups" and not root.exists(), "fresh short private backup root required")
    require(len(str(root / "s/.s.PGSQL.55613").encode()) < 104, "Unix socket path too long")
    require(shutil.disk_usage(root.parent).free >= 2 * 1024**3, "disk admission failed; run space-first")
    root.mkdir(mode=0o700)
    (root / "runner").mkdir(mode=0o700)
    for name in ("importer_selftest.py", "importer_fixture.py"):
        shutil.copy2(Path(__file__).parent / name, root / "runner" / name)
    for name in PYTHON_FILES:
        target = root / "source/scripts" / name
        target.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        shutil.copy2(REPO / "scripts" / name, target)
    for name in CANDIDATE_FILES:
        require(name in pins, "required file not pinned in candidate: " + name)
        target = root / "source" / name
        target.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        shutil.copy2(candidate / "source" / name, target)
    frozen = {str(path.relative_to(root)): sha(path) for path in root.rglob("*") if path.is_file()}
    save(root / "binding.json", {"candidate": str(candidate), "core_sha256": expected,
        "test_selection": args.scope,
        "writer_sha256": sha(root / "source/scripts/legacy_site_statistics.py"), "frozen_files": frozen,
        "programs": {"/usr/bin/" + name: sha("/usr/bin/" + name) for name in PG_PROGRAMS},
        "candidate_source_pin_count": len(pins), "core_recompiled": False})
    unit = "lmm-site-importer-" + secrets.token_hex(4) + ".service"
    command = ["systemd-run", "--user", "--unit=" + unit, "--wait", "--pipe", "--service-type=exec",
        "--property=MemoryAccounting=yes", "--property=MemoryMax=2G", "--property=MemoryHigh=1536M",
        "--property=MemorySwapMax=4G", "--property=CPUQuota=100%", "--property=TasksMax=256",
        "--property=TimeoutStartSec=1200", "/usr/bin/python3", "-B", str(root / "runner/importer_selftest.py"), "--root", str(root), "--worker"]
    save(root / "launch.json", {"argv": command, "unit": unit})
    with (root / "unit.log").open("w") as log:
        code = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, env=clean_environment()).returncode
    save(root / "launch-result.json", {"exit_code": code})
    if not (root / "result.json").exists():
        save(root / "result.json", {"status": "NOT_RUN", "groups": {name: "NOT_RUN" for name in TEST_GROUPS[args.scope]},
                                    "reason": "worker startup or admission failed; inspect unit.log"})
    print(json.dumps({"root": str(root), "exit_code": code, "result": str(root / "result.json")}))
    return code


if __name__ == "__main__":
    sys.exit(main())

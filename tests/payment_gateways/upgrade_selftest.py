#!/usr/bin/env python3
"""Synthetic PG18 28->29 upgrade CLI checks in uniquely named disposable DBs.

The predecessor is reconstructed from pinned Git source, not an old Native.
Requires an already running, disposable local PostgreSQL cluster. Never opens a
service or uses payment credentials. Databases and evidence are retained.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
from urllib.parse import urlencode

ROOT = Path(__file__).resolve().parents[2]
PRE_SOURCE = "6523c4b4b831643d69de3874c2e35933c91bfc2b"
PRE_MANIFEST = "207bc6bf3f39dc9803177e6cebda491150fb66370a5ff3fc37b26b863cfec97b"
PRE_CATALOG = "5521fb8224fbdfc994421f0fe04756df99a4e341ceec78b6302882a1f145d2d0"
META_FILES = [
    "services/core/schema/core_upgrade.sql", "services/core/schema/component_upgrade.sql",
    "services/core/schema/ledger_revision.sql", "services/core/src/core_schema/upgrade/console_receipt.sql",
    "services/core/schema/offline_upgrade.sql",
]
NAMES = "n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'"


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def sha(value):
    return hashlib.sha256(value).hexdigest()


def file_sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def framed(rows):
    h = hashlib.sha256()
    for row in rows:
        data = row.encode()
        h.update(len(data).to_bytes(8, "big"))
        h.update(data)
    return h.hexdigest()


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


class Test:
    def __init__(self, args):
        self.args = args
        self.out = args.output.resolve()
        self.out.mkdir(mode=0o700, parents=True, exist_ok=False)
        # Parent builds may continue in their own window. Every invocation in
        # this run must use the same observed executable, including check-db.
        source_sha = file_sha(args.core)
        shutil.copyfile(args.core, self.out / "lmm-core")
        args.core = self.out / "lmm-core"
        args.core.chmod(0o700)
        require(file_sha(args.core) == source_sha, "Native changed while freezing test executable")
        self.prefix = "pgup_" + secrets.token_hex(6)
        self.roles = {name: self.prefix + "_" + name for name in ("installer", "owner", "runtime", "extensions")}
        self.shadow = self.prefix + "_shadow"
        self.base = self.prefix + "_base"
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(("LMM_CORE_", "PG"))}
        self.env.update(TZ="UTC", LC_ALL="C")
        if args.library_path:
            self.env["LD_LIBRARY_PATH"] = args.library_path
        self.results = {}

    def save(self, name, value):
        path = self.out / name
        path.write_bytes(value if isinstance(value, bytes) else encoded(value))
        path.chmod(0o600)
        return path

    def run(self, argv, body=b"", name=None, ok=True):
        result = subprocess.run([str(a) for a in argv], input=body, capture_output=True,
                                env=self.env, cwd=ROOT, timeout=60)
        if name:
            self.save(name + ".stdout", result.stdout)
            self.save(name + ".stderr", result.stderr)
        if ok:
            require(result.returncode == 0, f"{name or argv[0]} failed: {result.stderr.decode(errors='replace')[-2000:]}")
        return result

    def old(self, path):
        return self.run(["git", "show", PRE_SOURCE + ":" + path]).stdout

    def sql(self, query, db="postgres", user=None, ok=True, name=None):
        result = self.run([self.args.psql, "-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-h",
                           self.args.host, "-p", self.args.port, "-U", user or self.args.admin,
                           "-d", db], query.encode(), name=name, ok=ok)
        return result.stdout.decode().strip() if ok else result

    def mark(self, name, detail=True):
        self.results[name] = detail
        self.save("progress.json", self.results)
        print("PASS", name, flush=True)

    def config(self, db, user, name, console=False, owner=False):
        params = {"host": self.args.host, "port": self.args.port, "sslmode": "disable"}
        if owner:
            params["options"] = "-c role=" + self.roles["owner"]
        url = f"postgresql://{user}@localhost/{db}?" + urlencode(params)
        secret = self.save(name + ".url", url.encode())
        text = f"version=1\n[database]\nurl_file={json.dumps(str(secret))}\nmax_connections=1\n"
        if console:
            text += "[console]\nenabled=true\n"
        return self.save(name + ".toml", text.encode())

    def bind(self):
        raw = self.run([self.args.core, "schema-manifest"], name="schema-manifest").stdout
        self.manifest = json.loads(raw)
        self.target = json.loads((ROOT / "services/core/src/core_schema/upgrade/payment_gateways_target.json").read_text())
        require(sha(raw) == self.target["manifest_sha256"], "Native manifest does not match compiled target")
        require(len(self.manifest["components"]) == 29 and self.manifest["components"][-1]["name"] == "payment-gateways", "wrong Native components")
        predecessor = copy.deepcopy(self.manifest)
        predecessor["components"].pop()
        require(sha(encoded(predecessor)) == PRE_MANIFEST, "predecessor manifest mismatch")
        self.cat = predecessor["catalog"]["query_utf8"]
        self.ddl = []
        for component in predecessor["components"]:
            stem = "site_statistics" if component["name"] == "site-finance" else component["name"].replace("-", "_")
            ddl = self.old("services/core/schema/" + stem + ".sql")
            if component["name"] == "trust-pricing":
                # The actual Rust SCHEMA is concat!(base, "\n", image delta).
                ddl += b"\n" + self.old("services/core/schema/image_trust_extension.sql")
            require(sha(ddl) == component["raw_schema_sha256"], "predecessor source drift: " + component["name"])
            self.ddl.append(ddl.decode())
        schema = self.old("services/core/src/core_schema.rs").decode()
        self.contract = json.loads('"' + schema.split('const CONTRACT: &str = "', 1)[1].split('";', 1)[0] + '"')
        self.metadata = [self.old(p).decode() for p in META_FILES]
        require(sha((self.contract + "".join(self.metadata)).encode()) == predecessor["installer_contract_sql_sha256"], "predecessor installer metadata drift")
        grants = self.old("services/core/src/identity/grants.rs").decode()
        self.oauth_cat = grants.split('const CATALOG: &str = r#"', 1)[1].split('"#;', 1)[0]
        self.pre = predecessor
        self.save("binding.json", {"scope": "synthetic-source-derived-PG18-predecessor; actual-new-Native-CLI; no-production",
            "predecessor_source": PRE_SOURCE, "core_sha256": file_sha(self.args.core),
            "script_sha256": sha(Path(__file__).read_bytes()), "predecessor_manifest_sha256": PRE_MANIFEST,
            "target_manifest_sha256": sha(raw), "target_catalog_sha256": self.target["catalog_sha256"],
            "prefix": self.prefix, "roles": self.roles})
        self.mark("native_and_source_binding")

    def setup(self):
        statements = []
        for name, role in self.roles.items():
            login = "LOGIN" if name in ("installer", "runtime") else "NOLOGIN"
            statements.append(f"CREATE ROLE {role} {login} NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;")
        statements += [f"CREATE ROLE {self.shadow} NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;",
            f"GRANT {self.roles['owner']} TO {self.roles['installer']} WITH INHERIT FALSE, SET TRUE;"]
        self.sql("\n".join(statements))
        self.sql(f"CREATE DATABASE {self.base} OWNER {self.roles['owner']} TEMPLATE template0 ENCODING 'UTF8';")
        self.fence(self.base)
        self.sql(f"REVOKE CREATE ON SCHEMA public FROM PUBLIC; GRANT USAGE,CREATE ON SCHEMA public TO {self.roles['owner']}; "
            f"REVOKE EXECUTE ON FUNCTION pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_control_system() TO {self.roles['owner']};", self.base)
        self.sql(f"SET ROLE {self.roles['owner']}; BEGIN;\n" + "\n".join(self.ddl) + self.contract + "\n" + "\n".join(self.metadata) + "\nCOMMIT;",
                 self.base, self.roles["installer"], name="install-predecessor")
        defs = self.sql(self.cat, self.base).splitlines()
        require(framed(defs) == PRE_CATALOG and len(defs) == 7035, "predecessor catalog mismatch")
        oauth = framed(self.sql(self.oauth_cat, self.base).splitlines())
        statements = [f"INSERT INTO core_meta.schema_contract(version,fingerprint) VALUES({self.pre['identity']['version']},decode('{self.pre['identity']['fingerprint_sha256']}','hex'));",
            f"INSERT INTO core_oauth.contract(version,ddl_fingerprint,catalog_fingerprint) VALUES(1,decode('{sha(self.old('services/core/schema/oauth_grants.sql'))}','hex'),decode('{oauth}','hex'));",
            f"INSERT INTO core_meta.runtime_contract(catalog_fingerprint) VALUES(decode('{PRE_CATALOG}','hex'));"]
        statements += [f"INSERT INTO core_meta.component_contract(name,fingerprint) VALUES('{c['name']}',decode('{c['raw_schema_sha256']}','hex'));" for c in self.pre["components"]]
        statements += ["WITH a AS (INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id) INSERT INTO core_identity.users(personal_account_id) SELECT id FROM a;",
            "INSERT INTO core_identity.user_profiles(user_id,username,display_name,email,password_login_allowed) SELECT id,'pgup_fixture','Synthetic upgrade account','fixture@example.invalid',false FROM core_identity.users;",
            "INSERT INTO core_payments.intents(account_id,created_by_user_id,channel_id,provider,merchant,environment,currency,amount_minor,credit_units,credits_per_minor,prepare_key,prepare_hash) SELECT personal_account_id,id,101,'stripe','acct_upgrade_fixture','test','USD',123,615000,5000,'synthetic-prepare-key-0001',decode(repeat('42',32),'hex') FROM core_identity.users;",
            "INSERT INTO core_console.options(key,value) VALUES('Notice','payment-upgrade-synthetic-preservation');"]
        self.sql(f"SET ROLE {self.roles['owner']}; BEGIN;" + "\n".join(statements) + "COMMIT;", self.base, self.roles["installer"])
        grants = self.old("deployment/postgres/grant-core-runtime.sql").decode().replace("'lmm_core_owner'", "'" + self.roles["owner"] + "'")
        settings = f"\\set runtime_role {self.roles['runtime']}\n\\set enable_console true\n\\set enable_web_identity true\n\\set enable_payments true\n"
        self.sql(settings + grants, self.base, name="canonical-runtime-grants")
        self.sql(f"ALTER ROLE {self.roles['runtime']} NOLOGIN;")
        self.fence(self.base)
        self.mark("source_28_predecessor", {"tables": 175, "sequences": 22, "catalog": PRE_CATALOG})

    def fence(self, db):
        self.sql(f"REVOKE CONNECT,CREATE,TEMPORARY ON DATABASE {db} FROM PUBLIC; REVOKE CONNECT ON DATABASE {db} FROM {self.roles['runtime']},{self.roles['extensions']}; GRANT CONNECT ON DATABASE {db} TO {self.roles['installer']};")

    def clone(self, suffix):
        db = self.prefix + "_" + suffix
        self.sql(f"CREATE DATABASE {db} OWNER {self.roles['owner']} TEMPLATE {self.base};")
        self.fence(db)
        return db

    def request(self, db):
        identity = json.loads(self.sql("SELECT json_build_object('name',current_database(),'oid',(SELECT oid::bigint FROM pg_database WHERE datname=current_database()),'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),'server_version_num',current_setting('server_version_num')::int)", db))
        return {"version": 1, "procedure": "core-payment-gateways-v1", "upgrade_id": "payment-upgrade-" + secrets.token_hex(12),
            "database": identity, "roles": self.roles.copy(), "predecessor_manifest_sha256": PRE_MANIFEST,
            "predecessor_catalog_sha256": PRE_CATALOG, "successor_manifest_sha256": self.target["manifest_sha256"]}

    def observe(self, db, names=None, include_contracts=False):
        if names is None:
            tables = self.sql(f"SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {NAMES} AND c.relkind IN ('r','p') ORDER BY n.nspname COLLATE \"C\",c.relname COLLATE \"C\"", db).splitlines()
            sequences = self.sql(f"SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {NAMES} AND c.relkind='S' ORDER BY n.nspname COLLATE \"C\",c.relname COLLATE \"C\"", db).splitlines()
            rels = self.sql(f"SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {NAMES} ORDER BY c.oid", db).splitlines()
            types = self.sql(f"SELECT t.oid FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE {NAMES} ORDER BY t.oid", db).splitlines()
            names = (tables, sequences, rels, types)
        tables, sequences, rels, types = names
        statements = []
        for table in tables:
            if include_contracts or table not in ("core_meta.component_contract", "core_meta.runtime_contract"):
                statements.append(f"SELECT json_build_array('table','{table}',coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb))::text FROM {table} t;")
        statements += [f"SELECT json_build_array('sequence','{seq}',last_value,log_cnt,is_called)::text FROM {seq};" for seq in sequences]
        rows = self.sql("SET timezone='UTC';\n" + "\n".join(statements), db)
        relset, typeset = ",".join(str(int(x)) for x in rels), ",".join(str(int(x)) for x in types)
        acl = self.sql(f"""WITH ns AS (SELECT * FROM pg_namespace n WHERE {NAMES}), facts AS (
 SELECT json_build_array('schema',nspname,nspowner,nspacl)::text f FROM ns
 UNION ALL SELECT json_build_array('relation',n.nspname,c.relname,c.relowner,c.relacl)::text FROM pg_class c JOIN ns n ON n.oid=c.relnamespace WHERE c.oid IN ({relset})
 UNION ALL SELECT json_build_array('column',n.nspname,c.relname,a.attnum,a.attacl)::text FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN ns n ON n.oid=c.relnamespace WHERE c.oid IN ({relset}) AND a.attnum>0 AND NOT a.attisdropped
 UNION ALL SELECT json_build_array('function',n.nspname,p.oid::regprocedure::text,p.proowner,p.proacl)::text FROM pg_proc p JOIN ns n ON n.oid=p.pronamespace
 UNION ALL SELECT json_build_array('type',n.nspname,t.typname,t.typowner,t.typacl)::text FROM pg_type t JOIN ns n ON n.oid=t.typnamespace WHERE t.oid IN ({typeset})
 UNION ALL SELECT json_build_array('default',defaclrole,defaclnamespace,defaclobjtype,defaclacl)::text FROM pg_default_acl
 UNION ALL SELECT json_build_array('database',datname,datdba,datacl,datallowconn,datconnlimit)::text FROM pg_database WHERE datname=current_database()
 UNION ALL SELECT json_build_array('role',oid,rolname,rolsuper,rolinherit,rolcreaterole,rolcreatedb,rolcanlogin,rolreplication,rolbypassrls,rolconnlimit,rolvaliduntil,rolconfig)::text FROM pg_roles
 UNION ALL SELECT json_build_array('membership',roleid,member,grantor,admin_option,inherit_option,set_option)::text FROM pg_auth_members)
 SELECT f FROM facts ORDER BY f COLLATE "C"
""", db)
        return {"data_sha256": sha(rows.encode()), "permissions_sha256": sha(acl.encode()), "tables": len(tables), "sequences": len(sequences)}, names

    def cli(self, name, req, expected=None):
        db = req["database"]["name"]
        config = self.config(db, self.roles["installer"], name)
        self.save(name + ".request.json", req)
        result = self.run([self.args.core, "--config", config, "upgrade-db"], encoded(req), name, ok=False)
        try:
            report = json.loads(result.stdout)
        except ValueError as error:
            raise AssertionError(f"{name} gave no JSON: {result.stderr.decode(errors='replace')}") from error
        require((result.returncode == 0) == (expected is None), f"{name}: {report}")
        if expected:
            require(report.get("code") == expected, f"{name}: {report}")
        return report

    def negative(self, suffix, mutation, code):
        db = self.clone(suffix)
        self.sql(mutation, db)
        before, names = self.observe(db, include_contracts=True)
        catalog = self.sql(self.cat, db)
        self.cli(suffix, self.request(db), code)
        after, _ = self.observe(db, names, include_contracts=True)
        require(before == after and self.sql(self.cat, db) == catalog, suffix + " mutated predecessor")
        require(self.sql("SELECT to_regclass('core_meta.payment_gateway_upgrades') IS NULL", db) == "t", suffix + " left receipt")
        self.mark(suffix, {"code": code, "preserved": before})

    def upgrade(self):
        db = self.clone("main")
        before, names = self.observe(db)
        require((before["tables"], before["sequences"]) == (175, 22), "wrong predecessor object counts")
        self.req = self.request(db)
        report = self.cli("normal-upgrade", self.req)
        require(report["status"] == "committed" and report["components"] == 29, "upgrade did not commit")
        after, _ = self.observe(db, names)
        require(before == after, "old data, sequence state or permissions changed")
        require(self.sql(self.cat, db).splitlines() == self.target["definitions"], "upgraded catalog differs from fresh target")
        receipt = self.sql("SELECT jsonb_agg(to_jsonb(r))::text FROM core_meta.payment_gateway_upgrades r", db)
        require(self.sql("SELECT count(*) FROM core_meta.component_contract", db) == "29", "component receipt count")
        self.mark("normal_upgrade_and_preservation", {"report": report, "preserved": before})
        replay = self.cli("original-request-replay", self.req)
        require(replay["status"] == "already_committed" and replay["request_sha256"] == report["request_sha256"], "wrong replay identity")
        changed = copy.deepcopy(self.req)
        changed["roles"]["runtime"] = self.shadow
        self.cli("original-id-changed-request", changed, "PAYMENT_GATEWAYS_RECEIPT_CONFLICT")
        changed = copy.deepcopy(self.req)
        changed["upgrade_id"] += "-other"
        self.cli("new-id-after-commit", changed, "PAYMENT_GATEWAYS_ORIGINAL_RECEIPT_ABSENT")
        require(self.observe(db, names)[0] == before and self.sql("SELECT jsonb_agg(to_jsonb(r))::text FROM core_meta.payment_gateway_upgrades r", db) == receipt, "replay/conflict changed data or receipt")
        immutable = self.sql("UPDATE core_meta.payment_gateway_upgrades SET request='{}';", db, ok=False, name="immutable-receipt")
        require(immutable.returncode != 0, "receipt mutation accepted")
        require(self.sql(f"SELECT NOT has_table_privilege('{self.roles['runtime']}','core_meta.payment_gateway_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN') AND NOT has_any_column_privilege('{self.roles['runtime']}','core_meta.payment_gateway_upgrades','SELECT,INSERT,UPDATE,REFERENCES')", db) == "t", "runtime can access receipt")
        self.mark("original_replay_and_receipt_conflict")
        return db

    def runtime(self, db):
        self.sql(f"ALTER ROLE {self.roles['runtime']} LOGIN; GRANT CONNECT ON DATABASE {db} TO {self.roles['runtime']};")
        config = self.config(db, self.roles["runtime"], "runtime", console=True)
        def check(name, ok):
            result = self.run([self.args.core, "--config", config, "check-db"], name=name, ok=False)
            report = json.loads(result.stdout)
            require((result.returncode == 0) == ok, f"{name}: {report}")
            if not ok:
                require(report.get("code") == "CHECK_DB_RUNTIME_UNSAFE", f"{name}: {report}")
            return report
        check("runtime-check", True)
        for grant, revoke, label in [("SELECT", "SELECT", "table"), ("SELECT(request_sha256)", "SELECT(request_sha256)", "column")]:
            self.sql(f"GRANT {grant} ON core_meta.payment_gateway_upgrades TO {self.roles['runtime']};", db)
            check("receipt-" + label + "-overgrant", False)
            self.sql(f"REVOKE {revoke} ON core_meta.payment_gateway_upgrades FROM {self.roles['runtime']};", db)
        check("runtime-restored-check", True)
        self.sql(f"ALTER ROLE {self.roles['runtime']} NOLOGIN;")
        self.fence(db)
        self.mark("runtime_check_and_private_receipt_overgrant_rejection")

    def execute(self):
        self.bind()
        self.setup()
        self.negative("bad_saved_catalog", "UPDATE core_meta.runtime_contract SET catalog_fingerprint=decode(repeat('00',32),'hex');", "PAYMENT_GATEWAYS_PREDECESSOR_RECEIPT_MISMATCH")
        self.negative("bad_live_catalog", "ALTER TABLE core_console.options ADD COLUMN pgup_fixture_extra text;", "PAYMENT_GATEWAYS_PREDECESSOR_CATALOG_MISMATCH")
        self.negative("default_acl", f"ALTER DEFAULT PRIVILEGES FOR ROLE {self.roles['owner']} IN SCHEMA core_meta GRANT SELECT ON TABLES TO {self.roles['runtime']};", "PAYMENT_GATEWAYS_RECEIPT_PRIVILEGES_INVALID")
        self.negative("reused_transaction", "UPDATE core_payments.intents SET provider_transaction='synthetic_reused_transaction'; INSERT INTO core_payments.intents(account_id,created_by_user_id,channel_id,provider,merchant,environment,currency,amount_minor,credit_units,credits_per_minor,prepare_key,prepare_hash,provider_transaction) SELECT account_id,created_by_user_id,102,provider,merchant,environment,currency,amount_minor,credit_units,credits_per_minor,'synthetic-prepare-key-0002',prepare_hash,provider_transaction FROM core_payments.intents;", "PAYMENT_GATEWAYS_EXISTING_TRANSACTION_REUSED")
        db = self.upgrade()
        self.runtime(db)
        self.save("result.json", {"status": "passed", "checks": self.results, "databases_prefix": self.prefix,
            "not_checked": ["old Native initialization", "production database", "provider network payments", "lost COMMIT acknowledgement"]})
        print("PASS complete", self.out, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--core", type=Path, required=True)
    parser.add_argument("--psql", type=Path, required=True)
    parser.add_argument("--host", required=True, help="Socket directory of an explicitly disposable PG18 cluster")
    parser.add_argument("--port", required=True)
    parser.add_argument("--admin", required=True)
    parser.add_argument("--library-path")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    require(args.host.startswith("/"), "only a local Unix socket test cluster is supported")
    require(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", args.admin), "invalid test admin role")
    args.core = args.core.resolve(strict=True)
    test = Test(args)
    try:
        test.execute()
    except Exception as error:
        test.save("failure.json", {"status": "failed", "type": type(error).__name__,
            "reason": str(error), "completed_checks": test.results, "databases_prefix": test.prefix})
        raise


if __name__ == "__main__":
    main()

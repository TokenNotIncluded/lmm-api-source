#!/usr/bin/env python3
"""Real PostgreSQL evidence for the existing-schema subscription admin SQL.

Uses a fresh PostgreSQL 18 *_test database, synthetic users and the production
admin projection/revocation queries under a restricted runtime role. This does
not run Rust HTTP/session authorization or the charging process; those require
the accompanying integration tests when a Rust test binary is available.
"""
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import secrets
import subprocess


ROOT = Path(__file__).resolve().parents[1]
CORE = ROOT / "services/core"
spec = importlib.util.spec_from_file_location(
    "commerce_schema_fixture", ROOT / "scripts/test-commerce-self-purchase.py"
)
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
need = fixture.need
sql = fixture.sql


def literal(value):
    if value is None:
        return "NULL"
    if isinstance(value, int):
        return str(value)
    if not isinstance(value, str):
        value = json.dumps(value, separators=(",", ":"))
    return "'" + value.replace("'", "''") + "'"


def command_input(grant, key, reason="Synthetic administrator revocation"):
    return ["invalidate_grant", grant, {"operation_key": key, "user_id": 8302, "reason": reason}]


def query_page(role, at, query="", user=None, plan=None, status="all", size=20, page=1):
    statement = (CORE / "src/subscriptions/admin_records.sql").read_text()
    arguments = [at, query, user, plan, status, size, (page - 1) * size, page]
    return json.loads(sql(
        "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"
        "PREPARE records(bigint,text,bigint,bigint,text,bigint,bigint,bigint) AS\n"
        + statement + ";\nEXECUTE records(" + ",".join(map(literal, arguments)) + "); COMMIT;", role
    ))


def invalidate(role, grant, key, at, *, allow_failure=False):
    # Same immutable request, lock order and replay read as admin.rs. The actual
    # mutation/receipt/audit statement is loaded from the production SQL file.
    request = command_input(grant, key)
    args = [grant, 8302, 8301, None, key, request, at, request[2]["reason"]]
    statement = (CORE / "src/subscriptions/admin_invalidate.sql").read_text()
    body = rf'''SET ROLE "{role}";
BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL lock_timeout='2s';
SELECT pg_advisory_xact_lock(hashtextextended('core-subscriptions:8301',0));
SELECT EXISTS(SELECT FROM core_subscriptions.commands WHERE actor_user_id=8301
    AND operation_key={literal(key)} AND request={literal(request)}::jsonb) AS replay \gset
\if :replay
SELECT result FROM core_subscriptions.commands WHERE actor_user_id=8301 AND operation_key={literal(key)};
\else
SELECT id FROM core_subscriptions.purchases WHERE subscription_id={grant} FOR UPDATE;
SELECT u.id,s.active FROM core_charging.subscriptions s JOIN core_identity.users u
    ON u.personal_account_id=s.account_id WHERE s.id={grant} FOR UPDATE OF s;
PREPARE revoke(bigint,bigint,bigint,bigint,text,jsonb,bigint,text) AS
{statement};
EXECUTE revoke({','.join(map(literal, args))});
\endif
COMMIT;
'''
    result = subprocess.run(
        ["psql", "-X", "-q", "-At", "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=verbose", os.environ["DATABASE_URL"]],
        input=body, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15,
    )
    if result.returncode:
        if allow_failure:
            return result.stderr
        raise AssertionError("revocation SQL failed:\n" + result.stderr)
    receipts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
    need(len(receipts) == 1, "revocation did not return one durable receipt")
    return receipts[0]


def snapshot(owner):
    names = ["core_billing.ledger_journals", "core_billing.ledger_entries", "core_billing.balance_state",
             "core_charging.requests", "core_charging.subscription_opening_consumption"]
    rows = [sql(f"SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM {name} t", owner)
            for name in names]
    rows.append(sql("SELECT jsonb_agg(to_jsonb(s)-'active' ORDER BY id) FROM core_charging.subscriptions s", owner))
    rows.append(sql("SELECT jsonb_agg(to_jsonb(p)-'canceled_at' ORDER BY id) FROM core_subscriptions.purchases p", owner))
    return rows


def main():
    facts = json.loads(sql("SELECT jsonb_build_array(current_database(),current_setting('server_version_num')::int,"
                          "EXISTS(SELECT FROM pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema','public') AND left(nspname,3)<>'pg_'),"
                          "EXISTS(SELECT FROM pg_class WHERE relnamespace='public'::regnamespace))"))
    need(facts[0].endswith("_test") and 180000 <= facts[1] < 190000 and not facts[2] and not facts[3],
         "fresh isolated PostgreSQL 18 *_test database required")
    suffix = secrets.token_hex(5)
    owner, runtime = "lmm_subscription_owner_" + suffix, "lmm_subscription_runtime_" + suffix
    database = '"' + facts[0].replace('"', '""') + '"'
    sql(f'CREATE ROLE "{owner}" NOLOGIN; CREATE ROLE "{runtime}" NOLOGIN; '
        f'ALTER DATABASE {database} OWNER TO "{owner}"; '
        f'REVOKE ALL ON DATABASE {database} FROM PUBLIC; REVOKE ALL ON SCHEMA public FROM PUBLIC; '
        f'GRANT USAGE,CREATE ON SCHEMA public TO "{owner}"')
    components, metadata, manifest = fixture.source_contract()
    sql("BEGIN;\n" + "\n".join(ddl for _, ddl in components) + "\n" + "\n".join(metadata) + "\nCOMMIT;", owner)
    pin = json.loads((CORE / "src/core_schema/upgrade/fresh_target.json").read_text())
    need(fixture.manifest_hash(manifest) == pin["manifest_sha256"], "current schema source changed unexpectedly")
    need(fixture.framed(sql(manifest["catalog"]["query_utf8"], owner).splitlines()) == pin["catalog_sha256"],
         "fresh PostgreSQL schema differs from the current frozen pin")
    grants = (ROOT / "deployment/postgres/grant-core-runtime.sql").read_text()
    grants = grants.split("-- BEGIN subscriptions runtime grants\n", 1)[1].split("-- END subscriptions runtime grants", 1)[0]
    grants = grants.replace(':"runtime_role"', f'"{runtime}"')
    sql(grants + f'''
GRANT USAGE ON SCHEMA core_identity,core_billing TO "{runtime}";
GRANT SELECT ON core_identity.users,core_identity.user_profiles,core_billing.ledger_journals,core_billing.ledger_accounts TO "{runtime}";
GRANT INSERT ON core_identity.audit TO "{runtime}";
GRANT USAGE ON SEQUENCE core_identity.audit_id_seq TO "{runtime}";
''', owner)
    seed = (CORE / "tests/core_schema/subscription_administration.sql").read_text()
    sql(seed, owner)
    at = int(sql("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint", owner))
    full = query_page(runtime, at)
    need(full["total"] == 5 and len(full["items"]) == 5, "grant catalogue omitted records")
    by_id = {row["id"]: row for row in full["items"]}
    need(by_id[501]["plan"]["id"] == 101 and by_id[501]["source"] == "wallet"
         and by_id[501]["used_units"] == "65" and by_id[501]["reserved_units"] == "40",
         "native purchase/usage/hold projection changed")
    need(by_id[502]["plan"] is None and by_id[502]["source"] == "imported"
         and by_id[502]["limit_units"] == "700" and by_id[502]["used_units"] == "40",
         "imported quota was normalized or a plan was invented")
    later = query_page(runtime, at + 180, user=8302)
    imported_later = next(row for row in later["items"] if row["id"] == 502)
    need(imported_later["limit_units"] == "900" and imported_later["used_units"] == "0",
         "explicit imported next-period basis was lost")
    page = query_page(runtime, at, status="scheduled", size=1)
    need(page["total"] == 1 and [row["id"] for row in page["items"]] == [504], "filter applied after pagination")
    need(query_page(runtime, at, query="%")['total'] == 0, "literal search became a wildcard")
    need(query_page(runtime, at, plan=101)['total'] == 1, "plan filter guessed an imported association")
    need(query_page(runtime, at, user=8303)['total'] == 1, "target-user filter leaked another user's grants")
    before = snapshot(owner)
    print("PASS: actual admin query, pagination, native plan snapshots and imported current/next quota basis", flush=True)
    key = "subscription_invalidate_original_001"
    with ThreadPoolExecutor(max_workers=8) as pool:
        attempts = list(pool.map(lambda _: invalidate(runtime, 501, key, at, allow_failure=True), range(8)))
    successes = [row for row in attempts if isinstance(row, dict)]
    failures = [row for row in attempts if isinstance(row, str)]
    need(successes and all(row == successes[0] for row in successes),
         "concurrent successful receipts differ: " + (failures[0] if failures else ""))
    need(all("40001" in row for row in failures), "unexpected concurrent database failure: " + "\n".join(failures))
    need(invalidate(runtime, 501, key, at + 1) == successes[0], "fresh-connection replay changed original result")
    need(sql("SELECT count(*) FROM core_subscriptions.commands WHERE actor_user_id=8301 AND operation_key=" + literal(key), owner) == "1",
         "duplicate command receipt")
    need(sql("SELECT count(*) FROM core_identity.audit WHERE action='subscription.grant.invalidate'", owner) == "1", "duplicate audit")
    need(snapshot(owner) == before, "revocation changed original money, grant identity or consumption")
    changed = next(row for row in query_page(runtime, at)["items"] if row["id"] == 501)
    need(not changed["active"] and changed["status"] == "invalidated" and changed["renewal_canceled"] is True,
         "revocation did not stop new use and renewal")
    need(changed["used_units"] == "65" and changed["reserved_units"] == "40", "revocation erased usage or held quota")
    need(sql("SELECT NOT EXISTS(SELECT FROM core_charging.subscriptions WHERE id=501 AND active)", runtime) == "t", "new charging source remained eligible")
    print(f"PASS: 8 concurrent revocations, {len(failures)} serialization conflicts; original request recovery, one receipt and audit", flush=True)
    sql("ALTER TABLE core_identity.audit ADD CONSTRAINT synthetic_audit_failure CHECK (action<>'subscription.grant.invalidate') NOT VALID", owner)
    failure = invalidate(runtime, 505, "subscription_audit_rollback_001", at, allow_failure=True)
    need(isinstance(failure, str) and "23514" in failure, "audit failure fixture did not fail")
    need(sql("SELECT active FROM core_charging.subscriptions WHERE id=505", owner) == "t"
         and sql("SELECT count(*) FROM core_subscriptions.commands WHERE operation_key='subscription_audit_rollback_001'", owner) == "0",
         "audit failure committed grant or receipt")
    sql("ALTER TABLE core_identity.audit DROP CONSTRAINT synthetic_audit_failure", owner)
    need(sql("SELECT NOT has_table_privilege(current_user,'core_charging.subscriptions','DELETE,TRUNCATE')"
             " AND NOT has_column_privilege(current_user,'core_charging.subscriptions','limit_credits','UPDATE')"
             " AND NOT has_schema_privilege(current_user,'core_charging','CREATE')", runtime) == "t",
         "runtime acquired grant deletion, quota rewrite or DDL authority")
    need(snapshot(owner) == before, "failed revocation changed original money or usage")
    print("PASS: mandatory audit failure rolls back revocation and receipt; existing runtime permissions suffice", flush=True)
    print(json.dumps({"evidence": "PostgreSQL SQL only; Rust HTTP/session/charging process tests not executed",
                      "schema_manifest_sha256": pin["manifest_sha256"], "schema_unchanged": True}), flush=True)


if __name__ == "__main__":
    main()

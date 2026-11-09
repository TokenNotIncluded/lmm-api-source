#!/usr/bin/env python3
"""Run only against the disposable PG18 fixture named by the test harness.

This exercises the compiled SQL inputs and a real restricted connection. It is
not a substitute for the Rust HTTP/authentication tests in display_exchange_rate.rs.
"""
import json
import os
from pathlib import Path
import re
import subprocess
from urllib.parse import urlsplit, urlunsplit


def sql(connection, statement, *, failure=False):
    result = subprocess.run(
        ["psql", connection, "-X", "-Atq", "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=sqlstate"],
        input=statement, text=True, capture_output=True, check=False,
    )
    if failure:
        assert result.returncode != 0, "expected SQL rejection"
        return result.stderr
    if result.returncode:
        raise RuntimeError(result.stderr)
    return result.stdout.strip()


def literal(value):
    return "NULL" if value is None else "'" + value.replace("'", "''") + "'"


def main():
    owner_url = os.environ["DATABASE_URL"]
    fixture = Path(os.environ["LMM_PROGRESS_TEST_FIXTURE"]).resolve(strict=True)
    identity = json.loads(sql(owner_url, """
        SELECT json_build_object('database',current_database(),
          'data',current_setting('data_directory'),
          'version',current_setting('server_version_num')::int,
          'system',system_identifier::text) FROM pg_control_system();
    """))
    assert re.fullmatch(r"lmm_[a-z_]+_test", identity["database"])
    assert 180000 <= identity["version"] < 190000
    assert Path(identity["data"]).resolve() == fixture / "data"
    assert identity["system"] == (fixture / "system-id").read_text().strip()
    assert sql(owner_url, "SELECT to_regnamespace('core_identity') IS NULL") == "t"
    schema = Path(__file__).resolve().parents[1] / "schema"
    ddl = "\n".join((schema / name).read_text() for name in (
        "identity.sql", "ledger.sql", "console.sql", "display_exchange_rate.sql",
    ))
    sql(owner_url, f"""
        CREATE ROLE lmm_fx_sql_owner NOLOGIN;
        CREATE ROLE lmm_fx_sql_runtime LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
        ALTER DATABASE {identity['database']} OWNER TO lmm_fx_sql_owner;
        REVOKE CREATE,TEMPORARY ON DATABASE {identity['database']} FROM PUBLIC;
        REVOKE CREATE ON SCHEMA public FROM PUBLIC;
        GRANT CONNECT ON DATABASE {identity['database']} TO lmm_fx_sql_runtime;
        BEGIN;
        SET LOCAL ROLE lmm_fx_sql_owner;
        {ddl}
        INSERT INTO core_identity.accounts(kind) VALUES('personal');
        INSERT INTO core_identity.users(id,personal_account_id) VALUES(1,1);
        SELECT core_billing.open_ledger_wallet(1);
        SELECT core_billing.post_ledger('{{"scope":"display-exchange-rate-test",
          "operation_key":"synthetic-opening","actor_user_id":1,
          "reason":"Synthetic historical funds in a disposable database",
          "action":{{"kind":"credit","account_id":1,"amount_units":1250000,
            "payment_reference":"synthetic:display-exchange-rate"}}}}'::jsonb);
        INSERT INTO core_console.options(key,value) VALUES('Notice','preserved notice');
        GRANT USAGE ON SCHEMA core_console TO lmm_fx_sql_runtime;
        GRANT SELECT ON core_console.options TO lmm_fx_sql_runtime;
        GRANT EXECUTE ON FUNCTION core_console.write_display_exchange_rate(text),
          core_console.write_public_option(text,text) TO lmm_fx_sql_runtime;
        COMMIT;
    """)
    snapshot_sql = """
        SELECT jsonb_build_object(
          'accounts',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM core_billing.ledger_accounts t),
          'balances',(SELECT jsonb_agg(to_jsonb(t) ORDER BY ledger_account_id) FROM core_billing.balance_state t),
          'journals',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM core_billing.ledger_journals t),
          'entries',(SELECT jsonb_agg(to_jsonb(t) ORDER BY journal_id,ledger_account_id) FROM core_billing.ledger_entries t),
          'operations',(SELECT jsonb_agg(to_jsonb(t) ORDER BY scope,operation_key) FROM core_billing.ledger_operations t));
    """
    before = sql(owner_url, snapshot_sql)
    history = json.loads(before)
    assert history["journals"][0]["amount_units"] == 1250000
    assert history["operations"][0]["result"]["status"] == "posted"
    print("PASS: original identity, ledger, console and display SQL installed by NOLOGIN owner")

    parsed = urlsplit(owner_url)
    runtime_url = urlunsplit(parsed._replace(netloc="lmm_fx_sql_runtime@" + parsed.netloc.rsplit("@", 1)[-1]))
    assert sql(runtime_url, "SELECT session_user=current_user AND current_user='lmm_fx_sql_runtime'") == "t"
    assert sql(runtime_url, "SELECT core_console.write_display_exchange_rate('7.25')") == "1"
    assert sql(runtime_url, "SELECT value||':'||revision FROM core_console.options WHERE key='USDExchangeRate'") == "7.25:1"
    print("PASS: restricted runtime saves and reads the display option through its exact function")

    for value in [None, "", "0", "-1", "NaN", "Infinity", "1e309", "1e10", "0.00009", " 7", "+7", "07", "7" * 33]:
        error = sql(runtime_url, f"SELECT core_console.write_display_exchange_rate({literal(value)})", failure=True)
        assert "22023" in error or "22003" in error, error
    assert sql(runtime_url, "SELECT value||':'||revision FROM core_console.options WHERE key='USDExchangeRate'") == "7.25:1"
    for value in ["0.0001", "1000000000", "7.25"]:
        sql(runtime_url, f"SELECT core_console.write_display_exchange_rate({literal(value)})")
    print("PASS: missing, malformed, nonfinite and out-of-range inputs reject; exact bounds save")

    for statement in [
        "UPDATE core_console.options SET value='8' WHERE key='USDExchangeRate'",
        "INSERT INTO core_console.options(key,value) VALUES('OAuthSecret','denied')",
        "DELETE FROM core_console.options WHERE key='USDExchangeRate'",
        "TRUNCATE core_console.options",
        "CREATE TABLE core_console.denied(x int)",
        "UPDATE core_billing.balance_state SET balance_units=0",
    ]:
        assert "42501" in sql(runtime_url, statement, failure=True)
    assert "22023" in sql(runtime_url, "SELECT core_console.write_public_option('USDExchangeRate','8')", failure=True)
    assert sql(owner_url, "SELECT NOT has_function_privilege('public','core_console.write_display_exchange_rate(text)','EXECUTE')") == "t"
    print("PASS: direct option/ledger writes, DDL, unrelated setter and PUBLIC execution reject")

    saved = sql(runtime_url, "SELECT value||':'||revision FROM core_console.options WHERE key='USDExchangeRate'")
    assert "22012" in sql(runtime_url, "BEGIN; SELECT core_console.write_display_exchange_rate('8'); SELECT 1/0; COMMIT;", failure=True)
    assert sql(runtime_url, "SELECT value||':'||revision FROM core_console.options WHERE key='USDExchangeRate'") == saved
    assert sql(runtime_url, "SELECT value FROM core_console.options WHERE key='Notice'") == "preserved notice"
    assert sql(owner_url, snapshot_sql) == before
    print("PASS: transaction failure rolls back; original balances, unit, journals, entries and operations remain byte-identical")


if __name__ == "__main__":
    main()

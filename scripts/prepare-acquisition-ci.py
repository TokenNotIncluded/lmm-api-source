#!/usr/bin/env python3
"""Prepare private acquisition fixtures on the explicit CI loopback database."""
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
from urllib.parse import unquote, urlsplit

admin = urlsplit(os.environ["DATABASE_URL"])
assert admin.scheme in {"postgres", "postgresql"}
assert admin.hostname in {"127.0.0.1", "localhost"}
pg = {
    "PGHOST": admin.hostname,
    "PGPORT": str(admin.port or 5432),
    "PGUSER": unquote(admin.username or ""),
    "PGPASSWORD": unquote(admin.password or ""),
    "PGDATABASE": unquote(admin.path.removeprefix("/")),
    "PGSSLMODE": "disable",
}
environment = {**os.environ, **pg}
def sql(statement):
    subprocess.run(
        ["psql", "--no-psqlrc", "--quiet", "--set", "ON_ERROR_STOP=1"],
        input=statement, text=True, env=environment, check=True,
    )

sql("""DO $fixture$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='lmm_core_owner') THEN
        CREATE ROLE lmm_core_owner NOLOGIN NOSUPERUSER NOCREATEDB
            NOCREATEROLE NOREPLICATION NOBYPASSRLS;
    END IF;
END;
$fixture$;""")
exports = dict(pg)
for kind, variable in [
    ("native", "ACQUISITION_CONNECTION_DATABASE_URL"),
    ("history", "ACQUISITION_CONNECTION_HISTORY_DATABASE_URL"),
]:
    suffix = secrets.token_hex(6)
    role = "lmm_acq_" + kind + "_" + suffix
    database = role + "_db"
    password = secrets.token_hex(24)
    if os.environ.get("GITHUB_ACTIONS") == "true":
        print("::add-mask::" + password)
    sql(f"""CREATE ROLE {role} LOGIN NOINHERIT NOSUPERUSER NOCREATEDB
        NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '{password}';
        CREATE DATABASE {database} OWNER {role};
        REVOKE CONNECT, CREATE, TEMP ON DATABASE {database} FROM PUBLIC;""")
    exports[variable] = (
        f"postgresql://{role}:{password}@{pg['PGHOST']}:{pg['PGPORT']}/{database}"
        "?sslmode=disable"
    )
evidence_parent = Path.home() / ".local/state/lmm-api/connection-filter"
evidence_parent.mkdir(mode=0o700, parents=True, exist_ok=True)
evidence = Path(tempfile.mkdtemp(prefix="ci-", dir=evidence_parent))
exports["CONNECTION55_EVIDENCE"] = str(evidence.resolve())
with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as output:
    for name, value in exports.items():
        if "\n" in value or "\r" in value:
            raise ValueError("fixture environment value contains a line break")
        output.write(name + "=" + value + "\n")

#!/usr/bin/env python3
"""Capture a Stripe successor candidate in a rolled-back PG18 owner transaction.

Does not change checked-in pins, install permanently, or build/run applications.
Run only in the explicitly assigned isolated PG window.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import subprocess
from urllib.parse import urlparse


def framed(facts):
    digest = hashlib.sha256()
    for fact in facts:
        data = fact.encode("utf-8")
        digest.update(struct.pack(">Q", len(data)))
        digest.update(data)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--owner-url-file", required=True)
    parser.add_argument("--owner-role", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    credential = Path(args.owner_url_file)
    if credential.is_symlink() or stat.S_IMODE(credential.stat().st_mode) != 0o600:
        raise SystemExit("private regular credential file required")
    dsn = credential.read_text().strip()
    parsed = urlparse(dsn)
    if parsed.hostname != "127.0.0.1" or not parsed.path.startswith("/lmm_payments") or not parsed.path.endswith("_test"):
        raise SystemExit("private loopback payments test database required")
    if not re.fullmatch(r"[a-z][a-z0-9_]{0,62}", args.owner_role):
        raise SystemExit("invalid owner role")
    root = Path(__file__).resolve().parents[1]
    base = root / "services/extensions/internal/modules/payments"
    paths = [base / name for name in ("schema.sql", "subscription_epay_schema.sql", "subscription_stripe_schema.sql", "subscription_stripe_catalog.sql", "subscription_stripe_storage_contract.json")]
    source = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
    contract = json.loads(paths[-1].read_text())
    if source[str(paths[2].relative_to(root))] != contract["stripe_schema_sha256"]:
        raise SystemExit("frozen Stripe SQL hash mismatch")
    catalog = paths[3].read_text().strip().rstrip(";")
    def query(phase, successor):
        body = catalog.replace("$1::boolean", "true" if successor else "false")
        return "SELECT json_build_object('phase','" + phase + "','facts',(SELECT json_agg(fact ORDER BY fact COLLATE \"C\") FROM (" + body + ") capture));\n"
    sql = 'BEGIN; SET LOCAL search_path=public,pg_catalog; SET LOCAL lock_timeout=\'2s\'; SET LOCAL statement_timeout=\'10s\'; SET LOCAL ROLE "' + args.owner_role + '";\n'
    sql += """DO $$ BEGIN
 IF current_setting('server_version_num')::int NOT BETWEEN 180000 AND 189999
 OR current_user<>(SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database())
 OR NOT EXISTS(SELECT FROM pg_roles WHERE rolname=current_user AND NOT rolcanlogin AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication AND NOT rolbypassrls)
 OR EXISTS(SELECT FROM pg_roles WHERE rolname=session_user AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 OR EXISTS(SELECT FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid())
 OR EXISTS(SELECT FROM pg_namespace WHERE nspname LIKE 'core\\_%' ESCAPE '\\')
 OR to_regclass('public.payment_stripe_subscription_checkouts') IS NOT NULL
 OR to_regclass('public.payment_stripe_subscription_inbox') IS NOT NULL
 THEN RAISE EXCEPTION 'isolated PG18 offline predecessor owner required'; END IF;
 END $$;
"""
    sql += query("before", False) + paths[2].read_text() + "\n" + query("successor", True) + "ROLLBACK;\n"
    env = os.environ.copy()
    env.update(PGDATABASE=dsn, PGCLIENTENCODING="UTF8", PGAPPNAME="lmm-sub01-stripe-catalog-capture")
    def retain_process(returncode, stdout, stderr, timed_out=False):
        def text(value):
            return value.decode("utf-8", "replace") if isinstance(value, bytes) else value
        # Retain the original failure/capture without printing private connection
        # diagnostics or permitting a second attempt to replace its evidence.
        fd = os.open(args.output + ".psql.json", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as evidence:
            json.dump({"returncode": returncode, "timed_out": timed_out, "stdout": text(stdout), "stderr": text(stderr), "source_sha256": source}, evidence, indent=2)
            evidence.write("\n")
    try:
        result = subprocess.run(["psql", "-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1"], input=sql, text=True, capture_output=True, env=env, timeout=45)
    except subprocess.TimeoutExpired as exc:
        retain_process(None, exc.stdout, exc.stderr, True)
        raise SystemExit("capture timed out; inspect retained private evidence and backend before any next operation") from None
    retain_process(result.returncode, result.stdout, result.stderr)

    if result.returncode:
        # No connection URL or server error detail is copied into reports.
        raise SystemExit("PG18 rollback capture failed; no candidate emitted")
    records = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    if len(records) != 2 or records[0]["phase"] != "before" or records[1]["phase"] != "successor":
        raise SystemExit("unexpected capture output")
    before, after = (framed(record["facts"]) for record in records)
    if before != contract["predecessor_catalog_sha256"]:
        raise SystemExit("actual predecessor catalog differs from frozen contract")
    if source != {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}:
        raise SystemExit("input drift during capture")
    receipt = {"validation_status": "PG18_ROLLBACK_CANDIDATE_ONLY_NOT_AN_INSTALL_RECEIPT", "transaction_rolled_back": True, "stripe_schema_sha256": contract["stripe_schema_sha256"], "predecessor_catalog_sha256": before, "successor_catalog_sha256": after, "source_sha256": source, "facts": records}
    with Path(args.output).open("x") as output:
        json.dump(receipt, output, ensure_ascii=False, indent=2)
        output.write("\n")
    print("PG18 candidate captured; source pin unchanged; transaction rolled back")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""One real identity commit followed by synthetic username/password HTTP probes.

Run ``prepare`` only in a new shared Core78 fixture while runtime is NOLOGIN.
The existing controller then opens its synthetic runtime role, starts its
hash-pinned Core on loopback, and calls ``verify``. The controller owns service
and PG cleanup. No production connection, source restore, provider call, or
COMMIT-loss replay belongs to this driver. The four reviewed identity files
remain unchanged. Test passwords below are public, synthetic fixture inputs.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import uuid
from urllib.parse import urlsplit

import migrate_current_identity as M
import test_current_identity as T
import test_current_identity_pg as G
import test_migrate_current_runtime_keys_pg as K


ADAPTER_SHA256 = "c3430d1d03f201dcf254bb31678e7af281d94136cf64b6e661881907f6b3b376"
SCOPE = "current-identity-password-runtime-v1"
# Generated and independently checked with libc crypt, including a wrong
# password rejection. Core itself must verify these hashes in the actual test.
CASES = (
    {"label": "new", "source_user_id": 105, "username": "synthetic-user-5",
     "password": "LmmNewFixture#2026",
     "bcrypt": "$2b$04$abcdefghijklmnopqrstuu0SKSv2vrVhEdVmr4g.JM1Iu5KmM7kaq",
     "expected_auth_version": 1},
    {"label": "history", "source_user_id": 108, "username": "synthetic-user-8",
     "password": "LmmHistory#2026",
     "bcrypt": "$2b$04$vwxyzABCDEFGHIJKLMNOuuQ.1SbCSOzlzH2Ky/RkqC4i28fMUGIoC",
     "expected_auth_version": 3},
)
WRONG_PASSWORD = "WrongPassword#2026"
HTTP_MUTABLE_TABLES = frozenset({
    "core_identity.auth_attempts", "core_identity.password_logins",
    "core_identity.user_profiles", "core_identity.credentials",
    "core_identity.session_authentication", "core_identity.web_sessions",
    "core_identity.web_session_credentials", "core_identity.audit",
})


def check(ok, message):
    if not ok:
        raise AssertionError(message)


def file_sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def password_fixture():
    bundle, projection = G.shifted_fixture()
    by_id = {r["id"]: r for r in bundle["source"]["data"]["users"]}
    for case in CASES:
        row = by_id[case["source_user_id"]]
        check(row["username"] == case["username"], "synthetic username changed")
        row["password"] = case["bcrypt"]
    # This constructs new synthetic facts before planning, not an amendment to
    # a sealed real-source export, original mapping, or committed material.
    bundle["source"] = T.source_object(bundle["source"]["data"]["users"])
    bundle["current"] = bundle["source"]["current_policy_records"]
    M.validate_source(bundle["source"])
    M.validate_oauth_projection(bundle["source"], bundle["oauth"])
    return bundle, projection


def money_state(state, *, include_policy=True):
    return {k: v for k, v in state["tables"].items()
            if k.startswith(("core_billing.", "core_charging."))
            and (include_policy or k != "core_billing.account_policies")}


def stable_tables(state):
    return {k: v for k, v in state["tables"].items() if k not in HTTP_MUTABLE_TABLES}


def protected_rows_sql(case_user_ids):
    ids = ",".join(str(M.integer(i, "invalid synthetic owner")) for i in case_user_ids)
    return "SELECT jsonb_build_object(" + ",".join(
        M.literal(table) + ",COALESCE((SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM "
        + M.F.relation(table) + " t WHERE " + predicate + "),'[]'::jsonb)"
        for table, predicate in (
            ("core_identity.password_logins", f"user_id NOT IN ({ids})"),
            ("core_identity.user_profiles", f"user_id NOT IN ({ids})"),
            ("core_identity.credentials", f"user_id NOT IN ({ids})"),
            ("core_identity.session_authentication", f"user_id NOT IN ({ids})"),
            ("core_identity.web_sessions", f"user_id NOT IN ({ids})"),
        )) + ")"


def credential_count_sql():
    return "SELECT count(*) FROM core_identity.credentials"


class Preparation(G.Regression):
    def seed_password_identity(self):
        self.bundle, projection = password_fixture()
        sql = ["BEGIN ISOLATION LEVEL SERIALIZABLE;"]
        for key, table in (("accounts", "core_identity.accounts"),
                           ("users", "core_identity.users"),
                           ("profiles", "core_identity.user_profiles"),
                           ("developer", "core_identity.developer_access"),
                           ("account_policies", "core_billing.account_policies"),
                           ("oauth", "core_identity.oauth_identities")):
            for row in projection[key]:
                override = " OVERRIDING SYSTEM VALUE" if key == "accounts" else ""
                sql.append(f"INSERT INTO {table}(" + ",".join(row) + ")" + override + " VALUES("
                           + ",".join(M.literal(v) for v in row.values()) + ");")
        for row in projection["passwords"]:
            sql.append("INSERT INTO core_identity.password_logins(user_id,login_name,password_hash) VALUES("
                       + ",".join(M.literal(v) for v in (row["user_id"], row["login_name"], T.PASSWORD)) + ");")
        for receipt in projection["original_imports"] + projection["history_imports"]:
            sql.append("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES("
                       + ",".join(M.literal(v) for v in (receipt["migration_id"], receipt.get("source_archive_sha256", "b" * 64),
                           receipt.get("mapping_sha256", M.digest(receipt["manifest"])), receipt["manifest"])) + ");")
        for sequence in M.SEQUENCES:
            sql.append("SELECT nextval(" + M.literal(sequence) + "::regclass) FROM generate_series(1,200);")
        self.run_sql("\n".join(sql + ["COMMIT;"]))

    def prepare_once(self, fixture_path):
        self.seed_sessions_and_budget()
        self.seed_password_identity()
        migration = "synthetic-identity-password-" + uuid.uuid4().hex[:12]
        material = self.prepare_identity(migration)
        before = self.state(migration)
        committed = self.query(M.build_sql(material, fixture_nonce=self.nonce))
        check(committed["status"] == "COMMITTED", "identity commit was not acknowledged")
        receipt = self.query("SELECT manifest FROM core_migration.import_runs WHERE migration_id=" + M.literal(migration))
        check(receipt["actual_identity_authority"] == self.query(M.identity_authority_sql(material)),
              "committed identity authority differs from actual readback")
        mapper = {r["source_user_id"]: r for r in material["plan"]["mapping"]}
        cases = []
        for case in CASES:
            mapped = mapper[case["source_user_id"]]
            row = next(r for r in receipt["actual_identity_authority"] if r["source_user_id"] == case["source_user_id"])
            check(row["auth_version"] == case["expected_auth_version"] and row["password_login_allowed"],
                  "committed password authority differs")
            case = deepcopy(case)
            case.update({k: mapped[k] for k in ("target_user_id", "target_personal_account_id")})
            cases.append(case)
        after = self.state(migration)
        check(money_state(before, include_policy=False) == money_state(after, include_policy=False),
              "identity completion changed financial state")
        probe = {"format_version": 1, "scope": SCOPE, "synthetic_only": True,
                 "fixture_file_sha256": file_sha(fixture_path), "fixture_nonce": self.nonce,
                 "database": self.connection["database"], "adapter_sha256": ADAPTER_SHA256,
                 "expected_core78_manifest_sha256": M.U.CORE78_MANIFEST_SHA256,
                 "inventory": self.inventory, "migration_id": migration,
                 "committed_manifest_sha256": M.digest(receipt), "cases": cases,
                 "before_http_money": money_state(after), "before_http_stable_tables": stable_tables(after),
                 "before_http_protected_rows_sha256": M.digest(self.query(protected_rows_sql([c["target_user_id"] for c in cases]))),
                 "before_http_credential_count": self.query(credential_count_sql()),
                 "preexisting_session": {"bearer": self.session_bearers[0], "credential_id": self.session_ids[0],
                    "user_id": self.owner["user_id"], "owner_account_id": self.owner["personal_account_id"]},
                 "expired_session_bearer": self.session_bearers[1]}
        M.F.write_private(self.output, "runtime-probes.private.json", M.canonical(probe) + "\n")
        result = {"status": "PREPARED_RUNTIME_NOT_RUN", "scope": SCOPE, "identity_commits": 1,
                  "synthetic_only": True, "production_access": False, "real_google_login": "NOT_RUN",
                  "password_runtime_login": "NOT_RUN", "financial_state_unchanged_by_identity": True}
        M.F.write_private(self.output, "result.public.json", M.canonical(result) + "\n")
        print(json.dumps(result))


class Reader:
    def __init__(self, fixture, output):
        self.connection, self.output, self.counter = fixture["connection"], output, 0
        check(self.connection.get("transport") == "unix"
              and re.fullmatch(r"lmm_runtime_keys_test_[a-z0-9_]+", self.connection["database"])
              and re.fullmatch(r"[a-f0-9]{64}", fixture["fixture_nonce"]), "synthetic fixture required")
        output.mkdir(mode=0o700, parents=True, exist_ok=False)
        marker = "lmm-runtime-keys-synthetic:" + fixture["fixture_nonce"]
        actual = self.query("SELECT jsonb_build_object('database',current_database(),'marker',"
                            "shobj_description((SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database'),"
                            "'nonce',current_setting('lmm.migration_instance_nonce'))")
        check(actual == {"database": self.connection["database"], "marker": marker, "nonce": fixture["fixture_nonce"]},
              "runtime reader belongs to another database")

    def query(self, sql):
        self.counter += 1
        code, raw = M.F.run_sql(self.connection, "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n" + sql.rstrip(";\n") + ";\nCOMMIT;",
                              self.output, "identity-password-read-" + uuid.uuid4().hex,
                              label="read-" + str(self.counter))
        check(code == 0, "read-only runtime evidence failed")
        return K.parse_single_result(raw)


def request(origin, method, path, *, body=None, bearer=None):
    parsed = urlsplit(origin)
    check(parsed.scheme == "http" and parsed.hostname == "127.0.0.1" and parsed.port
          and parsed.path in ("", "/") and not parsed.query and not parsed.fragment
          and not parsed.username and not parsed.password, "loopback HTTP origin required")
    headers = {"Origin": origin, "User-Agent": "lmm-synthetic-identity-password-runtime"}
    raw = None
    if body is not None:
        raw = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    if bearer is not None:
        headers["Authorization"] = "Bearer " + bearer
    connection = http.client.HTTPConnection("127.0.0.1", parsed.port, timeout=8)
    try:
        connection.request(method, path, body=raw, headers=headers)
        response = connection.getresponse()
        content = response.read(1024 * 1024 + 1)
        check(len(content) <= 1024 * 1024, "runtime response exceeded bound")
        return response.status, response.getheaders(), json.loads(content)
    finally:
        connection.close()


def verify(fixture_path, probes_path, origin, output, core_pid, expected_core_sha256):
    fixture, probe = M.private_json(fixture_path), M.private_json(probes_path)
    check(probe.get("format_version") == 1 and probe.get("scope") == SCOPE and probe.get("synthetic_only") is True
          and probe["fixture_file_sha256"] == file_sha(fixture_path)
          and probe["database"] == fixture["connection"]["database"] and probe["fixture_nonce"] == fixture["fixture_nonce"]
          and probe["adapter_sha256"] == ADAPTER_SHA256
          and probe["expected_core78_manifest_sha256"] == M.U.CORE78_MANIFEST_SHA256, "runtime probe binding differs")
    check(core_pid > 1 and re.fullmatch(r"[a-f0-9]{64}", expected_core_sha256), "pinned running Core required")
    process = Path("/proc") / str(core_pid)
    start = (process / "stat").read_text().rsplit(") ", 1)[1].split()[19]
    check(file_sha(process / "exe") == expected_core_sha256, "running Core bytes differ")
    reader = Reader(fixture, output)
    events, issued_ids = [], []
    try:
        receipt_sql = "SELECT manifest FROM core_migration.import_runs WHERE migration_id=" + M.literal(probe["migration_id"])
        check(M.digest(reader.query(receipt_sql)) == probe["committed_manifest_sha256"], "committed identity receipt changed")
        for case in probe["cases"]:
            uid, aid = case["target_user_id"], case["target_personal_account_id"]
            credentials = reader.query(credential_count_sql())
            status, _, rejected = request(origin, "POST", "/api/user/login", body={
                "username": case["username"], "password": WRONG_PASSWORD})
            check(status == 401 and rejected.get("success") is False and rejected.get("code") == "AUTH_UNAUTHORIZED",
                  case["label"] + " wrong password was not rejected")
            check(reader.query(credential_count_sql()) == credentials, "wrong password issued a credential")
            events.append({"case": case["label"] + "_wrong_password", "status": status})
            for phase in ("bcrypt", "persisted_argon"):
                status, headers, logged_in = request(origin, "POST", "/api/user/login", body={
                    "username": case["username"], "password": case["password"]})
                check(status == 200 and logged_in.get("success") is True, case["label"] + " username login failed")
                data = logged_in["data"]
                check(data["user"]["id"] == uid and data["user"]["username"] == case["username"]
                      and data["session"]["login_method"] == "password" and data["token_type"] == "Bearer",
                      "browser login returned another owner or method")
                bearer = data["access_token"]
                status, _, principal = request(origin, "GET", "/core/v1/identity", bearer=bearer)
                check(status == 200 and principal["user_id"] == uid and principal["owner_account_id"] == aid
                      and principal["owner"] == {"kind": "personal", "id": uid}
                      and principal["credential_kind"] == "session", "issued browser Session has wrong authority")
                credential_id = principal["credential_id"]
                digest = hashlib.sha256(bearer.encode()).hexdigest()
                actual = reader.query("SELECT jsonb_build_object('credential_id',c.id,'user_id',c.user_id,"
                    "'owner_account_id',c.owner_account_id,'user_version',c.user_version,'current_version',u.auth_version,"
                    "'session_version',w.user_version,'assurance_version',a.user_version,'factor_level',a.factor_level,"
                    "'sid_matches',w.sid=" + M.literal(data["session"]["sid"]) + ","
                    "'bcrypt_allowed',p.legacy_bcrypt_allowed,'email_login_allowed',p.email_login_allowed,"
                    "'algorithm',CASE WHEN pl.password_hash LIKE '$argon2id$%' THEN 'argon2id' ELSE 'other' END) "
                    "FROM core_identity.credentials c JOIN core_identity.users u ON u.id=c.user_id "
                    "JOIN core_identity.user_profiles p ON p.user_id=u.id JOIN core_identity.password_logins pl ON pl.user_id=u.id "
                    "JOIN core_identity.web_sessions w ON w.current_credential_id=c.id "
                    "JOIN core_identity.session_authentication a ON a.credential_id=c.id "
                    "WHERE c.id=" + str(credential_id) + " AND c.digest=decode(" + M.literal(digest) + ",'hex')")
                expected = {"credential_id": credential_id, "user_id": uid, "owner_account_id": aid,
                            "user_version": case["expected_auth_version"], "current_version": case["expected_auth_version"],
                            "session_version": case["expected_auth_version"], "assurance_version": case["expected_auth_version"],
                            "factor_level": 1, "sid_matches": True, "bcrypt_allowed": False,
                            "email_login_allowed": False, "algorithm": "argon2id"}
                check(actual == expected, "actual password Session/readback differs from committed generation")
                check(any(k.lower() == "set-cookie" and "new_api_refresh=" in v and "HttpOnly" in v
                          for k, v in headers), "browser login did not issue its private refresh cookie")
                issued_ids.append(credential_id)
                events.append({"case": case["label"] + "_username_" + phase, "status": status,
                               "user_id": uid, "owner_account_id": aid, "auth_version": actual["user_version"],
                               "credential_id": credential_id, "stored_password_algorithm": actual["algorithm"]})
        previous = probe["preexisting_session"]
        status, _, principal = request(origin, "GET", "/core/v1/identity", bearer=previous["bearer"])
        check(status == 200 and all(principal[k] == previous[k] for k in ("credential_id", "user_id", "owner_account_id")),
              "unrelated native Session changed authority")
        status, _, _ = request(origin, "GET", "/core/v1/identity", bearer=probe["expired_session_bearer"])
        check(status == 401, "expired native Session was revived")
        after = reader.query(M.U.state_sql(probe["inventory"], probe["migration_id"]))
        check(money_state(after) == probe["before_http_money"], "password runtime changed financial state")
        check(stable_tables(after) == probe["before_http_stable_tables"], "password runtime changed unrelated application state")
        check(M.digest(reader.query(protected_rows_sql([c["target_user_id"] for c in probe["cases"]])))
              == probe["before_http_protected_rows_sha256"], "password runtime changed another owner's identity or Session")
        check(reader.query(credential_count_sql()) == probe["before_http_credential_count"] + len(issued_ids)
              and len(set(issued_ids)) == len(issued_ids), "unexpected credential issuance")
        check(M.digest(reader.query(receipt_sql)) == probe["committed_manifest_sha256"], "runtime rewrote identity receipt")
        check(file_sha(process / "exe") == expected_core_sha256
              and (process / "stat").read_text().rsplit(") ", 1)[1].split()[19] == start, "running Core changed during probe")
        result = {"status": "PASS", "scope": SCOPE, "actual_core_sha256": expected_core_sha256,
                  "password_runtime_login": "PASS", "events": events,
                  "history_session_auth_version": 3, "new_session_auth_version": 1,
                  "bcrypt_verified_then_upgraded_and_argon_relogin_verified": True,
                  "financial_state_unchanged": True, "preexisting_identity_and_sessions_preserved": True,
                  "production_access": False, "production_authorized": False, "real_google_login": "NOT_RUN",
                  "external_provider_calls": 0, "turnstile_provider_calls": 0,
                  "cleanup_owner": "parent disposable-fixture controller"}
        M.F.write_private(output, "result.public.json", M.canonical(result) + "\n")
        print(json.dumps(result))
    except BaseException as error:
        M.F.write_private(output, "failure.public.json", M.canonical({"status": "FAIL", "scope": SCOPE,
            "error_type": type(error).__name__, "events_completed": events, "production_access": False,
            "real_google_login": "NOT_RUN"}) + "\n")
        raise


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("prepare", "verify"))
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--probes", type=Path)
    parser.add_argument("--origin")
    parser.add_argument("--core-pid", type=int)
    parser.add_argument("--expected-core-sha256")
    args = parser.parse_args(argv)
    os.umask(0o077)
    check(file_sha(M.__file__) == ADAPTER_SHA256, "frozen identity adapter bytes differ")
    if args.phase == "prepare":
        Preparation(M.private_json(args.fixture), args.output).prepare_once(args.fixture)
    else:
        check(args.probes is not None and args.origin and args.core_pid and args.expected_core_sha256,
              "verify needs probes, origin, Core PID and exact executable SHA256")
        verify(args.fixture, args.probes, args.origin, args.output, args.core_pid, args.expected_core_sha256)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Cutover import from a restored legacy snapshot into a fresh current Core.

Reads the legacy schema read-only and writes one owner-only SERIALIZABLE
transaction per phase into a freshly initialized Core database. Each phase
refuses a non-empty target for the tables it owns, so a phase cannot run twice.
Legacy user IDs are kept as Core user and personal account IDs.

Connections use local Unix sockets only (libpq environment), never DSNs in argv.
Private SQL plans and reports are written under --output (0700, under ~/Backups).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

import legacy_cutover_history as HISTORY
import legacy_cutover_keys as KEYS
import legacy_cutover_money as MONEY

LOGIN = re.compile(r"[a-z0-9][a-z0-9_.-]{2,63}\Z")
BCRYPT = re.compile(r"\$2[aby]\$(0[4-9]|1[0-6])\$[./A-Za-z0-9]{53}\Z")
EMAIL = re.compile(r"[^@\s]+@[^@\s]+\.[^@\s]+\Z")
ROLE = {1: "user", 10: "admin", 100: "superadmin"}
OAUTH_ISSUERS = {
    "github_id": "https://github.com",
    "linux_do_id": "https://connect.linux.do",
    "discord_id": "https://discord.com",
}
CUSTOM_OAUTH_ISSUERS = {"google": "https://accounts.google.com"}
PHASES = ("identity", "opening") + KEYS.PHASES + MONEY.PHASES + HISTORY.PHASES


def fail(code: str) -> None:
    raise SystemExit(f"error: {code}")


def lit(value) -> str:
    if value is None:
        return "NULL"
    if isinstance(value, bool):
        return "TRUE" if value else "FALSE"
    if isinstance(value, int):
        return str(value)
    return "'" + str(value).replace("'", "''") + "'"


def jlit(value) -> str:
    return lit(json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))) + "::jsonb"


def digest(value) -> str:
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True, default=str,
                                     separators=(",", ":")).encode()).hexdigest()


class Db:
    def __init__(self, socket: Path, port: int, database: str, user: str, *, role: str | None = None,
                 readonly: bool):
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        self.env.update(PGHOST=str(socket), PGPORT=str(port), PGDATABASE=database, PGUSER=user,
                        PGAPPNAME="lmm-legacy-cutover-import", PGCONNECT_TIMEOUT="5")
        options = "-c default_transaction_read_only=" + ("on" if readonly else "off")
        if role:
            options += " -c role=" + role
        self.env["PGOPTIONS"] = options

    def run(self, sql: str) -> str:
        result = subprocess.run(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
                                input=sql, text=True, capture_output=True, env=self.env, check=False)
        if result.returncode:
            sys.stderr.write(result.stderr[-4000:])
            fail("psql_failed")
        return result.stdout


def load_source(src: Db, schema: str) -> dict:
    def q(sql: str) -> list[dict]:
        return json.loads(src.run(f"SET search_path={schema};SELECT COALESCE(json_agg(t),'[]') FROM ({sql}) t"))
    return {
        "users": q("SELECT * FROM users ORDER BY id"),
        "two_fas": q("SELECT user_id,is_enabled,deleted_at FROM two_fas ORDER BY id"),
        "oauth_bindings": q("SELECT b.user_id,b.provider_user_id,p.slug FROM user_oauth_bindings b "
                            "JOIN custom_oauth_providers p ON p.id=b.provider_id ORDER BY b.id"),
        "options": {r["key"]: r["value"] for r in q("SELECT key,value FROM options")},
        **KEYS.load_source(q),
    }


def developer_decision(user: dict) -> tuple[bool, str]:
    role = user.get("role")
    if role in (10, 100):
        return True, "admin_role"
    override = user.get("trust_level_override")
    if override is not None:
        return override > 0, "explicit_override_allow" if override else "explicit_override_deny"
    if (user.get("console_activated_at") or 0) > 0:
        return True, "approved_console_activation"
    return False, "not_activated"


def plan_identity(data: dict, archive_sha: str) -> tuple[list[str], dict]:
    users = data["users"]
    mfa = {r["user_id"] for r in data["two_fas"] if r["is_enabled"] and not r["deleted_at"]}
    usable = json.loads(data["options"].get("UserUsableGroups") or "{}")
    sql, stats, logins, emails = [], Counter(), {}, {}
    email_counts: dict[str, int] = {}
    for u in users:
        e = (u["email"] or "").strip().lower()
        if e:
            email_counts[e] = email_counts.get(e, 0) + 1
    for u in users:
        uid = u["id"]
        role = ROLE.get(u["role"])
        if role is None:
            fail("unknown_role")
        deleted = u["deleted_at"] is not None
        active = u["status"] == 1 and not deleted
        password = u["password"] or ""
        bcrypt = BCRYPT.fullmatch(password) is not None
        login = (u["username"] or "").lower()
        if bcrypt:
            # Browser login resolves the exact legacy username (or a unique email)
            # to this internal name, so non-ASCII or colliding names keep working.
            if not LOGIN.fullmatch(login) or login in logins:
                stats["password_login_internal_name"] += 1
                login = f"legacy-u{uid}"
            logins[login] = uid
        email = (u["email"] or "").strip().lower()
        if email and EMAIL.fullmatch(email) and 3 <= len(email) <= 254:
            emails.setdefault(email, []).append(uid)
        stats["active" if active else "inactive"] += 1
        stats["role_" + role] += 1
        sql.append("INSERT INTO core_identity.accounts(id,kind,service_level,active) OVERRIDING SYSTEM VALUE "
                   f"VALUES({uid},'personal',0,{lit(active)});")
        sql.append("INSERT INTO core_identity.users(id,personal_account_id,platform_role,active,auth_version) "
                   f"VALUES({uid},{uid},{lit(role)},{lit(active)},{max(1, u['auth_version'] or 1)});")
        # Legacy accepted username or email with the password; keep email login
        # only where the profile email is unique, exactly as stored.
        email_login = bool(u["email"]) and email_counts.get(email) == 1 and bcrypt
        stats["email_login_allowed"] += email_login
        sql.append("INSERT INTO core_identity.user_profiles(user_id,username,display_name,email,setting,"
                   "password_login_allowed,second_factor_required,legacy_bcrypt_allowed,email_login_allowed) VALUES("
                   f"{uid},{lit(u['username'])},{lit((u['display_name'] or '')[:256])},"
                   f"{lit((u['email'] or '')[:320])},{lit(u['setting'] or '{}')},"
                   f"{lit(bcrypt)},{lit(uid in mfa)},{lit(bcrypt)},{lit(email_login)});")
        granted, reason = developer_decision(u)
        sql.append("INSERT INTO core_identity.developer_access(user_id,granted,source_reference,revision) "
                   f"VALUES({uid},{lit(granted)},{lit('legacy:' + reason)},1);")
        # Legacy keys may use the user's own group as well as every usable group.
        own = (u.get("group") or "default").strip() or "default"
        for group in dict.fromkeys([*usable, own]):
            sql.append("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) "
                       f"VALUES({uid},{lit(group)},TRUE,"
                       f"{lit('legacy:UserUsableGroups' if group in usable else 'legacy:user_group')});")
        for column, issuer in OAUTH_ISSUERS.items():
            subject = (u.get(column) or "").strip()
            if subject:
                sql.append("INSERT INTO core_identity.oauth_identities(issuer,subject,user_id) "
                           f"VALUES({lit(issuer)},{lit(subject[:255])},{uid}) ON CONFLICT DO NOTHING;")
                stats["oauth_" + column] += 1
        if uid in mfa:
            stats["second_factor_required"] += 1
    for login, uid in logins.items():
        password = next(u["password"] for u in users if u["id"] == uid)
        sql.append("INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) "
                   f"VALUES({lit(login)},{uid},{lit(password)});")
    for email, owners in emails.items():
        if len(owners) == 1:
            sql.append("INSERT INTO core_identity.email_addresses(email_key,user_id,state,email_version) "
                       f"VALUES({lit(email)},{owners[0]},'legacy_unverified',0);")
        else:
            sql.append("INSERT INTO core_identity.email_addresses(email_key,user_id,state,email_version) "
                       f"VALUES({lit(email)},NULL,'conflict',0);")
            stats["email_conflict"] += 1
    for b in data["oauth_bindings"]:
        issuer = CUSTOM_OAUTH_ISSUERS.get(b["slug"])
        if issuer is None:
            stats["oauth_unsupported_provider"] += 1
            continue
        sql.append("INSERT INTO core_identity.oauth_identities(issuer,subject,user_id) "
                   f"VALUES({lit(issuer)},{lit(b['provider_user_id'][:255])},{b['user_id']}) ON CONFLICT DO NOTHING;")
        stats["oauth_" + b["slug"]] += 1
    top = max(u["id"] for u in users)
    sql.append(f"SELECT setval(pg_get_serial_sequence('core_identity.users','id'),{top});")
    sql.append(f"SELECT setval(pg_get_serial_sequence('core_identity.accounts','id'),{top});")
    stats.update(users=len(users), password_logins=len(logins), emails=len(emails),
                 routing_groups_per_user=len(usable))
    return sql, dict(stats)


def plan_opening(data: dict, archive_sha: str, migration_id: str, observed_at: int) -> tuple[list[str], dict]:
    users = data["users"]
    mapping = digest([[u["id"], u["id"]] for u in users])
    sql, stats = [], Counter()
    for u in users:
        uid = u["id"]
        row = digest({k: u[k] for k in ("id", "quota", "used_quota", "request_count", "aff_quota", "aff_history")})
        base = {"migration_id": migration_id, "source_entity_key": f"user:{uid}",
                "source_archive_sha256": archive_sha, "mapping_sha256": mapping,
                "source_row_sha256": row, "observed_at": observed_at, "account_id": uid}
        sql.append("SELECT core_billing.import_opening_balance(" + jlit({
            **base, "available_units": u["quota"] or 0, "reserved_units": 0,
            "affiliate_units": u["aff_quota"] or 0}) + ");")
        sql.append("SELECT core_billing.import_account_opening_counters(" + jlit({
            **base, "user_id": uid, "used_quota": u["used_quota"] or 0,
            "request_count": u["request_count"] or 0, "aff_history": u["aff_history"] or 0}) + ");")
        stats["available_units"] += u["quota"] or 0
        stats["affiliate_units"] += u["aff_quota"] or 0
        stats["used_quota"] += u["used_quota"] or 0
        stats["debt_accounts"] += (u["quota"] or 0) < 0
    stats["accounts"] = len(users)
    return sql, dict(stats)


GUARDS = {
    "identity": "SELECT count(*) FROM core_identity.users",
    "opening": "SELECT count(*) FROM core_billing.opening_imports",
    **KEYS.GUARDS,
    **MONEY.GUARDS,
    **HISTORY.GUARDS,
}


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--socket", type=Path, required=True)
    p.add_argument("--port", type=int, required=True)
    p.add_argument("--source-db", required=True)
    p.add_argument("--source-user", required=True)
    p.add_argument("--source-schema", required=True)
    p.add_argument("--target-db", required=True)
    p.add_argument("--target-login", required=True)
    p.add_argument("--target-owner", required=True)
    p.add_argument("--archive-sha256", required=True)
    p.add_argument("--migration-id", required=True)
    p.add_argument("--observed-at", type=int, required=True, help="Unix seconds of the source boundary")
    p.add_argument("--phase", choices=PHASES, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--apply", action="store_true")
    p.add_argument("--api-key-encryption-key-file", type=Path, help="keys: Core 32-byte recovery key")
    HISTORY.add_arguments(p)
    a = p.parse_args()
    if not re.fullmatch(r"[0-9a-f]{64}", a.archive_sha256):
        fail("archive_sha256_invalid")
    a.output.mkdir(mode=0o700, parents=False, exist_ok=True)
    src = Db(a.socket, a.port, a.source_db, a.source_user, readonly=True)
    data = load_source(src, a.source_schema)
    target = Db(a.socket, a.port, a.target_db, a.target_login, role=a.target_owner, readonly=True)
    if a.phase in KEYS.PHASES:
        sql, stats = KEYS.plan(a.phase, data, a, target)
    elif a.phase in MONEY.PHASES:
        data.update(MONEY.load_source(src, a.source_schema))
        sql, stats = MONEY.plan(a.phase, data, a.archive_sha256, a.migration_id, a.observed_at)
    elif a.phase in HISTORY.PHASES:
        sql, stats = HISTORY.plan(a.phase, a, src, target)
    elif a.phase == "identity":
        sql, stats = plan_identity(data, a.archive_sha256)
    else:
        sql, stats = plan_opening(data, a.archive_sha256, a.migration_id, a.observed_at)
    script = ("BEGIN ISOLATION LEVEL SERIALIZABLE;\nSET LOCAL synchronous_commit=on;\n"
              f"DO $$BEGIN IF ({GUARDS[a.phase]})<>0 THEN RAISE EXCEPTION 'phase target not empty'; END IF; END$$;\n"
              + "\n".join(sql) + "\nCOMMIT;\n")
    plan = a.output / f"{a.phase}.sql"
    fd = os.open(plan, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as stream:
        stream.write(script)
    report = {"phase": a.phase, "migration_id": a.migration_id, "statements": len(sql),
              "plan_sha256": hashlib.sha256(script.encode()).hexdigest(), "stats": stats}
    if a.apply:
        Db(a.socket, a.port, a.target_db, a.target_login, role=a.target_owner, readonly=False).run(script)
        report["applied"] = True
        if a.phase in KEYS.PHASES:
            report.update(KEYS.after_apply(a.phase, data, a.output, target))
    (a.output / f"{a.phase}-report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False))
    print(json.dumps(report, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())

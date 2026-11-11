#!/usr/bin/env python3
"""Cutover import from a restored legacy snapshot into a fresh current Core.

Reads the legacy schema read-only and writes one owner-only SERIALIZABLE
transaction per phase into a freshly initialized Core database. Each phase
refuses a non-empty target for the tables it owns, so a phase cannot run twice.
Continuation phases consume the committed safe identity map. Native user IDs
are preserved; personal accounts are resolved independently through that map.

Sources use private Unix sockets. Targets support receipt-bound loopback SCRAM
with private passfiles and an independent installer LOGIN.
Private SQL plans and reports are written under --output (0700, under ~/.local/state/lmm-api).
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
import legacy_referral_import as REFERRALS

from legacy_sql import literal as lit

LOGIN = re.compile(r"[a-z0-9][a-z0-9_.-]{2,63}\Z")
BCRYPT = re.compile(r"\$2[aby]\$(0[4-9]|1[0-6])\$[./A-Za-z0-9]{53}\Z")
EMAIL = re.compile(r"[^@\s]+@[^@\s]+\.[^@\s]+\Z")
ROLE = {1: "user", 10: "admin", 100: "superadmin"}
OAUTH_ISSUERS = {
    "github_id": "https://github.com",
    "linux_do_id": "https://connect.linux.do",
    "discord_id": "https://discord.com",
}
# The production custom Google provider is projected into Core59's native
# Google identity domain ONLY after checking its actual source configuration.
# A slug alone is not evidence of the authority that issued a subject.
GOOGLE_SOURCE_CONTRACT = {
    "slug": "google",
    "authorization_endpoint": "https://accounts.google.com/o/oauth2/v2/auth",
    "token_endpoint": "https://oauth2.googleapis.com/token",
    "user_info_endpoint": "https://openidconnect.googleapis.com/v1/userinfo",
    "well_known": "https://accounts.google.com/.well-known/openid-configuration",
    "user_id_field": "sub",
    "username_field": "email",
    "display_name_field": "name",
    "email_field": "email",
}
PHASES = ("identity", "opening", "routing_groups") + KEYS.PHASES + MONEY.PHASES + HISTORY.PHASES + ("referrals",)
MAPPED_PHASES = ("opening", "routing_groups", "keys", "subscriptions", "bounties", "wallet", "history", "passkeys")


def fail(code: str) -> None:
    raise SystemExit(f"error: {code}")


def jlit(value) -> str:
    return lit(json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))) + "::jsonb"


def digest(value) -> str:
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True, default=str,
                                     separators=(",", ":")).encode()).hexdigest()


def load_identity_binding(map_path: Path, manifest_path: Path, archive_sha: str,
                          source_user_ids: list[int]) -> dict:
    """Reuse the formal financial planner's identity provenance contract."""
    import financial_import as F
    map_path = F.IDENTITY.private_backup_path(map_path)
    manifest_path = F.IDENTITY.private_backup_path(manifest_path)
    rows = [F.PLANNER.parse_json(line) for line in map_path.read_text().splitlines() if line]
    accounts = F.PLANNER.identity_index(rows)
    manifest = F.PLANNER.parse_json(manifest_path.read_text())
    provenance = F.PLANNER.bind_identity_provenance(manifest,
        map_file_sha256=F.IDENTITY.sha256_file(map_path), source_archive_sha256=archive_sha)
    if len(source_user_ids) != len(set(source_user_ids)) or set(accounts) != set(source_user_ids):
        fail("identity_source_user_coverage_mismatch")
    return {"accounts": accounts, "provenance": provenance}


def verify_identity_binding(target, binding: dict) -> None:
    actual = json.loads(target.run("SELECT json_build_object('users',"
        "(SELECT coalesce(json_agg(json_build_object('id',u.id,'personal_account_id',u.personal_account_id,"
        "'kind',a.kind)),'[]') FROM core_identity.users u LEFT JOIN core_identity.accounts a "
        "ON a.id=u.personal_account_id),'runs',"
        "(SELECT coalesce(json_agg(to_jsonb(r)),'[]') FROM core_migration.import_runs r "
        "WHERE manifest->>'scope'='identity-only'))"))
    proof = binding["provenance"]
    runs = actual["runs"]
    if len(runs) != 1 or any(runs[0][key] != expected for key, expected in (
        ("source_archive_sha256", proof["source_archive_sha256"]),
        ("mapping_sha256", proof["identity_mapping_sha256"]),
        ("manifest", proof["expected_identity_manifest"]))):
        fail("target_identity_import_run_mismatch")
    users = actual["users"]
    if (len(users) != len(binding["accounts"])
            or any(row["kind"] != "personal" for row in users)
            or {row["id"]: row["personal_account_id"] for row in users} != binding["accounts"]):
        fail("target_identity_account_map_mismatch")


def identity_guard_sql(binding: dict) -> str:
    """Repeat provenance and native user/account readback under transaction locks."""
    proof = binding["provenance"]
    rows = [{"id": uid, "personal_account_id": aid} for uid, aid in sorted(binding["accounts"].items())]
    return f"""DO $identity_binding$
BEGIN
 PERFORM pg_advisory_xact_lock(741803121);
 LOCK TABLE core_identity.users,core_identity.accounts,core_migration.import_runs IN SHARE MODE;
 IF (SELECT count(*) FROM core_migration.import_runs WHERE manifest->>'scope'='identity-only')<>1
 OR NOT EXISTS(SELECT 1 FROM core_migration.import_runs
  WHERE source_archive_sha256={lit(proof['source_archive_sha256'])}
   AND mapping_sha256={lit(proof['identity_mapping_sha256'])}
   AND manifest={jlit(proof['expected_identity_manifest'])}) THEN
  RAISE EXCEPTION 'target_identity_import_run_mismatch';
 END IF;
 IF (SELECT count(*) FROM core_identity.users)<>{len(rows)}
 OR EXISTS(SELECT 1 FROM jsonb_to_recordset({jlit(rows)}) e(id BIGINT,personal_account_id BIGINT)
  LEFT JOIN core_identity.users u ON u.id=e.id
  LEFT JOIN core_identity.accounts a ON a.id=u.personal_account_id
  WHERE u.personal_account_id IS DISTINCT FROM e.personal_account_id OR a.kind IS DISTINCT FROM 'personal') THEN
  RAISE EXCEPTION 'target_identity_account_map_mismatch';
 END IF;
END $identity_binding$;"""


class Db:
    def __init__(self, socket: Path, port: int, database: str, user: str, *, role: str | None = None,
                 readonly: bool, connection: dict | None = None):
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        if connection is not None:
            import financial_import as F
            owner, login = F.IDENTITY.connection_roles(connection)
            if (database, user, role) != (connection["database"], login, owner):
                fail("target_connection_binding_mismatch")
            self.env.update(F.IDENTITY.socket_connection(connection))
            self.env.update(PGAPPNAME="lmm-legacy-cutover-import", PGCONNECT_TIMEOUT="5",
                            PGOPTIONS=F.IDENTITY.psql_options(connection, readonly=readonly))
            return
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
        # Do not JOIN away orphaned bindings or read provider client secrets.
        "oauth_bindings": q("SELECT id,user_id,provider_id,provider_user_id FROM user_oauth_bindings ORDER BY id"),
        "oauth_providers": q("SELECT id,name,slug,enabled,client_id,authorization_endpoint,token_endpoint,"
                             "user_info_endpoint,scopes,well_known,user_id_field,username_field,"
                             "display_name_field,email_field,auth_style,access_policy "
                             "FROM custom_oauth_providers ORDER BY id"),
        "options": {r["key"]: r["value"] for r in q("SELECT key,value FROM options")},
        **KEYS.load_source(q),
    }


def load_routing_source(src: Db, schema: str) -> dict:
    # This continuation needs no passwords, token secrets or provider settings.
    return json.loads(src.run(f"SET search_path={schema};SELECT json_build_object("
        "'users',(SELECT coalesce(json_agg(u ORDER BY id),'[]') FROM (SELECT id,\"group\" FROM users) u),"
        "'options',(SELECT coalesce(json_object_agg(key,value),'{}') FROM options "
        "WHERE key='UserUsableGroups'))"))


def routing_group_rows(user: dict, usable: dict) -> list[dict]:
    if type(usable) is not dict or type(user["id"]) is not int or user["id"] < 1:
        fail("routing_groups_source_invalid")
    own = (user.get("group") or "default").strip() or "default"
    rows = []
    for group in dict.fromkeys([*usable, own]):
        if type(group) is not str or not 1 <= len(group) <= 64 or group == "auto":
            fail("routing_group_name_invalid")
        rows.append(dict(user_id=user["id"], group_name=group, enabled=True,
                         source_reference="legacy:UserUsableGroups" if group in usable else "legacy:user_group"))
    return rows


def routing_group_sql(row: dict) -> str:
    return ("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) "
            f"VALUES({row['user_id']},{lit(row['group_name'])},TRUE,{lit(row['source_reference'])});")


def plan_routing_groups(data: dict, archive_sha: str, migration_id: str) -> tuple[list[str], dict]:
    usable = json.loads(data["options"].get("UserUsableGroups") or "{}")
    rows = [row for user in data["users"] for row in routing_group_rows(user, usable)]
    proof = data["identity_binding"]["provenance"]
    manifest = dict(scope="legacy-cutover-routing-groups.v1", source_archive_sha256=archive_sha,
                    identity_mapping_sha256=proof["identity_mapping_sha256"],
                    identity_map_file_sha256=proof["identity_map_file_sha256"],
                    identity_manifest_sha256=proof["identity_manifest_sha256"],
                    source_user_groups_sha256=digest([{k: user.get(k) for k in ("id", "group")} for user in data["users"]]),
                    source_usable_groups_sha256=digest(usable), grants_sha256=digest(rows), grants=len(rows))
    sql = [routing_group_sql(row) for row in rows]
    sql.append("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) "
               f"VALUES({lit(migration_id)},{lit(archive_sha)},{lit(digest(rows))},{jlit(manifest)});")
    return sql, dict(users=len(data["users"]), grants=len(rows), grants_sha256=digest(rows))


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


def registration_time_receipt_sql(users: list[dict], archive_sha: str) -> str:
    """Seal source registration times only after exact owner-side readback.

    This can backfill an already imported, stopped rehearsal cohort. It never
    changes accounts, credentials or balances, and refuses a partial cohort.
    """
    if not isinstance(archive_sha, str) or re.fullmatch(r"[0-9a-f]{64}", archive_sha) is None:
        fail("archive_sha256_invalid")
    times = {}
    for user in users:
        uid, at = user["id"], user.get("created_at")
        if type(uid) is not int or not 0 < uid <= 9223372036854775807 or str(uid) in times:
            fail("registration_user_id_invalid")
        if at is not None and (type(at) is not int or not 0 <= at <= 253402300799):
            fail("registration_time_invalid")
        times[str(uid)] = at or 0
    if not times:
        fail("registration_cohort_empty")
    manifest = {"scope": "legacy-cutover-identity.v1", "source_archive_sha256": archive_sha,
                "registration_times": times,
                "summary": {"registration_time_known_count": sum(bool(v) for v in times.values()),
                            "registration_time_unknown_count": sum(not v for v in times.values())}}
    run_id = "legacy-identity-registration-" + archive_sha[:32]
    return f"""DO $registration_times$
DECLARE expected JSONB := {jlit(times)};
BEGIN
 IF current_user<>pg_get_userbyid((SELECT relowner FROM pg_class WHERE oid='core_identity.accounts'::regclass))
 OR current_user<>pg_get_userbyid((SELECT relowner FROM pg_class WHERE oid='core_migration.import_runs'::regclass))
 OR current_setting('transaction_isolation')<>'serializable' THEN
  RAISE EXCEPTION 'registration receipt requires serializable identity owner';
 END IF;
 LOCK TABLE core_identity.accounts,core_identity.users IN SHARE MODE;
 LOCK TABLE core_migration.import_runs IN SHARE ROW EXCLUSIVE MODE;
 IF (SELECT count(*) FROM core_identity.accounts)<>{len(times)}
 OR (SELECT count(*) FROM core_identity.users)<>{len(times)}
 OR EXISTS(SELECT 1 FROM jsonb_each(expected) e
  LEFT JOIN core_identity.accounts a ON a.id=e.key::bigint
  LEFT JOIN core_identity.users u ON u.id=a.id
  WHERE a.id IS NULL OR u.personal_account_id IS DISTINCT FROM a.id
  OR to_jsonb(floor(extract(epoch FROM a.created_at))::bigint) IS DISTINCT FROM e.value) THEN
  RAISE EXCEPTION 'registration source cohort or timestamps mismatch';
 END IF;
 INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
 VALUES({lit(run_id)},{lit(archive_sha)},{lit(digest(times))},{jlit(manifest)}) ON CONFLICT DO NOTHING;
 IF NOT EXISTS(SELECT 1 FROM core_migration.import_runs WHERE migration_id={lit(run_id)}
  AND source_archive_sha256={lit(archive_sha)} AND mapping_sha256={lit(digest(times))}
  AND manifest={jlit(manifest)}) THEN
  RAISE EXCEPTION 'registration receipt conflicts with prior import';
 END IF;
END $registration_times$;"""


def identity_oauth_rows(data: dict) -> list[tuple[str, str, int, str]]:
    """Validate source provenance/ownership before emitting any identity SQL.

    Core59 subjects have a 255-byte protocol limit. Reject unsupported input
    rather than truncating, trimming, relinking by email, or ignoring conflicts.
    The phase targets an empty identity store; ordinary INSERT also makes any
    unexpected target collision fail the surrounding SERIALIZABLE transaction.
    """
    rows, owners = [], {}
    user_ids = {u["id"] for u in data["users"]}

    def add(issuer, subject, user, counter):
        if (not isinstance(subject, str) or not subject or subject != subject.strip()
                or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in subject)):
            fail("oauth_subject_invalid")
        try:
            size = len(subject.encode("utf-8"))
        except UnicodeError:
            fail("oauth_subject_invalid")
        if size > 255:
            fail("oauth_subject_too_long")
        key = (issuer, subject)
        if key in owners:
            fail("oauth_subject_owner_collision" if owners[key] != user else "oauth_subject_duplicate")
        owners[key] = user
        rows.append((issuer, subject, user, counter))

    for user in data["users"]:
        for column, issuer in OAUTH_ISSUERS.items():
            subject = user.get(column)
            if subject is not None and subject != "":
                add(issuer, subject, user["id"], "oauth_" + column)

    bindings = data["oauth_bindings"]
    if bindings and "oauth_providers" not in data:
        fail("oauth_provider_source_missing")
    providers = {}
    for provider in data.get("oauth_providers", []):
        identifier = provider.get("id")
        if type(identifier) is not int or not 0 < identifier <= (1 << 63) - 1:
            fail("oauth_provider_source_invalid")
        if identifier in providers:
            fail("oauth_provider_source_duplicate")
        providers[identifier] = provider
    binding_ids = set()
    for binding in bindings:
        identifier = binding.get("id")
        if type(identifier) is not int or identifier <= 0 or identifier in binding_ids:
            fail("oauth_binding_source_invalid")
        binding_ids.add(identifier)
        provider_id, user = binding.get("provider_id"), binding.get("user_id")
        if type(provider_id) is not int or provider_id not in providers:
            fail("oauth_binding_provider_missing")
        if type(user) is not int or user not in user_ids:
            fail("oauth_binding_user_missing")
        provider = providers[provider_id]
        if any(provider.get(key) != value for key, value in GOOGLE_SOURCE_CONTRACT.items()):
            fail("google_provider_source_contract_mismatch")
        client = provider.get("client_id")
        scopes = provider.get("scopes")
        policy = provider.get("access_policy")
        if (provider.get("enabled") is not True
                or type(provider.get("auth_style")) is not int or provider["auth_style"] != 0
                or not isinstance(client, str) or not client or client != client.strip()
                or len(client.encode("utf-8")) > 256 or any(ord(c) < 32 or ord(c) == 127 for c in client)
                or not isinstance(scopes, str) or len(scopes.split()) != 3
                or set(scopes.split()) != {"openid", "profile", "email"}
                or policy is not None and (not isinstance(policy, str) or policy.strip())):
            fail("google_provider_source_contract_mismatch")
        add("https://accounts.google.com", binding.get("provider_user_id"), user, "oauth_google")
    return rows


def plan_identity(data: dict, archive_sha: str) -> tuple[list[str], dict]:
    users = data["users"]
    oauth_rows = identity_oauth_rows(data)
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
        registered_at = u.get("created_at")
        if registered_at is not None and (type(registered_at) is not int or not 0 <= registered_at <= 253402300799):
            fail("registration_time_invalid")
        stats["registration_time_known_count" if registered_at else "registration_time_unknown_count"] += 1
        # Preserve source registration time. Epoch zero marks an unknown time;
        # using the account default would silently substitute the import time.
        sql.append("INSERT INTO core_identity.accounts(id,kind,service_level,active,created_at) OVERRIDING SYSTEM VALUE "
                   f"VALUES({uid},'personal',0,{lit(active)},to_timestamp({registered_at or 0}));")
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
        sql.extend(routing_group_sql(row) for row in routing_group_rows(u, usable))
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
    for issuer, subject, user, counter in oauth_rows:
        sql.append("INSERT INTO core_identity.oauth_identities(issuer,subject,user_id) "
                   f"VALUES({lit(issuer)},{lit(subject)},{user});")
        stats[counter] += 1
    sql.append(registration_time_receipt_sql(users, archive_sha))
    top = max(u["id"] for u in users)
    sql.append(f"SELECT setval(pg_get_serial_sequence('core_identity.users','id'),{top});")
    sql.append(f"SELECT setval(pg_get_serial_sequence('core_identity.accounts','id'),{top});")
    stats.update(users=len(users), password_logins=len(logins), emails=len(emails),
                 routing_groups_per_user=len(usable))
    return sql, dict(stats)


def plan_opening(data: dict, archive_sha: str, migration_id: str, observed_at: int) -> tuple[list[str], dict]:
    users = data["users"]
    mapping = digest([[u["id"], KEYS.personal_account(data, u["id"])] for u in users])
    sql, stats = [], Counter()
    for u in users:
        uid = u["id"]
        row = digest({k: u[k] for k in ("id", "quota", "used_quota", "request_count", "aff_quota", "aff_history")})
        base = {"migration_id": migration_id, "source_entity_key": f"user:{uid}",
                "source_archive_sha256": archive_sha, "mapping_sha256": mapping,
                "source_row_sha256": row, "observed_at": observed_at,
                "account_id": KEYS.personal_account(data, uid)}
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
    "routing_groups": "SELECT count(*) FROM core_identity.user_routing_groups",
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
    p.add_argument("--subscriptions-plans-pending-only", action="store_true",
                   help="subscriptions: import plans/pending liabilities; defer grants/consumption to formal subscription opening")
    p.add_argument("--api-key-encryption-key-file", type=Path, help="keys: Core 32-byte recovery key")
    p.add_argument("--identity-map", type=Path, help="Original safe source-target-id-map.private.jsonl")
    p.add_argument("--identity-manifest", type=Path, help="Original committed identity-only manifest.private.json")
    p.add_argument("--target-receipt", type=Path,
                   help="Actual endpoint receipt with connection and instance_identity; supports bound loopback SCRAM")
    p.add_argument("--referral-source", type=Path, help="referrals: private frozen source export")
    p.add_argument("--referral-review", type=Path, help="referrals: source-bound reviewed credit terms")
    HISTORY.add_arguments(p)
    a = p.parse_args()
    if a.subscriptions_plans_pending_only and a.phase != "subscriptions":
        fail("subscriptions_plans_pending_only_requires_subscriptions_phase")
    phase_guard = (MONEY.SUBSCRIPTION_PLANS_PENDING_GUARD
                   if a.subscriptions_plans_pending_only else GUARDS[a.phase])
    if not re.fullmatch(r"[0-9a-f]{64}", a.archive_sha256):
        fail("archive_sha256_invalid")
    if a.phase in MAPPED_PHASES and (a.identity_map is None or a.identity_manifest is None):
        fail("committed_identity_map_and_manifest_required")
    target_connection = None
    if a.target_receipt is not None:
        import financial_import as F
        endpoint = F.PLANNER.parse_json(F.IDENTITY.private_backup_path(a.target_receipt).read_text())
        if endpoint.get("source_archive_sha256") != a.archive_sha256:
            fail("target_endpoint_source_mismatch")
        target_connection = endpoint["connection"]
        target_probe = Db(a.socket, a.port, a.target_db, a.target_login, role=a.target_owner,
                          readonly=True, connection=target_connection)
        actual = json.loads(target_probe.run(F.IDENTITY.IDENTITY_SQL))
        F.IDENTITY.verify_instance(actual, target_connection, endpoint["instance_identity"], source=False)
    a.output.mkdir(mode=0o700, parents=False, exist_ok=True)
    if a.phase == "referrals":
        if a.referral_source is None or a.referral_review is None:
            fail("referrals_require_source_and_review")
        source = json.loads(a.referral_source.read_text())
        review = json.loads(a.referral_review.read_text())
        script = REFERRALS.plan(source, review, a.archive_sha256, a.migration_id)
        sql = script.splitlines()
        stats = {"bindings": len(source["users"]), "raw_rewards": len(source["rewards"]),
                 "historical_entries": len(source["ledger_entries"] if "ledger_entries" in source else source["ledger"]),
                 "executable_pending": len(review["pending"])}
    else:
        src = Db(a.socket, a.port, a.source_db, a.source_user, readonly=True)
        data = (load_routing_source(src, a.source_schema) if a.phase == "routing_groups"
                else load_source(src, a.source_schema))
        target = Db(a.socket, a.port, a.target_db, a.target_login, role=a.target_owner,
                    readonly=True, connection=target_connection)
        guards = []
        if a.phase in MAPPED_PHASES:
            binding = load_identity_binding(a.identity_map, a.identity_manifest, a.archive_sha256,
                                           [u["id"] for u in data["users"]])
            verify_identity_binding(target, binding)
            data["identity_binding"] = a.identity_binding = binding
            guards.append(identity_guard_sql(binding))
        if a.phase in KEYS.PHASES:
            sql, stats = KEYS.plan(a.phase, data, a, target)
        elif a.phase in MONEY.PHASES:
            data.update(MONEY.load_source(src, a.source_schema))
            if a.phase in ("subscriptions", "bounties", "wallet"):
                MONEY.bind_opening(data, target, a.archive_sha256, a.observed_at)
                guards.append(MONEY.opening_guard_sql(data))
            if a.subscriptions_plans_pending_only:
                sql, stats = MONEY.plan_subscriptions(data, a.archive_sha256, a.migration_id, a.observed_at,
                                                      plans_pending_only=True)
            else:
                sql, stats = MONEY.plan(a.phase, data, a.archive_sha256, a.migration_id, a.observed_at)
        elif a.phase in HISTORY.PHASES:
            sql, stats = HISTORY.plan(a.phase, a, src, target)
        elif a.phase == "routing_groups":
            sql, stats = plan_routing_groups(data, a.archive_sha256, a.migration_id)
        elif a.phase == "identity":
            sql, stats = plan_identity(data, a.archive_sha256)
        else:
            sql, stats = plan_opening(data, a.archive_sha256, a.migration_id, a.observed_at)
        script = ("BEGIN ISOLATION LEVEL SERIALIZABLE;\nSET LOCAL synchronous_commit=on;\n"
                  + "\n".join(guards) + "\n"
                  f"DO $$BEGIN IF ({phase_guard})<>0 THEN RAISE EXCEPTION 'phase target not empty'; END IF; END$$;\n"
                  + "\n".join(sql) + "\nCOMMIT;\n")
    if target_connection is not None:
        # Persist and execute the exact same endpoint-bound SQL.
        stable = {key: value for key, value in actual.items() if key != "read_only"}
        guard = ("DO $$BEGIN IF ((" + F.IDENTITY.IDENTITY_SQL.removesuffix(";").removeprefix("SELECT ")
                 + ")::jsonb - 'read_only') IS DISTINCT FROM " + jlit(stable)
                 + " THEN RAISE EXCEPTION 'target_endpoint_changed'; END IF; END$$;\n")
        script = script.replace("SET LOCAL synchronous_commit=on;",
                                "SET LOCAL synchronous_commit=on;\n" + guard, 1)
    plan = a.output / f"{a.phase}.sql"
    fd = os.open(plan, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as stream:
        stream.write(script)
    report = {"phase": a.phase, "migration_id": a.migration_id, "statements": len(sql),
              "plan_sha256": hashlib.sha256(script.encode()).hexdigest(), "stats": stats}
    if a.subscriptions_plans_pending_only:
        report["subscription_grants"] = "deferred-to-formal-subscription-opening"
    if a.apply:
        Db(a.socket, a.port, a.target_db, a.target_login, role=a.target_owner,
           readonly=False, connection=target_connection).run(script)
        report["applied"] = True
        if a.phase in MAPPED_PHASES:
            verify_identity_binding(target, binding)
            report["identity_binding_readback"] = {"matched_users": len(binding["accounts"]),
                "identity_manifest_sha256": binding["provenance"]["identity_manifest_sha256"]}
        if a.phase in KEYS.PHASES:
            report.update(KEYS.after_apply(a.phase, data, a.output, target))
    (a.output / f"{a.phase}-report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False))
    print(json.dumps(report, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())

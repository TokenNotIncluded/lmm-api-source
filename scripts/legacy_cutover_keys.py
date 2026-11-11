"""Cutover phases `keys` and `console` for legacy_cutover_import.py.

keys: legacy tokens -> Core API-key credentials, settings, funding rule and a
lifetime key budget with its imported opening consumption. Token IDs are kept
as credential IDs. Raw keys only become SHA-256 digests (and, when the source
key was recoverable, AES-GCM ciphertext under the Core api_key_encryption_key).
After a committed apply the phase publishes the credential receipt and the
legacy wire-alias manifest that Core's [model].legacy_key_aliases loads, so
both `<key>` and `sk-<key>` authenticate.

console: the legacy options Core consumes, announcement read receipts and the
setup record -> core_console.*.
"""
from __future__ import annotations

import hashlib
import ipaddress
import json
import os
import re
import secrets
import struct
from collections import Counter
from pathlib import Path

from legacy_sql import literal as lit

PHASES = ("keys", "console")
GUARDS = {
    "keys": "SELECT count(*) FROM core_identity.credentials WHERE kind='api_key'",
    "console": "SELECT (SELECT count(*) FROM core_console.options)+(SELECT count(*) FROM core_console.announcement_reads)"
               "+(SELECT count(*) FROM core_console.initialization)",
}
NEVER = 253402300799
LIFETIME_START, LIFETIME_END = -62135596800, 253402300800
PREFERENCES = {"subscription_first", "wallet_first", "subscription_only", "wallet_only"}
RESERVED_PREFIXES = ("lmmk_", "lmms_", "lmmr_", "lmmi_", "lmmf_", "lmmt_", "lmmo_")
# Options the current Core reads from core_console.options (options_http.rs,
# console_compat.rs, http.rs, edge_access.rs). Everything else is reported.
CONSOLE_KEYS = ("Notice", "console_setting.announcements", "Announcements", "HeaderNavModules",
                "IPAccessRoutingRules", "IPAccessRoutingBypassEligibilityV1",
                "payment_setting.compliance_confirmed", "payment_setting.compliance_terms_version")
RECEIPT_FILE = "credential-mapping-receipt.private.json"
ALIAS_FILE = "legacy-api-key-wire-aliases.private.json"
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
_pending: dict = {}


def fail(code: str) -> None:
    raise SystemExit(f"error: {code}")


def personal_account(data: dict, user_id: int) -> int:
    """Resolve an account through the verified identity map, never a user ID."""
    accounts = data.get("identity_binding", {}).get("accounts", {})
    account = accounts.get(user_id)
    if type(user_id) is not int or type(account) is not int or account <= 0:
        fail("personal_account_mapping_missing")
    return account


def arr(values, cast: str = "text[]") -> str:
    return "ARRAY[" + ",".join(lit(v) for v in values) + f"]::{cast}"


def canonical(value) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_source(q) -> dict:
    return {
        "tokens": q("SELECT * FROM tokens ORDER BY id"),
        "announcement_reads": q("SELECT user_id,announcement_id,revision,read_at FROM announcement_reads ORDER BY id"),
        "setups": q("SELECT * FROM setups ORDER BY id"),
    }


def ordinary(raw: str) -> bool:
    """Core model_http/legacy_keys.rs ordinary()."""
    lower = raw.lower()
    return (re.fullmatch(r"[A-Za-z0-9_]{32,256}", raw) is not None and "lmm_" not in lower
            and "oauth_managed_" not in lower and not lower.startswith(RESERVED_PREFIXES))


def _cipher(key: bytes):
    from cryptography.hazmat.primitives import hashes
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    from cryptography.hazmat.primitives.kdf.hkdf import HKDF
    # Exact Core identity/compat.rs recovery format.
    return AESGCM(HKDF(algorithm=hashes.SHA256(), length=32, salt=b"lmm-api-key-secret-v1",
                       info=b"aes-256-gcm").derive(key))


def encrypt(cipher, credential_id: int, user_id: int, secret: str) -> str:
    aad = struct.pack(">Qqq", 1, credential_id, user_id)
    nonce = os.urandom(12)
    out = nonce + cipher.encrypt(nonce, secret.encode(), aad)
    if cipher.decrypt(out[:12], out[12:], aad) != secret.encode() or not 44 <= len(out) <= 300:
        fail("api_key_ciphertext_invalid")
    return out.hex()


def ip_limits(raw) -> list[str] | None:
    values = [p.strip().replace(",", "") for p in (raw or "").replace(" ", "").split("\n")]
    try:
        result = [str(ipaddress.ip_interface(v)) for v in values if v]
    except ValueError:
        return None
    return result if len(result) <= 64 and len(set(result)) == len(result) else None


def target_info(target) -> dict:
    row = json.loads(target.run(
        "SELECT json_build_object('database',current_database(),'database_oid',"
        "(SELECT oid::bigint FROM pg_database WHERE datname=current_database()),'nonce',"
        "(SELECT substring(c FROM length('lmm.migration_instance_nonce=')+1) FROM pg_db_role_setting s,"
        " unnest(s.setconfig) c WHERE s.setdatabase=(SELECT oid FROM pg_database WHERE datname=current_database())"
        " AND s.setrole=0 AND c LIKE 'lmm.migration_instance_nonce=%'),'versions',"
        "(SELECT json_object_agg(id,auth_version) FROM core_identity.users))"))
    return row


def plan_keys(data: dict, a, target) -> tuple[list[str], dict]:
    users = {u["id"]: u for u in data["users"]}
    info = target_info(target)
    versions = {int(k): v for k, v in (info["versions"] or {}).items()}
    nonce = info["nonce"]
    if nonce is not None and (not HEX64.fullmatch(nonce) or nonce == "0" * 64):
        fail("target_instance_nonce_invalid")
    sql, stats, unrepresentable = [], Counter(), Counter()
    if nonce is None:
        # Core's legacy alias verification binds the manifest to this
        # database-level setting. Only a superuser may set the placeholder, so
        # emit that one statement and stop; rerun the phase after applying it.
        path = a.output / "instance-nonce.superuser.sql"
        if not path.exists():
            write_private(path, f"ALTER DATABASE \"{info['database']}\" SET lmm.migration_instance_nonce="
                                f"{lit(secrets.token_hex(32))};\n")
        fail(f"instance_nonce_missing: apply {path} as the cluster superuser, then rerun")
    # Fail the transaction if the bound instance setting changed after planning.
    sql.append("DO $$BEGIN IF NOT EXISTS(SELECT 1 FROM pg_db_role_setting s, unnest(s.setconfig) c WHERE "
               "s.setdatabase=(SELECT oid FROM pg_database WHERE datname=current_database()) AND s.setrole=0 "
               f"AND c={lit('lmm.migration_instance_nonce=' + nonce)}) THEN "
               "RAISE EXCEPTION 'instance nonce changed'; END IF; END$$;")
    key = None
    if a.api_key_encryption_key_file:
        key = Path(a.api_key_encryption_key_file).read_bytes()
        if len(key) != 32:
            fail("api_key_encryption_key_must_be_32_bytes")
    cipher = _cipher(key) if key else None
    records, digests = [], set()
    for t in data["tokens"]:
        tid, uid, raw = t["id"], t["user_id"], t["key"] or ""
        user = users.get(uid)
        if user is None or uid not in versions:
            fail("token_owner_unmapped")
        account = personal_account(data, uid)
        digest = sha(raw.encode())
        if digest in digests or not 32 <= len(raw) <= 256 or re.fullmatch(r"[A-Za-z0-9_-]+", raw) is None:
            fail("token_secret_unsupported")
        digests.add(digest)
        deleted = t["deleted_at"] is not None
        managed = bool(t["oauth_managed"]) or t["creation_source"] == "assistant_runtime" or not ordinary(raw)
        # Managed credentials have no Core API-key equivalent; keep them visible
        # to the alias layer as revoked so clients receive a rekey error.
        revoked_at = (lit(t["deleted_at"]) + "::timestamptz" if deleted
                      else f"to_timestamp({a.observed_at})" if managed else "NULL")
        if managed:
            stats["managed_revoked_rekey_required"] += 1
            if not deleted:
                stats["managed_revoked_rekey_required_live_at_snapshot"] += 1
        stats["deleted_revoked"] += deleted
        stats["status_" + str(t["status"])] += 1
        if t["status"] not in (1, 2):
            fail("token_status_unknown")
        expiry = t["expired_time"]
        if expiry is None or expiry == -1:
            expires = NEVER
        else:
            expires = expiry + 1  # legacy expiry second is inclusive; Core expires_at is exclusive
            stats["expired_at_snapshot"] += expiry < a.observed_at
        name = (t["name"] or "").strip()[:128] or f"legacy-key-{tid}"
        stats["name_trimmed_or_defaulted"] += name != (t["name"] or "")
        group = t["group"] or ""
        if group == "":
            group = user.get("group") or "default"
            stats["group_inherited_from_user_snapshot"] += 1
        if len(group) > 64:
            unrepresentable["group_name_too_long"] += 1
            fail("token_group_unrepresentable")
        auto, retry = [], bool(t["cross_group_retry"])
        try:
            parsed = json.loads(t["auto_groups"]) if t["auto_groups"] else []
        except ValueError:
            parsed = None
        if group == "auto":
            if not isinstance(parsed, list) or len(parsed) > 16 or not all(isinstance(g, str) and 0 < len(g) <= 64
                                                                          and g != "auto" for g in parsed):
                unrepresentable["auto_groups_invalid_kept_global"] += 1
                parsed = []
            auto = list(dict.fromkeys(parsed))
            stats["auto_group_" + ("custom" if auto else "global")] += 1
        else:
            if parsed:
                unrepresentable["inactive_auto_groups_dropped"] += 1
            if retry:
                unrepresentable["inactive_cross_group_retry_dropped"] += 1
            retry = False
        models = list(dict.fromkeys(m for m in (t["model_limits"] or "").split(",") if m))
        if len(models) > 200:
            fail("token_model_limits_unrepresentable")
        ips = ip_limits(t["allow_ips"])
        if ips is None:
            fail("token_allow_ips_unrepresentable")
        setting = {}
        try:
            setting = json.loads(user.get("setting") or "{}")
        except ValueError:
            pass
        preference = setting.get("billing_preference") if isinstance(setting, dict) else None
        preference = preference if preference in PREFERENCES else "subscription_first"
        recoverable = not t["one_time_reveal"] and revoked_at == "NULL"
        if recoverable and cipher is None:
            fail("api_key_encryption_key_required_for_recoverable_keys")
        encrypted = f"decode({lit(encrypt(cipher, tid, uid, raw))},'hex')" if recoverable else "NULL"
        stats["recoverable_encrypted" if recoverable else "one_time_reveal"] += 1
        used, remain, unlimited = t["used_quota"] or 0, t["remain_quota"] or 0, bool(t["unlimited_quota"])
        basis = used + remain
        if used < 0:
            fail("token_used_quota_negative")
        stats["unlimited" if unlimited else "limited"] += 1
        stats["negative_remaining"] += remain < 0
        created = t["created_time"] or 0
        accessed = f"to_timestamp({t['accessed_time']})" if (t["accessed_time"] or 0) > 0 else "NULL"
        version = versions[uid]
        sql.append(
            "INSERT INTO core_identity.credentials(id,digest,kind,user_id,user_version,owner_account_id,"
            "personal_billing_preference,expires_at,revoked_at,created_at,session_started_at) OVERRIDING SYSTEM VALUE "
            f"VALUES({tid},decode({lit(digest)},'hex'),'api_key',{uid},{version},{account},{lit(preference)},"
            f"to_timestamp({expires}),{revoked_at},to_timestamp({created}),to_timestamp({created}));")
        sql.append(
            "INSERT INTO core_identity.api_key_settings(credential_id,name,enabled,one_time_reveal,encrypted_secret,"
            "model_limits_enabled,model_limits,allow_ips,group_name,auto_groups,cross_group_retry,account_balance_read,"
            f"accessed_at) VALUES({tid},{lit(name)},{lit(t['status'] == 1)},{lit(not recoverable)},{encrypted},"
            f"{lit(bool(t['model_limits_enabled']))},{arr(models)},{arr(ips)}::inet[],{lit(group)},{arr(auto)},"
            f"{lit(retry)},{lit(bool(t['account_balance_read']))},{accessed});")
        sql.append(f"INSERT INTO core_identity.key_funding_rules(credential_id,position,payer_account_id) VALUES({tid},0,{account});")
        sql.append(
            "INSERT INTO core_charging.budgets(scope,key_id,issuer_user_id,period,limit_credits,unlimited,"
            f"display_limit_credits) VALUES('key',{tid},{uid},'lifetime',{0 if unlimited else basis},{lit(unlimited)},"
            f"{basis if unlimited else 'NULL'});")
        sql.append(
            "INSERT INTO core_charging.budget_opening_consumption(scope,key_id,period,start_at,end_at,"
            f"consumed_credits,import_reference) VALUES('key',{tid},'lifetime',{LIFETIME_START},{LIFETIME_END},{used},"
            f"{lit(f'key-quota:{a.migration_id}:{tid}')});")
        records.append({"source_key_id": tid, "target_credential_id": tid, "target_user_id": uid,
                        "target_account_id": account, "credential_user_version": version,
                        "source_secret_sha256": digest, "sk_alias_sha256": sha(("sk-" + raw).encode()),
                        "revoked_rekey_required": managed})
    if digests & {r["sk_alias_sha256"] for r in records}:
        fail("legacy_alias_digest_collision")
    sql.append("DO $$BEGIN IF EXISTS(SELECT 1 FROM core_identity.credentials c "
               "JOIN core_identity.users u ON u.id=c.user_id WHERE c.kind='api_key' AND "
               "(c.owner_account_id IS DISTINCT FROM u.personal_account_id OR NOT EXISTS("
               "SELECT 1 FROM core_identity.key_funding_rules f WHERE f.credential_id=c.id "
               "AND f.position=0 AND f.payer_account_id=u.personal_account_id))) THEN "
               "RAISE EXCEPTION 'credential_account_readback_mismatch'; END IF; END$$;")
    if records:
        sql.append("SELECT setval(pg_get_serial_sequence('core_identity.credentials','id'),"
                   f"{max(r['source_key_id'] for r in records)});")
    receipt = {"scope": "a02-credential-mapping-receipt", "source_archive_sha256": a.archive_sha256,
               "records": [{k: r[k] for k in ("source_key_id", "target_credential_id", "target_user_id",
                                               "target_account_id", "source_secret_sha256")} for r in records]}
    receipt_text = canonical(receipt) + "\n"
    keys = []
    for r in records:
        binding = {k: v for k, v in r.items() if k != "revoked_rekey_required"}
        if r["revoked_rekey_required"]:
            binding["revoked_rekey_required"] = True
        keys.append(binding)
    manifest = {"version": 1, "scope": "legacy-api-key-wire-aliases-v1", "source_archive_sha256": a.archive_sha256,
                "credential_receipt_sha256": sha(receipt_text.encode()),
                "target": {"database": info["database"], "database_oid": info["database_oid"],
                           "instance_nonce": nonce}, "keys": keys}
    raw_values = [t["key"] for t in data["tokens"]]
    manifest_text = canonical(manifest) + "\n"
    if any(v in manifest_text or v in receipt_text for v in raw_values):
        fail("secret_echo_in_artifact")
    _pending["keys"] = {RECEIPT_FILE: receipt_text, ALIAS_FILE: manifest_text}
    stats.update(tokens=len(records), alias_keys=len(keys), credential_id_equals_token_id=1)
    granted = set(json.loads(data["options"].get("UserUsableGroups") or "{}"))
    ungranted = Counter(g for g in (t["group"] or (users[t["user_id"]].get("group") or "default")
                                    for t in data["tokens"]) if g != "auto" and g not in granted)
    stats["groups_not_in_UserUsableGroups"] = dict(sorted(ungranted.items()))
    stats["unrepresentable"] = dict(unrepresentable)
    return sql, dict(stats)


def plan_console(data: dict, a) -> tuple[list[str], dict]:
    options = data["options"]
    sql, stats = [], Counter()
    for key in CONSOLE_KEYS:
        if key not in options:
            continue
        value = options[key] or ""
        if key in ("console_setting.announcements", "Announcements") and value.strip():
            parsed = json.loads(value)
            if not isinstance(parsed, list):
                fail("announcements_array_required")
            stats["announcements"] = len(parsed)
        sql.append(f"INSERT INTO core_console.options(key,value) VALUES({lit(key)},{lit(value)});")
        stats["options_copied"] += 1
    stats["legacy_options_not_consumed_by_core"] = len(set(options) - set(CONSOLE_KEYS))
    users = {u["id"] for u in data["users"]}
    seen = set()
    for r in data["announcement_reads"]:
        k = (r["user_id"], r["announcement_id"], r["revision"])
        if r["user_id"] not in users or not HEX64.fullmatch(r["revision"] or "") or r["announcement_id"] <= 0 \
                or (r["read_at"] or 0) <= 0:
            fail("announcement_read_unrepresentable")
        if k in seen:
            stats["duplicate_reads_deduplicated"] += 1
            continue
        seen.add(k)
        sql.append("INSERT INTO core_console.announcement_reads(user_id,announcement_id,revision,read_at) "
                   f"VALUES({r['user_id']},{r['announcement_id']},{lit(r['revision'])},{r['read_at']});")
    stats["announcement_reads"] = len(seen)
    if len(data["setups"]) > 1:
        fail("multiple_setup_records")
    for s in data["setups"]:
        version = (s["version"] or "")[:128]
        if not version or not 0 < (s["initialized_at"] or 0) <= NEVER:
            fail("setup_record_invalid")
        sql.append("INSERT INTO core_console.initialization(singleton,version,initialized_at) "
                   f"VALUES(TRUE,{lit(version)},to_timestamp({s['initialized_at']}));")
        stats["initialization"] = 1
    return sql, dict(stats)


def plan(phase: str, data: dict, a, target) -> tuple[list[str], dict]:
    return plan_keys(data, a, target) if phase == "keys" else plan_console(data, a)


def write_private(path: Path, text: str) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as stream:
        stream.write(text)


def reconcile_keys(data: dict, target) -> dict:
    rows = json.loads(target.run(
        "SELECT COALESCE(json_agg(json_build_object('id',c.id,'remaining',"
        "(SELECT min(coalesce(display_limit_credits,limit_credits)) FROM core_charging.budgets b"
        " WHERE b.scope='key' AND b.key_id=c.id AND b.period='lifetime')"
        "-(SELECT coalesce(sum(consumed_credits),0) FROM core_charging.budget_opening_consumption o"
        " WHERE o.scope='key' AND o.key_id=c.id AND o.period='lifetime'),"
        "'used',(SELECT coalesce(sum(consumed_credits),0) FROM core_charging.budget_opening_consumption o"
        " WHERE o.scope='key' AND o.key_id=c.id),'unlimited',(SELECT bool_and(unlimited) FROM core_charging.budgets b"
        " WHERE b.scope='key' AND b.key_id=c.id),'digest',encode(c.digest,'hex'),'user_id',c.user_id,"
        "'owner_account_id',c.owner_account_id,'personal_account_id',u.personal_account_id,"
        "'payer_account_id',(SELECT payer_account_id FROM core_identity.key_funding_rules f "
        "WHERE f.credential_id=c.id AND position=0),"
        "'enabled',s.enabled,'revoked',c.revoked_at IS NOT NULL)),'[]') FROM core_identity.credentials c"
        " JOIN core_identity.users u ON u.id=c.user_id"
        " JOIN core_identity.api_key_settings s ON s.credential_id=c.id WHERE c.kind='api_key'"))
    core = {r["id"]: r for r in rows}
    mismatches = Counter()
    for t in data["tokens"]:
        r = core.get(t["id"])
        if r is None:
            mismatches["missing"] += 1
            continue
        mismatches["remaining"] += r["remaining"] != (t["remain_quota"] or 0)
        mismatches["used"] += r["used"] != (t["used_quota"] or 0)
        mismatches["unlimited"] += r["unlimited"] != bool(t["unlimited_quota"])
        mismatches["digest"] += r["digest"] != sha((t["key"] or "").encode())
        mismatches["user"] += r["user_id"] != t["user_id"]
        account = personal_account(data, t["user_id"])
        for field in ("owner_account_id", "personal_account_id", "payer_account_id"):
            mismatches[field] += r[field] != account
        mismatches["enabled"] += r["enabled"] != (t["status"] == 1)
        mismatches["revoked_deleted"] += (t["deleted_at"] is not None) and not r["revoked"]
    return {"legacy_tokens": len(data["tokens"]), "core_api_keys": len(core),
            "extra_core_keys": len(set(core) - {t["id"] for t in data["tokens"]}),
            "mismatches": {k: v for k, v in mismatches.items() if v},
            "legacy_remaining_sum": sum(t["remain_quota"] or 0 for t in data["tokens"]),
            "core_remaining_sum": sum(r["remaining"] for r in core.values()),
            "legacy_used_sum": sum(t["used_quota"] or 0 for t in data["tokens"]),
            "core_used_sum": sum(r["used"] for r in core.values())}


def reconcile_console(data: dict, target) -> dict:
    got = json.loads(target.run(
        "SELECT json_build_object('options',(SELECT COALESCE(json_object_agg(key,value),'{}') FROM core_console.options),"
        "'reads',(SELECT count(*) FROM core_console.announcement_reads),"
        "'init',(SELECT json_build_object('version',version,'at',extract(epoch FROM initialized_at)::bigint)"
        " FROM core_console.initialization))"))
    expected = {k: data["options"][k] or "" for k in CONSOLE_KEYS if k in data["options"]}
    setup = data["setups"][0] if data["setups"] else None
    return {"options_match": got["options"] == expected, "options": sorted(got["options"]),
            "reads_core": got["reads"], "reads_legacy": len(data["announcement_reads"]),
            "initialization_match": got["init"] == (setup and {"version": setup["version"],
                                                                "at": setup["initialized_at"]})}


def after_apply(phase: str, data: dict, output: Path, target) -> dict:
    """Publish Core inputs only after the transaction committed; then reconcile."""
    result = {}
    if phase == "keys":
        files = _pending.pop("keys")
        for name, text in files.items():
            write_private(output / name, text)
        result["artifacts"] = {name: {"path": str(output / name), "sha256": sha(text.encode())}
                               for name, text in files.items()}
        result["reconciliation"] = reconcile_keys(data, target)
    else:
        result["reconciliation"] = reconcile_console(data, target)
    return result

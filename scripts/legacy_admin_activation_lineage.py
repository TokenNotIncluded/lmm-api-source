"""Offline proof of the one approved imported Root activation, never authority to activate.

Wire hashes mirror identity/admin_approval.rs and identity/permissions.rs. All
native evidence must additionally be re-read by the importing SQL transaction.
No synthetic approval, caller boolean, or current-version assertion is a receipt.
"""
from __future__ import annotations

from copy import deepcopy
import hashlib
import json
import re
import struct

KEY = "admin_activation_bindings"
ACTIONS = ("ChannelRead", "ChannelOperate", "ChannelWrite", "ChannelSensitiveWrite",
           "ChannelSecretView", "AcquisitionRead", "AcquisitionWrite",
           "AcquisitionDetails", "AcquisitionExport")
CONTRACT = "core.permissions.v1"
HEX = re.compile(r"[0-9a-f]{64}\Z")
FACT_HASHES = ("archive_sha256", "a2_semantic_sha256", "a2_file_sha256", "a2_sql_sha256",
               "system_id", "instance_nonce", "import_run", "password_hash_sha256", "login_name_sha256")
FACT_TO_APPROVAL = {"source_enabled": "source_enabled", "source_deleted": "source_deleted",
    "password_verifier_recognized": "password_verifier_recognized", "password_login_enabled": "password_login_enabled",
    "source_role": "role", "source_second_factor_required": "second_factor_required", "source_auth_version": "auth_version"}
APPROVAL_FIELDS = set(FACT_HASHES) | {"approval_id", "approver", "expiry_unix", "migration_id", "user_id",
    "personal_account_id", "database_oid", "source_enabled", "source_deleted", "password_verifier_recognized",
    "password_login_enabled", "second_factor_required", "role", "auth_version", "policy_revision", "policy_sha256", "policy"}


class LineageError(ValueError):
    pass


def require(ok, code="admin_activation_lineage_invalid"):
    if not ok:
        raise LineageError(code)


def fields(value, names):
    require(type(value) is dict and set(value) == set(names))


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"), allow_nan=False)


def same(left, right):
    return canonical(left) == canonical(right)


def digest(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def integer(value):
    require(type(value) is int and 0 < value <= 9223372036854775807)
    return struct.pack(">q", value)


def hash_bytes(value):
    require(type(value) is str and HEX.fullmatch(value) is not None and value != "0" * 64)
    return bytes.fromhex(value)


def policy_bytes(a):
    fields(a["policy"], ACTIONS)
    out = CONTRACT.encode() + b"\0" + integer(a["policy_revision"]) + b"\0" + integer(a["user_id"]) + b"\0"
    for action in ACTIONS:
        effect = a["policy"][action]
        require(effect in ("allow", "deny"))
        out += action.encode() + b"\0" + effect.encode() + b"\0"
    return out


def approval_hash(a):
    """Exact native payload_sha256; deliberately not a JSON digest."""
    fields(a, APPROVAL_FIELDS)
    for name in (*FACT_HASHES, "approval_id", "approver", "policy_sha256"):
        hash_bytes(a[name])
    for name in ("expiry_unix", "user_id", "personal_account_id", "database_oid", "auth_version", "policy_revision"):
        integer(a[name])
    for name in ("source_enabled", "source_deleted", "password_verifier_recognized", "password_login_enabled", "second_factor_required"):
        require(type(a[name]) is bool)
    require(a["role"] == "superadmin", "admin_activation_root_only")
    require(type(a["migration_id"]) is str and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", a["migration_id"]) is not None)
    require(hashlib.sha256(policy_bytes(a)).hexdigest() == a["policy_sha256"], "admin_activation_policy_hash_mismatch")
    out = b"core.admin-approval.v1\0" + CONTRACT.encode()
    out += hash_bytes(a["approval_id"]) + hash_bytes(a["approver"]) + integer(a["expiry_unix"])
    for name in ("archive_sha256", "a2_semantic_sha256", "a2_file_sha256", "a2_sql_sha256"):
        out += hash_bytes(a[name])
    migration = a["migration_id"].encode()
    out += integer(len(migration)) + migration + integer(a["user_id"]) + integer(a["personal_account_id"])
    out += bytes(int(a[name]) for name in ("source_enabled", "source_deleted", "password_verifier_recognized", "password_login_enabled"))
    out += hash_bytes(a["password_hash_sha256"]) + hash_bytes(a["login_name_sha256"])
    out += bytes((int(a["second_factor_required"]), 100)) + integer(a["auth_version"])
    out += hash_bytes(a["system_id"]) + integer(a["database_oid"]) + hash_bytes(a["instance_nonce"]) + hash_bytes(a["import_run"])
    out += integer(a["policy_revision"]) + hash_bytes(a["policy_sha256"]) + b"\x01"
    return hashlib.sha256(out).hexdigest()


def validate_bindings(bindings, archive, mappings, map_hash):
    require(type(bindings) is list and len(bindings) <= 1, "admin_activation_unique_root_required")
    result = {}
    for b in bindings:
        fields(b, ("source_user_id", "target_user_id", "target_personal_account_id", "source_target_id_map_sha256",
                   "approved_apply_request", "native_evidence"))
        m = {k: b[k] for k in ("source_user_id", "target_user_id", "target_personal_account_id")}
        require(m in mappings and all(type(v) is int for v in m.values()), "admin_activation_mapping_mismatch")
        require(b["source_target_id_map_sha256"] == map_hash, "admin_activation_mapping_mismatch")
        req = b["approved_apply_request"]
        fields(req, ("confirmation_sha256", "approval"))
        a = req["approval"]
        payload = approval_hash(a)
        require(payload == req["confirmation_sha256"], "admin_activation_confirmation_mismatch")
        require(a["auth_version"] == 6 and a["archive_sha256"] == archive and a["user_id"] == m["target_user_id"]
                and a["personal_account_id"] == m["target_personal_account_id"], "admin_activation_source_mismatch")
        require(a["source_enabled"] and not a["source_deleted"] and a["password_verifier_recognized"] and a["password_login_enabled"],
                "admin_activation_source_disabled")
        e = b["native_evidence"]
        fields(e, ("facts", "receipts", "audits", "policy", "policy_rows", "current", "instance", "identity_import"))
        f = e["facts"]
        fields(f, set(FACT_HASHES) | set(FACT_TO_APPROVAL) | {"user_id", "migration_id", "database_oid"})
        for name in (*FACT_HASHES, "user_id", "migration_id", "database_oid"):
            require(type(f[name]) is type(a[name]) and f[name] == a[name], "admin_activation_frozen_facts_mismatch")
        for name, approved_name in FACT_TO_APPROVAL.items():
            require(type(f[name]) is type(a[approved_name]) and f[name] == a[approved_name], "admin_activation_frozen_facts_mismatch")
        require(same(e["receipts"], [{"approval_id": a["approval_id"], "payload_sha256": payload,
            "user_id": a["user_id"], "expected_auth_version": 6}]), "admin_activation_consumed_receipt_required")
        require(type(e["audits"]) is list and len(e["audits"]) == 1, "admin_activation_unique_audit_required")
        audit = e["audits"][0]
        fields(audit, ("id", "actor_user_id", "action", "target"))
        integer(audit["id"])
        require(audit["actor_user_id"] == a["user_id"] and audit["action"] == "privileged_activation"
                and same(audit["target"], {"approval_id": a["approval_id"], "auth_version": 7}), "admin_activation_audit_mismatch")
        require(same(e["policy"], {"user_id": a["user_id"], "revision": a["policy_revision"], "contract": CONTRACT,
            "sha256": a["policy_sha256"], "canonical_payload": policy_bytes(a).hex(), "verified_complete": True}), "admin_activation_policy_mismatch")
        require(same(e["policy_rows"], [{"user_id": a["user_id"], "action": k, "effect": a["policy"][k]}
                                     for k in sorted(ACTIONS)]), "admin_activation_policy_mismatch")
        require(same(e["current"], {"user_id": a["user_id"], "personal_account_id": a["personal_account_id"],
            "platform_role": "superadmin", "active": True, "auth_version": 7, "account_kind": "personal", "account_active": True,
            "password_login_allowed": True, "second_factor_required": a["second_factor_required"],
            "password_hash_sha256": a["password_hash_sha256"], "login_name_sha256": a["login_name_sha256"]}), "admin_activation_current_identity_mismatch")
        require(same(e["instance"], {k: a[k] for k in ("system_id", "database_oid", "instance_nonce")}), "admin_activation_instance_mismatch")
        r = e["identity_import"]
        require(type(r) is dict and r.get("migration_id") == a["migration_id"] and r.get("source_archive_sha256") == archive
                and r.get("mapping_sha256") == a["a2_semantic_sha256"], "admin_activation_import_mismatch")
        manifest = r.get("manifest", {})
        require(type(manifest) is dict, "admin_activation_import_mismatch")
        require(manifest.get("scope") == "identity-only" and manifest.get("mapping_file_sha256") == a["a2_file_sha256"]
                and manifest.get("source_target_id_map_sha256") == map_hash, "admin_activation_import_mismatch")
        require(hashlib.sha256(a["migration_id"].encode()).hexdigest() == a["import_run"], "admin_activation_import_mismatch")
        result[m["target_user_id"]] = b
    return result


def expected_version(source_user, observed, binding):
    if binding is None:
        return source_user["auth_version"]
    require(type(source_user["auth_version"]) is int and source_user["auth_version"] == 6
            and source_user["status"] == 1 and source_user["deleted_at"] in (None, ""), "admin_activation_source_disabled")
    c = binding["native_evidence"]["current"]
    require(all(observed[k] == c[k] for k in ("user_id", "personal_account_id", "active", "auth_version")),
            "admin_activation_current_identity_mismatch")
    return 7


def bind_instance(bindings, instance, identity_import):
    for b in bindings:
        e = b["native_evidence"]
        sid = struct.pack(">q", int(instance["system_identifier"])) + bytes(24)
        require(e["instance"] == {"system_id": sid.hex(), "database_oid": int(instance["database_oid"]),
            "instance_nonce": instance["server_instance_nonce"]} and e["identity_import"] == identity_import,
                "admin_activation_instance_mismatch")


def effective_console_facts(proof, bindings, instance):
    """Derive only native activation deltas, retaining the original A2 facts."""
    rows = deepcopy(proof["facts"])
    mappings = [{"source_user_id": r["user_id"], "target_user_id": r["user_id"], "target_personal_account_id": r["account_id"]} for r in rows]
    checked = validate_bindings(bindings, proof["source_archive_sha256"], mappings, proof["source_target_id_map_sha256"])
    for row in rows:
        b = checked.get(row["user_id"])
        if b is None:
            continue
        a, e = b["approved_apply_request"]["approval"], b["native_evidence"]
        # Console's existing identity query exposes OID/nonce; the SQL evidence
        # guard below additionally checks pg_control_system before any write.
        require(e["instance"]["database_oid"] == int(instance["database_oid"])
                and e["instance"]["instance_nonce"] == instance["server_instance_nonce"], "admin_activation_instance_mismatch")
        if "system_identifier" in instance:
            bind_instance([b], instance, e["identity_import"])
        require(e["identity_import"]["manifest"] == proof["manifest"]
                and a["a2_file_sha256"] == proof["mapping_file_sha256"]
                and a["a2_semantic_sha256"] == proof["mapping_sha256"], "admin_activation_import_mismatch")
        require(row["platform_role"] == "superadmin" and row["active"] is False and row["auth_version"] == 6
                and row["password_login_allowed"] is False and row["password_hash_sha256"] == a["password_hash_sha256"]
                and row["second_factor_required"] == a["second_factor_required"], "admin_activation_original_identity_mismatch")
        row.update(active=True, auth_version=7, password_login_allowed=True)
    return rows


def _row_sql(alias, hashes):
    return "(to_jsonb(" + alias + ")" + "".join("-'" + k + "'" for k in hashes) + ")||jsonb_build_object(" + ",".join(
        "'" + k + "',encode(" + alias + "." + k + ",'hex')" for k in hashes) + ")"


def evidence_sql(user_id):
    integer(user_id)
    uid = str(user_id)
    return """SELECT jsonb_build_object(
 'facts',(SELECT """ + _row_sql("f", FACT_HASHES) + """ FROM core_identity.privileged_import_facts f WHERE f.user_id=""" + uid + """),
 'receipts',(SELECT coalesce(jsonb_agg(""" + _row_sql("r", ("approval_id", "payload_sha256")) + """ ORDER BY r.approval_id),'[]') FROM core_identity.admin_activation_receipts r WHERE r.user_id=""" + uid + """),
 'audits',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',a.id,'actor_user_id',a.actor_user_id,'action',a.action,'target',a.target) ORDER BY a.id),'[]') FROM core_identity.audit a WHERE a.actor_user_id=""" + uid + """ AND a.action='privileged_activation'),
 'policy',(SELECT """ + _row_sql("p", ("sha256", "canonical_payload")) + """ FROM core_identity.permission_policies p WHERE p.user_id=""" + uid + """),
 'policy_rows',(SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY p.action COLLATE "C"),'[]') FROM core_identity.permission_policy_rows p WHERE p.user_id=""" + uid + """),
 'current',(SELECT jsonb_build_object('user_id',u.id,'personal_account_id',u.personal_account_id,'platform_role',u.platform_role,'active',u.active,'auth_version',u.auth_version,
 'account_kind',a.kind,'account_active',a.active,'password_login_allowed',p.password_login_allowed,'second_factor_required',p.second_factor_required,
 'password_hash_sha256',encode(sha256(convert_to(l.password_hash,'UTF8')),'hex'),'login_name_sha256',encode(sha256(convert_to(l.login_name,'UTF8')),'hex'))
 FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_identity.user_profiles p ON p.user_id=u.id LEFT JOIN core_identity.password_logins l ON l.user_id=u.id WHERE u.id=""" + uid + """),
 'instance',jsonb_build_object('system_id',encode(int8send((SELECT system_identifier::bigint FROM pg_control_system()))||decode(repeat('00',24),'hex'),'hex'),
 'database_oid',(SELECT oid::bigint FROM pg_database WHERE datname=current_database()),'instance_nonce',current_setting('lmm.migration_instance_nonce',true)),
 'identity_import',(SELECT to_jsonb(r) FROM core_migration.import_runs r JOIN core_identity.privileged_import_facts f ON f.migration_id=r.migration_id WHERE f.user_id=""" + uid + "))"


def guard_sql(bindings):
    """Existing owner transaction calls this before writes; no authority writes."""
    sql = ""
    for b in bindings:
        uid = b["target_user_id"]
        integer(uid)
        value = canonical(b["native_evidence"]).replace("'", "''")
        # Block concurrent user/account/profile/verifier changes through commit.
        sql += f"""PERFORM 1 FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
 JOIN core_identity.user_profiles p ON p.user_id=u.id JOIN core_identity.password_logins l ON l.user_id=u.id
 WHERE u.id={uid} FOR SHARE OF u,a,p,l;
 IF ({evidence_sql(uid)}) IS DISTINCT FROM '{value}'::jsonb THEN
 RAISE EXCEPTION 'admin_activation_durable_evidence_changed'; END IF;
"""
    return sql


def evidence_array_sql(bindings):
    if not bindings:
        return "SELECT '[]'::jsonb"
    return "SELECT jsonb_build_array(" + ",".join("(" + evidence_sql(b["target_user_id"]) + ")" for b in bindings) + ")"

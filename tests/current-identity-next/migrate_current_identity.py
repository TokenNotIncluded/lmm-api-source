#!/usr/bin/env python3
"""Append or complete legacy identities in a used, exactly Core78 database.

Private source facts and an original committed mapping are required. This is
not the fresh importer: existing users/accounts, privileged authority, login
bindings, credentials, billing state and history are retained. Only explicit
ordinary-user status/auth-version and developer-policy drift is reconciled.
No email verification, privileged activation, password reset, money opening,
provider credential provisioning or real OAuth login is inferred here.

Production SQL needs an independently reviewed source fence and authorization
bound to the exact material/SQL. The test entry point is database+nonce bound.
Sequence allocations use nextval; rollbacks may leave gaps. An unknown commit
is resolved read-only and never automatically retried.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys

import migrate_current_runtime_keys as U
import legacy_cutover_import as O

F, I, C = U.F, U.I, U.C
SCOPE = "current-identity-append-completion-v1"
DECISION = "execute-current-identity-append-completion"
POLICY = "preserve-native-privileged-state-explicit-ordinary-generation-stages-v2"
SOURCE_FORMAT = "lmm.current-identity-selected-source.v1"
SOURCE_COLUMNS = ("id", "username", "display_name", "email", "setting", "created_at", "auth_version",
                  "role", "status", "deleted_at", "password", "trust_level_override", "console_activated_at",
                  "github_id", "linux_do_id", "discord_id", "telegram_id", "oidc_id", "wechat_id")
PROFILE_COLUMNS = ("user_id", "username", "display_name", "email", "email_login_allowed", "setting",
                   "password_login_allowed", "second_factor_required", "legacy_bcrypt_allowed")
USER_COLUMNS = ("id", "personal_account_id", "platform_role", "active", "auth_version")
ACCOUNT_COLUMNS = ("id", "kind", "service_level", "active", "created_at")
DEV_COLUMNS = ("user_id", "granted", "source_reference", "revision")
SEQUENCES = ("core_identity.accounts_id_seq", "core_identity.users_id_seq")
MUTABLE = {
    "core_identity.accounts": "id", "core_identity.users": "id",
    "core_identity.user_profiles": "user_id", "core_identity.password_logins": "user_id",
    "core_identity.developer_access": "user_id", "core_billing.account_policies": "account_id",
    "core_identity.oauth_identities": None,
    "core_identity.user_routing_groups": None,
}
ROUTING_OPTION_KEYS = ("AutoGroups", "GroupGroupRatio", "GroupRatio", "MaxTokenAutoGroups",
                       "ModelRequestRateLimitEnabled", "UserUsableGroups", "group_ratio_setting.group_ratio",
                       "group_ratio_setting.group_special_usable_group")
ROUTING_USER_COLUMNS = ("id", "group", "status", "deleted_at", "auth_version", "setting")
ABILITY_COLUMNS = ("group", "model", "channel_id", "enabled", "priority", "weight", "tag")
CHANNEL_CAPABILITY_COLUMNS = ("id", "type", "status", "group", "models", "priority", "weight", "model_mapping")
HEX = re.compile(r"[0-9a-f]{64}\Z")
MAX_ID = (1 << 63) - 1


class Error(ValueError):
    pass


def need(ok, code):
    if not ok:
        raise Error(code)


def canonical(x):
    return json.dumps(x, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def digest(x):
    return hashlib.sha256(canonical(x).encode()).hexdigest()


def literal(x):
    return F.literal(x)


def private_bytes(path):
    p = Path(path).absolute()
    need(not p.is_symlink(), "identity_input_symlink")
    fd = os.open(p, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        s = os.fstat(fd)
        need(stat.S_ISREG(s.st_mode) and s.st_uid == os.getuid() and not s.st_mode & 0o077,
             "identity_input_not_private_owned_file")
        with os.fdopen(fd, "rb", closefd=False) as f:
            return f.read()
    finally:
        os.close(fd)


def private_json(path):
    return json.loads(private_bytes(path), object_pairs_hook=U.P.unique_object)


def file_ref(path):
    return {"path": str(Path(path).absolute()), "sha256": hashlib.sha256(private_bytes(path)).hexdigest()}


def verify_files(refs):
    need(type(refs) is dict and refs, "identity_file_closure_missing")
    for ref in refs.values():
        need(type(ref) is dict and set(ref) == {"path", "sha256"} and file_ref(ref["path"]) == ref,
             "identity_input_file_changed")


def integer(x, code, minimum=1):
    need(type(x) is int and minimum <= x <= MAX_ID, code)
    return x


def identity_facts(data):
    return {k: data[k] for k in ("users", "two_fas", "options")}


def mapped_current(data, archive):
    need(data["options"] in ([{"key": "PasswordLoginEnabled", "value": "true"}],
                             [{"key": "PasswordLoginEnabled", "value": "false"}]),
         "identity_global_password_policy_unknown")
    factors = []
    for f in data["two_fas"]:
        need(set(f) == {"user_id", "is_enabled", "deleted_at"}, "identity_mfa_fact_shape")
        if f["deleted_at"] is None:
            factors.append({"user_id": f["user_id"], "enabled": f["is_enabled"] is not False,
                            "policy_unknown": type(f["is_enabled"]) is not bool})
    mapped = I.map_identity(data["users"], factors, source_sha256=archive,
                           password_login_enabled=data["options"][0]["value"] == "true")
    records = deepcopy(mapped["records"])
    for r in records:
        del r["account_id"]  # The old dense allocation is not used on a nonempty target.
    return records


def validate_source(source):
    need(source.get("format") == SOURCE_FORMAT and source.get("status") ==
         "SOURCE_IDENTITY_FACTS_NOT_REVIEW_NOT_APPROVAL", "identity_source_format_invalid")
    need(source.get("original_source_columns_complete") is False and source.get("financial_approval") is False
         and source.get("raw_api_keys_selected") is False and source.get("client_secret_selected") is False
         and source.get("production_writes") == 0 and type(source.get("query_exit_code")) is int
         and source["query_exit_code"] == 0, "identity_source_scope_invalid")
    need(source["selected_columns"] == list(SOURCE_COLUMNS), "identity_selected_columns_invalid")
    for row in source["data"]["users"]:
        need(set(row) == set(SOURCE_COLUMNS), "identity_source_row_shape")
    need(source["source_identity_facts_sha256"] == digest(identity_facts(source["data"])),
         "identity_source_facts_digest_changed")
    current = mapped_current(source["data"], source["source_archive_sha256"])
    need(current == source["current_policy_records"], "identity_policy_mapping_changed")
    return current


def load_source(source_path, oauth_path, restore_path, original_mapping_path):
    source, oauth, restore, original = map(private_json, (source_path, oauth_path, restore_path, original_mapping_path))
    records = validate_source(source)
    refs = {k: file_ref(p) for k, p in (("source", source_path), ("oauth", oauth_path),
            ("restore", restore_path), ("original_mapping", original_mapping_path))}
    need(refs["restore"]["sha256"] == source["restore_receipt_sha256"], "identity_restore_file_mismatch")
    need(restore.get("status") == "RESTORED_SOURCE_ONLY_READONLY_PREVIEW_NOT_FINANCIAL_APPROVAL"
         and restore.get("restore_actual_exit") == 0 and restore.get("original_archive_preserved") is True
         and restore.get("target_database_accessed_by_restore") is False and restore.get("production_writes") == 0
         and restore.get("financial_approval") is False, "identity_actual_restore_unproven")
    refs["archive"] = file_ref(restore["archive"]["path"])
    need(refs["archive"]["sha256"] == restore["archive"]["sha256"] == source["source_archive_sha256"]
         == oauth["source_archive_sha256"], "identity_archive_binding_mismatch")
    ri, si, oi = restore["restored_instance"], source["restored_identity"], oauth["restored_identity"]
    for key in ("system_identifier", "database", "nonce", "data_directory", "listen_addresses"):
        need(si[key] == oi[key] == ri[key], "identity_restore_instance_mismatch")
    need(si["listen_addresses"] == "" and si["read_only"] == oi["read_only"] == "on"
         and source["data"]["database"] == si["database"] and source["data"]["nonce"] == si["nonce"]
         and source["data"]["read_only"] == "on", "identity_source_not_isolated_readonly")
    rp = restore["readonly_reader_proof"]
    need(rp["read_only"] == "on" and rp["row_security"] == "off" and rp["role_memberships"] == 0
         and rp["database_create"] is False and rp["database_temp"] is False
         and all(v is False for v in rp["role_flags"].values())
         and all(rp[k] == 0 for k in ("application_table_write_privileges", "application_schema_create_privileges",
                                    "application_security_definer_execute", "rls_tables")),
         "identity_source_reader_not_readonly")
    for name, ref in (("baseline", source["baseline_ref"]), ("selected_financial", source["source_selected_financial_ref"])):
        need(file_ref(ref["path"]) == ref, "identity_source_link_changed")
        refs[name] = ref
    baseline = private_json(refs["baseline"]["path"])
    namespace = {k: restore["production_source_observation"][k] for k in ("system_identifier", "database", "schema")}
    U.namespace_key(namespace)
    b = [r for r in baseline["tables"] if r["schema"] == namespace["schema"] and r["table"] == "users"]
    need(len(b) == 1 and b[0]["rows"] == len(records) and len(baseline["tables"]) == restore["table_baseline_count"],
         "identity_actual_baseline_count_mismatch")
    old_selected = private_json(refs["selected_financial"]["path"])["source"]["users"]
    selected = {r["id"]: r for r in old_selected}
    need(len(selected) == len(records), "identity_selected_user_coverage_mismatch")
    for r in source["data"]["users"]:
        fact = {k: r[k] for k in ("id", "role", "status", "username", "created_at", "deleted_at", "display_name")}
        need(r["id"] in selected and digest(fact) == selected[r["id"]]["source_identity_facts_sha256"],
             "identity_source_selection_conflict")
    for obj, prefix in ((source, "source_query"), (oauth, "source_query")):
        ref = file_ref(obj[prefix + "_path"])
        need(ref["sha256"] == obj[prefix + "_sha256"], "identity_source_query_changed")
        refs["sql_" + ref["sha256"]] = ref
    need(source["mapper_sha256"] == I.sha256_file(Path(I.__file__).resolve()), "identity_mapper_bytes_changed")
    need(oauth.get("client_secret_selected") is False and oauth.get("raw_keys_selected") is False,
         "identity_oauth_projection_scope_invalid")
    oauth_rows = validate_oauth_projection(source, oauth)
    return {"kind": "actual-restored-selected-identity-v1", "source": source, "oauth": oauth,
            "original": original, "current": records, "oauth_rows": oauth_rows,
            "namespace": namespace, "files": refs}


def validate_oauth_projection(source, oauth):
    users = {r["id"]: r for r in source["data"]["users"]}
    p = oauth["projection"]
    need(len(p["users"]) == len(users), "identity_oauth_source_coverage_mismatch")
    for r in p["users"]:
        need(r["id"] in users and all(users[r["id"]][k] == v for k, v in r.items()),
             "identity_oauth_source_user_mismatch")
    try:
        mapped = O.identity_oauth_rows(p)
    except SystemExit as error:
        raise Error(str(error)) from None
    rows = [{"issuer": issuer, "subject": subject, "source_user_id": uid, "source_family": family}
            for issuer, subject, uid, family in mapped]
    need(rows == oauth["oauth_records"], "identity_oauth_canonical_mapping_changed")
    return rows


def routing_policy(facts):
    need(set(facts) == {"users", "options"}, "identity_routing_facts_shape")
    rows = facts["options"]
    need(type(rows) is list and all(set(r) == {"key", "value"} and r["key"] in ROUTING_OPTION_KEYS for r in rows),
         "identity_routing_option_shape")
    values = {r["key"]: r["value"] for r in rows}
    need(len(values) == len(rows), "identity_routing_option_duplicate")
    keys = {"user_usable_groups": "UserUsableGroups", "special_usable_groups": "group_ratio_setting.group_special_usable_group",
            "group_ratios": "GroupRatio", "group_group_ratios": "GroupGroupRatio"}
    need(all(k in values for k in keys.values()), "identity_source_routing_policy_incomplete")
    try:
        parsed = {name: json.loads(values[key], object_pairs_hook=U.P.unique_object) for name, key in keys.items()}
        if "group_ratio_setting.group_ratio" in values:
            need(json.loads(values["group_ratio_setting.group_ratio"], object_pairs_hook=U.P.unique_object)
                 == parsed["group_ratios"], "identity_source_group_ratio_alias_conflict")
        auto = json.loads(values.get("AutoGroups", '["default"]'), object_pairs_hook=U.P.unique_object)
        maximum = int(values.get("MaxTokenAutoGroups", "5"))
    except (ValueError, TypeError, RecursionError):
        raise Error("identity_source_routing_policy_invalid") from None
    policy = {"source_options_sha256": digest(rows), "group_authority": parsed,
              "auto_groups": [] if auto is None else auto, "max_token_auto_groups": maximum,
              "user_model_rate_limit_enabled": values.get("ModelRequestRateLimitEnabled", "false") == "true"}
    U.P.validate_group_policy(policy)
    return policy


def attach_routing(bundle, facts, reference=None):
    policy = routing_policy(facts)
    source = {u["id"]: u for u in bundle["source"]["data"]["users"]}
    seen = set()
    for r in facts["users"]:
        need(set(r) == set(ROUTING_USER_COLUMNS) and r["id"] in source and r["id"] not in seen,
             "identity_routing_user_coverage_invalid")
        need(all(source[r["id"]][k] == r[k] for k in ROUTING_USER_COLUMNS if k != "group"),
             "identity_routing_source_user_conflict")
        seen.add(r["id"])
    need(seen == set(source), "identity_routing_source_user_coverage_incomplete")
    bundle["routing"] = {"facts": facts, "policy": policy}
    if reference is not None:
        need(file_ref(reference["path"]) == reference, "identity_routing_source_file_changed")
        bundle["files"]["routing"] = reference
    return bundle


def load_routing(bundle, path):
    x = private_json(path)
    need(x.get("format") == "lmm.current-routing-selected-source.v1"
         and x.get("scope") == "ACTUAL_RESTORED_SOURCE_FACTS_NOT_APPROVAL"
         and x.get("selected_columns_only") is True and x.get("financial_approval") is False
         and x.get("passwords_selected") is False and x.get("raw_api_keys_selected") is False
         and type(x.get("query_exit_code")) is int and x["query_exit_code"] == 0
         and x.get("production_writes") == 0, "identity_routing_selected_source_contract_invalid")
    need(x["source_archive_sha256"] == bundle["source"]["source_archive_sha256"]
         and F.digest(x["data"]) == x["source_facts_sha256"], "identity_routing_typed_facts_digest_mismatch")
    data = x["data"]
    source_instance = bundle["source"]["restored_identity"]
    need(data["database"] == source_instance["database"] and data["nonce"] == source_instance["nonce"]
         and data["read_only"] == "on", "identity_routing_restored_instance_mismatch")
    need(set(x["requested_option_keys"]) == set(ROUTING_OPTION_KEYS)
         and sorted(x["absent_option_keys"]) == sorted(set(ROUTING_OPTION_KEYS) - {r["key"] for r in data["options"]}),
         "identity_routing_option_coverage_mismatch")
    restore_ref = x["restore_receipt_ref"]
    need(file_ref(restore_ref["path"]) == restore_ref
         and private_json(restore_ref["path"]) == private_json(bundle["files"]["restore"]["path"]),
         "identity_routing_original_restore_semantics_mismatch")
    sql_ref = file_ref(x["source_query_path"])
    need(sql_ref["sha256"] == x["source_query_sha256"], "identity_routing_query_bytes_changed")
    archive_ref = file_ref(x["source_archive_ref"]["path"])
    need(archive_ref["sha256"] == x["source_archive_ref"]["sha256"] == x["source_archive_sha256"],
         "identity_routing_archive_mismatch")
    bundle["files"].update(routing_restore=restore_ref, routing_sql=sql_ref, routing_archive=archive_ref)
    return attach_routing(bundle, {k: data[k] for k in ("users", "options")}, file_ref(path))


def capability_facts(facts):
    need(type(facts) is dict and set(facts) == {"abilities", "channels"}
         and type(facts["abilities"]) is list and type(facts["channels"]) is list,
         "identity_capability_facts_shape")
    channels, abilities = {}, {}
    for row in facts["channels"]:
        need(set(row) == set(CHANNEL_CAPABILITY_COLUMNS), "identity_capability_channel_columns")
        uid = integer(row["id"], "identity_capability_channel_id")
        need(uid not in channels and type(row["status"]) is int and type(row["group"]) is str
             and type(row["models"]) is str, "identity_capability_channel_invalid")
        channels[uid] = deepcopy(row)
    for row in facts["abilities"]:
        need(set(row) == set(ABILITY_COLUMNS) and type(row["group"]) is str and type(row["model"]) is str
             and type(row["enabled"]) is bool and row["channel_id"] in channels,
             "identity_capability_ability_invalid")
        key = row["group"], row["model"], row["channel_id"]
        need(key not in abilities, "identity_capability_duplicate_ability")
        abilities[key] = deepcopy(row)
    return {"abilities": [abilities[k] for k in sorted(abilities)],
            "channels": [channels[k] for k in sorted(channels)]}


def attach_capabilities(bundle, facts):
    normalized = capability_facts(facts)
    bundle["routing_capabilities"] = {"facts": normalized, "facts_sha256": digest(normalized)}
    return bundle


def dormant_default_proof(bundle, group):
    # A source own-group membership is retained, but no serving route is
    # synthesized. This narrow exception covers only the observed default
    # group with no abilities or channel membership, including disabled rows.
    need(group == "default" and "routing_capabilities" in bundle,
         "identity_source_allowed_group_not_active_native")
    capabilities = bundle["routing_capabilities"]
    facts = capability_facts(capabilities["facts"])
    need(digest(facts) == capabilities["facts_sha256"], "identity_capability_facts_changed")
    abilities = sum(row["group"] == group for row in facts["abilities"])
    channels = sum(group in row["group"].split(",") for row in facts["channels"])
    need(abilities == channels == 0, "identity_dormant_default_has_source_capability")
    return {"kind": "dormant_source_own_default_membership", "source_capability_facts_sha256": digest(facts),
            "all_state_ability_rows": abilities, "all_state_channel_group_rows": channels,
            "native_route_created": False, "group_substitution": False}


def load_routing_capabilities(bundle, path):
    """Rebind small selected facts to the already captured canonical export.

    Every immutable byte hash, catalog metadata and all row counts are checked.
    Only users/abilities/channels rows are decoded; token rows are never parsed
    or retained, and channel secrets are discarded. This is a selected identity
    and capability binding, not a new full financial approval or runtime proof.
    """
    x = private_json(path)
    need(x.get("format") == "lmm.current-default-routing-static-facts.v1"
         and x.get("status") == "STATIC_SELECTED_ROUTING_FACTS_NOT_APPROVAL"
         and x.get("production_writes") == 0 and x.get("channel_secret_fields_retained") is False
         and x.get("raw_token_rows_parsed_or_retained") is False
         and x["source_archive_sha256"] == bundle["source"]["source_archive_sha256"],
         "identity_capability_source_contract_invalid")
    refs = {name: x[name] for name in ("source_export_ref", "source_export_proof_ref", "source_receipt_ref")}
    verify_files(refs)
    planner = F.PLANNER
    receipt = private_json(refs["source_receipt_ref"]["path"])
    proof = private_json(refs["source_export_proof_ref"]["path"])
    need(receipt.get("status") == "snapshot-restored-baseline-verified-frozen-and-classified"
         and receipt.get("financial_approval") is False and receipt.get("runtime_activation_authorized") is False
         and receipt.get("production_writes") == 0 and receipt.get("target_database_accessed") is False
         and receipt["archive"]["sha256"] == proof["source_archive_sha256"] == x["source_archive_sha256"]
         and proof["source_export_sha256"] == refs["source_export_ref"]["sha256"],
         "identity_capability_canonical_receipt_invalid")
    restore_ref = receipt["original_restore_receipt"]
    need(file_ref(restore_ref["path"]) == restore_ref
         and private_json(restore_ref["path"]) == private_json(bundle["files"]["restore"]["path"]),
         "identity_capability_original_restore_mismatch")
    archive_ref = file_ref(receipt["archive"]["path"])
    need(archive_ref["sha256"] == x["source_archive_sha256"], "identity_capability_archive_changed")
    columns, baseline, metadata_hashes, _, _ = planner.metadata(receipt)
    selected = {table for table in columns if planner.table_families(table) or planner.FINANCIAL_NAME.search(table)}
    need(proof.get("metadata_hashes") == metadata_hashes and proof.get("source_column_coverage_exact") is True
         and proof.get("source_row_baseline_exact") is True
         and proof["selected_table_counts"] == {table: baseline[table] for table in selected},
         "identity_capability_canonical_baseline_mismatch")
    source_instance = bundle["source"]["restored_identity"]
    actual_instance = proof["source_instance"]
    canonical_instance = receipt["source_instance_identity"]
    planner.IDENTITY.verify_instance(actual_instance, receipt["readonly_reader"], canonical_instance, source=True)
    # The readonly export identity intentionally lacks pg_control_system and
    # data_directory. Their actual restore/admin metadata is in the receipt;
    # bind both observations through the same database/OID and instance nonce.
    need(canonical_instance["system_identifier"] == source_instance["system_identifier"]
         and actual_instance["database"] == canonical_instance["database"] == source_instance["database"]
         and str(actual_instance["database_oid"]) == str(canonical_instance["database_oid"])
         and actual_instance["server_instance_nonce"] == canonical_instance["server_instance_nonce"] == source_instance["nonce"]
         and canonical_instance["data_directory"] == source_instance["data_directory"]
         and receipt["source"]["schema"] == bundle["namespace"]["schema"],
         "identity_capability_restored_instance_mismatch")
    from collections import Counter
    counts, framing = Counter(), Counter()
    rows, user_hashes = {"abilities": [], "channels": []}, {}
    users = {row["id"]: row for row in bundle["source"]["data"]["users"]}
    row_header = re.compile(r'^\s*\{\s*"kind"\s*:\s*"row"\s*,\s*"table"\s*:\s*"([^"]+)"\s*,\s*"row"\s*:')
    with Path(refs["source_export_ref"]["path"]).open() as stream:
        for line in stream:
            header = row_header.match(line[:160])
            if header is None:
                tagged = planner.parse_json(line)
                need(type(tagged) is dict and tagged.get("kind") in ("identity", "schema"),
                     "identity_capability_export_framing_invalid")
                framing[tagged["kind"]] += 1
                if tagged["kind"] == "identity":
                    need(tagged["row"] == actual_instance, "identity_capability_export_instance_changed")
                else:
                    need(tagged["row"]["schema"] == receipt["source"]["schema"]
                         and tagged["row"]["schema_owner"] == receipt["source_instance_identity"]["schema_owner"],
                         "identity_capability_export_schema_changed")
                continue
            table = header.group(1)
            need(table in selected, "identity_capability_export_unknown_table")
            counts[table] += 1
            need(counts[table] <= baseline[table], "identity_capability_export_row_count_changed")
            if table not in ("users", "abilities", "channels"):
                continue
            tagged = planner.parse_json(line)
            row = tagged["row"]
            need(tagged["table"] == table and type(row) is dict and set(row) == columns[table],
                 "identity_capability_selected_row_columns_changed")
            if table == "users":
                uid = row["id"]
                need(uid in users and uid not in user_hashes and {k: row[k] for k in SOURCE_COLUMNS} == users[uid],
                     "identity_selected_user_disagrees_with_canonical_export")
                user_hashes[uid] = planner.digest(row)
            else:
                wanted = ABILITY_COLUMNS if table == "abilities" else CHANNEL_CAPABILITY_COLUMNS
                rows[table].append({key: row[key] for key in wanted})
    need(framing == {"identity": 1, "schema": 1} and all(counts[t] == baseline[t] for t in selected)
         and set(user_hashes) == set(users), "identity_capability_export_coverage_incomplete")
    facts = capability_facts(rows)
    need(facts == capability_facts(x["selected_source_routing_facts"]), "identity_capability_selected_facts_changed")
    for name in receipt["private_metadata_files"]:
        p = Path(name)
        if p.name in metadata_hashes:
            reference = file_ref(p)
            need(reference["sha256"] == metadata_hashes[p.name], "identity_capability_metadata_changed")
            bundle["files"]["canonical_metadata_" + p.name] = reference
    verify_files(refs)
    bundle["files"].update({"canonical_" + key: value for key, value in refs.items()})
    bundle["files"].update(canonical_restore=restore_ref, canonical_archive=archive_ref, routing_capabilities=file_ref(path))
    bundle["canonical_source"] = {"source_export_sha256": proof["source_export_sha256"],
        "source_export_proof_sha256": refs["source_export_proof_ref"]["sha256"],
        "source_receipt_sha256": refs["source_receipt_ref"]["sha256"],
        "full_source_row_digest_format": planner.DIGEST_FORMAT,
        "source_user_row_digests": [{"source_user_id": uid, "source_row_sha256": user_hashes[uid]} for uid in sorted(user_hashes)],
        "selected_binding_scope": "canonical-bytes-metadata-framing-counts-and-selected-identity-routing-rows",
        "financial_approval": False, "runtime_observation_established": False}
    return attach_capabilities(bundle, facts)


def projection_sql():
    def rows(table, fields, order):
        obj = "jsonb_build_object(" + ",".join(literal(k) + "," + k for k in fields) + ")"
        return f"(SELECT coalesce(jsonb_agg({obj} ORDER BY {order}),'[]'::jsonb) FROM {table})"
    fields = {
        "users": rows("core_identity.users", USER_COLUMNS, "id"),
        "accounts": rows("core_identity.accounts", ACCOUNT_COLUMNS, "id"),
        "profiles": rows("core_identity.user_profiles", PROFILE_COLUMNS, "user_id"),
        "developer": rows("core_identity.developer_access", DEV_COLUMNS, "user_id"),
        "passwords": "(SELECT coalesce(jsonb_agg(jsonb_build_object('user_id',user_id,'login_name',login_name,"
                     "'password_sha256',encode(sha256(convert_to(password_hash,'UTF8')),'hex')) ORDER BY user_id),'[]'::jsonb) FROM core_identity.password_logins)",
        "oauth": rows("core_identity.oauth_identities", ("issuer", "subject", "user_id"), "issuer,subject"),
        "account_policies": rows("core_billing.account_policies", ("account_id", "preference", "revision"), "account_id"),
        "original_imports": "(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY migration_id),'[]'::jsonb) FROM core_migration.import_runs r WHERE manifest->>'scope'='identity-only')",
        "history_imports": "(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY migration_id),'[]'::jsonb) FROM core_migration.import_runs r WHERE manifest->>'scope'='history-only-identity-supplement.v1')",
        "current_identity_imports": "(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY migration_id),'[]'::jsonb) FROM core_migration.import_runs r WHERE manifest->>'scope'=" + literal(SCOPE) + ")",
        "routing_grants": rows("core_identity.user_routing_groups", ("user_id", "group_name", "enabled", "source_reference"), "user_id,group_name"),
        "active_routing_groups": "(SELECT coalesce(jsonb_agg(g.name ORDER BY g.name),'[]'::jsonb) FROM public.routing_active_release r JOIN public.routing_groups g ON g.version=r.config_version WHERE r.singleton AND g.enabled)",
        "defined_routing_groups": "(SELECT coalesce(jsonb_agg(g.name ORDER BY g.name),'[]'::jsonb) FROM public.routing_active_release r JOIN public.routing_groups g ON g.version=r.config_version WHERE r.singleton)",
    }
    return "SELECT jsonb_build_object(" + ",".join(literal(k) + "," + v for k, v in fields.items()) + ")"


def snapshot_sql(contract, inventory, migration_id):
    sql = U.snapshot_sql(contract, inventory, migration_id)
    needle = "'native_users',(" + C.native_users_sql() + "),"
    need(sql.count(needle) == 1, "identity_shared_snapshot_interface_changed")
    return sql.replace(needle, needle + "'identity_projection',(" + projection_sql() + "),")


def bind_original(original, projection):
    runs = projection["original_imports"]
    need(len(runs) == 1, "identity_original_import_not_unique")
    run = runs[0]
    archive, records = run["source_archive_sha256"], original["records"]
    need(digest({"source_sha256": archive, "records": records}) == original["mapping_sha256"]
         == run["mapping_sha256"] == run["manifest"]["mapping_sha256"], "identity_original_mapping_semantics_changed")
    need(hashlib.sha256((canonical(original) + "\n").encode()).hexdigest() == run["manifest"]["mapping_file_sha256"],
         "identity_original_mapping_bytes_changed")
    need(hashlib.sha256(I.id_map_bytes(records).encode()).hexdigest() == run["manifest"]["source_target_id_map_sha256"],
         "identity_original_idmap_changed")
    return {r["user_id"]: r for r in records}


def next_id(state, maximum):
    need(state["increment"] == 1 and state["cache"] == 1 and state["cycle"] is False,
         "identity_sequence_not_safe_monotonic")
    value = state["last_value"] + (1 if state["is_called"] else 0)
    need(value > maximum and 0 < value <= state["max"] <= MAX_ID, "identity_sequence_behind_existing_ids")
    return value


def plan_operations(bundle, projection, sequence_state):
    """Pure planning also supports a read-only 76 observation; execution needs78."""
    source = bundle["source"]
    current = {r["user_id"]: r for r in validate_source(source)}
    need(bundle["oauth_rows"] == validate_oauth_projection(source, bundle["oauth"]),
         "identity_oauth_bundle_changed")
    raw = {r["id"]: r for r in source["data"]["users"]}
    old = bind_original(bundle["original"], projection)
    need(not projection["current_identity_imports"], "identity_prior_current_import_requires_chain_review")
    nu = {r["id"]: r for r in projection["users"]}
    na = {r["id"]: r for r in projection["accounts"]}
    np = {r["user_id"]: r for r in projection["profiles"]}
    nd = {r["user_id"]: r for r in projection["developer"]}
    pw = {r["user_id"]: r for r in projection["passwords"]}
    policies = {r["account_id"]: r for r in projection["account_policies"]}
    need(all(len(x) == len(projection[k]) for x, k in ((nu, "users"), (na, "accounts"), (np, "profiles"),
             (nd, "developer"), (pw, "passwords"), (policies, "account_policies"))), "identity_native_projection_duplicates")
    mapping = {uid: (uid, r["account_id"], "original") for uid, r in old.items()}
    history = {}
    history_versions = {}
    for receipt in projection["history_imports"]:
        m = receipt["manifest"]
        facts = {r["facts"]["id"]: r for r in m["source_user_facts"]}
        for row in m["mapping"]:
            uid, target, account = row["source_user_id"], row["target_user_id"], row["target_personal_account_id"]
            need(uid not in mapping and uid not in history and uid in raw and uid in facts,
                 "identity_history_mapping_collision")
            fact = facts[uid]
            need(digest(fact["facts"]) == fact["source_fact_sha256"] and
                 fact["facts"] == {k: raw[uid][k] for k in fact["facts"]}, "identity_history_source_identity_unproven")
            need(m["target_policy"]["users_active"] is False and m["target_policy"]["accounts_active"] is False
                 and m["target_policy"]["credentials_imported"] is False and m["target_policy"]["password_login_allowed"] is False,
                 "identity_history_dormant_policy_unproven")
            history_versions[uid] = integer(m["target_policy"].get("auth_version"),
                                            "identity_history_initial_generation_unproven")
            mapping[uid] = (target, account, "history")
            history[uid] = fact
    need(set(mapping) <= set(current), "identity_source_missing_committed_owners")
    need(len({v[0] for v in mapping.values()}) == len(mapping) and len({v[1] for v in mapping.values()}) == len(mapping),
         "identity_committed_mapping_not_injective")
    for uid, (target, account, _) in mapping.items():
        need(target in nu and account in na and target in np and nu[target]["personal_account_id"] == account
             and na[account]["kind"] == "personal", "identity_native_owner_mapping_changed")
    new_u = next_id(sequence_state[SEQUENCES[1]], max(nu, default=0))
    new_a = next_id(sequence_state[SEQUENCES[0]], max(na, default=0))
    additions = sorted(set(current) - set(mapping))
    need(new_u + len(additions) <= MAX_ID and new_a + len(additions) <= MAX_ID, "identity_id_exhaustion")
    for n, uid in enumerate(additions):
        mapping[uid] = (new_u + n, new_a + n, "new")
    operations, holds, drift = [], [], []

    def op(table, key, before, after, reason):
        if before != after:
            operations.append({"table": table, "key": key, "before": deepcopy(before), "after": deepcopy(after), "reason": reason})

    for uid in sorted(current):
        r, target, account, cohort = current[uid], *mapping[uid]
        if cohort in ("new", "history"):
            need(r["platform_role"] == "user", "identity_new_privileged_owner_requires_separate_review")
            need(not r["second_factor_required"], "identity_new_mfa_owner_requires_supported_mfa_completion")
            if cohort == "history":
                need(nu[target]["platform_role"] == "user" and nu[target]["active"] is False
                     and na[account]["active"] is False and target not in pw
                     and np[target]["username"] == r["username"] and not np[target]["password_login_allowed"]
                     and not np[target]["email_login_allowed"], "identity_history_placeholder_changed")
                need(nu[target]["auth_version"] == history_versions[uid],
                     "identity_history_independent_native_generation_drift")
            native_version = nu.get(target, {}).get("auth_version", 1)
            desired_u = {"id": target, "personal_account_id": account, "platform_role": "user", "active": r["active"],
                         "auth_version": max(r["auth_version"], native_version + int(cohort == "history" and r["active"]))}
            desired_a = deepcopy(na[account]) if cohort == "history" else {"id": account, "kind": "personal", "service_level": 0,
                "active": r["active"], "created_at": datetime.fromtimestamp(r["registered_at"] or 0, __import__('datetime').timezone.utc).isoformat()}
            desired_a["active"] = r["active"]
            desired_p = {k: r[k] for k in PROFILE_COLUMNS if k != "user_id"}
            desired_p["user_id"] = target
            op("core_identity.accounts", account, na.get(account), desired_a, cohort + "_account")
            op("core_identity.users", target, nu.get(target), desired_u, cohort + "_user")
            op("core_identity.user_profiles", target, np.get(target), desired_p, cohort + "_profile")
            if r["password_hash"] is not None:
                login = "legacy-" + str(target)
                need(not any(p["login_name"] == login for p in pw.values()), "identity_password_login_name_collision")
                op("core_identity.password_logins", target, None, {"user_id": target, "login_name": login,
                    "password_hash": r["password_hash"]}, cohort + "_password")
            before_d = nd.get(target)
            need(before_d is None, "identity_history_developer_authority_already_exists")
            op("core_identity.developer_access", target, None, {"user_id": target, "granted": r["developer_access_granted"],
                "source_reference": r["developer_access_source_reference"], "revision": 1}, cohort + "_developer")
            if account not in policies:
                op("core_billing.account_policies", account, None, {"account_id": account, "preference": "wallet_only", "revision": 1}, cohort + "_billing_policy")
            continue
        b = old[uid]
        # Password/profile/MFA changes need an independently scoped adapter;
        # do not overwrite a native reset or silently restore an old verifier.
        for k in ("username", "email", "password_hash", "second_factor_required", "platform_role"):
            need(r[k] == b[k], "identity_unhandled_source_" + k + "_drift")
        need(r["auth_version"] >= b["auth_version"], "identity_source_auth_version_regressed")
        if r["platform_role"] != "user":
            need(all(r[k] == b[k] for k in ("source_enabled", "source_deleted", "auth_version", "source_password_login_enabled")),
                 "identity_privileged_source_drift_requires_separate_review")
            holds.append({"source_user_id": uid, "target_user_id": target, "reason": "preserve_native_privileged_authority"})
            continue
        need(nu[target]["platform_role"] == "user" and np[target]["username"] == b["username"]
             and np[target]["email"] == b["email"] and np[target]["email_login_allowed"] is False,
             "identity_native_profile_or_role_changed")
        expected_pw = hashlib.sha256(b["password_hash"].encode()).hexdigest() if b["password_hash"] is not None else None
        need((pw.get(target) or {}).get("password_sha256") == expected_pw, "identity_native_password_changed")
        need(nu[target]["auth_version"] == b["auth_version"] and nu[target]["active"] == b["active"]
             and na[account]["active"] == b["active"], "identity_native_authority_diverged_from_original")
        desired_u, desired_a, desired_p = deepcopy(nu[target]), deepcopy(na[account]), deepcopy(np[target])
        status_changed = any(r[k] != b[k] for k in ("active", "source_enabled", "source_deleted"))
        auth_changed = r["auth_version"] != b["auth_version"]
        if status_changed or auth_changed:
            need(nu[target]["active"] == b["active"] and na[account]["active"] == b["active"]
                 and nu[target]["auth_version"] == b["auth_version"], "identity_native_authority_diverged_from_original")
            desired_u["active"] = desired_a["active"] = r["active"]
            desired_u["auth_version"] = max(r["auth_version"], nu[target]["auth_version"] + int(status_changed))
            desired_p["password_login_allowed"] = r["password_login_allowed"]
            drift.append({"source_user_id": uid, "kind": "ordinary_status_or_auth_version"})
        if r["source_password_login_enabled"] != b["source_password_login_enabled"]:
            need(np[target]["password_login_allowed"] == b["password_login_allowed"], "identity_native_password_policy_diverged")
            desired_p["password_login_allowed"] = r["password_login_allowed"]
        need(target in nd, "identity_original_developer_fact_missing")
        developer_changed = r["developer_access_granted"] != b["developer_access_granted"]
        if developer_changed:
            need(nd[target]["granted"] == b["developer_access_granted"]
                 and nd[target]["source_reference"] == b["developer_access_source_reference"]
                 and nd[target]["revision"] == b["developer_access_revision"], "identity_native_developer_authority_diverged")
            desired_d = {"user_id": target, "granted": r["developer_access_granted"],
                         "source_reference": r["developer_access_source_reference"], "revision": nd[target]["revision"] + 1}
            revoke = nd[target]["granted"] and not desired_d["granted"]
            op("core_identity.developer_access", target, nd[target], desired_d, "ordinary_developer_fact")
            drift.append({"source_user_id": uid, "kind": "ordinary_developer_revoke" if revoke else "ordinary_developer_grant"})
        else:
            revoke = False
        op("core_identity.accounts", account, na[account], desired_a, "ordinary_status")
        op("core_identity.users", target, nu[target], desired_u, "ordinary_status_auth_or_revocation")
        op("core_identity.user_profiles", target, np[target], desired_p, "ordinary_password_policy")
    generation_lineage = generation_stages(operations, mapping, current, raw, old, history_versions, nu)
    # Exact issuer+subject ownership and the independent (owner,issuer) unique
    # constraint are both checked. Never reassign a subject or link by email.
    existing = {(r["issuer"], r["subject"]): r for r in projection["oauth"]}
    by_owner = {(r["user_id"], r["issuer"]): r for r in projection["oauth"]}
    need(len(existing) == len(by_owner) == len(projection["oauth"]), "identity_native_oauth_duplicate")
    for r in bundle["oauth_rows"]:
        target = mapping[r["source_user_id"]][0]
        key = r["issuer"], r["subject"]
        wanted = {"issuer": r["issuer"], "subject": r["subject"], "user_id": target}
        if key in existing:
            need(existing[key] == wanted, "identity_oauth_subject_owner_collision")
        else:
            need((target, r["issuer"]) not in by_owner, "identity_oauth_owner_has_other_subject")
            op("core_identity.oauth_identities", list(key), None, wanted, "verified_source_provider_binding")
            by_owner[target, r["issuer"]] = wanted
    # Source allowlists and account group semantics are the same pure rules as
    # the key proposal. No unavailable native group is silently substituted.
    need("routing" in bundle, "identity_current_source_routing_facts_required")
    routing = bundle["routing"]
    need(routing["policy"] == routing_policy(routing["facts"]), "identity_routing_policy_changed")
    route_users = {r["id"]: r for r in routing["facts"]["users"]}
    grants = {(r["user_id"], r["group_name"]): r for r in projection["routing_grants"]}
    need(len(grants) == len(projection["routing_grants"]), "identity_native_routing_grant_duplicate")
    active_groups = set(projection["active_routing_groups"])
    defined_groups = set(projection["defined_routing_groups"])
    need(active_groups <= defined_groups, "identity_native_routing_projection_inconsistent")
    route_proofs = []
    for uid, (target, account, cohort) in mapping.items():
        if cohort not in ("new", "history"):
            continue
        u = route_users[uid]
        need(all(raw[uid][k] == u[k] for k in ROUTING_USER_COLUMNS if k != "group"), "identity_routing_source_drift")
        try:
            setting = json.loads(u["setting"] or "{}", object_pairs_hook=U.P.unique_object)
            need(type(setting) is dict, "identity_routing_setting_invalid")
        except (ValueError, TypeError, RecursionError):
            raise Error("identity_routing_setting_invalid") from None
        snapshot = {k: u[k] for k in ("id", "status", "deleted_at", "auth_version", "group")}
        snapshot["billing_preference"] = setting.get("billing_preference")
        idmap = {"source_user_id": uid, "target_user_id": target, "target_personal_account_id": account}
        groups = set(routing["policy"]["group_authority"]["user_usable_groups"]) | {u["group"]}
        for group in sorted(groups):
            proof = U.P.group_authority_record(routing["policy"], snapshot, idmap, "" if group == u["group"] else group)
            need(proof["allowed"] and proof["effective_group"] == group and group != "auto", "identity_source_routing_group_not_proven")
            availability = {"kind": "active_native_routing_group"}
            if group not in active_groups:
                need(group not in defined_groups and group == u["group"],
                     "identity_source_allowed_group_not_active_native")
                availability = dormant_default_proof(bundle, group)
            key = (target, group)
            if key in grants:
                need(grants[key]["enabled"] is True, "identity_native_group_independently_disabled")
            else:
                op("core_identity.user_routing_groups", list(key), None, {"user_id": target, "group_name": group,
                    "enabled": True, "source_reference": "legacy-current:" + source["source_archive_sha256"] + ":source-group-authority"},
                    cohort + "_source_routing_grant")
            route_proofs.append({"source_user_id": uid, "target_user_id": target, "group_name": group,
                                 "proof": proof, "native_availability": availability})
    # Profiles are unique by exact username, independently from internal login
    # names. Let the exact Core catalog enforce its additional constraints too.
    names = {r["username"]: r["user_id"] for r in projection["profiles"]}
    for operation in operations:
        if operation["table"] == "core_identity.user_profiles":
            a = operation["after"]
            need(a["username"] not in names or names[a["username"]] == a["user_id"], "identity_username_collision")
            names[a["username"]] = a["user_id"]
    order = {t: n for n, t in enumerate(MUTABLE)}
    operations.sort(key=lambda x: (order[x["table"]], x["key"] if type(x["key"]) is int else canonical(x["key"])))
    by_target = {r["target_user_id"]: r for r in generation_lineage}
    by_account = {r["target_personal_account_id"]: r for r in generation_lineage}
    for row in generation_lineage:
        row["planned_authority_operations"] = []
    canonical_rows = {r["source_user_id"]: r["source_row_sha256"]
                      for r in bundle.get("canonical_source", {}).get("source_user_row_digests", [])}
    if bundle["kind"] == "actual-restored-selected-identity-v1":
        need(set(canonical_rows) == set(current), "identity_canonical_user_row_coverage_missing")
    for row in generation_lineage:
        row["source_canonical_row_sha256"] = canonical_rows.get(row["source_user_id"])
    for operation in operations:
        if operation["table"] in ("core_identity.accounts", "core_billing.account_policies"):
            row = by_account[operation["key"]]
        else:
            target = operation["after"]["id"] if operation["table"] == "core_identity.users" else operation["after"]["user_id"]
            row = by_target[target]
        row["planned_authority_operations"].append({"table": operation["table"],
            "operation": "insert" if operation["before"] is None else "update", "reason": operation["reason"]})
        if row["binding_kind"] == "unchanged_source_identity":
            row["binding_kind"] = "reviewed_ordinary_source_delta"
    result = {"mapping": [{"source_user_id": uid, "target_user_id": t, "target_personal_account_id": a, "cohort": c}
                           for uid, (t, a, c) in sorted(mapping.items())], "operations": operations,
              "privileged_holds": holds, "ordinary_drift": drift, "new_user_count": len(additions),
              "history_completion_count": len(history), "first_user_id": new_u, "first_account_id": new_a,
              "source_users": len(current), "unsupported_password_without_mapped_method": [
                  uid for uid, r in current.items() if r["active"] and "password_format_unsupported" in r["blocked_reasons"]
                  and not any(o["source_user_id"] == uid for o in bundle["oauth_rows"])],
              "routing_grant_proofs": route_proofs, "routing_permissions_for_new_accounts_not_in_this_scope": False}
    result["auth_version_lineage"] = generation_lineage
    result["auth_version_lineage_sha256"] = digest(generation_lineage)
    result["source_user_set_sha256"] = digest(sorted(current))
    result["expected_identity_authority"] = expected_identity_authority(projection, result)
    return result


def generation_stages(operations, mapping, current, raw, original, history_versions, native_users):
    """Separate the explicit user update from actual Core78 trigger effects.

    A status/role update and an existing profile policy update are independent
    generation changes. The profile trigger cannot share the user's increment.
    New profiles, developer grants and OAuth bindings do not increment it.
    No source fact is rewritten to match the resulting native generation.
    """
    indexed = {(o["table"], o["key"]): o for o in operations if type(o["key"]) is int}
    lineage = []
    for uid, (target, account, cohort) in sorted(mapping.items()):
        before = native_users.get(target)
        user_op = indexed.get(("core_identity.users", target))
        user = deepcopy(user_op["after"] if user_op else before)
        need(user is not None, "identity_generation_user_missing")
        profile_op = indexed.get(("core_identity.user_profiles", target))
        developer_op = indexed.get(("core_identity.developer_access", target))
        profile_increment = int(profile_op is not None and profile_op["before"] is not None and any(
            profile_op["before"][k] != profile_op["after"][k]
            for k in ("password_login_allowed", "second_factor_required")))
        revoke_increment = int(developer_op is not None and developer_op["before"] is not None
                               and developer_op["before"]["granted"] and not developer_op["after"]["granted"])
        explicit = integer(user["auth_version"], "identity_explicit_generation_invalid")
        authority_changed = before is not None and any(user[k] != before[k] for k in ("active", "platform_role"))
        if before is not None:
            need(explicit >= before["auth_version"] + int(authority_changed),
                 "identity_explicit_user_generation_insufficient")
        final = integer(explicit + profile_increment + revoke_increment, "identity_final_generation_exhausted")
        changes = []
        if before is None:
            changes.append({"kind": "new_native_identity", "reason": "new_user"})
        elif authority_changed:
            changes.append({"kind": "user_active_or_role", "reason": user_op["reason"],
                            "before": {k: before[k] for k in ("active", "platform_role")},
                            "after": {k: user[k] for k in ("active", "platform_role")}})
        if before is not None and explicit > before["auth_version"]:
            changes.append({"kind": "explicit_user_generation", "reason": user_op["reason"],
                            "before": before["auth_version"], "after": explicit})
        if profile_increment:
            changes.append({"kind": "profile_policy_generation", "reason": profile_op["reason"],
                            "before": {k: profile_op["before"][k] for k in ("password_login_allowed", "second_factor_required")},
                            "after": {k: profile_op["after"][k] for k in ("password_login_allowed", "second_factor_required")}})
        if revoke_increment:
            changes.append({"kind": "developer_access_revocation", "reason": developer_op["reason"],
                            "before": True, "after": False})
        user["auth_version"] = final
        if user_op is not None:
            user_op["after"] = user
        elif final != before["auth_version"]:
            operation = {"table": "core_identity.users", "key": target, "before": deepcopy(before),
                         "after": user, "reason": "ordinary_trigger_generation"}
            operations.append(operation)
            indexed["core_identity.users", target] = operation
        privileged = current[uid]["platform_role"] != "user"
        need(not privileged or (not changes and final == before["auth_version"]),
             "identity_privileged_generation_must_be_preserved")
        lineage.append({"source_user_id": uid, "target_user_id": target, "target_personal_account_id": account,
                        "cohort": cohort, "source_auth_version": raw[uid]["auth_version"],
                        "original_source_auth_version": original[uid]["auth_version"] if cohort == "original" else None,
                        "history_initial_native_auth_version": history_versions.get(uid),
                        "source_identity_row_sha256": digest(raw[uid]),
                        "native_before": deepcopy(before), "explicit_user_auth_version": explicit,
                        "profile_policy_trigger_increment": profile_increment,
                        "developer_revocation_trigger_increment": revoke_increment,
                        "planned_final_auth_version": final, "planned_authority_changes": changes,
                        "binding_kind": "preserved_privileged_requires_separate_lineage" if privileged else
                            "new_source_identity" if cohort == "new" else
                            "reviewed_history_completion" if cohort == "history" else
                            "reviewed_ordinary_source_delta" if changes else "unchanged_source_identity"})
    return lineage


def expected_identity_authority(projection, plan):
    tables = {name: {r[key]: deepcopy(r) for r in projection[name]}
              for name, key in (("users", "id"), ("accounts", "id"), ("profiles", "user_id"), ("developer", "user_id"))}
    names = {"core_identity.users": "users", "core_identity.accounts": "accounts",
             "core_identity.user_profiles": "profiles", "core_identity.developer_access": "developer"}
    for operation in plan["operations"]:
        if operation["table"] in names:
            tables[names[operation["table"]]][operation["key"]] = deepcopy(operation["after"])
    result = []
    for row in plan["mapping"]:
        uid, account = row["target_user_id"], row["target_personal_account_id"]
        u, a, p, d = tables["users"][uid], tables["accounts"][account], tables["profiles"][uid], tables["developer"].get(uid)
        result.append({"source_user_id": row["source_user_id"], "target_user_id": uid,
                       "target_personal_account_id": u["personal_account_id"],
                       "active": u["active"], "account_active": a["active"], "platform_role": u["platform_role"],
                       "auth_version": u["auth_version"], "password_login_allowed": p["password_login_allowed"],
                       "second_factor_required": p["second_factor_required"],
                       "developer_granted": d["granted"] if d else None,
                       "developer_revision": d["revision"] if d else None})
    return result


def prepare(bundle, snapshot, contract, migration_id, runtime_roles):
    U.validate_snapshot(snapshot, contract)
    need(I.MIGRATION_ID.fullmatch(migration_id) is not None, "identity_operation_id_invalid")
    need(type(runtime_roles) is list and runtime_roles and runtime_roles == sorted(set(runtime_roles))
         and all(U.IDENTIFIER.fullmatch(x) for x in runtime_roles), "identity_runtime_roles_invalid")
    need(snapshot["instance"]["role"] not in runtime_roles and snapshot["instance"]["session_user"] not in runtime_roles,
         "identity_runtime_roles_overlap_migration")
    p = plan_operations(bundle, snapshot["identity_projection"], snapshot["state"]["sequences"])
    namespace = bundle["namespace"]
    result = {"format_version": 1, "scope": SCOPE, "policy": POLICY, "migration_id": migration_id,
              "mapping_hash_domain": "sha256-canonical-json-utf8-v1",
              "source_archive_sha256": bundle["source"]["source_archive_sha256"],
              "source_namespace": namespace, "source_namespace_sha256": U.namespace_key(namespace),
              "source_kind": bundle["kind"], "source_files": bundle["files"],
              "source_identity_facts_sha256": bundle["source"]["source_identity_facts_sha256"],
              "source_oauth_facts_sha256": digest({k: bundle["oauth"]["projection"][k] for k in ("users", "oauth_providers", "oauth_bindings")}),
              "source_routing_facts_sha256": digest(bundle["routing"]["facts"]),
              "source_routing_capability_facts_sha256": bundle.get("routing_capabilities", {}).get("facts_sha256"),
              "source_canonical_binding": bundle.get("canonical_source"),
              "source_canonical_export_sha256": bundle.get("canonical_source", {}).get("source_export_sha256"),
              "source_canonical_export_proof_sha256": bundle.get("canonical_source", {}).get("source_export_proof_sha256"),
              "source_canonical_receipt_sha256": bundle.get("canonical_source", {}).get("source_receipt_sha256"),
              "source_canonical_row_digest_format": bundle.get("canonical_source", {}).get("full_source_row_digest_format"),
              "source_identity_columns": list(SOURCE_COLUMNS),
              "source_user_set_sha256": p["source_user_set_sha256"],
              "auth_version_lineage_sha256": p["auth_version_lineage_sha256"],
              "snapshot": snapshot, "before_state": snapshot["state"], "contract": contract,
              "runtime_roles": runtime_roles, "plan": p, "mapping_sha256": digest(p),
              "identity_map_sha256": digest(p["mapping"]), "production_authorized": False,
              "financial_reconciliation": False, "provider_runtime_login_tested": False}
    return result


def validate_material(m):
    need(m.get("format_version") == 1 and m.get("scope") == SCOPE and m.get("policy") == POLICY
         and m.get("mapping_hash_domain") == "sha256-canonical-json-utf8-v1",
         "identity_material_scope_invalid")
    U.validate_snapshot(m["snapshot"], m["contract"])
    need(m["mapping_sha256"] == digest(m["plan"]) and m["identity_map_sha256"] == digest(m["plan"]["mapping"])
         and m["before_state"] == m["snapshot"]["state"]
         and m["source_namespace_sha256"] == U.namespace_key(m["source_namespace"]), "identity_material_digest_mismatch")
    need(m["production_authorized"] is False and m["financial_reconciliation"] is False
         and m["provider_runtime_login_tested"] is False, "identity_material_claims_out_of_scope")
    if m["source_kind"] == "actual-restored-selected-identity-v1":
        need(type(m["source_canonical_binding"]) is dict
             and HEX.fullmatch(m["source_canonical_binding"]["source_export_sha256"])
             and m["source_canonical_export_sha256"] == m["source_canonical_binding"]["source_export_sha256"]
             and m["source_canonical_export_proof_sha256"] == m["source_canonical_binding"]["source_export_proof_sha256"]
             and m["source_canonical_receipt_sha256"] == m["source_canonical_binding"]["source_receipt_sha256"]
             and m["source_canonical_row_digest_format"] == F.PLANNER.DIGEST_FORMAT
             and type(m["source_routing_capability_facts_sha256"]) is str
             and HEX.fullmatch(m["source_routing_capability_facts_sha256"]),
             "identity_actual_canonical_and_capability_binding_missing")
    for o in m["plan"]["operations"]:
        need(o["table"] in MUTABLE and o["after"] is not None, "identity_mutation_out_of_scope")
    validate_generation_plan(m)
    # Rebuild from private sources before execution, not just hash this plan.


def validate_generation_plan(m):
    p = m["plan"]
    rows = p["auth_version_lineage"]
    mapping = {r["source_user_id"]: r for r in p["mapping"]}
    actual = {r["source_user_id"]: r for r in rows}
    native = {r["id"]: r for r in m["snapshot"]["identity_projection"]["users"]}
    ops = {(o["table"], o["key"]): o for o in p["operations"] if type(o["key"]) is int}
    need(len(rows) == len(actual) == len(mapping) == p["source_users"]
         and set(actual) == set(mapping) and m["source_identity_columns"] == list(SOURCE_COLUMNS)
         and digest(sorted(mapping)) == p["source_user_set_sha256"] == m["source_user_set_sha256"]
         and digest(rows) == p["auth_version_lineage_sha256"] == m["auth_version_lineage_sha256"],
         "identity_generation_lineage_coverage_mismatch")
    for uid, row in actual.items():
        target = mapping[uid]["target_user_id"]
        need(all(row[k] == mapping[uid][k] for k in ("source_user_id", "target_user_id", "target_personal_account_id", "cohort"))
             and row["native_before"] == native.get(target)
             and type(row["source_identity_row_sha256"]) is str and HEX.fullmatch(row["source_identity_row_sha256"]),
             "identity_generation_lineage_owner_changed")
        integer(row["source_auth_version"], "identity_source_generation_invalid")
        user_op = ops.get(("core_identity.users", target))
        after = user_op["after"] if user_op else native[target]
        profile = ops.get(("core_identity.user_profiles", target))
        developer = ops.get(("core_identity.developer_access", target))
        pi = int(profile is not None and profile["before"] is not None and any(
            profile["before"][k] != profile["after"][k] for k in ("password_login_allowed", "second_factor_required")))
        di = int(developer is not None and developer["before"] is not None
                 and developer["before"]["granted"] and not developer["after"]["granted"])
        explicit = integer(row["explicit_user_auth_version"], "identity_explicit_generation_invalid")
        need(row["profile_policy_trigger_increment"] == pi and row["developer_revocation_trigger_increment"] == di
             and row["planned_final_auth_version"] == after["auth_version"] == explicit + pi + di,
             "identity_generation_stage_arithmetic_changed")
        if target in native:
            change = any(native[target][k] != after[k] for k in ("active", "platform_role"))
            need(explicit >= native[target]["auth_version"] + int(change),
                 "identity_explicit_user_generation_insufficient")
        if row["binding_kind"] == "preserved_privileged_requires_separate_lineage":
            need(user_op is None and not pi and not di and not row["planned_authority_changes"],
                 "identity_privileged_generation_must_be_preserved")
    need(p["expected_identity_authority"] == expected_identity_authority(m["snapshot"]["identity_projection"], p),
         "identity_expected_authority_projection_changed")
    defined = set(m["snapshot"]["identity_projection"]["defined_routing_groups"])
    active = set(m["snapshot"]["identity_projection"]["active_routing_groups"])
    for row in p["routing_grant_proofs"]:
        availability = row["native_availability"]
        if availability["kind"] == "active_native_routing_group":
            need(availability == {"kind": "active_native_routing_group"} and row["group_name"] in active,
                 "identity_active_routing_proof_changed")
        else:
            need(row["group_name"] == "default" and row["group_name"] not in defined
                 and row["proof"]["raw_group"] == ""
                 and availability == {"kind": "dormant_source_own_default_membership",
                     "source_capability_facts_sha256": m["source_routing_capability_facts_sha256"],
                     "all_state_ability_rows": 0, "all_state_channel_group_rows": 0,
                     "native_route_created": False, "group_substitution": False},
                 "identity_dormant_routing_proof_changed")


def key_where(table, operation, alias="t"):
    if table == "core_identity.oauth_identities":
        return f"{alias}.issuer={literal(operation['key'][0])} AND {alias}.subject={literal(operation['key'][1])}"
    if table == "core_identity.user_routing_groups":
        return f"{alias}.user_id={integer(operation['key'][0], 'identity_bad_routing_owner')} AND {alias}.group_name={literal(operation['key'][1])}"
    return f"{alias}.{MUTABLE[table]}={integer(operation['key'], 'identity_bad_key')}"


def affected_where(table, operations, alias="t"):
    return " OR ".join("(" + key_where(table, o, alias) + ")" for o in operations if o["table"] == table) or "false"


def preservation_sql(m):
    rows = []
    for table in m["before_state"]["inventory"]["tables"]:
        where = ""
        if table in MUTABLE:
            where = " WHERE NOT (" + affected_where(table, m["plan"]["operations"]) + ")"
        elif table == "core_migration.import_runs":
            where = " WHERE t.migration_id<>" + literal(m["migration_id"])
        sub = f"SELECT to_jsonb(t) AS r FROM {F.relation(table)} t{where}"
        if table in MUTABLE:
            sub += " UNION ALL SELECT row_data FROM pg_temp.lmm_identity_before_rows WHERE table_name=" + literal(table)
        rows.append(f"SELECT {literal(table)} AS name,jsonb_build_object('rows',count(*),'sha256',"
                    "encode(sha256(coalesce(string_agg(sha256(convert_to(r::text,'UTF8')),''::bytea "
                    f"ORDER BY convert_to(r::text,'UTF8')),''::bytea)),'hex')) AS value FROM ({sub}) s")
    return "SELECT jsonb_object_agg(name,value) FROM (" + " UNION ALL ".join(rows) + ") all_rows"


def expected_projection_sql(operation):
    table = operation["table"]
    columns = list(operation["after"])
    if table == "core_identity.password_logins":
        obj = "jsonb_build_object('user_id',user_id,'login_name',login_name,'password_sha256',encode(sha256(convert_to(password_hash,'UTF8')),'hex'))"
    else:
        obj = "jsonb_build_object(" + ",".join(literal(k) + "," + k for k in columns) + ")"
    return f"SELECT {obj} FROM {F.relation(table)} t WHERE " + key_where(table, operation)


def after_for_comparison(operation):
    row = deepcopy(operation["after"])
    if operation["table"] == "core_identity.password_logins":
        row["password_sha256"] = hashlib.sha256(row.pop("password_hash").encode()).hexdigest()
    return row


def readback_sql(m):
    out = []
    for o in m["plan"]["operations"]:
        out.append("IF (" + expected_projection_sql(o) + ") IS DISTINCT FROM " + literal(after_for_comparison(o)) +
                   "::jsonb THEN RAISE EXCEPTION 'identity_affected_row_readback_mismatch'; END IF;")
    return "\n".join(out)


def untouched_columns_sql(m):
    out = []
    for o in m["plan"]["operations"]:
        if o["before"] is None:
            continue
        columns = list(o["after"])
        if o["table"] == "core_identity.user_profiles":
            # This is the catalog-defined generated normalization of email.
            columns.append("email_normalized")
        removed = "ARRAY[" + ",".join(literal(k) for k in columns) + "]::text[]"
        key = MUTABLE[o["table"]]
        out.append("IF (SELECT to_jsonb(t)-" + removed + " FROM " + F.relation(o["table"]) +
                   " t WHERE " + key_where(o["table"], o) + ") IS DISTINCT FROM (SELECT row_data-" + removed +
                   " FROM pg_temp.lmm_identity_before_rows WHERE table_name=" + literal(o["table"]) +
                   " AND (row_data->>" + literal(key) + ")::bigint=" + str(o["key"]) +
                   ") THEN RAISE EXCEPTION 'identity_untouched_columns_changed'; END IF;")
    return "\n".join(out)


def write_sql(m):
    out = []
    stages = {r["target_user_id"]: r for r in m["plan"]["auth_version_lineage"]}
    for o in m["plan"]["operations"]:
        table, after = o["table"], deepcopy(o["after"])
        if table == "core_identity.users":
            stage = stages[after["id"]]
            need(after["auth_version"] == stage["planned_final_auth_version"], "identity_generation_plan_changed")
            after["auth_version"] = stage["explicit_user_auth_version"]
            if o["before"] is not None:
                changed = any(after[k] != o["before"][k] for k in ("active", "platform_role"))
                need(after["auth_version"] >= o["before"]["auth_version"] + int(changed),
                     "identity_explicit_user_generation_insufficient")
        if o["before"] is None:
            if table in ("core_identity.accounts", "core_identity.users"):
                seq = SEQUENCES[0] if table.endswith("accounts") else SEQUENCES[1]
                out.append(f"IF nextval({literal(seq)}::regclass)<>{after['id']} THEN RAISE EXCEPTION 'identity_sequence_allocation_changed'; END IF;")
            cols = ",".join(after)
            override = " OVERRIDING SYSTEM VALUE" if table == "core_identity.accounts" else ""
            out.append(f"INSERT INTO {F.relation(table)}({cols}){override} VALUES(" + ",".join(literal(v) for v in after.values()) + ");")
        else:
            key = MUTABLE[table]
            out.append(f"UPDATE {F.relation(table)} t SET " + ",".join(k + "=" + literal(v) for k, v in after.items() if k != key)
                       + " WHERE " + key_where(table, o) + ";")
    return "\n".join(out)


def identity_authority_sql(m):
    mapping = [{k: row[k] for k in ("source_user_id", "target_user_id", "target_personal_account_id")}
               for row in m["plan"]["mapping"]]
    return """SELECT coalesce(jsonb_agg(jsonb_build_object(
 'source_user_id',m.source_user_id,'target_user_id',u.id,'target_personal_account_id',u.personal_account_id,
 'active',u.active,'account_active',a.active,'platform_role',u.platform_role,'auth_version',u.auth_version,
 'password_login_allowed',p.password_login_allowed,'second_factor_required',p.second_factor_required,
 'developer_granted',d.granted,'developer_revision',d.revision) ORDER BY m.source_user_id),'[]'::jsonb)
 FROM jsonb_to_recordset(""" + literal(mapping) + """::jsonb)
 AS m(source_user_id bigint,target_user_id bigint,target_personal_account_id bigint)
 JOIN core_identity.users u ON u.id=m.target_user_id AND u.personal_account_id=m.target_personal_account_id
 JOIN core_identity.accounts a ON a.id=u.personal_account_id AND a.kind='personal'
 JOIN core_identity.user_profiles p ON p.user_id=u.id
 LEFT JOIN core_identity.developer_access d ON d.user_id=u.id"""


def build_sql(m, *, fixture_nonce=None):
    validate_material(m)
    fixture = ""
    if fixture_nonce is not None:
        need(type(fixture_nonce) is str and HEX.fullmatch(fixture_nonce)
             and re.fullmatch(r"lmm_runtime_keys_test_[a-z0-9_]{1,30}", m["snapshot"]["instance"]["database"]),
             "identity_synthetic_database_required")
        fixture = "IF current_database()<>" + literal(m["snapshot"]["instance"]["database"]) + " OR " + \
            "shobj_description((SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database') IS DISTINCT FROM " + \
            literal("lmm-runtime-keys-synthetic:" + fixture_nonce) + \
            " THEN RAISE EXCEPTION 'identity_synthetic_fixture_identity_changed'; END IF;"
    inv = m["before_state"]["inventory"]
    state = U.state_sql(inv, m["migration_id"])
    locks = ",".join(F.relation(t) for t in inv["tables"])
    seq = deepcopy(m["before_state"]["sequences"])
    for name, first in zip(SEQUENCES, (m["plan"]["first_account_id"], m["plan"]["first_user_id"])):
        if m["plan"]["new_user_count"]:
            seq[name]["last_value"] = first + m["plan"]["new_user_count"] - 1
            seq[name]["is_called"] = True
    capture = []
    for table in MUTABLE:
        capture.append("INSERT INTO pg_temp.lmm_identity_before_rows SELECT " + literal(table) +
                       f",to_jsonb(t) FROM {F.relation(table)} t WHERE " + affected_where(table, m["plan"]["operations"]) + ";")
    body = f"""
DECLARE m jsonb := {literal(m)}::jsonb; previous core_migration.import_runs%ROWTYPE; post_state jsonb; post_identity_authority jsonb;
BEGIN
 {fixture}
 {U._guard_sql(m)}
 LOCK TABLE {locks} IN ACCESS EXCLUSIVE MODE;
 {U._guard_sql(m)}
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256 IS DISTINCT FROM m->>'source_archive_sha256'
   OR previous.mapping_sha256 IS DISTINCT FROM m->>'mapping_sha256'
   OR previous.manifest->>'scope' IS DISTINCT FROM {literal(SCOPE)}
   OR previous.manifest->'material' IS DISTINCT FROM m
   OR previous.manifest->'auth_version_lineage' IS DISTINCT FROM m->'plan'->'auth_version_lineage'
   OR previous.manifest->'actual_identity_authority' IS DISTINCT FROM m->'plan'->'expected_identity_authority'
  THEN RAISE EXCEPTION 'identity_replay_receipt_conflict'; END IF;
  {readback_sql(m)}
  IF previous.manifest->'post_state' IS DISTINCT FROM ({state}) THEN RAISE EXCEPTION 'identity_replay_state_changed'; END IF;
 ELSE
  IF ({state}) IS DISTINCT FROM m->'before_state' THEN RAISE EXCEPTION 'identity_target_state_changed'; END IF;
  IF ({projection_sql()}) IS DISTINCT FROM m->'snapshot'->'identity_projection' THEN RAISE EXCEPTION 'identity_native_projection_changed'; END IF;
  CREATE TEMP TABLE lmm_identity_before_rows(table_name text NOT NULL,row_data jsonb NOT NULL) ON COMMIT DROP;
  {' '.join(capture)}
  {write_sql(m)}
  SET CONSTRAINTS ALL IMMEDIATE;
  {readback_sql(m)}
  SELECT ({identity_authority_sql(m)}) INTO post_identity_authority;
  IF post_identity_authority IS DISTINCT FROM m->'plan'->'expected_identity_authority'
   THEN RAISE EXCEPTION 'identity_complete_authority_readback_mismatch'; END IF;
  {untouched_columns_sql(m)}
  IF ({preservation_sql(m)}) IS DISTINCT FROM m->'before_state'->'tables' THEN RAISE EXCEPTION 'identity_preexisting_state_not_preserved'; END IF;
  SELECT ({state}) INTO post_state;
  IF post_state->'sequences' IS DISTINCT FROM {literal(seq)}::jsonb THEN RAISE EXCEPTION 'identity_sequence_delta_unexpected'; END IF;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->>'source_archive_sha256',m->>'mapping_sha256',jsonb_build_object(
   'scope',{literal(SCOPE)},'source_namespace_sha256',m->>'source_namespace_sha256','mapping',m->'plan'->'mapping',
   'material',m,'auth_version_lineage',m->'plan'->'auth_version_lineage',
   'actual_identity_authority',post_identity_authority,'post_state',post_state));
 END IF;
 {U._guard_sql(m)}
END
"""
    tag = "$current_identity_" + hashlib.sha256(body.encode()).hexdigest()[:24] + "$"
    need(tag not in body, "identity_sql_delimiter_collision")
    return f"""SET standard_conforming_strings=on; SET timezone='UTC'; SET lock_timeout='5s'; SET statement_timeout='180s';
DO $fence$ BEGIN IF NOT pg_try_advisory_lock({U.FENCE}) THEN RAISE EXCEPTION 'identity_import_busy'; END IF; END $fence$;
BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on; SET LOCAL row_security=off; SET LOCAL search_path=pg_catalog,public;
DO {tag}{body}{tag}; COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{literal(m['migration_id'])},'material_sha256',{literal(digest(m))});
"""


def resolve_sql(m, application_name):
    validate_material(m)
    need(type(application_name) is str and re.fullmatch(r"lmm-current-identity-[0-9a-f]{32}", application_name),
         "identity_attempt_name_invalid")
    body = f"""DECLARE previous core_migration.import_runs%ROWTYPE; BEGIN
 IF ({F.instance_sql()}) IS DISTINCT FROM {literal(m['snapshot']['instance'])}::jsonb THEN RAISE EXCEPTION 'identity_resolve_instance_changed'; END IF;
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())
  AND application_name={literal(application_name)} AND pid<>pg_backend_pid()) THEN RAISE EXCEPTION 'identity_original_attempt_still_active'; END IF;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id={literal(m['migration_id'])};
 IF FOUND THEN
  IF previous.manifest->>'scope' IS DISTINCT FROM {literal(SCOPE)} OR previous.manifest->'material' IS DISTINCT FROM {literal(m)}::jsonb
   OR previous.source_archive_sha256 IS DISTINCT FROM {literal(m['source_archive_sha256'])}
   OR previous.mapping_sha256 IS DISTINCT FROM {literal(m['mapping_sha256'])}
   OR previous.manifest->'auth_version_lineage' IS DISTINCT FROM {literal(m['plan']['auth_version_lineage'])}::jsonb
   OR previous.manifest->'actual_identity_authority' IS DISTINCT FROM {literal(m['plan']['expected_identity_authority'])}::jsonb
   THEN RAISE EXCEPTION 'identity_resolve_receipt_conflict'; END IF;
 END IF; END"""
    tag = "$identity_resolve_" + hashlib.sha256(body.encode()).hexdigest()[:24] + "$"
    # A commit receipt proves this transaction happened. Subsequent legitimate
    # password/status changes do not make it safe to retry an old identity plan.
    observations = ",".join("(" + str(n) + ",(" + expected_projection_sql(o) + "))"
                            for n, o in enumerate(m["plan"]["operations"]))
    actual = ("(SELECT coalesce(jsonb_agg(value ORDER BY position),'[]'::jsonb) FROM (VALUES " + observations +
              ") r(position,value))") if observations else "'[]'::jsonb"
    return f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC';
DO {tag}{body}{tag};
SELECT jsonb_build_object('status',CASE WHEN EXISTS(SELECT 1 FROM core_migration.import_runs WHERE migration_id={literal(m['migration_id'])})
 THEN 'COMMITTED' ELSE 'UNKNOWN' END,'automatic_retry_allowed',false,'current_identity_readback_matches',
 {literal([after_for_comparison(o) for o in m['plan']['operations']])}::jsonb = {actual}
 AND ({identity_authority_sql(m)}) = {literal(m['plan']['expected_identity_authority'])}::jsonb); COMMIT;
"""


def summary(m):
    from collections import Counter
    p = m["plan"]
    return {"status": "PREVIEW_NOT_APPROVED_NOT_APPLIED", "scope": SCOPE, "policy": POLICY,
            "source_users": p["source_users"], "new_users": p["new_user_count"],
            "history_completions": p["history_completion_count"],
            "operations_by_table": dict(Counter(o["table"] for o in p["operations"])),
            "ordinary_drift": dict(Counter(r["kind"] for r in p["ordinary_drift"])),
            "privileged_states_preserved": len(p["privileged_holds"]),
            "unsupported_source_login_without_known_alternative": len(p["unsupported_password_without_mapped_method"]),
            "identity_map_sha256": m["identity_map_sha256"], "material_sha256": digest(m),
            "new_routing_grants_out_of_scope": False, "provider_configuration_and_real_login_not_proven": True,
            "financial_reconciliation": False, "production_writes": 0}


def final_fence_observation_sql(namespace):
    """Read in the retained maintenance session after the real source barrier.

    This does not create the barrier or a claim about it. The source must have
    ALLOW_CONNECTIONS=false, no other clients and no prepared transactions.
    Exact source identities, security policy and provider ownership are read
    again. Any later source change requires a new preview and review.
    """
    U.namespace_key(namespace)
    schema = namespace["schema"]
    relation = lambda name: F.relation(schema + "." + name)
    obj = lambda columns: "jsonb_build_object(" + ",".join(literal(c) + ',"' + c + '"' for c in columns) + ")"
    providers = ("id", "name", "slug", "enabled", "client_id", "authorization_endpoint", "token_endpoint",
                 "user_info_endpoint", "scopes", "well_known", "user_id_field", "username_field", "display_name_field",
                 "email_field", "auth_style", "access_policy")
    oauth_users = ("id", "github_id", "linux_do_id", "discord_id", "telegram_id")
    bindings = ("id", "user_id", "provider_id", "provider_user_id")
    return f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC'; SET LOCAL statement_timeout='60s'; SET LOCAL row_security=off;
SELECT jsonb_build_object('format_version',2,'scope','current-source-identity-fence-observation-v2',
 'source_namespace',jsonb_build_object('system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
  'database',current_database(),'schema',{literal(schema)}),
 'observed_at',to_char(clock_timestamp(),'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
 'local_snapshot',pg_current_snapshot()::text,'transaction_read_only',current_setting('transaction_read_only'),
 'database_allows_connections',(SELECT datallowconn FROM pg_database WHERE datname=current_database()),
 'other_database_backends',(SELECT count(*) FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())
  AND pid<>pg_backend_pid() AND backend_type IS DISTINCT FROM 'autovacuum worker'),
 'prepared_transactions',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database()),
 'identity',jsonb_build_object(
  'users',(SELECT coalesce(jsonb_agg({obj(SOURCE_COLUMNS)} ORDER BY id),'[]'::jsonb) FROM {relation('users')}),
  'two_fas',(SELECT coalesce(jsonb_agg(jsonb_build_object('user_id',user_id,'is_enabled',is_enabled,'deleted_at',deleted_at) ORDER BY user_id),'[]'::jsonb) FROM {relation('two_fas')}),
  'options',(SELECT coalesce(jsonb_agg(jsonb_build_object('key',key,'value',value) ORDER BY key),'[]'::jsonb) FROM {relation('options')} WHERE key='PasswordLoginEnabled')),
 'oauth',jsonb_build_object(
  'users',(SELECT coalesce(jsonb_agg({obj(oauth_users)} ORDER BY id),'[]'::jsonb) FROM {relation('users')}),
  'oauth_providers',(SELECT coalesce(jsonb_agg({obj(providers)} ORDER BY id),'[]'::jsonb) FROM {relation('custom_oauth_providers')}),
  'oauth_bindings',(SELECT coalesce(jsonb_agg({obj(bindings)} ORDER BY id),'[]'::jsonb) FROM {relation('user_oauth_bindings')})),
 'routing',jsonb_build_object(
  'users',(SELECT coalesce(jsonb_agg({obj(ROUTING_USER_COLUMNS)} ORDER BY id),'[]'::jsonb) FROM {relation('users')}),
  'options',(SELECT coalesce(jsonb_agg(jsonb_build_object('key',key,'value',value) ORDER BY key),'[]'::jsonb)
   FROM {relation('options')} WHERE key IN ({','.join(literal(k) for k in ROUTING_OPTION_KEYS)}))),
 'routing_capabilities',jsonb_build_object(
  'abilities',(SELECT coalesce(jsonb_agg({obj(ABILITY_COLUMNS)} ORDER BY "group",model,channel_id),'[]'::jsonb) FROM {relation('abilities')}),
  'channels',(SELECT coalesce(jsonb_agg({obj(CHANNEL_CAPABILITY_COLUMNS)} ORDER BY id),'[]'::jsonb) FROM {relation('channels')})));
COMMIT;
"""


def utc_time(value):
    need(type(value) is str, "identity_time_invalid")
    try:
        result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        raise Error("identity_time_invalid") from None
    need(result.tzinfo is not None and result.utcoffset().total_seconds() == 0, "identity_time_not_utc")
    return result


def validate_final_fence(m, fence):
    fields = {"format_version", "scope", "source_namespace_sha256", "source_archive_sha256",
              "verification_actual_exit", "observation", "observation_sql", "verification_stderr", "operation_receipt"}
    need(type(fence) is dict and set(fence) == fields and fence["format_version"] == 2
         and fence["scope"] == "current-source-identity-fence-v2"
         and fence["source_namespace_sha256"] == m["source_namespace_sha256"]
         and fence["source_archive_sha256"] == m["source_archive_sha256"]
         and type(fence["verification_actual_exit"]) is int and fence["verification_actual_exit"] == 0,
         "identity_final_fence_binding_invalid")
    verify_files({k: fence[k] for k in ("observation", "observation_sql", "verification_stderr", "operation_receipt")})
    need(private_bytes(fence["observation_sql"]["path"]) == final_fence_observation_sql(m["source_namespace"]).encode(),
         "identity_final_fence_query_not_exact")
    x = private_json(fence["observation"]["path"])
    need(set(x) == {"format_version", "scope", "source_namespace", "observed_at", "local_snapshot", "transaction_read_only",
                   "database_allows_connections", "other_database_backends", "prepared_transactions", "identity", "oauth", "routing", "routing_capabilities"}
         and x["format_version"] == 2 and x["scope"] == "current-source-identity-fence-observation-v2"
         and x["source_namespace"] == m["source_namespace"] and x["transaction_read_only"] == "on"
         and x["database_allows_connections"] is False
         and all(type(x[k]) is int and x[k] == 0 for k in ("other_database_backends", "prepared_transactions"))
         and type(x["local_snapshot"]) is str and x["local_snapshot"], "identity_actual_source_barrier_missing")
    validate_observed_source_facts(m, x)
    operation = private_json(fence["operation_receipt"]["path"])
    need(set(operation) == {"format_version", "scope", "method", "source_namespace", "operator", "started_at", "finished_at",
                           "original_database_allows_connections", "original_services", "actions"}
         and operation["format_version"] == 1 and operation["scope"] == "source-database-write-fence-operations-v1"
         and operation["method"] == "database-disallow-connections-and-drain"
         and operation["source_namespace"] == m["source_namespace"]
         and type(operation["operator"]) is str and operation["operator"].strip()
         and type(operation["original_database_allows_connections"]) is bool,
         "identity_source_fence_operation_missing")
    need(utc_time(operation["started_at"]) <= utc_time(operation["finished_at"]) <= utc_time(x["observed_at"]),
         "identity_source_fence_time_order_invalid")
    need(type(operation["original_services"]) is list and operation["original_services"] and all(
        set(s) == {"unit", "active_state", "sub_state"} and all(type(v) is str and v.strip() for v in s.values())
        for s in operation["original_services"]), "identity_source_writer_states_missing")
    actions = operation["actions"]
    need(type(actions) is list and [a["action"] for a in actions] == ["stop_source_writers",
         "disallow_source_database_connections", "drain_source_database_backends"], "identity_fence_actions_missing")
    for action in actions:
        need(set(action) == {"action", "command", "exit_code", "stdout", "stderr"}
             and type(action["command"]) is str and action["command"].strip()
             and type(action["exit_code"]) is int and action["exit_code"] == 0, "identity_fence_action_failed")
        verify_files({"stdout": action["stdout"], "stderr": action["stderr"]})


def validate_observed_source_facts(m, observation):
    need(digest(observation["identity"]) == m["source_identity_facts_sha256"]
         and digest(observation["oauth"]) == m["source_oauth_facts_sha256"]
         and digest(observation["routing"]) == m["source_routing_facts_sha256"],
         "identity_source_changed_after_preview")
    capabilities = capability_facts(observation["routing_capabilities"])
    need(digest(capabilities) == m["source_routing_capability_facts_sha256"],
         "identity_source_capabilities_changed_after_preview")
    for row in m["plan"]["routing_grant_proofs"]:
        if row["native_availability"]["kind"] == "dormant_source_own_default_membership":
            bundle = attach_capabilities({}, capabilities)
            need(dormant_default_proof(bundle, row["group_name"]) == row["native_availability"],
                 "identity_dormant_default_proof_changed_at_fence")


def validate_review_and_authorization(m, sql, review_path, approval_path):
    need(m["source_kind"] == "actual-restored-selected-identity-v1", "identity_production_requires_actual_source")
    verify_files(m["source_files"])
    review, approval = private_json(review_path), private_json(approval_path)
    reference = file_ref(review_path)
    expected = {"format_version": 1, "scope": SCOPE, "decision": "review-current-source-identity-and-provider-ownership",
                "policy": POLICY, "source_archive_sha256": m["source_archive_sha256"],
                "source_namespace_sha256": m["source_namespace_sha256"],
                "source_identity_facts_sha256": m["source_identity_facts_sha256"],
                "source_oauth_facts_sha256": m["source_oauth_facts_sha256"],
                "source_routing_facts_sha256": m["source_routing_facts_sha256"],
                "source_routing_capability_facts_sha256": m["source_routing_capability_facts_sha256"],
                "source_canonical_export_sha256": m["source_canonical_export_sha256"],
                "source_canonical_export_proof_sha256": m["source_canonical_export_proof_sha256"],
                "source_canonical_binding_sha256": digest(m["source_canonical_binding"]),
                "auth_version_lineage_sha256": m["auth_version_lineage_sha256"],
                "identity_map_sha256": m["identity_map_sha256"], "mapping_sha256": m["mapping_sha256"],
                "source_review_evidence": review.get("source_review_evidence"), "final_source_fence": review.get("final_source_fence")}
    need(review == expected, "identity_external_source_review_mismatch")
    verify_files({"review_evidence": review["source_review_evidence"], "source_fence": review["final_source_fence"]})
    need(Path(review["source_review_evidence"]["path"]).absolute() != Path(review_path).absolute(), "identity_review_cannot_attest_itself")
    validate_final_fence(m, private_json(review["final_source_fence"]["path"]))
    binding = {"format_version": 1, "scope": SCOPE, "decision": DECISION, "material_sha256": digest(m),
               "sql_sha256": hashlib.sha256(sql.encode()).hexdigest(), "source_review_sha256": reference["sha256"]}
    record = approval.get("authorization_record")
    need(type(record) is dict and approval == {**binding, "authorization_record": record}, "identity_execution_authorization_mismatch")
    verify_files({"authorization": record})
    authorization = private_json(record["path"])
    need(set(authorization) == set(binding) | {"actor", "message_reference", "authorized_at", "exact_authorization"}
         and all(authorization[k] == v for k, v in binding.items())
         and all(type(authorization[k]) is str and authorization[k].strip()
                 for k in ("actor", "message_reference", "authorized_at", "exact_authorization")),
         "identity_independent_authorization_origin_missing")
    utc_time(authorization["authorized_at"])
    # An existing user authorization may be reused when it actually covers the
    # reviewed action. This parser does not fabricate or authenticate messages.


if __name__ == "__main__":
    raise SystemExit("Use the reviewed preview/execution driver; this module never connects implicitly.")

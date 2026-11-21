"""Current ordinary identity lineage; the frozen privileged bridge is unchanged.

The caller must read the import row and current authority in the same target
snapshot, and recheck both inside the locked key transaction. A proposed
identity plan is never accepted in place of that actual import row.
"""
from collections import Counter
from copy import deepcopy
import hashlib
import json
from pathlib import Path

import migrate_current_runtime_keys as U
import migrate_current_identity as ID

C, P, F = U.C, U.P, U.F
need, Error, digest = U.need, U.Error, U.digest
SCOPE = "current-ordinary-identity-generation-binding-v1"
IDENTITY_SCOPE = "current-identity-append-completion-v1"
IDENTITY_MODULE_SHA256 = "c3430d1d03f201dcf254bb31678e7af281d94136cf64b6e661881907f6b3b376"
SOURCE_COLUMNS = ("id", "username", "display_name", "email", "setting", "created_at", "auth_version",
                  "role", "status", "deleted_at", "password", "trust_level_override", "console_activated_at",
                  "github_id", "linux_do_id", "discord_id", "telegram_id", "oidc_id", "wechat_id")
MAP_FIELDS = ("source_user_id", "target_user_id", "target_personal_account_id")
USER_FIELDS = ("id", "personal_account_id", "platform_role", "active", "auth_version")
ORDINARY_KINDS = {"unchanged_source_identity", "new_source_identity", "reviewed_history_completion",
                  "reviewed_ordinary_source_delta"}
ABILITY_COLUMNS = ("group", "model", "channel_id", "enabled", "priority", "weight", "tag")
CHANNEL_COLUMNS = ("id", "type", "status", "group", "models", "priority", "weight", "model_mapping")


def integer(n, code):
    need(type(n) is int and 0 < n <= 9223372036854775807, code)
    return n


def frozen_identity_module():
    need(hashlib.sha256(Path(ID.__file__).read_bytes()).hexdigest() == IDENTITY_MODULE_SHA256,
         "current_identity_validator_bytes_changed")


def identity_authority_sql(mapping):
    """Same concrete Core projection as the current identity writer."""
    return """SELECT coalesce(jsonb_agg(jsonb_build_object(
 'source_user_id',m.source_user_id,'target_user_id',u.id,'target_personal_account_id',u.personal_account_id,
 'active',u.active,'account_active',a.active,'platform_role',u.platform_role,'auth_version',u.auth_version,
 'password_login_allowed',p.password_login_allowed,'second_factor_required',p.second_factor_required,
 'developer_granted',d.granted,'developer_revision',d.revision) ORDER BY m.source_user_id),'[]'::jsonb)
 FROM jsonb_to_recordset(""" + U.literal(mapping) + """::jsonb)
 AS m(source_user_id bigint,target_user_id bigint,target_personal_account_id bigint)
 JOIN core_identity.users u ON u.id=m.target_user_id AND u.personal_account_id=m.target_personal_account_id
 JOIN core_identity.accounts a ON a.id=u.personal_account_id AND a.kind='personal'
 JOIN core_identity.user_profiles p ON p.user_id=u.id
 LEFT JOIN core_identity.developer_access d ON d.user_id=u.id"""


def validate(source_rows, source, target, namespace, run, actual_authority):
    """Recompute all owner bindings from full canonical source rows and receipt.

This deliberately does not edit source.auth_version or manufacture a target
projection with the old version. Privileged owners still require the frozen
legacy_admin_activation_lineage proof in P/C, regardless of this receipt.
"""
    need(type(run) is dict and type(run.get("manifest")) is dict,
         "current_identity_committed_run_required")
    manifest = run["manifest"]
    m = manifest.get("material", {})
    plan = m.get("plan", {})
    frozen_identity_module()
    ID.validate_material(m)
    ns = U.namespace_key(namespace)
    need(manifest.get("scope") == m.get("scope") == IDENTITY_SCOPE
         and run.get("source_archive_sha256") == m.get("source_archive_sha256") == source["source_archive_sha256"]
         and manifest.get("source_namespace_sha256") == m.get("source_namespace_sha256") == ns
         and m.get("source_namespace") == namespace,
         "current_identity_receipt_source_binding_invalid")
    need(m.get("source_canonical_export_sha256") == source["source_export_sha256"]
         and m.get("source_canonical_row_digest_format") == F.PLANNER.DIGEST_FORMAT,
         "current_identity_canonical_source_binding_missing")
    mapping = target["source_target_id_map"]
    clean_map = [{k: r[k] for k in MAP_FIELDS} for r in plan.get("mapping", [])]
    need(clean_map == mapping and manifest.get("mapping") == plan.get("mapping")
         and run.get("mapping_sha256") == m.get("mapping_sha256")
         and m.get("source_identity_columns") == list(SOURCE_COLUMNS),
         "current_identity_receipt_mapping_invalid")
    native = {r["user_id"]: r for r in target["users"]}
    source_by_id = {r["id"]: r for r in source_rows}
    projected_by_id = {r["id"]: r for r in source["users"]}
    rows = plan.get("auth_version_lineage", [])
    by_source = {r["source_user_id"]: r for r in rows}
    authority = {r["source_user_id"]: r for r in actual_authority}
    need(len(rows) == len(by_source) == len(mapping) == len(source_rows) == len(source_by_id)
         == len(authority) == len(actual_authority) == len(projected_by_id)
         and set(by_source) == set(source_by_id) == set(projected_by_id) == set(authority)
         and {r["source_user_id"] for r in mapping} == set(source_by_id)
         and len({r["target_user_id"] for r in mapping}) == len(mapping)
         and len({r["target_personal_account_id"] for r in mapping}) == len(mapping)
         and m.get("source_user_set_sha256") == plan.get("source_user_set_sha256") == ID.digest(sorted(source_by_id))
         and m.get("auth_version_lineage_sha256") == plan.get("auth_version_lineage_sha256") == ID.digest(rows)
         and manifest.get("auth_version_lineage") == rows
         and manifest.get("actual_identity_authority") == plan.get("expected_identity_authority") == actual_authority,
         "current_identity_lineage_actual_coverage_invalid")
    ops = {(o["table"], o["key"]): o for o in plan.get("operations", []) if type(o.get("key")) is int}
    before_users = {r["id"]: r for r in m["snapshot"]["identity_projection"]["users"]}
    bindings = {}
    for mp in mapping:
        sid, tid, account = (mp[k] for k in MAP_FIELDS)
        for value in (sid, tid, account):
            integer(value, "current_identity_owner_id_invalid")
        row, full, final = by_source[sid], source_by_id[sid], authority[sid]
        need(all(row[k] == final[k] == mp[k] for k in MAP_FIELDS)
             and all(k in full for k in SOURCE_COLUMNS)
             and row["source_identity_row_sha256"] == ID.digest({k: full[k] for k in SOURCE_COLUMNS})
             and row.get("source_canonical_row_sha256") == F.PLANNER.digest(full)
             and row["source_auth_version"] == full["auth_version"] == projected_by_id[sid]["auth_version"]
             and tid in native and native[tid]["personal_account_id"] == account,
             "current_identity_lineage_source_or_owner_changed")
        before = before_users.get(tid)
        need(row["native_before"] == before, "current_identity_lineage_before_changed")
        uop, pop, dop = (ops.get((t, tid)) for t in (
            "core_identity.users", "core_identity.user_profiles", "core_identity.developer_access"))
        after = uop["after"] if uop else before
        need(type(after) is dict, "current_identity_final_user_missing")
        pi = int(pop is not None and pop["before"] is not None and any(
            pop["before"][k] != pop["after"][k] for k in ("password_login_allowed", "second_factor_required")))
        di = int(dop is not None and dop["before"] is not None
                 and dop["before"]["granted"] and not dop["after"]["granted"])
        explicit = integer(row["explicit_user_auth_version"], "current_identity_explicit_version_invalid")
        final_version = integer(explicit + pi + di, "current_identity_final_version_invalid")
        need(type(row["profile_policy_trigger_increment"]) is int and row["profile_policy_trigger_increment"] == pi
             and type(row["developer_revocation_trigger_increment"]) is int and row["developer_revocation_trigger_increment"] == di
             and row["planned_final_auth_version"] == after["auth_version"] == final["auth_version"]
             == native[tid]["auth_version"] == final_version
             and after["active"] == final["active"] and after["platform_role"] == final["platform_role"]
             and native[tid]["active"] == (final["active"] and final["account_active"])
             and native[tid]["developer_access_granted"] == (final["developer_granted"] is True),
             "current_identity_final_authority_not_actual")
        if before is not None:
            changed = any(before[k] != after[k] for k in ("active", "platform_role"))
            need(explicit >= before["auth_version"] + int(changed),
                 "current_identity_generation_rewound")
        if full["role"] != 1 or final["platform_role"] != "user":
            need(row["binding_kind"] == "preserved_privileged_requires_separate_lineage"
                 and uop is None and not pi and not di and not row["planned_authority_changes"],
                 "current_identity_privileged_bridge_forbidden")
            continue
        need(row["binding_kind"] in ORDINARY_KINDS
             and final["active"] == (full["status"] == 1 and full["deleted_at"] is None),
             "current_identity_ordinary_authority_invalid")
        if row["binding_kind"] == "unchanged_source_identity":
            need(not row["planned_authority_changes"] and final_version == full["auth_version"],
                 "current_identity_unexplained_ordinary_drift")
        elif row["binding_kind"] == "new_source_identity":
            need(before is None and row["cohort"] == "new" and final_version == full["auth_version"],
                 "current_identity_new_version_unexplained")
        else:
            # OAuth, routing and developer grants can be reviewed operations
            # without advancing the authentication generation. The frozen
            # identity validator checks the separate real trigger arithmetic.
            need(before is not None and bool(row["planned_authority_changes"] or row["planned_authority_operations"]),
                 "current_identity_changed_version_without_action")
        bindings[str(tid)] = {"scope": SCOPE, "identity_migration_id": run["migration_id"],
            "identity_material_sha256": ID.digest(m), "source_archive_sha256": source["source_archive_sha256"],
            "source_export_sha256": source["source_export_sha256"], "source_namespace_sha256": ns,
            "lineage": row, "actual_authority": final}
    return bindings


def expected_version(source_user, observed, privileged_binding, current_binding):
    if current_binding is None:
        return P.expected_auth_version(source_user, observed, privileged_binding)
    need(privileged_binding is None and current_binding["scope"] == SCOPE,
         "current_identity_lineage_collision")
    row, actual = current_binding["lineage"], current_binding["actual_authority"]
    need(source_user["id"] == row["source_user_id"]
         and source_user["auth_version"] == row["source_auth_version"]
         and observed["user_id"] == row["target_user_id"]
         and observed["personal_account_id"] == row["target_personal_account_id"]
         and observed["auth_version"] == row["planned_final_auth_version"] == actual["auth_version"],
         "current_identity_key_owner_binding_changed")
    return actual["auth_version"]


def dormant_bindings(tables, source, target, identity_run, native):
    """Exactly retain the source own-default fallback's zero model capability.

All-state source abilities/channels are checked, including disabled entries.
The native group must be absent from every routing release; membership remains
enabled with its original name, without a route or an alternate-group grant.
"""
    channels, abilities = {}, {}
    for original in tables["channels"]:
        row = {k: original[k] for k in CHANNEL_COLUMNS}
        cid = integer(row["id"], "current_dormant_channel_id_invalid")
        need(cid not in channels and type(row["group"]) is str and type(row["models"]) is str
             and type(row["status"]) is int, "current_dormant_channel_invalid")
        channels[cid] = row
    for original in tables["abilities"]:
        row = {k: original[k] for k in ABILITY_COLUMNS}
        key = row["group"], row["model"], row["channel_id"]
        need(type(row["enabled"]) is bool and type(row["group"]) is str and type(row["model"]) is str
             and row["channel_id"] in channels and key not in abilities, "current_dormant_ability_invalid")
        abilities[key] = row
    facts = {"abilities": [abilities[k] for k in sorted(abilities)], "channels": [channels[k] for k in sorted(channels)]}
    capability_hash = ID.digest(facts)
    material = identity_run["manifest"]["material"]
    need(material["source_routing_capability_facts_sha256"] == capability_hash,
         "current_dormant_source_capability_binding_changed")
    default_count = sum(r["group"] == "default" for r in facts["abilities"])
    channel_count = sum("default" in r["group"].split(",") for r in facts["channels"])
    users = {r["id"]: r for r in source["users"]}
    maps = {r["source_user_id"]: r for r in target["source_target_id_map"]}
    target_users = {r["user_id"]: r for r in target["users"]}
    memberships = {(r["user_id"], r["group_name"]): r for r in native["core_identity.user_routing_groups"]}
    proofs = material["plan"]["routing_grant_proofs"]
    expected = {"kind": "dormant_source_own_default_membership", "source_capability_facts_sha256": capability_hash,
        "all_state_ability_rows": 0, "all_state_channel_group_rows": 0,
        "native_route_created": False, "group_substitution": False}
    bindings = {}
    for token in source["tokens"]:
        user, mp = users[token["user_id"]], maps[token["user_id"]]
        # Preserve the original nonempty-group/deprecated and Auto rules.
        # This reviewed route covers only the original empty own-group fallback.
        if token["group"] != "" or user["group"] != "default":
            continue
        if "default" in target_users[mp["target_user_id"]]["routing_groups"]:
            # The established active-group path keeps its original mapper.
            continue
        authority = P.group_authority_record(source["policy"], user, mp, token["group"])
        need(authority["allowed"] and authority["effective_group"] == "default"
             and default_count == channel_count == 0
             and all(r["name"] != "default" for r in native["public.routing_groups"]),
             "current_dormant_default_not_zero_capability")
        proof = [p for p in proofs if p["source_user_id"] == user["id"] and p["group_name"] == "default"]
        membership = memberships.get((mp["target_user_id"], "default"))
        before_memberships = material["snapshot"]["identity_projection"]["routing_grants"]
        cohort = next(r["cohort"] for r in material["plan"]["mapping"] if r["source_user_id"] == user["id"])
        newly_bound = len(proof) == 1 and proof[0]["proof"] == authority and proof[0]["native_availability"] == expected
        # The current identity writer adds proofs for new/history owners. An
        # original owner's existing membership must instead be the exact row
        # preserved from that committed identity transaction's before-state.
        preserved_original = not proof and cohort == "original" and membership in before_memberships
        need((newly_bound or preserved_original) and membership is not None and membership["enabled"] is True,
             "current_dormant_membership_not_committed")
        bindings[str(token["id"])] = {"scope": "current-dormant-own-default-key-binding-v1",
            "source_token_id": token["id"], "source_token_row_sha256": digest(token),
            "source_user_id": user["id"], "target_user_id": mp["target_user_id"],
            "source_authority": authority, "availability": expected, "native_membership": membership,
            "identity_migration_id": identity_run["migration_id"],
            "identity_material_sha256": ID.digest(material), "native_callable_models": 0}
    return bindings


def dormant_row_allowed(row, binding):
    if binding is None:
        return False
    need(binding["scope"] == "current-dormant-own-default-key-binding-v1"
         and binding["source_token_id"] == row["source_token_id"]
         and binding["source_user_id"] == row["source_user_id"]
         and binding["target_user_id"] == row["target_user_id"]
         and binding["source_authority"] == row["source_group_authority"]
         and row["group"]["value"] == "" and row["native_effective_group"] == "default"
         and row["source_group_authority"]["allowed"]
         and binding["availability"]["all_state_ability_rows"] == binding["availability"]["all_state_channel_group_rows"] == 0
         and binding["availability"]["native_route_created"] is False
         and binding["availability"]["group_substitution"] is False
         and row.get("dormant_group_binding_sha256") == digest(binding),
         "current_dormant_key_binding_changed")
    return True


def plan(source, target, bindings, dormant):
    result = P.plan_api_keys(source, target)
    privileged = P.activation_bindings(target)
    native = {r["user_id"]: r for r in target["users"]}
    users = {r["id"]: r for r in source["users"]}
    for row in result["records"]:
        binding = bindings.get(str(row["target_user_id"]))
        if binding is not None:
            version = expected_version(users[row["source_user_id"]], native[row["target_user_id"]],
                                       privileged.get(row["target_user_id"]), binding)
            row["expected_target_auth_version"] = version
            row["auth_version_binding_sha256"] = digest(binding)
            row["blocked_reasons"] = [r for r in row["blocked_reasons"] if r != "owner_auth_version_changed"]
        group = dormant.get(str(row["source_token_id"]))
        if group is not None:
            row["dormant_group_binding_sha256"] = digest(group)
            need(dormant_row_allowed(row, group), "current_dormant_key_binding_missing")
            row["blocked_reasons"] = [r for r in row["blocked_reasons"] if r != "target_routing_grant_missing"]
    reasons = Counter(reason for row in result["records"] for reason in row["blocked_reasons"])
    result["status"] = "blocked" if reasons else "proposal"
    result["summary"]["blocked_token_count"] = sum(bool(r["blocked_reasons"]) for r in result["records"])
    result["summary"]["blocked_reason_counts"] = dict(sorted(reasons.items()))
    result["current_ordinary_identity_bindings_sha256"] = digest(bindings)
    result["current_dormant_group_bindings_sha256"] = digest(dormant)
    result.pop("proposal_sha256")
    result["proposal_sha256"] = P.sha256(result)
    return result


# Frozen C._supported_rows policy body; only the explicit version binding differs.
PROPOSAL, LINEAGE, require, canonical = P, C.LINEAGE, C.require, C.canonical


def supported_rows(proposal: dict, target: dict, first_id: int, *, bindings: dict, dormant: dict, revoke_unsupported: bool = False) -> list[dict]:
    observed = {row["user_id"]: row for row in target["users"]}
    mappings, _ = PROPOSAL.target_mapping(target, proposal["source_archive_sha256"])
    lineage = PROPOSAL.activation_bindings(target)
    require(type(proposal.get("source_policy")) is dict, "credential_source_group_policy_invalid")
    try:
        policy = proposal["source_policy"]
        PROPOSAL.validate_group_policy(policy)
        require(proposal["policy_sha256"] == PROPOSAL.sha256(policy), "credential_source_policy_hash_mismatch")
    except PROPOSAL.ProposalError:
        raise Error("credential_source_group_policy_invalid") from None
    rows = []
    for offset, original in enumerate(proposal["records"]):
        blocked = set(original["blocked_reasons"]) - {"recoverable_secret_encryption_required"}
        authority = original.get("source_group_authority")
        require(type(authority) is dict and type(authority.get("user_snapshot")) is dict,
                "credential_source_group_audit_invalid")
        mapping = mappings[original["source_user_id"]]
        require(original["target_user_id"] == mapping["target_user_id"]
                and original["target_personal_account_id"] == mapping["target_personal_account_id"],
                "credential_source_group_mapping_mismatch")
        try:
            expected = PROPOSAL.group_authority_record(policy, authority["user_snapshot"],
                mapping, original["group"]["value"])
        except PROPOSAL.ProposalError:
            raise Error("credential_source_group_audit_invalid") from None
        require(authority == expected and original["native_effective_group"] == expected["effective_group"],
                "credential_source_group_audit_mismatch")
        binding = lineage.get(original["target_user_id"])
        source_user = authority["user_snapshot"]
        current_binding = bindings.get(str(original["target_user_id"]))
        expected_auth = expected_version(source_user, observed[original["target_user_id"]], binding, current_binding)
        auth_hash = digest(current_binding) if current_binding else LINEAGE.digest(binding) if binding else None
        require(original.get("source_auth_version") == source_user["auth_version"]
                and original.get("expected_target_auth_version") == expected_auth
                and original.get("auth_version_binding_sha256") == auth_hash
                and original["target_auth_version"] == observed[original["target_user_id"]]["auth_version"] == expected_auth,
                "credential_owner_auth_version_changed")
        if not expected["allowed"]:
            blocked.add(expected["denial_reason"])
        native_group = original["native_effective_group"]
        dormant_allowed = dormant_row_allowed(original, dormant.get(str(original["source_token_id"])))
        if native_group != "auto" and native_group not in observed[original["target_user_id"]]["routing_groups"] and not dormant_allowed:
            blocked.add("target_routing_grant_missing")
        # Apply the same reviewed origin boundary as the mapper. Retain the raw
        # label in the record/material; this does not grant any other authority.
        if (original["creation_source"] not in PROPOSAL.AUDIT_ONLY_CREATION_SOURCES
                or "的初始令牌" in original["name"]):
            blocked.add("creation_source_metadata_requires_adapter")
        # An inactive owner stays inactive. This imports historical authority,
        # without activating the user, changing auth_version or granting access.
        if not original["source_owner_enabled"] and not observed[original["target_user_id"]]["active"]:
            blocked.discard("target_owner_inactive")
        authority_blocked = blocked & {"target_owner_inactive", "target_developer_grant_missing", "target_routing_grant_missing"}
        if original["source_ordinary_api_key"] and original["source_usable_at_snapshot"] and (binding or authority_blocked):
            require(not blocked, "credential_live_source_key_requires_complete_native_authority")
        row = json.loads(canonical(original))
        row["blocked_reasons"] = sorted(set(original["blocked_reasons"]) | blocked)
        row["target_credential_id"] = first_id + offset
        row["encrypted_secret_hex"] = None
        status, expiry = row["source_status"], row["expiry"]
        expired = expiry["kind"] != "never" and expiry["source_epoch"] < proposal["snapshot_at"]
        exhausted = not row["quota"]["unlimited"] and row["quota"]["remaining"] <= 0
        status_mismatch = not (status not in (3, 4) or (status == 3 and expired) or (status == 4 and not expired and exhausted))
        wire_unsupported = not original["source_ordinary_api_key"] or not original["source_stored_key_roundtrips_relay_parser"]
        if blocked or wire_unsupported or status_mismatch:
            if not revoke_unsupported:
                require(not (blocked or wire_unsupported), "credential_native_semantics_unsupported")
                require(not status_mismatch, "credential_status_native_mismatch")
            row["revoked_rekey_required"] = True
            row["revoked"] = True
            row["enabled"] = False
            row["one_time_reveal"] = True
            if not (1 <= len(row["name"].strip()) <= 128):
                row["name"] = f"revoked-legacy-key-{row['source_token_id']}"
            if not (1 <= len(row["native_effective_group"]) <= 64) or row["native_effective_group"] == "auto":
                row["native_effective_group"] = "default"
            row["auto_groups"]["values"] = []
            row["cross_group_retry"] = False
            row["model_limits_enabled"] = False
            row["model_limits"] = []
        else:
            # Native list views derive states 3/4 from expiry/budget, rather than
            # a stored status. Never turn an expired/exhausted key into disabled 2.
            row["enabled"] = status != 2
        rows.append(row)
    require(bool(rows), "credential_batch_empty")
    return rows

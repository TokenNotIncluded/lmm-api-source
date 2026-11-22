#!/usr/bin/env python3
"""Source-only ordinary-key partition contract. No DB, writer, approval or IDs.

The CLI rebuilds the complete parent through the frozen full-export verifier.
Only its derived ordinary view may later be supplied to unchanged A513/L0835.
This module cannot authorize migration, and has no target-continuation reader.
"""
import argparse
from collections import Counter, defaultdict
from copy import deepcopy
from datetime import datetime
import hashlib
import importlib
import json
from pathlib import Path
import re
import sys

SCOPE = "current-ordinary-key-source-partition-v1"
PINS = {
    "legacy_api_key_migration.py": "7ca343cbfcf6f58bb2162c4117fc51b3cf31a9be7484e183ecfa9fb0311e6707",
    "legacy_credential_import.py": "c274063a5d40d95821605a849a745fade0aafeb5e3d075105bb58a54c9f8f69a",
    "legacy_key_quota_opening.py": "61519913184922d82361739f6d0ec004d1ea4189564fd1464783f8de110e9aa3",
    "legacy_routing_migration.py": "8744d679aa352c852b262ad30b955f4187a8a1b3f1a6b492bc38e5b9cee1ff6b",
    "migrate-legacy-financial.py": "fa21a4f97bfcd038ee2fdcc588481e1a8704964b2c59102147c4384dbcfc1742",
    "migrate-legacy-snapshot.py": "abf4c0e95a0e0fc406c84a082e5c965f805a99a16deef517e8be38577f054ec2",
}
FROZEN_WRITERS = {
    "current_runtime_keys81.py": "51313c45f0f6fc9f614b8f70deb6d3540d2d9625cca16f600029a7604e81a7e0",
    "current_key_lineage.py": "0835d57cb70a58ed8bdf368d5c25d588aca0139dfdb44389e2550981953c9da7",
}
SOURCE_SQL_SHA = "da6e1a0f7c10a5c332fbb026ec15dccfc17b241b4f2a27fbd70b782303d87ad2"
SOURCE_COLLECTOR_SHA = "27d9c29afa0e10696591f04c68b518e1bea95dd2abb422914388aae7e789dc7c"
SOURCE_RUNTIME_SHA = "92675373e12c5bf70f499f08013ee7dad00fcd8069494e2ba2bb200197e49de4"
TABLES = ("oauth_server_grants", "oauth_server_tokens", "oauth_server_codes",
          "oauth_server_authorizations", "oauth_server_mcp_clients", "o_auth_billing_bindings")


class Error(ValueError):
    pass


def need(condition, code):
    if not condition:
        raise Error(code)


def file_hash(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_helpers(directory):
    directory = directory.resolve()
    for name, expected in PINS.items():
        need(file_hash(directory / name) == expected, "frozen_helper_changed")
    sys.path.insert(0, str(directory))
    P = importlib.import_module("legacy_api_key_migration")
    R = importlib.import_module("legacy_routing_migration")
    C = importlib.import_module("legacy_credential_import")
    for m in (P, R, C):
        need(Path(m.__file__).resolve().parent == directory, "helper_module_shadowed")
    return P, R, C


def ms(text):
    try:
        instant = datetime.fromisoformat(text.replace("Z", "+00:00"))
        need(instant.tzinfo is not None, "source_time_timezone_missing")
        return int(instant.timestamp() * 1000)
    except (AttributeError, TypeError, ValueError, OverflowError):
        raise Error("source_time_invalid") from None


def integer(value, code):
    need(type(value) is int and 0 <= value <= 9223372036854775807, code)
    return value


def indexed(rows, key, code):
    need(type(rows) is list and all(type(r) is dict and key in r for r in rows), code)
    result = {r[key]: r for r in rows}
    need(len(result) == len(rows), code)
    return result


def parent_context(P, F, parent, provenance, canonical_members, verified_files, *, origin):
    """Pure binding core; only the CLI obtains FULL_CANONICAL_EXPORT_REBUILT.

    Unit fixtures must label their origin SYNTHETIC_PURE_TEST. Neither label
    carries execution authority; the output always says PREVIEW_ONLY.
    """
    need(origin in ("FULL_CANONICAL_EXPORT_REBUILT", "SYNTHETIC_PURE_TEST"), "source_origin_invalid")
    P.fields(parent, P.SOURCE_FIELDS, "parent_shape_invalid")
    need(parent["source_projection_sha256"] == P.sha256(provenance), "parent_provenance_mismatch")
    need(provenance.get("scope") == "legacy-api-key-source-projection"
         and provenance.get("source_row_counts", {}).get("tokens") == len(parent["tokens"])
         and provenance.get("source_row_counts", {}).get("users") == len(parent["users"])
         and all(provenance.get(k) == parent[k] for k in
                 ("source_archive_sha256", "source_export_sha256", "source_schema_sha256", "snapshot_at", "source_revision")),
         "parent_coverage_mismatch")
    indexed(parent["tokens"], "id", "parent_token_ids_not_unique")
    indexed(parent["users"], "id", "parent_user_ids_not_unique")
    for row in parent["tokens"]:
        P.fields(row, P.TOKEN_FIELDS, "parent_token_columns_changed")
    return {"origin": origin, "parent_sha256": P.sha256(parent),
        "parent_provenance_sha256": P.sha256(provenance),
        "canonical_oauth_members_sha256": F.digest(sorted(canonical_members, key=lambda r: r["digest"])),
        "canonical_oauth_member_count": len(canonical_members),
        "verified_files": deepcopy(verified_files)}


def validate_capture(P, F, observation, capture, sql_path, collector_path):
    """Verify the existing complete-query acquisition, without running it."""
    need(file_hash(sql_path) == SOURCE_SQL_SHA == capture.get("source_sql_sha256"),
         "oauth_complete_query_not_reviewed")
    need(file_hash(collector_path) == SOURCE_COLLECTOR_SHA == capture.get("collector_sha256"),
         "oauth_collector_not_reviewed")
    need(capture.get("scope") == "LIVE_COHERENT_READONLY_SOURCE_NOT_PRODUCTION_FENCE_NOT_APPROVAL"
         and capture.get("production_writes") == 0, "oauth_capture_scope_invalid")
    need(observation.get("read_only") == "on"
         and observation.get("runtime", {}).get("exe_sha256") == SOURCE_RUNTIME_SHA,
         "oauth_capture_runtime_or_readonly_invalid")
    for key in ("database", "database_oid", "system_identifier", "observed_at"):
        need(capture.get(key) == observation.get(key), "oauth_capture_identity_mismatch")
    for table in (*TABLES, "managed_tokens", "users", "policy_options"):
        need(type(observation.get(table)) is list
             and capture.get("counts", {}).get(table) == len(observation[table]),
             "oauth_capture_incomplete")
    return {"scope": "reviewed-complete-live-oauth-query-v1",
        "source_observation_sha256": F.digest(observation),
        "capture_sha256": P.sha256(capture), "source_sql_sha256": SOURCE_SQL_SHA,
        "collector_sha256": SOURCE_COLLECTOR_SHA,
        "instance": {k: observation[k] for k in ("database", "database_oid", "system_identifier")},
        "observed_at": observation["observed_at"], "source_production_fenced": False}


def oauth_closure(P, F, parent, context, obs, acquisition, resolutions):
    need(acquisition.get("source_observation_sha256") == F.digest(obs)
         and acquisition.get("observed_at") == obs["observed_at"]
         and acquisition.get("source_production_fenced") is False,
         "oauth_observation_binding_changed")
    now = ms(obs["observed_at"])
    need(now >= parent["snapshot_at"] * 1000, "oauth_observation_predates_parent")
    grants = indexed(obs["oauth_server_grants"], "id", "oauth_grants_not_unique")
    tokens = indexed(obs["oauth_server_tokens"], "digest", "oauth_tokens_not_unique")
    codes = indexed(obs["oauth_server_codes"], "digest", "oauth_codes_not_unique")
    authorizations = indexed(obs["oauth_server_authorizations"], "digest", "oauth_authorizations_not_unique")
    clients = indexed(obs["oauth_server_mcp_clients"], "id", "oauth_clients_not_unique")
    need(len(tokens) == context["canonical_oauth_member_count"]
         and F.digest(sorted(tokens.values(), key=lambda r: r["digest"])) == context["canonical_oauth_members_sha256"],
         "oauth_full_canonical_member_set_changed")
    users = {u["id"]: u for u in parent["users"]}
    families = defaultdict(list)
    family_codes = defaultdict(list)
    for table, rows in (("tokens", tokens.values()), ("codes", codes.values())):
        for row in rows:
            need(row.get("family_id") in grants, "oauth_member_family_missing")
            need(type(row.get("digest")) is str and re.fullmatch(r"[0-9a-f]{64}", row["digest"]),
                 "oauth_digest_invalid")
            for field in ("created_at_ms", "expires_at_ms", "used_at_ms"):
                integer(row.get(field), "oauth_member_time_invalid")
            need(row["created_at_ms"] <= now and row["expires_at_ms"] >= row["created_at_ms"]
                 and (row["used_at_ms"] == 0 or row["created_at_ms"] <= row["used_at_ms"] <= now),
                 "oauth_member_time_inconsistent")
            need(row.get("issuer") == grants[row["family_id"]].get("issuer"), "oauth_member_issuer_mismatch")
            if table == "tokens":
                need(row.get("kind") in ("access", "refresh"), "oauth_member_kind_unknown")
                families[row["family_id"]].append(row)
            else:
                family_codes[row["family_id"]].append(row)
    terminal, unresolved = [], []
    for key, grant in sorted(grants.items()):
        need(grant.get("user_id") in users, "oauth_owner_not_in_full_parent")
        for field in ("created_at_ms", "absolute_expires_at_ms", "revoked_at_ms"):
            integer(grant.get(field), "oauth_family_time_invalid")
        need(grant["created_at_ms"] <= now and grant["absolute_expires_at_ms"] >= grant["created_at_ms"]
             and (grant["revoked_at_ms"] == 0 or grant["created_at_ms"] <= grant["revoked_at_ms"] <= now),
             "oauth_family_time_inconsistent")
        members = sorted(families[key], key=lambda r: r["digest"])
        current_access = [r for r in members if r["kind"] == "access" and r["expires_at_ms"] > now]
        current_refresh = [r for r in members if r["kind"] == "refresh" and r["used_at_ms"] == 0 and r["expires_at_ms"] > now]
        current_codes = [r for r in family_codes[key] if r["used_at_ms"] == 0 and r["expires_at_ms"] > now]
        reason = ("family_revoked" if grant["revoked_at_ms"] > 0 else
                  "family_expired" if grant["absolute_expires_at_ms"] <= now else
                  "no_live_members_or_codes" if not current_access + current_refresh + current_codes else None)
        fact = {"source_grant_id": key, "source_grant_sha256": F.digest(grant),
            "all_family_members_sha256": F.digest(members),
            "all_family_codes_sha256": F.digest(sorted(family_codes[key], key=lambda r: r["digest"])),
            "source_observation_sha256": acquisition["source_observation_sha256"]}
        if reason:
            terminal.append({"kind": "source_terminal_authority", "reason": reason, **fact})
        else:
            # No policy denial, missing billing row, arbitrary budget or claimed
            # native receipt may erase a retained potentially usable authority.
            if str(grant.get("client_id", "")).startswith("lmm-mcp-"):
                need(grant["client_id"] in clients, "live_oauth_dynamic_client_missing")
            unresolved.append({**fact, "client_class": "dynamic_mcp" if str(grant.get("client_id", "")).startswith("lmm-mcp-") else grant.get("client_id"),
                "live_access_count": len(current_access), "live_unused_refresh_count": len(current_refresh),
                "live_unused_code_count": len(current_codes),
                "reason": "exact_legacy_oauth_continuation_reader_required"})
    pending = []
    for row in authorizations.values():
        for field in ("created_at_ms", "expires_at_ms", "consumed_at_ms"):
            integer(row.get(field), "oauth_authorization_time_invalid")
        need(row["created_at_ms"] <= now and row["expires_at_ms"] >= row["created_at_ms"]
             and (row["consumed_at_ms"] == 0 or row["created_at_ms"] <= row["consumed_at_ms"] <= now),
             "oauth_authorization_time_inconsistent")
        if row["consumed_at_ms"] == 0 and row["expires_at_ms"] > now:
            pending.append(F.digest(row))
    need(type(resolutions) is list, "oauth_resolution_shape_invalid")
    expected = {r["source_grant_id"]: r for r in terminal}
    seen = set()
    for resolution in resolutions:
        need(type(resolution) is dict and resolution.get("kind") == "source_terminal_authority",
             "oauth_continuation_contract_not_implemented")
        key = resolution.get("source_grant_id")
        need(key not in seen and expected.get(key) == resolution, "oauth_terminal_evidence_not_actual")
        seen.add(key)
    return {"source_acquisition": acquisition, "complete_table_sha256": {t: F.digest(obs[t]) for t in TABLES},
        "terminal_facts": terminal, "explicit_terminal_evidence": deepcopy(resolutions),
        "unresolved": unresolved, "pending_authorization_sha256": pending,
        "target_continuation_supported": False,
        "source_policy_denials_not_treated_as_terminal": True}


def assemble(P, F, C, parent, provenance, context, obs, acquisition, resolutions=()):
    need(context["parent_sha256"] == P.sha256(parent)
         and context["parent_provenance_sha256"] == P.sha256(provenance), "verified_parent_changed")
    need(parent["source_projection_sha256"] == P.sha256(provenance), "parent_provenance_mismatch")
    ordinary, internal = [], []
    for row in parent["tokens"]:
        separate = row["oauth_managed"] or row["creation_source"] == "assistant_runtime" or row["key"].startswith("oauth_managed_")
        (internal if separate else ordinary).append(deepcopy(row))
    need(bool(ordinary), "ordinary_partition_empty")
    actual_internal = indexed(obs["managed_tokens"], "id", "internal_observation_ids_not_unique")
    need(set(actual_internal) == {r["id"] for r in internal}, "internal_source_coverage_changed")
    billing = indexed(obs["o_auth_billing_bindings"], "token_id", "billing_source_token_not_unique")
    for row in internal:
        actual = actual_internal[row["id"]]
        for field, value in row.items():
            if field == "key":
                same = hashlib.sha256(value.encode()).hexdigest() == actual.get("key_sha256")
            elif field == "deleted_at" and value and actual.get(field):
                same = ms(value) == ms(actual[field])
            else:
                same = value == actual.get(field)
            need(same, "internal_source_row_changed")
        if row["oauth_managed"]:
            b = billing.get(row["id"])
            need(b is not None and b["user_id"] == row["user_id"] and b["group"] == row["group"],
                 "internal_billing_binding_missing")
            expected_key = "oauth_managed_" + hashlib.sha256((b["grant_id"] + "\0" + b["group"]).encode()).hexdigest()
            need(row["key"] == expected_key, "internal_billing_hash_mismatch")
        else:
            need(row["creation_source"] == "assistant_runtime", "unknown_internal_wire_semantics")
    need(Counter(F.digest(r) for r in ordinary + internal) == Counter(F.digest(r) for r in parent["tokens"]),
         "partition_not_full_parent")
    row_ref = lambda r: {"source_token_id": r["id"], "source_user_id": r["user_id"], "source_row_sha256": F.digest(r)}
    audit = {"parent_source_sha256": context["parent_sha256"], "parent_provenance_sha256": context["parent_provenance_sha256"],
        "source_archive_sha256": parent["source_archive_sha256"], "source_export_sha256": parent["source_export_sha256"],
        "ordinary": [row_ref(r) for r in ordinary], "internal": [row_ref(r) for r in internal],
        "full_count": len(parent["tokens"]), "ordinary_count": len(ordinary), "internal_count": len(internal)}
    derived = {"format_version": 1, "scope": "derived-ordinary-api-key-view-provenance-v1",
        "parent_source_projection_sha256": parent["source_projection_sha256"],
        "partition_audit_sha256": P.sha256(audit), "parent_source_sha256": context["parent_sha256"]}
    view = deepcopy(parent)
    view["tokens"] = ordinary
    view["source_projection_sha256"] = P.sha256(derived)
    need(view["source_projection_sha256"] != parent["source_projection_sha256"], "derived_view_reuses_full_provenance")
    C.validate_wire_source(view, allow_revoked_unsupported=False)
    archive = {"scope": "internal-billing-source-rows-not-native-credentials",
        "parent_source_sha256": context["parent_sha256"], "rows": internal}
    oauth = oauth_closure(P, F, parent, context, obs, acquisition, list(resolutions))
    blocked = bool(oauth["unresolved"] or oauth["pending_authorization_sha256"])
    material = {"format_version": 1, "scope": SCOPE, "mode": "PREVIEW_ONLY",
        "source_context": context, "partition_audit": audit, "derived_provenance": derived,
        "ordinary_view_sha256": P.sha256(view), "internal_archive_sha256": P.sha256(archive),
        "oauth": oauth, "status": "BLOCKED_OAUTH_CONTINUATION_REQUIRED" if blocked else "PARTITION_VALID_PREVIEW_ONLY",
        "execution_authorized": False, "apply_supported": False, "source_review_generated": False,
        "credential_ids_allocated": 0, "frozen_helpers": PINS, "unchanged_writer_pins": FROZEN_WRITERS}
    need(not any(r["key"] in P.canonical(material) for r in parent["tokens"]), "secret_echo_in_material")
    return material, view, archive


def validate_saved(P, F, C, parent, provenance, context, obs, acquisition, material, view, archive, resolutions=()):
    expected = assemble(P, F, C, parent, provenance, context, obs, acquisition, resolutions)
    need((material, view, archive) == expected, "saved_partition_not_exact_rebuild")
    return material


def verified_source(args, P, R):
    # Same original archive/catalog/all-row verifier and projector as P's CLI;
    # retain OAuth members too so missing descendants cannot be hidden.
    F = R.F
    paths = {name: F.IDENTITY.private_backup_path(getattr(args, name)) for name in
             ("parent", "parent_provenance", "source_receipt", "export_proof", "runtime_observation")}
    values = {name: P.private_json_with_digest(path) for name, path in paths.items()}
    parent, provenance = values["parent"][0], values["parent_provenance"][0]
    hashes = {paths[name]: value[1] for name, value in values.items()}
    tables, proof, _ = R.load_routing_export(values["source_receipt"][0], paths["export_proof"], hashes,
        retained_tables=("users", "tokens", "options", "oauth_server_tokens"))
    need(proof == values["export_proof"][0], "export_proof_changed")
    _, _, _, _, schema = F.metadata(values["source_receipt"][0])
    need(all(F.digest(proof["source_column_schema"][t]) == F.digest(schema[t]) for t in ("users", "tokens", "options")),
         "parent_source_schema_changed")
    rebuilt, rebuilt_provenance = P.source_projection_from_export(tables, proof,
        snapshot_at=parent["snapshot_at"], runtime_observation=values["runtime_observation"][0],
        provenance={"source_receipt_sha256": values["source_receipt"][1],
            "source_export_proof_sha256": values["export_proof"][1],
            "legacy_runtime_observation_sha256": values["runtime_observation"][1]})
    need(parent == rebuilt and provenance == rebuilt_provenance, "parent_not_full_verified_source")
    R.verify_files(hashes)
    context = parent_context(P, F, parent, provenance, tables["oauth_server_tokens"],
        {str(p): digest for p, digest in hashes.items()}, origin="FULL_CANONICAL_EXPORT_REBUILT")
    return parent, provenance, context, hashes


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    for name in ("support-scripts", "parent", "parent-provenance", "source-receipt", "export-proof",
                 "runtime-observation", "oauth-observation", "oauth-capture", "oauth-sql", "oauth-collector", "output"):
        ap.add_argument("--" + name, type=Path, required=True)
    ap.add_argument("--resolutions", type=Path)
    a = ap.parse_args(argv)
    try:
        P, R, C = load_helpers(a.support_scripts)
        F, I = R.F, R.F.IDENTITY
        parent, provenance, context, hashes = verified_source(a, P, R)
        evidence = {}
        for name in ("oauth_observation", "oauth_capture", "oauth_sql", "oauth_collector"):
            path = I.private_backup_path(getattr(a, name))
            hashes[path] = file_hash(path)
            if name in ("oauth_observation", "oauth_capture"):
                evidence[name] = P.private_json_with_digest(path)[0]
        obs, capture = evidence["oauth_observation"], evidence["oauth_capture"]
        need(file_hash(a.oauth_observation) == capture.get("observation_sha256"), "oauth_capture_file_changed")
        acquisition = validate_capture(P, F, obs, capture, a.oauth_sql, a.oauth_collector)
        resolutions = []
        if a.resolutions:
            path = I.private_backup_path(a.resolutions)
            hashes[path] = file_hash(path)
            resolutions = json.loads(path.read_text(), object_pairs_hook=P.unique_object)
        material, view, archive = assemble(P, F, C, parent, provenance, context, obs, acquisition, resolutions)
        validate_saved(P, F, C, parent, provenance, context, obs, acquisition, material, view, archive, resolutions)
        R.verify_files(hashes)
        I.private_backup_path(a.output.parent, directory=True)
        a.output.mkdir(mode=0o700)
        for name, value in (("partition-material.private.json", material), ("ordinary-source-view.private.json", view),
                            ("internal-billing-source-rows.private.json", archive)):
            I.write_private(a.output, name, P.canonical(value) + "\n")
        summary = {"status": material["status"], "scope": SCOPE, "mode": "PREVIEW_ONLY",
            "full_count": len(parent["tokens"]), "ordinary_count": len(view["tokens"]), "internal_count": len(archive["rows"]),
            "oauth_unresolved_families": len(material["oauth"]["unresolved"]),
            "pending_authorizations": len(material["oauth"]["pending_authorization_sha256"]),
            "terminal_families": len(material["oauth"]["terminal_facts"]),
            "material_sha256": P.sha256(material), "ordinary_view_sha256": P.sha256(view),
            "internal_archive_sha256": P.sha256(archive), "source_queries": 0, "target_queries": 0,
            "production_writes": 0, "credential_ids_allocated": 0, "apply_supported": False}
        I.write_private(a.output, "result.public.json", P.canonical(summary) + "\n")
        R.verify_files(hashes)
        print(P.canonical(summary))
        return 3 if material["oauth"]["unresolved"] or material["oauth"]["pending_authorization_sha256"] else 0
    except Error as e:
        print(json.dumps({"status": "REJECTED", "reason": str(e), "apply_supported": False}), file=sys.stderr)
        return 1
    except Exception:
        print('{"status":"REJECTED","reason":"partition_inputs_invalid","apply_supported":false}', file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

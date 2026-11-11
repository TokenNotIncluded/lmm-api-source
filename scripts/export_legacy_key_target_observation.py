#!/usr/bin/env python3
"""Export native Key facts from a fresh offline migration target, without secrets.

Coverage is deliberately the complete API-key set in this staged target.
No legacy-format inventory or economic authorization is generated.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

import financial_import as F
import legacy_credential_import as C
import export_financial_target_inventory as W

H = F.IDENTITY
SCOPE = "legacy-key-native-target-observation-v1"


def require(condition, code):
    H.require(condition, code)


def positive(value):
    return type(value) is int and 0 < value < 2**63


def validate_bindings(receipt, alias, audit, receipt_raw, alias_raw):
    require(type(receipt) is dict and set(receipt) == {
        "scope", "source_archive_sha256", "records"}, "key_receipt_shape_invalid")
    require(receipt["scope"] == C.RECEIPT_SCOPE, "key_receipt_scope_invalid")
    C.alias_digest(receipt["source_archive_sha256"])
    rows = receipt["records"]
    require(type(rows) is list and 0 < len(rows) <= C.MAX_ALIAS_KEYS, "key_mapping_empty_or_invalid")
    fields = {"source_key_id", "target_credential_id", "target_user_id",
              "target_account_id", "source_secret_sha256"}
    seen_source, seen_target = set(), set()
    for row in rows:
        require(type(row) is dict and set(row) == fields, "key_mapping_shape_invalid")
        require(all(positive(row[k]) for k in fields - {"source_secret_sha256"}),
                "key_mapping_id_invalid")
        C.alias_digest(row["source_secret_sha256"])
        require(row["source_key_id"] not in seen_source
                and row["target_credential_id"] not in seen_target, "key_mapping_duplicate")
        seen_source.add(row["source_key_id"])
        seen_target.add(row["target_credential_id"])
    require(type(alias) is dict and alias.get("version") == 1
            and type(alias["version"]) is int
            and alias.get("scope") == "legacy-api-key-wire-aliases-v1"
            and alias.get("source_archive_sha256") == receipt["source_archive_sha256"]
            and alias.get("credential_receipt_sha256") == F.sha(receipt_raw),
            "key_alias_binding_invalid")
    keys = alias.get("keys")
    require(type(keys) is list and len(keys) == len(rows), "key_alias_coverage_invalid")
    projected = []
    for row in keys:
        require(type(row) is dict and set(row) in (
            fields | {"credential_user_version", "sk_alias_sha256"},
            fields | {"credential_user_version", "sk_alias_sha256", "revoked_rekey_required"}),
            "key_alias_shape_invalid")
        require(positive(row["credential_user_version"]), "key_alias_version_invalid")
        C.alias_digest(row["sk_alias_sha256"])
        if "revoked_rekey_required" in row:
            require(row["revoked_rekey_required"] is True, "key_alias_revocation_invalid")
        projected.append({k: row[k] for k in fields})
    require(sorted(projected, key=lambda r: r["source_key_id"]) ==
            sorted(rows, key=lambda r: r["source_key_id"]), "key_alias_mapping_mismatch")
    require(type(audit) is dict and audit.get("format_version") == 1
            and type(audit["format_version"]) is int
            and audit.get("scope") == "legacy-credential-committed-artifacts-v1"
            and audit.get("status") == "COMMITTED"
            and type(audit.get("migration_id")) is str
            and H.MIGRATION_ID.fullmatch(audit["migration_id"])
            and audit.get("source_archive_sha256") == receipt["source_archive_sha256"]
            and type(audit.get("credential_count")) is int and audit["credential_count"] == len(rows)
            and audit.get("credential_receipt_sha256") == F.sha(receipt_raw)
            and audit.get("alias_manifest_sha256") == F.sha(alias_raw)
            and audit.get("target") == alias.get("target"), "key_commit_audit_invalid")
    C.alias_digest(audit.get("material_sha256"))
    return sorted(rows, key=lambda r: r["source_key_id"])


def build_sql(receipt, migration_id, contract):
    # Only the already projected, secret-free mapping is embedded in SQL.
    return f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC';
SET LOCAL statement_timeout='30s';
SET LOCAL lock_timeout='5s';
WITH imported AS (
 SELECT * FROM core_migration.import_runs WHERE migration_id={F.literal(migration_id)}
), observed AS (
 SELECT c.id AS target_key_id,c.user_id,c.user_version,c.owner_account_id,
 c.personal_billing_preference,encode(c.digest,'hex') AS secret_sha256,
 c.expires_at,c.revoked_at,c.created_at,
 u.auth_version AS current_user_version,u.personal_account_id,u.active AS user_active,
 a.kind AS account_kind,a.active AS account_active,
 (SELECT coalesce(jsonb_agg(jsonb_build_object(
 'enabled',s.enabled,'one_time_reveal',s.one_time_reveal,
 'model_limits_enabled',s.model_limits_enabled,'model_limits',s.model_limits,
 'allow_ips',s.allow_ips,'group_name',s.group_name,'auto_groups',s.auto_groups,
 'cross_group_retry',s.cross_group_retry,'account_balance_read',s.account_balance_read
 )),'[]'::jsonb) FROM core_identity.api_key_settings s WHERE s.credential_id=c.id) AS settings,
 (SELECT coalesce(jsonb_agg(jsonb_build_object('position',r.position,
 'payer_account_id',r.payer_account_id) ORDER BY r.position),'[]'::jsonb)
 FROM core_identity.key_funding_rules r WHERE r.credential_id=c.id) AS funding_rules,
 (SELECT coalesce(jsonb_agg(jsonb_build_object('scope',b.scope,'period',b.period,
 'account_id',b.account_id,'user_id',b.user_id,'issuer_user_id',b.issuer_user_id,
 'timezone',b.timezone,'anchor',b.anchor,'seconds',b.seconds,
 'limit_credits',b.limit_credits,'display_limit_credits',b.display_limit_credits,
 'unlimited',b.unlimited)),'[]'::jsonb)
 FROM core_charging.budgets b WHERE b.key_id=c.id) AS budgets,
 (SELECT coalesce(jsonb_agg(jsonb_build_object('scope',o.scope,'period',o.period,
 'account_id',o.account_id,'user_id',o.user_id,'timezone',o.timezone,
 'anchor',o.anchor,'seconds',o.seconds,'start_at',o.start_at,'end_at',o.end_at,
 'consumed_credits',o.consumed_credits,'import_reference',o.import_reference)),'[]'::jsonb)
 FROM core_charging.budget_opening_consumption o WHERE o.key_id=c.id) AS opening
 FROM core_identity.credentials c
 LEFT JOIN core_identity.users u ON u.id=c.user_id
 LEFT JOIN core_identity.accounts a ON a.id=c.owner_account_id WHERE c.kind='api_key'
)
SELECT jsonb_build_object(
 'instance',({F.instance_sql().removeprefix('SELECT ').rstrip(';')}),
 'read_only',current_setting('transaction_read_only'),
 'isolation',current_setting('transaction_isolation'),'snapshot',pg_current_snapshot()::text,
 'observed_at',transaction_timestamp(),
 'identity_contract',(SELECT jsonb_build_object('version',version,
 'fingerprint_sha256',encode(fingerprint,'hex')) FROM core_meta.schema_contract),
 'components',(SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract),
 'catalog',({H.catalog_hash_sql(contract)}),
 'receipt_count',(SELECT count(*) FROM imported),
 'receipt_matches',(SELECT coalesce(bool_and(
 source_archive_sha256={F.literal(receipt['source_archive_sha256'])}
 AND manifest->>'scope'={F.literal(C.SCOPE)}
 AND manifest->'material'->>'migration_id'={F.literal(migration_id)}
 AND manifest->'material'->>'source_archive_sha256'={F.literal(receipt['source_archive_sha256'])}
 AND manifest->'material'->'credential_receipt'={F.literal(receipt)}::jsonb
 AND jsonb_array_length(manifest->'material'->'records')={len(receipt['records'])}
 ),false) FROM imported),
 'records',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY target_key_id),'[]'::jsonb) FROM observed o));
ROLLBACK;
"""


def classify(observation, mapping, migration_id=None):
    require(observation.get("receipt_count") == 1
            and type(observation["receipt_count"]) is int
            and observation.get("receipt_matches") is True, "key_committed_receipt_mismatch")
    rows = observation.get("records")
    require(type(rows) is list and len(rows) == len(mapping), "key_target_coverage_incomplete")
    expected = {r["target_credential_id"]: r for r in mapping}
    seen, result = set(), []
    for row in rows:
        require(type(row) is dict and set(row) == {
            "target_key_id", "user_id", "user_version", "owner_account_id",
            "personal_billing_preference", "secret_sha256", "expires_at", "revoked_at",
            "created_at", "current_user_version", "personal_account_id", "user_active",
            "account_kind", "account_active", "settings", "funding_rules", "budgets", "opening"},
            "key_target_projection_invalid")
        for name, fields in {
            "settings": {"enabled", "one_time_reveal", "model_limits_enabled", "model_limits",
                         "allow_ips", "group_name", "auto_groups", "cross_group_retry",
                         "account_balance_read"},
            "funding_rules": {"position", "payer_account_id"},
            "budgets": {"scope", "period", "account_id", "user_id", "issuer_user_id",
                        "timezone", "anchor", "seconds", "limit_credits",
                        "display_limit_credits", "unlimited"},
            "opening": {"scope", "period", "account_id", "user_id", "timezone", "anchor",
                        "seconds", "start_at", "end_at", "consumed_credits", "import_reference"},
        }.items():
            require(type(row[name]) is list
                    and all(type(item) is dict and set(item) == fields for item in row[name]),
                    "key_target_nested_projection_invalid")
        cid = row.get("target_key_id")
        require(positive(cid) and cid in expected and cid not in seen, "key_target_id_unbound")
        seen.add(cid)
        binding = expected[cid]
        require(type(row.get("user_id")) is int and row["user_id"] == binding["target_user_id"]
                and type(row.get("owner_account_id")) is int
                and row["owner_account_id"] == binding["target_account_id"]
                and row.get("personal_account_id") == binding["target_account_id"]
                and row.get("account_kind") == "personal"
                and row.get("secret_sha256") == binding["source_secret_sha256"],
                "key_target_owner_or_digest_mismatch")
        require(len(row.get("settings", [])) == 1 and len(row.get("budgets", [])) == 1
                and len(row.get("opening", [])) == 1
                and row.get("funding_rules") == [{"position": 0,
                    "payer_account_id": binding["target_account_id"]}], "key_target_relations_incomplete")
        budget, opening = row["budgets"][0], row["opening"][0]
        require(all(budget.get(k) == v and opening.get(k) == v for k, v in {
            "scope": "key", "period": "lifetime", "account_id": None, "user_id": None,
            "timezone": "UTC", "anchor": 0, "seconds": 0}.items())
            and budget.get("issuer_user_id") == binding["target_user_id"]
            and opening.get("start_at") == C.QUOTA.LIFETIME_START
            and opening.get("end_at") == C.QUOTA.LIFETIME_END,
            "key_target_budget_scope_invalid")
        unlimited = budget.get("unlimited")
        limit, display, used = (budget.get("limit_credits"),
                               budget.get("display_limit_credits"), opening.get("consumed_credits"))
        require(type(unlimited) is bool and type(limit) is int and limit >= 0
                and type(used) is int and used >= 0
                and ((unlimited and type(display) is int) or (not unlimited and display is None)),
                "key_target_budget_values_invalid")
        if migration_id is not None:
            require(opening.get("import_reference") == "key-quota:" + migration_id + ":" + str(cid),
                    "key_target_opening_reference_mismatch")
        # These are staged opening facts. Runtime spending is outside this observation.
        result.append({**row, "source_key_id": binding["source_key_id"],
            "opening_used_credits": used,
            "opening_remaining_credits": (display if unlimited else limit) - used})
    return result


def query(connection, sql):
    # Never persist PostgreSQL diagnostics, which may contain unprojected values.
    environment = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
    environment.update(H.socket_connection(connection))
    environment.update(PGCONNECT_TIMEOUT="5", PGAPPNAME="lmm-key-target-observation",
                       PGOPTIONS=H.psql_options(connection, readonly=True))
    try:
        process = subprocess.run(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
            input=sql, text=True, capture_output=True, env=environment, timeout=45, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise H.MappingError("key_observation_connection_failed") from None
    require(process.returncode == 0, "key_observation_query_failed")
    return W.parse_json(process.stdout)


def run(args):
    inputs = {name: W.freeze_input(getattr(args, name)) for name in (
        "target_receipt", "credential_receipt", "alias_manifest", "commit_audit")}
    documents = {name: W.parse_json(item["raw"]) for name, item in inputs.items()}
    target, receipt, alias, audit = (documents[k] for k in (
        "target_receipt", "credential_receipt", "alias_manifest", "commit_audit"))
    mapping = validate_bindings(receipt, alias, audit, inputs["credential_receipt"]["raw"],
                                inputs["alias_manifest"]["raw"])
    require(target.get("application_or_worker_started") is False, "key_target_not_offline")
    connection = target["connection"]
    H.connection_roles(connection)
    expected = target["instance_identity"]
    require(alias["target"] == C.alias_target(expected), "key_alias_instance_mismatch")
    require(type(expected.get("system_identifier")) is str and expected["system_identifier"],
            "key_system_identity_missing")
    contract = H.current_contract()
    require(all(target.get("contract", {}).get(k) == contract[k] for k in F.CONTRACT_FIELDS),
            "key_target_contract_mismatch")
    inputs["pgpass"] = W.freeze_input(Path(connection["pgpass_file"]))
    code = {str(p): {"path": p, "sha256": H.sha256_file(p)} for p in (
        Path(__file__).resolve(), *(Path(__file__).with_name(n) for n in C.source_hashes()
                                   if "/" not in n), Path(W.__file__).resolve())}
    sql = build_sql(receipt, audit["migration_id"], contract)
    W.verify_frozen_inputs(inputs)
    W.verify_frozen_inputs(code, private=False)
    observation = query(connection, sql)
    H.verify_instance(observation["instance"], connection, expected, source=False)
    require(observation["instance"].get("system_identifier") == expected["system_identifier"]
            and C.alias_target(observation["instance"]) == alias["target"], "key_actual_instance_mismatch")
    require(observation.get("read_only") == "on" and observation.get("isolation") == "repeatable read"
            and observation.get("snapshot"), "key_snapshot_invalid")
    require(observation.get("components") == contract["component_fingerprints"]
            and observation.get("identity_contract") == {
                "version": contract["identity_version"],
                "fingerprint_sha256": contract["identity_fingerprint_sha256"]}
            and observation.get("catalog") == target["contract"].get("catalog_fingerprint_sha256"),
            "key_actual_contract_mismatch")
    records = classify(observation, mapping, audit["migration_id"])
    W.verify_frozen_inputs(inputs)
    W.verify_frozen_inputs(code, private=False)
    result = {"format_version": 1, "scope": SCOPE, "status": "COMPLETE_SCOPED_OBSERVATION",
        "migration_id": audit["migration_id"], "instance": observation["instance"],
        "snapshot": observation["snapshot"], "observed_at": observation["observed_at"],
        "coverage": "all-target-api-keys", "committed_receipt_mapping_verified": True,
        "audit_material_digest_verified": False,
        "economic_approval_created": False, "legacy_inventory_created": False,
        "runtime_consumption_observed": False, "records": records,
        "input_sha256": {k: v["sha256"] for k, v in inputs.items()},
        "tool_sha256": {k: v["sha256"] for k, v in code.items()},
        "query_sha256": F.sha(sql.encode())}
    H.private_backup_path(args.output.parent, directory=True)
    args.output.mkdir(mode=0o700)
    H.write_private(args.output, "target-key-observation.private.json", F.canonical(result) + "\n")
    return {"status": result["status"], "record_count": len(records)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("target-receipt", "credential-receipt", "alias-manifest", "commit-audit", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    try:
        print(F.canonical(run(args)))
        return 0
    except (OSError, ValueError, TypeError, KeyError, H.MappingError):
        print('{"status":"REFUSED","code":"key_target_observation_refused"}', file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

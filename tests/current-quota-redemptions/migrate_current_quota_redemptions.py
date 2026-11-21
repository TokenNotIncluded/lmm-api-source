#!/usr/bin/env python3
"""Append still-redeemable legacy quota codes to an already-used Core78.

Only new campaigns, issues, SHA256 code identities and an immutable import
receipt are inserted. Importing never posts money, invents a claim, changes an
old campaign, or consumes a sequence. The existing Core redemption transaction
remains the only way to claim a code and post its exactly-once gift journal.

Production inputs use the original complete canonical export verifier and the
frozen current identity/wallet contracts. The source must be genuinely fenced;
the current target, backup/restore and reservation evidence are revalidated.
The economic review binds this exact future liability and SQL, not a claim
that reset vouchers, gifts, checkins or all other continuing rights are done.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import re
import uuid

import current_key_production as K

U, W, L, R = K.U, K.W, K.L, K.R
C, P, F, I = K.C, K.P, K.F, K.I
need, Error, digest, canonical, literal = U.need, U.Error, U.digest, U.canonical, U.literal
SCOPE = "current-quota-redemption-append-v1"
DECISION = "append-exact-legacy-quota-code-liability-without-immediate-credit"
MAX_UNITS, MAX_TIME = 1_000_000_000_000, 253402300799
CHANGED = ("core_promotions.campaigns", "core_promotions.issues", "core_promotions.codes")
DEPENDENCIES = {
    "current_key_production.py": "b1e157a0db8b22ccc7a41263c54c0c6b6f0c4c2ba49fe825b91fc0e90a0ba8ba",
    "current_key_lineage.py": "0835d57cb70a58ed8bdf368d5c25d588aca0139dfdb44389e2550981953c9da7",
}


def frozen_dependencies():
    K.frozen_dependencies()
    for module in (K, L):
        path = Path(module.__file__)
        need(hashlib.sha256(path.read_bytes()).hexdigest() == DEPENDENCIES[path.name],
             "quota_append_frozen_dependency_changed")


def integer(value, minimum, maximum, code):
    need(type(value) is int and minimum <= value <= maximum, code)
    return value


def source_facts(rows, mapping, observed_at):
    """Read the old code contract, retaining only hashes in the output.

Old Go rejects ExpiredTime < now; therefore that second remains redeemable.
Core uses now < ends_at, requiring an explicit +1 second translation. Blank
reward_type is the original quota default. Reset rights stay a separate type.
"""
    integer(observed_at, 0, MAX_TIME - 1, "quota_append_observation_time_invalid")
    need(type(rows) is list and type(mapping) is list, "quota_append_source_shape_invalid")
    owners = {}
    target_ids, account_ids = set(), set()
    for m in mapping:
        need(set(m) == set(L.MAP_FIELDS), "quota_append_owner_map_invalid")
        for k in L.MAP_FIELDS:
            integer(m[k], 1, W.MAX, "quota_append_owner_map_invalid")
        need(m["source_user_id"] not in owners and m["target_user_id"] not in target_ids
             and m["target_personal_account_id"] not in account_ids, "quota_append_owner_map_duplicate")
        owners[m["source_user_id"]] = m
        target_ids.add(m["target_user_id"])
        account_ids.add(m["target_personal_account_id"])
    ids, keys, selected, omitted = set(), set(), [], Counter()
    for row in sorted(rows, key=lambda r: r["id"]):
        sid = integer(row["id"], 1, W.MAX, "quota_append_source_id_invalid")
        need(sid not in ids, "quota_append_source_id_duplicate")
        ids.add(sid)
        need(type(row["key"]) is str and re.fullmatch("[0-9a-f]{32}", row["key"]),
             "quota_append_original_key_format_unsupported")
        h = hashlib.sha256(row["key"].encode()).hexdigest()
        need(h not in keys, "quota_append_source_code_duplicate")
        keys.add(h)
        integer(row["status"], 1, 3, "quota_append_source_status_unknown")
        need(type(row["reward_type"]) is str, "quota_append_reward_type_invalid")
        reward = row["reward_type"].strip().lower() or "quota"
        need(reward in {"quota", "reset_voucher"}, "quota_append_reward_type_unknown")
        expires = integer(row["expired_time"], 0, W.MAX, "quota_append_source_expiry_invalid")
        if row["status"] != 1 or row["deleted_at"] is not None or (expires and expires < observed_at):
            omitted["nonredeemable_source_code"] += 1
            continue
        if reward != "quota":
            omitted["separate_reset_voucher_right"] += 1
            continue
        quota = integer(row["quota"], 1, MAX_UNITS, "quota_append_award_outside_core_contract")
        created = integer(row["created_time"], 0, observed_at, "quota_append_future_creation_unsupported")
        need(expires == 0 or expires < MAX_TIME, "quota_append_expiry_not_representable")
        creator = owners.get(row["user_id"])
        need(creator is not None, "quota_append_source_creator_mapping_missing")
        need(type(row["name"]) is str, "quota_append_source_name_invalid")
        selected.append({"source_redemption_id": sid, "source_creator_id": row["user_id"],
            "target_creator_id": creator["target_user_id"], "source_row_sha256": F.PLANNER.digest(row),
            "code_sha256": h, "source_name": row["name"], "award_units": quota,
            "source_created_at": created, "source_expires_at": expires,
            "native_ends_at": expires + 1 if expires else MAX_TIME})
    return {"source_rows": len(rows), "source_rows_sha256": F.PLANNER.digest(rows),
        "observed_at": observed_at, "records": selected, "omitted": dict(sorted(omitted.items())),
        "codes": len(selected), "future_liability_units": sum(r["award_units"] for r in selected),
        "unit": "credit_500k_usd", "source_expiry_comparison": "expired_time < now",
        "native_expiry_comparison": "now >= ends_at", "immediate_credit_units": 0,
        "reset_vouchers_imported": 0, "other_rights_completion_claimed": False}


def plan_records(facts, namespace):
    nk = U.namespace_key(namespace)
    groups = defaultdict(list)
    for r in facts["records"]:
        groups[(r["source_name"], r["award_units"], r["native_ends_at"])].append(r)
    campaigns, issues, codes, records = [], [], [], []
    for (name, units, ends), group in sorted(groups.items(), key=lambda x: min(r["source_redemption_id"] for r in x[1])):
        group.sort(key=lambda r: r["source_redemption_id"])
        # Every user can redeem every code. Splitting avoids imposing Core's
        # per-campaign 10,000-user-claim or 10^12-unit budget limits on the old
        # code contract. No limit is clamped and no code loses its reservation.
        chunk_size = min(10000, MAX_UNITS // units)
        for first in range(0, len(group), chunk_size):
            chunk = group[first:first + chunk_size]
            cid = digest([SCOPE, nk, "campaign", [r["source_redemption_id"] for r in chunk], name, units, ends])
            count = len(chunk)
            title = " ".join(name.split())[:150] or "redemption"
            campaigns.append({"id": cid, "revision": 1, "kind": "redemption", "title": "Legacy " + title,
                "award_units": units, "budget_units": count * units, "spent_units": 0,
                "reserved_units": count * units, "max_claims": count, "claims": 0,
                "reserved_claims": count, "max_claims_per_user": count,
                "starts_at": min(r["source_created_at"] for r in chunk), "ends_at": ends, "enabled": True})
            by_creator = defaultdict(list)
            for r in chunk:
                by_creator[r["target_creator_id"]].append(r)
            for creator, items in sorted(by_creator.items()):
                for start in range(0, len(items), 100):
                    section = items[start:start + 100]
                    iid = digest([SCOPE, nk, "issue", cid, creator, [r["source_redemption_id"] for r in section]])
                    issues.append({"id": iid, "campaign_id": cid, "actor_user_id": creator,
                        "quantity": len(section), "created_at": min(r["source_created_at"] for r in section)})
                    for pos, r in enumerate(section):
                        code = {"digest": r["code_sha256"], "issue_id": iid, "position": pos,
                                "campaign_id": cid, "revoked": False}
                        codes.append(code)
                        records.append({**r, "campaign_id": cid, "issue_id": iid, "position": pos})
    return {"campaigns": sorted(campaigns, key=lambda x: x["id"]),
        "issues": sorted(issues, key=lambda x: x["id"]), "codes": sorted(codes, key=lambda x: x["digest"]),
        "records": sorted(records, key=lambda x: x["source_redemption_id"])}


def native_sql(mapping, exclude_migration_id=None):
    where = " WHERE r.migration_id<>" + literal(exclude_migration_id) if exclude_migration_id else ""
    fields = ["'instance',(" + F.instance_sql() + ")",
        "'identity_authority',(" + L.identity_authority_sql(mapping) + ")",
        "'ledger',(" + W.ledger_facts_sql() + ")",
        "'imports',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY r.migration_id),'[]'::jsonb) FROM core_migration.import_runs r" + where + ")",
        "'prepared_transactions',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database())"]
    for name in (*CHANGED, "core_promotions.claims"):
        row = "to_jsonb(r)"
        if name.endswith(".codes"):
            row = "(to_jsonb(r)-'digest')||jsonb_build_object('digest',encode(r.digest,'hex'))"
        fields.append(literal(name) + ",(SELECT coalesce(jsonb_agg(" + row + " ORDER BY to_jsonb(r)::text),'[]'::jsonb) FROM " + F.relation(name) + " r)")
    return "SELECT jsonb_build_object(" + ",".join(fields) + ")"


def native_observation_sql(mapping):
    return "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC'; SET LOCAL row_security=off;\n" + native_sql(mapping) + ";\nCOMMIT;\n"


def check_conflicts(plan, namespace, native):
    wanted = {r["source_redemption_id"] for r in plan["records"]}
    for run in native["imports"]:
        manifest = run.get("manifest", {})
        if manifest.get("scope") == SCOPE and manifest.get("source_namespace_sha256") == U.namespace_key(namespace):
            need(not wanted.intersection(r["source_redemption_id"] for r in manifest["mapping"]),
                 "quota_append_source_identity_already_imported")
    for key, field in (("campaigns", "id"), ("issues", "id"), ("codes", "digest")):
        need(not {r[field] for r in plan[key]}.intersection(r[field] for r in native["core_promotions." + key]),
             "quota_append_native_" + key + "_collision")


def prepare_mechanics(facts, mapping, snapshot, contract, namespace, archive, migration_id, runtime_roles, native):
    U.validate_snapshot(snapshot, contract)
    F.checked_hash(archive)
    need(type(migration_id) is str and I.MIGRATION_ID.fullmatch(migration_id), "quota_append_migration_id_invalid")
    need(type(runtime_roles) is list and runtime_roles == sorted(set(runtime_roles)) and runtime_roles
         and all(type(r) is str and U.IDENTIFIER.fullmatch(r) for r in runtime_roles)
         and snapshot["instance"]["role"] not in runtime_roles
         and snapshot["instance"]["session_user"] not in runtime_roles, "quota_append_runtime_roles_invalid")
    need(set(CHANGED) <= set(snapshot["state"]["inventory"]["tables"]), "quota_append_inventory_incomplete")
    need(native["instance"] == snapshot["instance"] and native["prepared_transactions"] == 0,
         "quota_append_native_observation_instance_changed")
    need(facts["codes"] > 0, "quota_append_no_redeemable_quota_codes")
    plan = plan_records(facts, namespace)
    check_conflicts(plan, namespace, native)
    material = {"format_version": 1, "scope": SCOPE, "migration_id": migration_id,
        "source_archive_sha256": archive, "source_namespace": namespace,
        "source_namespace_sha256": U.namespace_key(namespace), "identity_map": mapping,
        "identity_map_sha256": digest(mapping), "source_facts": facts, "plan": plan,
        "mapping_sha256": digest(plan["records"]), "snapshot": snapshot, "contract": contract,
        "before_state": snapshot["state"], "native_observation": native,
        "runtime_roles": runtime_roles, "production_authorized": False}
    validate_material(material)
    return material


def validate_material(m):
    need(type(m) is dict and m.get("scope") == SCOPE and m.get("format_version") == 1
         and m.get("production_authorized") is False, "quota_append_material_invalid")
    U.validate_snapshot(m["snapshot"], m["contract"])
    need(I.MIGRATION_ID.fullmatch(m["migration_id"]) and m["before_state"] == m["snapshot"]["state"]
         and m["source_namespace_sha256"] == U.namespace_key(m["source_namespace"])
         and m["identity_map_sha256"] == digest(m["identity_map"]), "quota_append_material_binding_invalid")
    facts = m["source_facts"]
    need(facts["codes"] == len(facts["records"]) > 0
         and facts["future_liability_units"] == sum(r["award_units"] for r in facts["records"])
         and facts["immediate_credit_units"] == 0 and facts["reset_vouchers_imported"] == 0
         and facts["other_rights_completion_claimed"] is False, "quota_append_material_liability_invalid")
    owners = {r["source_user_id"]: r for r in m["identity_map"]}
    ids, keys = set(), set()
    for r in facts["records"]:
        integer(r["source_redemption_id"], 1, W.MAX, "quota_append_source_id_invalid")
        integer(r["award_units"], 1, MAX_UNITS, "quota_append_award_outside_core_contract")
        integer(r["source_created_at"], 0, facts["observed_at"], "quota_append_creation_invalid")
        integer(r["source_expires_at"], 0, MAX_TIME - 1, "quota_append_expiry_not_representable")
        need(not r["source_expires_at"] or r["source_expires_at"] >= facts["observed_at"], "quota_append_source_expired")
        need(r["native_ends_at"] == (r["source_expires_at"] + 1 if r["source_expires_at"] else MAX_TIME),
             "quota_append_expiry_translation_invalid")
        F.checked_hash(r["source_row_sha256"])
        F.checked_hash(r["code_sha256"])
        need(r["source_redemption_id"] not in ids and r["code_sha256"] not in keys,
             "quota_append_source_duplicate")
        ids.add(r["source_redemption_id"])
        keys.add(r["code_sha256"])
        need(r["source_creator_id"] in owners
             and owners[r["source_creator_id"]]["target_user_id"] == r["target_creator_id"],
             "quota_append_creator_mapping_changed")
    need(m["plan"] == plan_records(facts, m["source_namespace"])
         and m["mapping_sha256"] == digest(m["plan"]["records"]), "quota_append_plan_not_rebuilt")


def state_sql(m, *, preserve=False):
    inv = m["before_state"]["inventory"]
    sql = U.state_sql(inv, m["migration_id"])
    if not preserve:
        return sql
    # Exact row exclusions in the same all-table hash query. Sequences are
    # still compared without any exclusion: this writer does not allocate IDs.
    for key, field in (("campaigns", "id"), ("issues", "id"), ("codes", "digest")):
        table = "core_promotions." + key
        values = ",".join(literal(r[field]) for r in m["plan"][key])
        predicate = ("encode(t.digest,'hex')" if key == "codes" else "t.id") + " NOT IN (" + values + ")"
        fragment = "FROM " + F.relation(table) + " t"
        need(sql.count(fragment) == 1, "quota_append_preservation_query_shape_changed")
        sql = sql.replace(fragment, fragment + " WHERE (" + predicate + ")", 1)
    return sql


def readback_sql(m, *, current=False):
    statements = []
    for key in ("campaigns", "issues", "codes"):
        for row in m["plan"][key]:
            relation = F.relation("core_promotions." + key)
            if key == "codes":
                actual = "(to_jsonb(c)-'digest')||jsonb_build_object('digest',encode(c.digest,'hex'))"
                where = "c.digest=decode(" + literal(row["digest"]) + ",'hex')"
            else:
                actual, where = "to_jsonb(c)", "c.id=" + literal(row["id"])
            expected = row
            if not current and key == "codes":
                actual = "(" + actual + ")-'revoked'"
                expected = {k: v for k, v in row.items() if k != "revoked"}
            elif not current and key == "campaigns":
                actual = "jsonb_build_object('id',c.id,'kind',c.kind,'award_units',c.award_units)"
                expected = {k: row[k] for k in ("id", "kind", "award_units")}
            statements.append(" IF (SELECT " + actual + " FROM " + relation + " c WHERE " + where + ") IS DISTINCT FROM " +
                literal(expected) + "::jsonb THEN RAISE EXCEPTION 'quota_append_readback_mismatch'; END IF;")
    return "\n".join(statements)


def insert_sql(m):
    out = []
    for key in ("campaigns", "issues", "codes"):
        for row in m["plan"][key]:
            fields = list(row)
            values = ["decode(" + literal(row[f]) + ",'hex')" if key == "codes" and f == "digest" else literal(row[f]) for f in fields]
            out.append("INSERT INTO " + F.relation("core_promotions." + key) + "(" + ",".join(fields) + ") VALUES(" + ",".join(values) + ");")
    return "\n".join(out)


def transaction_sql(m, fixture_guard="", production_guard=""):
    validate_material(m)
    state, preserved = state_sql(m), state_sql(m, preserve=True)
    locks = ",".join(F.relation(t) for t in m["before_state"]["inventory"]["tables"])
    wanted = ",".join(str(r["source_redemption_id"]) for r in m["plan"]["records"])
    body = f"""
DECLARE m jsonb := {literal(m)}::jsonb; v_run core_migration.import_runs%ROWTYPE;
 before_state jsonb; post_state jsonb;
BEGIN
 {fixture_guard}
 {U._guard_sql(m)}
 IF EXISTS(SELECT 1 FROM pg_prepared_xacts WHERE database=current_database()) THEN
  RAISE EXCEPTION 'quota_append_prepared_transactions_present'; END IF;
 LOCK TABLE {locks} IN ACCESS EXCLUSIVE MODE;
 {U._guard_sql(m)}
 SELECT * INTO v_run FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF v_run.source_archive_sha256<>m->>'source_archive_sha256'
   OR v_run.mapping_sha256<>m->>'mapping_sha256'
   OR v_run.manifest->>'scope' IS DISTINCT FROM {literal(SCOPE)}
   OR v_run.manifest->'material' IS DISTINCT FROM m
   OR v_run.manifest->'mapping' IS DISTINCT FROM m->'plan'->'records'
   OR v_run.manifest->>'source_namespace_sha256' IS DISTINCT FROM m->>'source_namespace_sha256'
  THEN RAISE EXCEPTION 'quota_append_replay_conflict'; END IF;
  {readback_sql(m)}
 ELSE
  SELECT ({state}) INTO before_state;
  IF before_state IS DISTINCT FROM m->'before_state' THEN
   RAISE EXCEPTION 'quota_append_target_state_changed'; END IF;
  IF ({native_sql(m['identity_map'])}) IS DISTINCT FROM m->'native_observation' THEN
   RAISE EXCEPTION 'quota_append_native_evidence_changed'; END IF;
  IF EXISTS(SELECT 1 FROM core_migration.import_runs r,
   LATERAL jsonb_array_elements(CASE WHEN r.manifest->>'scope'={literal(SCOPE)}
    THEN r.manifest->'mapping' ELSE '[]'::jsonb END) p
   WHERE r.manifest->>'scope'={literal(SCOPE)}
   AND r.manifest->>'source_namespace_sha256'=m->>'source_namespace_sha256'
   AND (p->>'source_redemption_id')::bigint IN ({wanted})) THEN
   RAISE EXCEPTION 'quota_append_source_identity_already_imported'; END IF;
  IF floor(extract(epoch FROM clock_timestamp()))::bigint>={min(r['ends_at'] for r in m['plan']['campaigns'])} THEN
   RAISE EXCEPTION 'quota_append_code_expired_since_preview'; END IF;
  {production_guard}
  {insert_sql(m)}
  SET CONSTRAINTS ALL IMMEDIATE;
  {readback_sql(m, current=True)}
  IF ({preserved}) IS DISTINCT FROM before_state THEN
   RAISE EXCEPTION 'quota_append_preexisting_state_not_preserved'; END IF;
  SELECT ({state}) INTO post_state;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->>'source_archive_sha256',m->>'mapping_sha256',
   jsonb_build_object('scope',{literal(SCOPE)},'source_namespace_sha256',m->>'source_namespace_sha256',
    'mapping',m->'plan'->'records','material',m,'post_state',post_state));
 END IF;
 {U._guard_sql(m)}
END
"""
    tag = "$lmm_quota_" + hashlib.sha256(body.encode()).hexdigest()[:24] + "$"
    need(tag not in body, "quota_append_sql_delimiter_collision")
    return f"""SET standard_conforming_strings=on; SET timezone='UTC';
SET lock_timeout='5s'; SET statement_timeout='120s';
DO $fence$ BEGIN IF NOT pg_try_advisory_lock({U.FENCE}) THEN
 RAISE EXCEPTION 'quota_append_import_busy'; END IF; END $fence$;
BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on; SET LOCAL row_security=off; SET LOCAL search_path=pg_catalog,public;
DO {tag}{body}{tag};
COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{literal(m['migration_id'])},
 'material_sha256',{literal(digest(m))});
"""


def synthetic_sql(m, fixture_nonce):
    return transaction_sql(m, U._test_fixture_guard(m, fixture_nonce))


def resolve_sql(m, application):
    validate_material(m)
    need(type(application) is str and re.fullmatch("lmm-current-quota-[0-9a-f]{32}", application),
         "quota_append_original_attempt_invalid")
    body = f"""
DECLARE v_run core_migration.import_runs%ROWTYPE;
BEGIN
 IF ({F.instance_sql()}) IS DISTINCT FROM {literal(m['snapshot']['instance'])}::jsonb THEN
  RAISE EXCEPTION 'quota_append_resolve_instance_changed'; END IF;
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())
  AND application_name={literal(application)} AND pid<>pg_backend_pid()) THEN
  RAISE EXCEPTION 'quota_append_original_attempt_still_active'; END IF;
 SELECT * INTO v_run FROM core_migration.import_runs WHERE migration_id={literal(m['migration_id'])};
 IF FOUND THEN
  IF v_run.manifest->'material' IS DISTINCT FROM {literal(m)}::jsonb
   OR v_run.manifest->>'scope' IS DISTINCT FROM {literal(SCOPE)}
   OR v_run.source_archive_sha256<>{literal(m['source_archive_sha256'])}
   OR v_run.mapping_sha256<>{literal(m['mapping_sha256'])}
   OR v_run.manifest->'mapping' IS DISTINCT FROM {literal(m['plan']['records'])}::jsonb
   OR v_run.manifest->>'source_namespace_sha256' IS DISTINCT FROM {literal(m['source_namespace_sha256'])}
  THEN RAISE EXCEPTION 'quota_append_resolve_receipt_conflict'; END IF;
  {readback_sql(m)}
 END IF;
END
"""
    tag = "$lmm_quota_resolve_" + hashlib.sha256(body.encode()).hexdigest()[:24] + "$"
    return "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC'; SET LOCAL row_security=off;\n" + \
        "DO " + tag + body + tag + ";\nSELECT jsonb_build_object('status',CASE WHEN EXISTS(SELECT 1 FROM core_migration.import_runs WHERE migration_id=" + \
        literal(m["migration_id"]) + ") THEN 'COMMITTED' ELSE 'UNKNOWN' END,'migration_id'," + literal(m["migration_id"]) + \
        ",'material_sha256'," + literal(digest(m)) + ",'automatic_retry_allowed',false);\nCOMMIT;\n"


def inspect(evidence):
    frozen_dependencies()
    need(evidence.get("format_version") == 1 and evidence.get("scope") == SCOPE,
         "quota_append_evidence_scope_invalid")
    tables, proof, source, projection, unit = K.read_source(evidence)
    target, snapshot = (K.read_ref(evidence[k]) for k in ("target_projection", "target_snapshot"))
    contract = U.core78_contract(K.read_ref(evidence["core78_manifest"], parse=False))
    U.validate_snapshot(snapshot, contract)
    U.validate_activation_bindings(target, snapshot)
    P.target_mapping(target, source["source_archive_sha256"])
    mapping = target["source_target_id_map"]
    need({r["source_user_id"] for r in mapping} == {r["id"] for r in tables["users"]}
         and snapshot["native_users"] == target["users"], "quota_append_complete_current_owner_map_required")
    observed = K.read_ref(evidence["native_observation"])
    need(type(evidence.get("native_observation_actual_exit")) is int and evidence["native_observation_actual_exit"] == 0
         and K.read_ref(evidence["native_observation_sql"], parse=False) == native_observation_sql(mapping).encode()
         and observed["instance"] == snapshot["instance"]
         and type(observed["prepared_transactions"]) is int and observed["prepared_transactions"] == 0,
         "quota_append_actual_native_observation_invalid")
    K.read_ref(evidence["native_observation_stderr"], parse=False)
    identity = [r for r in observed["imports"] if r["migration_id"] == evidence["identity_migration_id"]]
    need(len(identity) == 1, "quota_append_actual_current_identity_receipt_missing")
    bindings = L.validate(tables["users"], source, target, evidence["source_namespace"], identity[0], observed["identity_authority"])
    im = identity[0]["manifest"]["material"]
    need(im["source_canonical_export_proof_sha256"] == evidence["source"]["proof"]["sha256"]
         and im["source_canonical_receipt_sha256"] == evidence["source"]["receipt"]["sha256"],
         "quota_append_identity_source_files_changed")
    wallet = K.read_ref(evidence["wallet_material"])
    wallet_facts = K.validate_wallet(tables, source, mapping, evidence["source_namespace"], wallet, observed)
    source_end = K.validate_full_source_fence(evidence, tables, proof, wallet)
    reservations = K.validate_reservations(evidence, tables, proof, wallet, source_end)
    at = int(W.utc_time(source_end["observed_at"], "quota_append_source_fence_time_invalid").timestamp())
    facts = source_facts(tables["redemptions"], mapping, at)
    # The helper's key-named operation selector changes only the exact backup
    # verification query's receipt exclusion. The original backup contract,
    # complete state comparison and isolated instance checks stay unchanged.
    backup = K.validate_backup({**evidence, "key_migration_id": evidence["migration_id"]}, snapshot, contract, target)
    authority = {"scope": SCOPE, "source_archive_sha256": proof["source_archive_sha256"],
        "source_export_sha256": proof["source_export_sha256"], "source_namespace_sha256": U.namespace_key(evidence["source_namespace"]),
        "unit": unit, "historical_units": wallet["units"], "identity_map_sha256": digest(mapping),
        "identity_receipt_sha256": digest(identity[0]), "identity_lineage_sha256": digest(bindings),
        "wallet": wallet_facts, "reservations": reservations, "backup_restore": backup,
        "source_fence_sha256": evidence["full_source_fence"]["output"]["sha256"],
        "target_state_sha256": digest(snapshot["state"]), "source_facts": facts,
        "other_rights_completion_claimed": False}
    return {"facts": facts, "authority": authority, "mapping": mapping, "snapshot": snapshot,
        "contract": contract, "native": observed, "source": source, "wallet": wallet}


def _material(evidence, runtime_roles, ctx):
    m = prepare_mechanics(ctx["facts"], ctx["mapping"], ctx["snapshot"], ctx["contract"],
        evidence["source_namespace"], ctx["source"]["source_archive_sha256"], evidence["migration_id"],
        runtime_roles, ctx["native"])
    m["production_closure"] = {"evidence_sha256": digest({k: v for k, v in evidence.items() if k != "economic_review"}),
                               "authority": ctx["authority"]}
    return m


def prepare(evidence, runtime_roles):
    ctx = inspect(evidence)
    return _material(evidence, runtime_roles, ctx), ctx


def production_sql(m, evidence):
    ctx = inspect(evidence)
    rebuilt = _material(evidence, m["runtime_roles"], ctx)
    need(m == rebuilt, "quota_append_material_not_rebuilt_from_actual_evidence")
    guard = W._readback_sql(ctx["wallet"], current_balances=True)
    return transaction_sql(m, production_guard=guard)


def review_binding(m, sql):
    return {"format_version": 1, "scope": SCOPE, "decision": DECISION,
        "material_sha256": digest(m), "sql_sha256": hashlib.sha256(sql.encode()).hexdigest(),
        "exact_future_liability": m["source_facts"], "production_closure": m["production_closure"]}


def validate_review(m, sql, evidence):
    review = K.read_ref(evidence["economic_review"])
    binding = review_binding(m, sql)
    need(type(review) is dict and set(review) == set(binding) | {"user_authorization_record"}
         and all(review[k] == v for k, v in binding.items()), "quota_append_exact_economic_review_required")
    record = K.read_ref(review["user_authorization_record"])
    need(set(record) == set(binding) | {"user_id", "message_reference", "approved_at", "exact_user_message"}
         and all(record[k] == v for k, v in binding.items())
         and all(type(record[k]) is str and record[k].strip() for k in (
             "user_id", "message_reference", "approved_at", "exact_user_message")),
         "quota_append_actual_user_review_origin_required")


def execute(m, evidence, connection, output):
    sql = production_sql(m, evidence)
    validate_review(m, sql, evidence)
    K.connection_binding(m, connection)
    app = "lmm-current-quota-" + uuid.uuid4().hex
    attempt = {"format_version": 1, "operation": "apply", "migration_id": m["migration_id"],
        "application_name": app, "material_sha256": digest(m), "sql_sha256": hashlib.sha256(sql.encode()).hexdigest(),
        "economic_review": evidence["economic_review"], "automatic_retry_allowed": False}
    F.write_private(output, "attempt.private.json", canonical(attempt) + "\n")
    F.write_private(output, "executed.private.sql", sql)
    result = {"status": "UNKNOWN", "migration_id": m["migration_id"], "material_sha256": digest(m)}
    try:
        code, raw = F.run_sql(connection, sql, output, app, label="apply")
        if code == 0 and json.loads(raw, object_pairs_hook=P.unique_object) == {**result, "status": "COMMITTED"}:
            result["status"] = "COMMITTED"
    except Exception:
        pass
    try:
        F.write_private(output, "result.private.json", canonical(result) + "\n")
    except Exception:
        return {**result, "artifact_publication": "FAILED_REQUIRES_READONLY_RESOLUTION", "automatic_retry_allowed": False}
    return result


def resolve(m, attempt, connection, output):
    need(attempt.get("operation") == "apply" and attempt.get("migration_id") == m["migration_id"]
         and attempt.get("material_sha256") == digest(m), "quota_append_attempt_binding_invalid")
    K.connection_binding(m, connection)
    sql = resolve_sql(m, attempt["application_name"])
    F.write_private(output, "resolve.private.sql", sql)
    result = {"status": "UNKNOWN", "migration_id": m["migration_id"], "material_sha256": digest(m), "automatic_retry_allowed": False}
    try:
        code, raw = F.run_sql(connection, sql, output, "lmm-current-quota-resolve-" + uuid.uuid4().hex, readonly=True, label="resolve")
        if code == 0 and json.loads(raw, object_pairs_hook=P.unique_object) == {**result, "status": "COMMITTED"}:
            result["status"] = "COMMITTED"
    except Exception:
        pass
    F.write_private(output, "result.private.json", canonical(result) + "\n")
    return result


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=("inspect", "prepare", "apply", "resolve", "native-observation-sql"))
    p.add_argument("--output", required=True, type=Path)
    for name in ("evidence", "runtime-roles", "material", "attempt", "connection", "target-projection"):
        p.add_argument("--" + name, type=Path)
    a = p.parse_args(argv)
    I.private_backup_path(a.output.parent, directory=True)
    a.output.mkdir(mode=0o700, exist_ok=False)
    if a.command == "native-observation-sql":
        sql = native_observation_sql(C.load_json(a.target_projection)["source_target_id_map"])
        F.write_private(a.output, "native-observation.private.sql", sql)
        result = {"status": "READONLY_QUERY_PREPARED", "queries_executed": 0}
    elif a.command == "resolve":
        result = resolve(C.load_json(a.material), C.load_json(a.attempt), C.load_json(a.connection), a.output)
    else:
        evidence = C.load_json(a.evidence)
        if a.command == "inspect":
            ctx = inspect(evidence)
            F.write_private(a.output, "authority.preview.private.json", canonical(ctx["authority"]) + "\n")
            result = {"status": "PREVIEW_ONLY", "codes": ctx["facts"]["codes"],
                      "future_liability_units": ctx["facts"]["future_liability_units"], "immediate_credit_units": 0}
        elif a.command == "prepare":
            m, ctx = prepare(evidence, C.load_json(a.runtime_roles))
            sql = production_sql(m, evidence)
            F.write_private(a.output, "material.private.json", canonical(m) + "\n")
            F.write_private(a.output, "preview.private.sql", sql)
            F.write_private(a.output, "economic-review-binding.NOT-APPROVAL.private.json", canonical(review_binding(m, sql)) + "\n")
            result = {"status": "PREPARED_NOT_AUTHORIZED", "codes": m["source_facts"]["codes"], "immediate_credit_units": 0}
        else:
            result = execute(C.load_json(a.material), evidence, C.load_json(a.connection), a.output)
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()

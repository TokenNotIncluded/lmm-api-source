#!/usr/bin/env python3
"""Owner-only weekly history import. No coupon issuance and no ledger mutation."""
from pathlib import Path
from collections import Counter
from copy import deepcopy
import argparse, hashlib, json, os, re, sys
import current_affiliate_aliases as A

COMPONENT_COUNT = 88
COMPONENT_NAME = "assistant-weekly-discounts"
FORMAT = "lmm.current-weekly-discount-import.v1"
SCOPE = "current-legacy-weekly-discounts-v1"
PREPARATION = "lmm.current-weekly-discount-preparation.v1"
# Bound only after the final88 SQL capture and independent review.
TARGET_BINDING_SHA256 = "e3b55b162c63b0875414860532f056732dff3359192b3e8c8c909711e97891a2"
ALIAS_HELPER_SHA256 = "ac52ab3beeaa2b1d46e6ec8bb11b106da3f98ad13f642e4719c7f22c6ea21f8e"
TABLES = ["core_weekly.weekly_discounts", "core_weekly.legacy_weekly_discount_bases",
          "core_meta.legacy_weekly_discount_imports"]
HEX = re.compile(r"[a-f0-9]{64}")
IDENTIFIER = re.compile(r"[A-Za-z0-9._:/-]{1,128}")
Error = A.Error
need, canonical, sha, digest, parse, file_ref, read_ref = (
    A.need, A.canonical, A.sha, A.digest, A.parse, A.file_ref, A.read_ref)

def frozen_helpers():
    need(sha(Path(A.__file__).read_bytes()) == ALIAS_HELPER_SHA256,
         "weekly_shared_source_owner_helpers_changed")
    return A.frozen_helpers()

def schema_path():
    return Path(__file__).resolve().parent.parent / "services/core/schema/assistant_weekly_discounts.sql"

def integer(value):
    need(type(value) is int and -(1<<63) <= value < (1<<63), "weekly_integer_required")
    return value

def project(users, weekly, coupons, refs, *, synthetic=False):
    _, _, R = frozen_helpers()
    owners = {integer(row["id"]) for row in users}
    need(len(owners) == len(users), "weekly_duplicate_source_users")
    by_code = {integer(row["id"]): row for row in coupons}
    need(len(by_code) == len(coupons), "weekly_duplicate_source_coupons")
    rows, seen_ids, seen_weeks = [], set(), set()
    referenced = []
    for row in sorted(weekly, key=lambda r: integer(r["id"])):
        ident, user = integer(row["id"]), integer(row["user_id"])
        week, percent = integer(row["week_start"]), integer(row["discount_percent"])
        created, claimed = integer(row["created_at"]), integer(row.get("claimed_at", 0))
        code = integer(row.get("code_id", 0))
        status, reason, conversation = row["status"], row["reason"], row["conversation_id"]
        need(ident > 0 and ident not in seen_ids and user in owners, "weekly_source_owner_or_id_invalid")
        need(week >= 345600 and (week-345600) % 604800 == 0 and (user, week) not in seen_weeks,
             "weekly_duplicate_or_non_utc_week")
        need(type(reason) is str and 2 <= len(reason) <= 240 and type(conversation) is int and integer(conversation) > 0,
             "weekly_source_text_shape_invalid")
        need(created >= week and created < week+604800 and 0 <= percent <= 99,
             "weekly_source_decision_shape_invalid")
        need((status == "declined" and percent == 0 and code == claimed == 0)
             or (status == "offered" and percent > 0 and code == claimed == 0)
             or (status == "claimed" and percent > 0 and code > 0 and claimed >= created),
             "weekly_source_status_or_claim_invalid")
        code_digest = None
        if status == "claimed":
            source_code = by_code.get(code)
            need(source_code is not None and source_code["owner_user_id"] == user
                 and source_code["discount_percent"] == percent and source_code["max_uses"] == 1
                 and source_code["expired_time"] == week+604800 and source_code["min_amount"] == 0,
                 "weekly_original_private_coupon_mismatch")
            referenced.append(source_code)
            code_digest = R.F.digest(source_code)
        seen_ids.add(ident); seen_weeks.add((user, week))
        rows.append({"source_weekly_id": ident, "source_user_id": user,
                     "week_start": week, "discount_percent": percent, "status": status,
                     "reason": reason, "source_conversation_id": conversation, "created_at": created,
                     "claimed_at": claimed if claimed else None, "source_code_id": code if code else None,
                     "source_coupon_row_sha256": code_digest, "source_row_sha256": R.F.digest(row)})
    need(len({r["id"] for r in referenced}) == len(referenced), "weekly_coupon_referenced_twice")
    for key in ["source_archive_sha256", "source_export_sha256", "source_export_proof_sha256"]:
        need(type(refs.get(key)) is str and HEX.fullmatch(refs[key]), "weekly_source_hash_missing")
    return {"source_kind": "SYNTHETIC_WEEKLY_FIXTURE" if synthetic else "VERIFIED_FULL_CANONICAL_SOURCE",
            **{key: refs[key] for key in ["source_archive_sha256", "source_export_sha256",
                                        "source_export_proof_sha256"]},
            "source_users_sha256": R.F.digest(users), "source_weekly_rows_sha256": R.F.digest(weekly),
            "source_coupon_rows_sha256": R.F.digest(sorted(referenced, key=lambda r: r["id"])),
            "source_digest_format": R.F.DIGEST_FORMAT, "user_count": len(users),
            "decision_count": len(rows), "rows": rows}

def verified_source(evidence):
    _, _, R = frozen_helpers()
    need(type(evidence) is dict and set(evidence) == {"receipt", "proof"}, "weekly_source_evidence_invalid")
    receipt = parse(read_ref(evidence["receipt"])); read_ref(evidence["proof"])
    hashes = {}
    tables, proof, baseline = R.load_routing_export(
        receipt, Path(evidence["proof"]["path"]), hashes,
        retained_tables=("users", "assistant_weekly_discounts", "discount_codes"))
    for name in ("users", "assistant_weekly_discounts", "discount_codes"):
        need(len(tables[name]) == baseline[name] == proof["selected_table_counts"][name],
             "weekly_complete_source_baseline_mismatch")
    projection = project(tables["users"], tables["assistant_weekly_discounts"], tables["discount_codes"],
        {"source_archive_sha256": proof["source_archive_sha256"],
         "source_export_sha256": proof["source_export_sha256"],
         "source_export_proof_sha256": evidence["proof"]["sha256"]})
    R.verify_files(hashes)
    return projection, tables["users"]

def source_summary(projection):
    return {"scope": SCOPE, "status": "SOURCE_PREVIEW_ONLY", "source_users": projection["user_count"],
            "decisions": projection["decision_count"],
            "status_counts": dict(sorted(Counter(r["status"] for r in projection["rows"]).items())),
            "percent_counts": dict(sorted(Counter(str(r["discount_percent"]) for r in projection["rows"]).items())),
            "original_coupon_references": sum(r["source_code_id"] is not None for r in projection["rows"]),
            "source_weekly_rows_sha256": projection["source_weekly_rows_sha256"],
            "target_binding": "PENDING_EXACT88_AND_ACTUAL_COMMITTED_C343_IDENTITY",
            "source_fence_observed": False, "production_authorization": False,
            "target_writes": 0, "coupons_created": 0, "ledger_writes": 0}

def contract_from_manifest(raw, binding, *, synthetic=False):
    if not synthetic:
        need(TARGET_BINDING_SHA256 is not None and digest(binding) == TARGET_BINDING_SHA256,
             "weekly_final88_target_unbound")
    manifest = parse(raw)
    need(manifest.get("scope") == "compiled-core-schema-contract" and manifest.get("format_version") == 1,
         "weekly_manifest_format_invalid")
    components = manifest["components"]
    names = {r["name"]: r["raw_schema_sha256"] for r in components}
    need(len(components) == len(names) == COMPONENT_COUNT
         and all(HEX.fullmatch(value) for value in names.values())
         and components[-1]["name"] == COMPONENT_NAME
         and names[COMPONENT_NAME] == sha(schema_path().read_bytes()), "weekly_component_assembly_changed")
    catalog = manifest["catalog"]
    need(catalog["query_sha256"] == sha(catalog["query_utf8"].encode())
         and catalog["query_hash_domain"] == "sha256-raw-utf8-v1"
         and catalog["fingerprint_hash_domain"] == "sha256-u64be-byte-length-prefixed-utf8-v1"
         and catalog["definition_order"] == "postgresql-c-collation", "weekly_catalog_algorithm_changed")
    need(binding["component_count"] == COMPONENT_COUNT and binding["manifest_sha256"] == sha(raw)
         and HEX.fullmatch(binding["catalog_sha256"]), "weekly_target_binding_invalid")
    return {"component_count": COMPONENT_COUNT, "manifest_sha256": sha(raw),
            "catalog_fingerprint_sha256": binding["catalog_sha256"], "catalog_query": catalog["query_utf8"],
            "component_fingerprints": names, "identity_version": manifest["identity"]["version"],
            "identity_fingerprint_sha256": manifest["identity"]["fingerprint_sha256"],
            "binding": deepcopy(binding), "synthetic_only": synthetic}

def weekly_fence_sql(namespace):
    L, _, _ = frozen_helpers(); L.U.namespace_key(namespace)
    weekly = L.F.relation(namespace["schema"]+".assistant_weekly_discounts")
    coupons = L.F.relation(namespace["schema"]+".discount_codes")
    return """BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC'; SET LOCAL row_security=off; SET LOCAL statement_timeout='60s';
SELECT jsonb_build_object('scope','current-source-weekly-discount-fence-v1',
 'source_namespace',jsonb_build_object('system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
  'database',current_database(),'schema',""" + L.U.literal(namespace["schema"]) + """),
 'observed_at',to_char(clock_timestamp(),'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
 'local_snapshot',pg_current_snapshot()::text,'transaction_read_only',current_setting('transaction_read_only'),
 'database_allows_connections',(SELECT datallowconn FROM pg_database WHERE datname=current_database()),
 'other_database_backends',(SELECT count(*) FROM pg_stat_activity
  WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid()
    AND backend_type IS DISTINCT FROM 'autovacuum worker'),
 'prepared_transactions',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database()),
 'weekly_rows',(SELECT coalesce(jsonb_agg(to_jsonb(w) ORDER BY w.id),'[]'::jsonb) FROM """+weekly+""" w),
 'coupon_rows',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY c.id),'[]'::jsonb) FROM """+coupons+""" c
   WHERE EXISTS(SELECT FROM """+weekly+""" w WHERE w.code_id=c.id AND w.code_id>0)));
COMMIT;
"""

def validate_source_fence(projection, identity_run, evidence):
    L, _, R = frozen_helpers()
    need(set(evidence) == {"identity_fence", "weekly_fence"}, "weekly_final_source_fence_required")
    identity_fence = parse(read_ref(evidence["identity_fence"]))
    material = identity_run["manifest"]["material"]
    L.ID.validate_final_fence(material, identity_fence)
    fence = parse(read_ref(evidence["weekly_fence"]))
    need(set(fence) == {"scope", "verification_actual_exit", "observation", "observation_sql",
                        "verification_stderr", "identity_fence_sha256"}
         and fence["scope"] == "current-source-weekly-discount-fence-v1"
         and type(fence["verification_actual_exit"]) is int and fence["verification_actual_exit"] == 0
         and fence["identity_fence_sha256"] == evidence["identity_fence"]["sha256"],
         "weekly_source_fence_binding_invalid")
    need(read_ref(fence["observation_sql"]) == weekly_fence_sql(material["source_namespace"]).encode(),
         "weekly_source_fence_sql_changed")
    read_ref(fence["verification_stderr"])
    observation = parse(read_ref(fence["observation"]))
    identity_observation = parse(L.ID.private_bytes(identity_fence["observation"]["path"]))
    need(observation["scope"] == fence["scope"] and observation["source_namespace"] == material["source_namespace"]
         and observation["transaction_read_only"] == "on" and observation["database_allows_connections"] is False
         and type(observation["other_database_backends"]) is int and observation["other_database_backends"] == 0
         and type(observation["prepared_transactions"]) is int and observation["prepared_transactions"] == 0
         and type(observation["local_snapshot"]) is str and observation["local_snapshot"]
         and L.ID.utc_time(observation["observed_at"]) >= L.ID.utc_time(identity_observation["observed_at"]),
         "weekly_real_source_barrier_missing")
    need(R.F.digest(observation["weekly_rows"]) == projection["source_weekly_rows_sha256"]
         and R.F.digest(observation["coupon_rows"]) == projection["source_coupon_rows_sha256"],
         "weekly_source_decisions_or_coupons_changed_after_snapshot")
    return digest(evidence)

# The owner boundary below follows the already reviewed alias importer, while
# the component88 binding and weekly tables are checked independently.
def validate_snapshot(snapshot, contract, actor, *, synthetic):
    L,T,_ = frozen_helpers()
    instance = snapshot['instance']
    L.C.alias_target(instance)
    need(contract['component_count'] == COMPONENT_COUNT
         and len(contract['component_fingerprints']) == COMPONENT_COUNT
         and contract['component_fingerprints'][COMPONENT_NAME] == sha(schema_path().read_bytes())
         and snapshot['component_fingerprints'] == contract['component_fingerprints']
         and snapshot['runtime_catalog'] == snapshot['catalog_fingerprint_sha256']
             == contract['catalog_fingerprint_sha256'], 'weekly_exact88_catalog_required')
    need(instance['database_owner'] == instance['role'] != instance['session_user']
         and instance['effective_role_can_login'] is False and instance['tcp_connection'] is False
         and HEX.fullmatch(instance['server_instance_nonce']), 'weekly_offline_owner_instance_required')
    boundary = snapshot['role_boundary']
    need(boundary['other_connections'] == boundary['prepared_transactions'] == 0
         and boundary['unsafe_runtime'] == [] and boundary['connectors'] == [instance['session_user']],
         'weekly_runtime_not_fenced')
    roles = {row['rolname']:row for row in boundary['roles']}
    need(len(roles) == len(boundary['roles']) and instance['role'] in roles and instance['session_user'] in roles,
         'weekly_owner_role_coverage_invalid')
    for name, role in roles.items():
        need(not any(role[key] for key in ('rolsuper','rolcreaterole','rolcreatedb','rolreplication','rolbypassrls'))
             and role['rolcanlogin'] == (name == instance['session_user']), 'weekly_offline_role_invalid')
    need(roles[instance['session_user']]['rolinherit'] is False
         and boundary['database_nonce'] == 'lmm.migration_instance_nonce='+instance['server_instance_nonce']
         and any(row['role'] == instance['role'] and row['member'] == instance['session_user']
                 and row['set'] is True and row['inherit'] is False and row['admin'] is False
                 for row in boundary['memberships'])
         and HEX.fullmatch(snapshot['ownership_sha256']), 'weekly_installer_boundary_invalid')
    actual = snapshot['migration_actor']
    need(actual['user_id'] == integer(actor) and actual['active'] is True
         and actual['account_active'] is True and actual['platform_role'] == 'superadmin',
         'weekly_enabled_actor_required')
    T.validate_inventory(snapshot['state']['inventory'])
    need(set(snapshot['state']['tables']) == set(snapshot['state']['inventory']['tables'])
         and set(snapshot['state']['sequences']) == set(snapshot['state']['inventory']['sequences'])
         and set(TABLES) <= set(snapshot['state']['tables']), 'weekly_inventory_incomplete')
    need(snapshot['weekly_count'] == snapshot['weekly_import_count'] == 0, 'weekly_existing_import_requires_readonly_resolution')
    if synthetic:
        need(contract['synthetic_only'] is True
             and re.fullmatch(r'lmm_runtime_keys_test_[a-z0-9_]{1,30}', instance['database'])
             and snapshot['database_marker'] == 'lmm-runtime-keys-synthetic:'+snapshot['fixture_nonce'],
             'weekly_synthetic_database_marker_required')
    else:
        need(contract['synthetic_only'] is False and TARGET_BINDING_SHA256 is not None
             and digest(contract['binding']) == TARGET_BINDING_SHA256, 'weekly_final88_target_UNBOUND')

def observation_sql(mapping, identity_id, projection, *, inventory, contract, runtime_roles, actor):
    L, _, _ = frozen_helpers(); lit = L.U.literal
    base = A.observation_sql(mapping, identity_id, inventory=inventory, contract=contract,
                            runtime_roles=runtime_roles, actor=actor)
    need(base.startswith("SELECT "), "weekly_owner_observation_contract_changed")
    return "SELECT (("+base[7:]+")-ARRAY['alias_count','import_count']) || jsonb_build_object(" \
        "'weekly_count',(SELECT count(*) FROM core_weekly.weekly_discounts WHERE origin='legacy')," \
        "'weekly_import_count',(SELECT count(*) FROM core_meta.legacy_weekly_discount_imports)," \
        "'coupon_mapping',(SELECT coalesce(jsonb_agg(jsonb_build_object(" \
        "'source_discount_id',b.source_discount_id,'code_id',b.code_id," \
        "'source_namespace_sha256',encode(b.source_namespace_sha256,'hex')," \
        "'source_archive_sha256',encode(b.source_archive_sha256,'hex')," \
        "'source_row_sha256',encode(b.source_row_sha256,'hex')," \
        "'owner_user_id',b.original_owner_user_id,'discount_percent',c.discount_percent," \
        "'max_uses',c.max_uses,'min_credit_units',c.min_credit_units,'ends_at',c.ends_at)" \
        " ORDER BY b.source_discount_id),'[]'::jsonb) FROM core_payments.legacy_discount_bases b" \
        " JOIN core_payments.discount_codes c ON c.id=b.code_id" \
        " JOIN core_migration.import_runs ir ON ir.migration_id="+lit(identity_id)+ \
        " WHERE encode(b.source_namespace_sha256,'hex')=ir.manifest->>'source_namespace_sha256'" \
        " AND EXISTS(SELECT FROM jsonb_array_elements("+lit(projection["rows"])+ \
        "::jsonb) w WHERE (w->>'source_code_id')::bigint=b.source_discount_id)))"

def validate_coupons(projection, mapping, snapshot, namespace_sha):
    mapped = {r["source_user_id"]: r for r in mapping}
    actual = snapshot["coupon_mapping"]
    by_code = {r["source_discount_id"]: r for r in actual}
    claimed = [r for r in projection["rows"] if r["status"] == "claimed"]
    need(len(actual) == len(by_code) == len(claimed), "weekly_current_coupon_mapping_incomplete")
    expected = []
    for row in claimed:
        coupon = by_code.get(row["source_code_id"])
        need(coupon is not None and coupon["source_namespace_sha256"] == namespace_sha
             and coupon["source_archive_sha256"] == projection["source_archive_sha256"]
             and coupon["source_row_sha256"] == row["source_coupon_row_sha256"]
             and coupon["owner_user_id"] == mapped[row["source_user_id"]]["target_user_id"]
             and coupon["discount_percent"] == row["discount_percent"] and coupon["max_uses"] == 1
             and coupon["min_credit_units"] == 0 and coupon["ends_at"] == row["week_start"]+604800,
             "weekly_existing_native_coupon_binding_changed")
        expected.append(deepcopy(coupon))
    return sorted(expected, key=lambda row: row["source_discount_id"])

def prepare_owner(projection, users, snapshot, contract, *, migration_id, actor,
                  source_evidence=None, fence_evidence=None, synthetic=False, synthetic_source=None):
    need(type(migration_id) is str and IDENTIFIER.fullmatch(migration_id), "weekly_migration_id_invalid")
    if synthetic:
        need(projection["source_kind"] == "SYNTHETIC_WEEKLY_FIXTURE"
             and snapshot["identity_run"]["manifest"]["material"]["source_kind"] == "synthetic-test-facts",
             "weekly_actual_synthetic_identity_required")
        need(type(synthetic_source) is dict and set(synthetic_source) == {"weekly", "coupons"}
             and projection == project(users, synthetic_source["weekly"], synthetic_source["coupons"],
                                       projection, synthetic=True), "weekly_synthetic_source_changed")
        fence_sha = None
    else:
        need(projection["source_kind"] == "VERIFIED_FULL_CANONICAL_SOURCE", "weekly_production_source_required")
        verified, verified_users = verified_source(source_evidence)
        need(verified == projection and verified_users == users, "weekly_canonical_source_changed")
        fence_sha = validate_source_fence(projection, snapshot["identity_run"], fence_evidence)
    validate_snapshot(snapshot, contract, actor, synthetic=synthetic)
    mapping = A.actual_mapping(snapshot["identity_run"], snapshot["identity_authority"], users, projection)
    ir, instance = snapshot["identity_run"], snapshot["instance"]
    im = ir["manifest"]["material"]
    coupons = validate_coupons(projection, mapping, snapshot, im["source_namespace_sha256"])
    target = {"component_count": COMPONENT_COUNT, "manifest_sha256": contract["manifest_sha256"],
              "catalog_sha256": contract["catalog_fingerprint_sha256"], "database": instance["database"],
              "database_oid": instance["database_oid"], "system_identifier": instance["system_identifier"],
              "server_instance_nonce": instance["server_instance_nonce"]}
    material = {"format": FORMAT, "scope": SCOPE, "migration_id": migration_id,
                "source_kind": projection["source_kind"], "source_namespace": im["source_namespace"],
                "source_namespace_sha256": im["source_namespace_sha256"], "source": deepcopy(projection),
                "mapping": mapping, "coupon_mapping": coupons, "migration_actor_user_id": actor,
                "source_fence_sha256": fence_sha, "target": target,
                "identity": {"migration_id": ir["migration_id"], "mapping_sha256": ir["mapping_sha256"],
                             "manifest_sha256": snapshot["identity_manifest_sha256"],
                             "actual_authority": snapshot["identity_authority"]}}
    if synthetic: material["fixture_nonce"] = snapshot["fixture_nonce"]
    return {"format": PREPARATION, "material": material, "contract": deepcopy(contract),
            "snapshot": deepcopy(snapshot), "source_evidence": source_evidence, "fence_evidence": fence_evidence,
            "synthetic_users": deepcopy(users) if synthetic else None,
            "synthetic_source": deepcopy(synthetic_source) if synthetic else None}

def validate_preparation(prepared):
    need(prepared["format"] == PREPARATION, "weekly_preparation_format")
    m = prepared["material"]; synthetic = m["source_kind"] == "SYNTHETIC_WEEKLY_FIXTURE"
    if synthetic:
        users = prepared["synthetic_users"]; raw = prepared["synthetic_source"]
        projection = project(users, raw["weekly"], raw["coupons"], m["source"], synthetic=True)
    else: projection, users = verified_source(prepared["source_evidence"])
    rebuilt = prepare_owner(projection, users, prepared["snapshot"], prepared["contract"],
        migration_id=m["migration_id"], actor=m["migration_actor_user_id"],
        source_evidence=prepared["source_evidence"], fence_evidence=prepared["fence_evidence"],
        synthetic=synthetic, synthetic_source=prepared["synthetic_source"])
    need(rebuilt == prepared, "weekly_saved_preparation_changed")

def reviewed(prepared, review):
    m = prepared["material"]
    if m["source_kind"] == "SYNTHETIC_WEEKLY_FIXTURE":
        expected = {"scope": "synthetic-weekly-test-only", "material_sha256": digest(m),
                    "production_authorization": False}
    else:
        expected = {"scope": "reviewed-current-legacy-weekly-discounts-v1", "material_sha256": digest(m),
                    "migration_actor_user_id": m["migration_actor_user_id"],
                    "source_fence_sha256": m["source_fence_sha256"], "target": m["target"]}
    need(review == expected, "weekly_exact_review_document_required")
    return canonical(review)

def imported_rows(material):
    mapping = {r["source_user_id"]: r for r in material["mapping"]}
    coupons = {r["source_discount_id"]: r["code_id"] for r in material["coupon_mapping"]}
    result = []
    for source in material["source"]["rows"]:
        m = mapping[source["source_user_id"]]
        ident = sha((material["source_namespace_sha256"]+":weekly:"+str(source["source_weekly_id"])).encode())
        result.append({**source, "id": ident, "user_id": m["target_user_id"],
                       "account_id": m["target_personal_account_id"],
                       "source_namespace_sha256": material["source_namespace_sha256"],
                       "source_archive_sha256": material["source"]["source_archive_sha256"],
                       "code_id": coupons.get(source["source_code_id"])})
    return result

def readback_expression(prepared, review):
    L, _, _ = frozen_helpers(); lit=L.U.literal; m=prepared["material"]
    rows = imported_rows(m)
    return "(SELECT count(*)=1 AND bool_and(import_id="+lit(m["migration_id"])+ \
        " AND material_utf8="+lit(canonical(m))+" AND material_sha256="+lit(digest(m))+ \
        " AND review_utf8="+lit(canonical(review))+" AND review_sha256="+lit(digest(review))+ \
        ") FROM core_meta.legacy_weekly_discount_imports) AND " \
        "(SELECT count(*) FROM core_weekly.legacy_weekly_discount_bases WHERE import_id="+lit(m["migration_id"])+ \
        ")="+str(len(rows))+" AND NOT EXISTS(SELECT FROM jsonb_array_elements("+lit(rows)+"::jsonb) s" \
        " LEFT JOIN core_weekly.weekly_discounts d ON d.id=s->>'id'" \
        " LEFT JOIN core_weekly.legacy_weekly_discount_bases b ON b.decision_id=d.id" \
        " WHERE d.id IS NULL OR b.import_id IS DISTINCT FROM "+lit(m["migration_id"])+ \
        " OR d.origin IS DISTINCT FROM 'legacy' OR d.user_id IS DISTINCT FROM (s->>'user_id')::bigint" \
        " OR d.account_id IS DISTINCT FROM (s->>'account_id')::bigint" \
        " OR d.week_start IS DISTINCT FROM (s->>'week_start')::bigint" \
        " OR d.discount_percent IS DISTINCT FROM (s->>'discount_percent')::smallint" \
        " OR d.reason IS DISTINCT FROM s->>'reason' OR d.created_at IS DISTINCT FROM (s->>'created_at')::bigint" \
        " OR b.source_row_sha256 IS DISTINCT FROM decode(s->>'source_row_sha256','hex')" \
        " OR b.original_status IS DISTINCT FROM s->>'status'" \
        " OR b.source_weekly_id IS DISTINCT FROM (s->>'source_weekly_id')::bigint" \
        " OR b.source_user_id IS DISTINCT FROM (s->>'source_user_id')::bigint" \
        " OR b.source_conversation_id IS DISTINCT FROM (s->>'source_conversation_id')::bigint" \
        " OR b.source_code_id IS DISTINCT FROM (s->>'source_code_id')::bigint" \
        " OR b.original_claimed_at IS DISTINCT FROM (s->>'claimed_at')::bigint" \
        " OR b.source_namespace_sha256 IS DISTINCT FROM decode(s->>'source_namespace_sha256','hex')" \
        " OR b.source_archive_sha256 IS DISTINCT FROM decode(s->>'source_archive_sha256','hex')" \
        " OR NOT (d.status=s->>'status' OR (s->>'status'='offered' AND d.status='claimed'))" \
        " OR (s->>'status'='claimed' AND (d.code_id IS DISTINCT FROM (s->>'code_id')::bigint" \
        " OR d.claimed_at IS DISTINCT FROM (s->>'claimed_at')::bigint)))"

def transaction_sql(prepared, review):
    validate_preparation(prepared); review_raw=reviewed(prepared,review)
    L,T,_=frozen_helpers();F=L.F;lit=L.U.literal
    m,s,c=prepared["material"],prepared["snapshot"],prepared["contract"]
    inventory=s["state"]["inventory"];state="("+T.state_sql(inventory)+")";before=lit(s["state"])+"::jsonb"
    roles=sorted(r["rolname"] for r in s["role_boundary"]["roles"]
                 if r["rolname"] not in (s["instance"]["role"],s["instance"]["session_user"]))
    guard=("IF ("+F.instance_sql()+") IS DISTINCT FROM "+lit(s["instance"])+"::jsonb"
        " OR ("+L.U.I.catalog_hash_sql(c)+")<>"+lit(c["catalog_fingerprint_sha256"])+
        " OR ("+T.role_boundary_sql(roles)+") IS DISTINCT FROM "+lit(s["role_boundary"])+"::jsonb"
        " OR ("+F.ownership_sql()+")<>"+lit(s["ownership_sha256"])+
        " THEN RAISE EXCEPTION 'weekly_target_or_owner_fence_changed'; END IF;")
    exact=readback_expression(prepared,review);rows=lit(imported_rows(m))+"::jsonb"
    excluded="ARRAY["+",".join(lit(t) for t in TABLES)+"]"
    body="""DECLARE after_state jsonb; BEGIN
 IF NOT pg_try_advisory_xact_lock("""+str(L.U.FENCE)+""") THEN RAISE EXCEPTION 'weekly_import_busy'; END IF;
 """+guard+"""
 LOCK TABLE """+",".join(F.relation(t) for t in inventory["tables"])+""" IN ACCESS EXCLUSIVE MODE;
 """+guard+"""
 IF EXISTS(SELECT FROM core_meta.legacy_weekly_discount_imports) THEN
   IF coalesce(("""+exact+"""),false) IS NOT TRUE THEN RAISE EXCEPTION 'weekly_replay_conflict'; END IF;
 ELSE
   IF """+state+""" IS DISTINCT FROM """+before+""" THEN RAISE EXCEPTION 'weekly_before_state_changed'; END IF;
   """+L.ID.readback_sql(s["identity_run"]["manifest"]["material"])+"""
   INSERT INTO core_meta.legacy_weekly_discount_imports(import_id,material_utf8,material_sha256,review_utf8,review_sha256)
   VALUES("""+",".join(lit(v) for v in (m["migration_id"],canonical(m),digest(m),review_raw,sha(review_raw.encode())))+""");
   INSERT INTO core_weekly.weekly_discounts(id,user_id,account_id,week_start,origin,status,discount_percent,
      reason,created_at,code_id,claimed_at)
   SELECT r->>'id',(r->>'user_id')::bigint,(r->>'account_id')::bigint,(r->>'week_start')::bigint,
      'legacy',r->>'status',(r->>'discount_percent')::smallint,r->>'reason',
      (r->>'created_at')::bigint,(r->>'code_id')::bigint,(r->>'claimed_at')::bigint
   FROM jsonb_array_elements("""+rows+""") r;
   INSERT INTO core_weekly.legacy_weekly_discount_bases(decision_id,import_id,source_namespace_sha256,
      source_archive_sha256,source_weekly_id,source_row_sha256,source_user_id,source_conversation_id,
      source_code_id,original_status,original_claimed_at)
   SELECT r->>'id',"""+lit(m["migration_id"])+""",decode("""+lit(m["source_namespace_sha256"])+""",'hex'),
      decode("""+lit(m["source"]["source_archive_sha256"])+""",'hex'),(r->>'source_weekly_id')::bigint,
      decode(r->>'source_row_sha256','hex'),(r->>'source_user_id')::bigint,(r->>'source_conversation_id')::bigint,
      (r->>'source_code_id')::bigint,r->>'status',(r->>'claimed_at')::bigint
   FROM jsonb_array_elements("""+rows+""") r;
   SET CONSTRAINTS ALL IMMEDIATE;
   IF coalesce(("""+exact+"""),false) IS NOT TRUE THEN RAISE EXCEPTION 'weekly_readback_mismatch'; END IF;
   SELECT """+state+""" INTO after_state;
   IF after_state->'sequences' IS DISTINCT FROM """+before+"""->'sequences'
      OR (after_state->'tables')-"""+excluded+""" IS DISTINCT FROM ("""+before+"""->'tables')-"""+excluded+"""
      OR after_state->'inventory' IS DISTINCT FROM """+before+"""->'inventory'
   THEN RAISE EXCEPTION 'weekly_existing_state_changed'; END IF;
 END IF;
 """+guard+"""
END"""
    tag="$weekly_import_"+sha(body.encode())[:24]+"$"
    need(tag not in body,"weekly_sql_delimiter_collision")
    return ("BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL timezone='UTC'; SET LOCAL row_security=off;"
        " SET LOCAL synchronous_commit=on; SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='180s';\n"
        "DO "+tag+body+tag+"; COMMIT;\n"
        "SELECT jsonb_build_object('status','COMMITTED','migration_id',"+lit(m["migration_id"])+
        ",'automatic_retry_allowed',false);\n")

def resolve_sql(prepared, review, application_name):
    validate_preparation(prepared); reviewed(prepared,review)
    L,_,_=frozen_helpers();lit=L.U.literal
    need(type(application_name) is str and IDENTIFIER.fullmatch(application_name),
         "weekly_original_application_name_required")
    return ("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC';"
        " SET LOCAL row_security=off; SET LOCAL statement_timeout='180s';\n"
        "SELECT jsonb_build_object('scope','weekly-commit-readonly-resolution',"
        "'instance_matches',("+L.F.instance_sql()+")="+lit(prepared["snapshot"]["instance"])+"::jsonb,"
        "'original_execution_connections',(SELECT count(*) FROM pg_stat_activity"
        " WHERE datname=current_database() AND application_name="+lit(application_name)+
        " AND pid<>pg_backend_pid()),'exact_committed',coalesce(("+readback_expression(prepared,review)+
        "),false),'automatic_retry_allowed',false); COMMIT;\n")

def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest="command",required=True)
    preview=sub.add_parser("source-preview")
    preview.add_argument("--support",type=Path,required=True)
    preview.add_argument("--source-receipt",type=Path,required=True)
    preview.add_argument("--source-proof",type=Path,required=True)
    preview.add_argument("--output",type=Path,required=True)
    args=parser.parse_args(argv); os.umask(0o077)
    sys.path.insert(1,str(args.support.resolve(strict=True)))
    evidence={"receipt":file_ref(args.source_receipt),"proof":file_ref(args.source_proof)}
    projection,_=verified_source(evidence)
    A.write_private(args.output,{"format":"lmm.weekly-discount-source-preview.v1",
                    "source_evidence":evidence,"source":projection})
    print(json.dumps({"path":str(args.output),"sha256":sha(args.output.read_bytes()),
                      "summary":source_summary(projection)},sort_keys=True))

if __name__=="__main__": main()

#!/usr/bin/env python3
"""Private draft: append legacy keys to an already-used, exactly Core78 target.

This module deliberately does not turn a credential review, an old opening, or
a partial-wallet receipt into current economic authority.  Production preview
is useful and read-only; production apply is BLOCKED until the current source /
native reconciliation validator is reviewed and connected at
require_current_reconciliation().  No CLI switch or environment variable can
bypass that boundary.  The isolated PostgreSQL regression exercises the same
transaction generator through a database-bound synthetic fixture entry point.

Only this adapter's inserted keys are eligible for insertion/readback. Existing
Sessions, keys, budgets, money and history are hashed before and after. IDs are
obtained using nextval, never a source token ID and never a sequence reset.
PostgreSQL sequences are intentionally NOT rollbackable: failed attempts can
leave gaps. A new preview is required after a confirmed rollback; unknown
commit outcomes are resolved read-only and are never automatically retried.

The fresh69 importer and its frozen admission profile are unchanged.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import re
import sys

import legacy_credential_import as C

P, Q, F, I = C.PROPOSAL, C.QUOTA, C.FIN, C.IDENTITY
SCOPE = "current-runtime-key-append-v1"
CORE78_MANIFEST_SHA256 = "b9d0a472bddc826e0f6bfa0e725e409e67cbaabad6b0ad80cd512de24da74c2b"
CORE78_CATALOG_SHA256 = "dbc1df98f792d1484700e36005e900ec97a54a21a56d752bab0863bd84ab7177"
BLOCKER = "BLOCKED_CURRENT_RECONCILIATION_CONTRACT_MISSING"
FENCE = 741803121  # Shared with the existing offline migration writers.
TABLES = C.CHANGED_TABLES
SEQUENCES = C.CHANGED_SEQUENCES
REQUIRED_EVIDENCE = (
    "immutable fresh full source export, private export proof and runtime observation",
    "stable source database/site namespace bound to that export",
    "complete current owner map including source additions and historical owners",
    "historical and current quota-unit proofs tied to their respective snapshots",
    "per-owner current wallet/debt/reserved/affiliate reconciliation with native receipts",
    "current subscriptions, pending settlements and continuing rights reconciliation",
    "complete in-flight/reservation settlement plus source and target write fences",
    "independently authorized source review bound to the complete current reconciliation",
    "current target identity/catalog/ACL/backup-restore evidence and exact preview approval",
)
IDENTIFIER = re.compile(r"[a-z_][a-z_0-9]*\Z")
HEX = re.compile(r"[0-9a-f]{64}\Z")


class Error(ValueError):
    pass


def need(condition, code):
    if not condition:
        raise Error(code)


def digest(value):
    return F.digest(value)


def canonical(value):
    return F.canonical(value)


def literal(value):
    return F.literal(value)


def require_current_reconciliation(*_args, **_kwargs):
    """The sole production authority seam. Intentionally no surrogate approval.

    It must eventually validate the actual reviewed source/native evidence and
    bind it to the input files, source namespace, complete identity map and CAS
    snapshot before the transaction generator is reachable.  A digest proves
    byte identity, not economic correctness or approval.  The old fresh69 full
    opening cannot describe a used target, and an available-wallet partial
    receipt cannot establish deferred affiliate/subscription entitlements.
    """
    raise Error(BLOCKER)


def core78_contract(raw: bytes) -> dict:
    need(hashlib.sha256(raw).hexdigest() == CORE78_MANIFEST_SHA256,
         "current_keys_compiled_manifest_not_reviewed_core78")
    m = json.loads(raw, object_pairs_hook=P.unique_object)
    need(m.get("scope") == "compiled-core-schema-contract" and m.get("format_version") == 1,
         "current_keys_manifest_shape_invalid")
    need(len(m["components"]) == 78, "current_keys_component_count_not_78")
    components = {r["name"]: r["raw_schema_sha256"] for r in m["components"]}
    need(len(components) == 78, "current_keys_duplicate_components")
    catalog = m["catalog"]
    need(hashlib.sha256(catalog["query_utf8"].encode()).hexdigest() == catalog["query_sha256"]
         and catalog["query_hash_domain"] == "sha256-raw-utf8-v1"
         and catalog["fingerprint_hash_domain"] == "sha256-u64be-byte-length-prefixed-utf8-v1"
         and catalog["definition_order"] == "postgresql-c-collation",
         "current_keys_catalog_algorithm_mismatch")
    return {"manifest_sha256": CORE78_MANIFEST_SHA256,
            "catalog_fingerprint_sha256": CORE78_CATALOG_SHA256,
            "catalog_query": catalog["query_utf8"], "component_fingerprints": components,
            "identity_version": m["identity"]["version"],
            "identity_fingerprint_sha256": m["identity"]["fingerprint_sha256"]}


def validate_inventory(inventory):
    # FIN.validate_inventory also invokes the fresh69 source seal. Do not call
    # it here or weaken it; the actual catalog is independently pinned to 78.
    need(type(inventory) is dict and set(inventory) == {"tables", "sequences"},
         "current_keys_inventory_invalid")
    for names in inventory.values():
        need(type(names) is list and names == sorted(set(names)), "current_keys_inventory_invalid")
        for name in names:
            F.relation(name)
    need(set(TABLES) | {"core_migration.import_runs", "core_identity.users",
                       "core_meta.component_contract", "core_meta.schema_contract",
                       "core_meta.runtime_contract"} <= set(inventory["tables"]),
         "current_keys_inventory_incomplete")
    need(set(SEQUENCES) <= set(inventory["sequences"]), "current_keys_sequences_missing")


def state_sql(inventory, migration_id, *, inserted=None):
    """Same duplicate-preserving row-hash algorithm as FIN, without fresh69.

    `inserted` removes exactly this batch's new identities for preservation
    comparison. It never exempts the entire credentials/budgets table.
    """
    validate_inventory(inventory)
    exclusions = {}
    if inserted is not None:
        cids = [r["target_credential_id"] for r in inserted["records"]]
        bids = [inserted["first_budget_id"] + n for n in range(len(cids))]
        for values in (cids, bids):
            need(values and all(type(v) is int and 0 < v <= Q.BIGINT_MAX for v in values),
                 "current_keys_inserted_identity_invalid")
        cv, bv = ",".join(map(str, cids)), ",".join(map(str, bids))
        exclusions = {TABLES[0]: f"t.id NOT IN ({cv})", TABLES[1]: f"t.credential_id NOT IN ({cv})",
                      TABLES[2]: f"t.credential_id NOT IN ({cv})", TABLES[3]: f"t.id NOT IN ({bv})",
                      TABLES[4]: f"t.key_id IS NULL OR t.key_id NOT IN ({cv})"}
    rows = []
    for name in inventory["tables"]:
        where = exclusions.get(name)
        if name == "core_migration.import_runs":
            where = "t.migration_id<>" + literal(migration_id)
        rows.append(f"""SELECT {literal(name)} AS name,jsonb_build_object('rows',count(*),'sha256',
 encode(sha256(coalesce(string_agg(sha256(convert_to(to_jsonb(t)::text,'UTF8')),
 ''::bytea ORDER BY convert_to(to_jsonb(t)::text,'UTF8')),''::bytea)),'hex')) AS value
 FROM {F.relation(name)} t{" WHERE (" + where + ")" if where else ""}""")
    seq = []
    for name in inventory["sequences"]:
        seq.append(f"""SELECT {literal(name)} AS name,jsonb_build_object(
 'last_value',s.last_value,'is_called',s.is_called,'start',p.seqstart,'increment',p.seqincrement,
 'min',p.seqmin,'max',p.seqmax,'cache',p.seqcache,'cycle',p.seqcycle) AS value
 FROM {F.relation(name)} s CROSS JOIN pg_sequence p WHERE p.seqrelid={literal(name)}::regclass""")
    return ("SELECT jsonb_build_object('tables',(SELECT jsonb_object_agg(name,value) FROM (" +
            " UNION ALL ".join(rows) + ") t),'sequences',(SELECT jsonb_object_agg(name,value) FROM (" +
            " UNION ALL ".join(seq) + ") s),'inventory',(" + F.inventory_sql() + "))")


def snapshot_sql(contract, inventory, migration_id, admin_bindings=()):
    """Private read-only preview; no sequence calls, credentials or money writes."""
    validate_inventory(inventory)
    need(I.MIGRATION_ID.fullmatch(migration_id) is not None, "current_keys_migration_id_invalid")
    return """BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC'; SET LOCAL row_security=off;
SELECT jsonb_build_object(
 'instance',(""" + F.instance_sql() + "),'catalog_fingerprint_sha256',(" + I.catalog_hash_sql(contract) + ")," + \
        "'ownership_sha256',(" + F.ownership_sql() + ")," + \
        "'component_fingerprints',(SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract)," + \
        "'identity_contract',(SELECT jsonb_build_object('version',version,'fingerprint',encode(fingerprint,'hex')) FROM core_meta.schema_contract WHERE singleton)," + \
        "'runtime_catalog',(SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton)," + \
        "'native_users',(" + C.native_users_sql() + ")," + \
        "'admin_activation_evidence',(" + C.LINEAGE.evidence_array_sql(admin_bindings) + ")," + \
        "'state',(" + state_sql(inventory, migration_id) + ")," + \
        "'max_ids',jsonb_build_object('credentials',(SELECT coalesce(max(id),0) FROM core_identity.credentials)," + \
        "'budgets',(SELECT coalesce(max(id),0) FROM core_charging.budgets))," + \
        "'key_imports',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY migration_id),'[]'::jsonb) FROM core_migration.import_runs r " + \
        "WHERE manifest->>'scope' IN (" + literal(SCOPE) + "," + literal(C.SCOPE) + ")));\nCOMMIT;\n"


def validate_snapshot(snapshot, contract):
    need(contract.get("manifest_sha256") == CORE78_MANIFEST_SHA256
         and contract.get("catalog_fingerprint_sha256") == CORE78_CATALOG_SHA256,
         "current_keys_contract_not_78")
    need(snapshot.get("catalog_fingerprint_sha256") == CORE78_CATALOG_SHA256
         and snapshot.get("runtime_catalog") == CORE78_CATALOG_SHA256
         and snapshot.get("component_fingerprints") == contract["component_fingerprints"]
         and len(contract["component_fingerprints"]) == 78
         and snapshot.get("identity_contract") == {"version": contract["identity_version"],
             "fingerprint": contract["identity_fingerprint_sha256"]}, "current_keys_target_not_exact_core78")
    need(HEX.fullmatch(snapshot["ownership_sha256"]) is not None, "current_keys_ownership_hash_invalid")
    instance = snapshot["instance"]
    C.alias_target(instance)
    need(instance["database_owner"] == instance["role"] != instance["session_user"]
         and instance["effective_role_can_login"] is False, "current_keys_owner_session_invalid")
    need(type(instance.get("system_identifier")) is str and instance["system_identifier"].isdigit(),
         "current_keys_system_identifier_missing")
    state = snapshot["state"]
    validate_inventory(state["inventory"])
    need(set(state["tables"]) == set(state["inventory"]["tables"])
         and set(state["sequences"]) == set(state["inventory"]["sequences"]),
         "current_keys_state_inventory_mismatch")
    for v in state["tables"].values():
        need(type(v.get("rows")) is int and v["rows"] >= 0
             and type(v.get("sha256")) is str and HEX.fullmatch(v["sha256"]),
             "current_keys_table_hash_invalid")


def namespace_key(namespace):
    need(type(namespace) is dict and set(namespace) == {"system_identifier", "database", "schema"},
         "current_keys_source_namespace_invalid")
    need(type(namespace["system_identifier"]) is str and namespace["system_identifier"].isdigit(),
         "current_keys_source_namespace_invalid")
    for field in ("database", "schema"):
        v = namespace[field]
        need(type(v) is str and 1 <= len(v.encode()) <= 63 and not any(ord(c) < 32 for c in v),
             "current_keys_source_namespace_invalid")
    return digest(namespace)


def tool_sources():
    return {**C.source_hashes(), Path(__file__).name: hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}


def validate_activation_bindings(target, snapshot):
    bindings = target.get(C.LINEAGE.KEY, [])
    need(snapshot.get("admin_activation_evidence", []) == [b["native_evidence"] for b in bindings],
         "current_keys_activation_evidence_stale")
    for binding in bindings:
        # evidence_array_sql observes each privileged owner's actual immutable
        # identity run, which can precede the new full source snapshot. Keep
        # the original lineage validator's instance and receipt binding.
        C.LINEAGE.bind_instance([binding], snapshot["instance"], binding["native_evidence"]["identity_import"])


def check_source_identity_conflicts(namespace, rows, receipts):
    nk = namespace_key(namespace)
    wanted = {r["source_token_id"] for r in rows}
    for run in receipts:
        manifest = run.get("manifest", {})
        if manifest.get("scope") == C.SCOPE:
            # The old receipt has no stable source namespace. Never assume it
            # represents another site or silently import over its identities.
            raise Error("current_keys_prior_legacy_import_requires_namespace_reconciliation")
        if manifest.get("scope") == SCOPE and manifest.get("source_namespace_sha256") == nk:
            old = manifest.get("mapping", [])
            need(not (wanted & {r["source_key_id"] for r in old}),
                 "current_keys_source_identity_already_imported")


def prepare_mechanics(source, target, review, *, snapshot, contract, migration_id,
                      source_namespace, runtime_roles, encryption_key=None):
    """Build an auditable private draft, not a production authorization.

    Caller must obtain source/target through the existing export-provenance
    adapter. The production CLI below does so. This pure function also serves
    generated synthetic fixtures and never touches a database.
    """
    validate_snapshot(snapshot, contract)
    need(type(migration_id) is str and I.MIGRATION_ID.fullmatch(migration_id),
         "current_keys_migration_id_invalid")
    need(type(runtime_roles) is list and runtime_roles and runtime_roles == sorted(set(runtime_roles))
         and all(type(r) is str and IDENTIFIER.fullmatch(r) for r in runtime_roles),
         "current_keys_runtime_roles_invalid")
    need(snapshot["instance"]["role"] not in runtime_roles
         and snapshot["instance"]["session_user"] not in runtime_roles, "current_keys_runtime_roles_invalid")
    namespace_key(source_namespace)
    revoke = C._revoke_unsupported(review)
    C.validate_wire_source(source, allow_revoked_unsupported=revoke)
    proposal = P.plan_api_keys(source, target)
    C.validate_review(review, source, target, proposal)
    need(snapshot["native_users"] == target["users"], "current_keys_target_projection_stale")
    validate_activation_bindings(target, snapshot)
    state = snapshot["state"]
    count = len(proposal["records"])
    first = C._next_id(state, SEQUENCES[0], count)
    first_budget = C._next_id(state, SEQUENCES[1], count)
    need(first > snapshot["max_ids"]["credentials"] and first_budget > snapshot["max_ids"]["budgets"],
         "current_keys_sequence_behind_existing_rows")
    rows = C._supported_rows(proposal, target, first, revoke_unsupported=revoke)
    check_source_identity_conflicts(source_namespace, rows, snapshot["key_imports"])
    raw = {r["id"]: r for r in source["tokens"]}
    for row in rows:
        token = raw[row["source_token_id"]]
        row["source_row_sha256"] = digest(token)
        row["source_deleted_at"] = token["deleted_at"]
        if not row["one_time_reveal"]:
            need(encryption_key is not None, "current_keys_encryption_key_required")
            row["encrypted_secret_hex"] = C.encrypt_secret(encryption_key, row["target_credential_id"],
                                                          row["target_user_id"], token["key"])
            C.verify_encrypted_secret(encryption_key, row, token["key"])
    receipt = C.mapping_receipt(source["source_archive_sha256"], rows)
    user_map = {r["source_user_id"]: {"user_id": r["target_user_id"],
                "account_id": r["target_personal_account_id"]} for r in target["source_target_id_map"]}
    quota = Q.build_plan_from_tokens(source["tokens"], user_map, receipt, migration_id=migration_id,
                                    source_archive_sha256=source["source_archive_sha256"])
    material = {"format_version": 1, "scope": SCOPE, "production_authorized": False,
                "production_blocker": BLOCKER, "migration_id": migration_id,
                "source_archive_sha256": source["source_archive_sha256"],
                "source_namespace": source_namespace, "source_namespace_sha256": namespace_key(source_namespace),
                "source_input_sha256": P.sha256(source), "source_review": review,
                "target_projection": target, "proposal": proposal, "records": rows,
                "credential_receipt": receipt, "quota_plan": quota, "first_budget_id": first_budget,
                "snapshot": snapshot, "before_state": state, "contract": contract,
                "runtime_roles": runtime_roles, "mapping_sha256": digest(receipt["records"]),
                "tool_source_hashes": tool_sources()}
    validate_mechanics(material)
    need(not any(r["key"] in canonical(material) for r in source["tokens"]), "current_keys_secret_echo")
    return material


def validate_mechanics(m):
    need(type(m) is dict and m.get("format_version") == 1 and m.get("scope") == SCOPE
         and m.get("production_authorized") is False and m.get("production_blocker") == BLOCKER,
         "current_keys_private_draft_required")
    validate_snapshot(m["snapshot"], m["contract"])
    need(m.get("tool_source_hashes") == tool_sources(), "current_keys_tool_source_changed")
    validate_activation_bindings(m["target_projection"], m["snapshot"])
    need(m["before_state"] == m["snapshot"]["state"], "current_keys_state_binding_mismatch")
    need(m["source_namespace_sha256"] == namespace_key(m["source_namespace"]),
         "current_keys_namespace_binding_mismatch")
    need(I.MIGRATION_ID.fullmatch(m["migration_id"]), "current_keys_migration_id_invalid")
    need(m["source_archive_sha256"] == m["proposal"]["source_archive_sha256"]
         and m["source_input_sha256"] == m["proposal"]["source_input_sha256"]
         and m["target_projection"]["users"] == m["snapshot"]["native_users"],
         "current_keys_source_target_binding_mismatch")
    rows = m["records"]
    need(type(rows) is list and 1 <= len(rows) <= C.MAX_ALIAS_KEYS, "current_keys_count_invalid")
    first = C._next_id(m["before_state"], SEQUENCES[0], len(rows))
    budget = C._next_id(m["before_state"], SEQUENCES[1], len(rows))
    need(m["first_budget_id"] == budget and first > m["snapshot"]["max_ids"]["credentials"]
         and budget > m["snapshot"]["max_ids"]["budgets"], "current_keys_sequence_binding_invalid")
    expected = C._supported_rows(m["proposal"], m["target_projection"], first,
                                 revoke_unsupported=C._revoke_unsupported(m["source_review"]))
    fields = {"encrypted_secret_hex", "source_row_sha256", "source_deleted_at"}
    need(len(rows) == len(expected) and all({k: v for k, v in a.items() if k not in fields} ==
         {k: v for k, v in b.items() if k not in fields} for a, b in zip(rows, expected)),
         "current_keys_mapper_result_modified")
    for row in rows:
        F.checked_hash(row["source_row_sha256"])
        need((P.deleted(row.get("source_deleted_at")) or bool(row.get("revoked_rekey_required"))) == row["revoked"],
             "current_keys_revocation_binding_invalid")
        encrypted = row["encrypted_secret_hex"]
        need((row["one_time_reveal"] and encrypted is None) or (not row["one_time_reveal"]
             and type(encrypted) is str and re.fullmatch(r"[0-9a-f]{88,600}", encrypted)
             and len(encrypted) % 2 == 0), "current_keys_ciphertext_invalid")
    receipt = C.mapping_receipt(m["source_archive_sha256"], rows)
    need(m["credential_receipt"] == receipt and m["mapping_sha256"] == digest(receipt["records"]),
         "current_keys_mapping_receipt_modified")
    Q.validate_key_quota_plan(m["quota_plan"])
    need(m["quota_plan"]["migration_id"] == m["migration_id"]
         and m["quota_plan"]["source_archive_sha256"] == m["source_archive_sha256"],
         "current_keys_quota_batch_binding_invalid")
    expected_quota = [{"source_key_id": r["source_token_id"], "target_credential_id": r["target_credential_id"],
        "target_user_id": r["target_user_id"], "target_account_id": r["target_personal_account_id"],
        "used_units": r["quota"]["opening_consumed"], "remaining_units": r["quota"]["remaining"],
        "unlimited": r["quota"]["unlimited"], "source_secret_sha256": r["digest_sha256"],
        "source_status": r["source_status"], "source_row_sha256": r["source_row_sha256"]} for r in rows]
    need(m["quota_plan"]["key_records"] == expected_quota, "current_keys_quota_mapping_modified")


def _guard_sql(m):
    t, contract = m["snapshot"], m["contract"]
    roles = ",".join(literal(r) for r in m["runtime_roles"])
    return f"""
 PERFORM pg_stat_clear_snapshot();
 IF ({F.instance_sql()}) IS DISTINCT FROM {literal(t['instance'])}::jsonb THEN
  RAISE EXCEPTION 'current_keys_target_instance_changed'; END IF;
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())
  AND pid<>pg_backend_pid()) THEN RAISE EXCEPTION 'current_keys_target_has_other_clients'; END IF;
 IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=current_user AND NOT rolcanlogin AND NOT rolsuper
  AND NOT rolcreaterole AND NOT rolcreatedb AND NOT rolreplication AND NOT rolbypassrls)
 OR NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=session_user AND rolcanlogin AND NOT rolsuper
  AND NOT rolcreaterole AND NOT rolcreatedb AND NOT rolreplication AND NOT rolbypassrls)
 OR (SELECT count(*) FROM pg_roles WHERE rolname IN ({roles}) AND NOT rolcanlogin AND NOT rolsuper
  AND NOT rolcreaterole AND NOT rolcreatedb AND NOT rolreplication AND NOT rolbypassrls)<>{len(m['runtime_roles'])}
 OR NOT pg_has_role(session_user,current_user,'MEMBER')
 OR EXISTS(SELECT 1 FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user
  AND has_database_privilege(oid,current_database(),'CONNECT'))
 THEN RAISE EXCEPTION 'current_keys_offline_roles_invalid'; END IF;
 IF EXISTS(SELECT 1 FROM pg_namespace n WHERE {F.TABLE_FILTER} AND n.nspname<>'public'
  AND pg_get_userbyid(n.nspowner)<>current_user)
 OR EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {F.TABLE_FILTER}
  AND pg_get_userbyid(c.relowner)<>current_user)
 OR EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE {F.TABLE_FILTER}
  AND pg_get_userbyid(p.proowner)<>current_user)
 THEN RAISE EXCEPTION 'current_keys_owner_mismatch'; END IF;
 IF ({I.catalog_hash_sql(contract)}) IS DISTINCT FROM {literal(CORE78_CATALOG_SHA256)}
 OR ({F.ownership_sql()}) IS DISTINCT FROM {literal(t['ownership_sha256'])}
 OR (SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract)
  IS DISTINCT FROM {literal(contract['component_fingerprints'])}::jsonb
 OR NOT EXISTS(SELECT 1 FROM core_meta.runtime_contract WHERE singleton AND
  encode(catalog_fingerprint,'hex')={literal(CORE78_CATALOG_SHA256)})
 OR NOT EXISTS(SELECT 1 FROM core_meta.schema_contract WHERE singleton AND version={contract['identity_version']}
  AND encode(fingerprint,'hex')={literal(contract['identity_fingerprint_sha256'])})
 OR ({F.inventory_sql()}) IS DISTINCT FROM {literal(m['before_state']['inventory'])}::jsonb
 THEN RAISE EXCEPTION 'current_keys_target_contract_changed'; END IF;
"""


def _insert_sql(m):
    out = []
    arr = lambda vv: "ARRAY[" + ",".join(literal(v) for v in vv) + "]::text[]"
    for offset, row in enumerate(m["records"]):
        cid, uid, aid = row["target_credential_id"], row["target_user_id"], row["target_personal_account_id"]
        bid, q = m["first_budget_id"] + offset, row["quota"]
        revoked = (literal(row["source_deleted_at"]) + "::timestamptz" if P.deleted(row["source_deleted_at"])
                   else f"to_timestamp({m['proposal']['snapshot_at']})" if row["revoked"] else "NULL")
        cipher = "NULL" if row["one_time_reveal"] else "decode(" + literal(row["encrypted_secret_hex"]) + ",'hex')"
        access = f"to_timestamp({row['accessed_time']})" if row["accessed_time"] else "NULL"
        b = lambda v: "TRUE" if v else "FALSE"
        display = str(q["display_limit_credits"]) if q["unlimited"] else "NULL"
        # IDs are forecast only so AES-GCM can bind the new credential ID. The
        # transaction obtains every one from the sequence and checks the CAS.
        # A sequence mismatch fails closed; no rewinding/resetting is attempted.
        out.append(f"""
 IF nextval({literal(SEQUENCES[0])}::regclass)<>{cid} THEN
  RAISE EXCEPTION 'current_keys_credential_allocation_changed'; END IF;
 IF nextval({literal(SEQUENCES[1])}::regclass)<>{bid} THEN
  RAISE EXCEPTION 'current_keys_budget_allocation_changed'; END IF;
 INSERT INTO core_identity.credentials(id,digest,kind,user_id,user_version,owner_account_id,
 personal_billing_preference,expires_at,revoked_at,created_at,session_started_at) OVERRIDING SYSTEM VALUE
 VALUES({cid},decode({literal(row['digest_sha256'])},'hex'),'api_key',{uid},{row['target_auth_version']},{aid},
 {literal(row['funding']['personal_billing_preference'])},to_timestamp({row['expiry']['native_exclusive_epoch']}),
 {revoked},to_timestamp({row['created_time']}),to_timestamp({row['created_time']}));
 INSERT INTO core_identity.key_funding_rules(credential_id,position,payer_account_id) VALUES({cid},0,{aid});
 INSERT INTO core_charging.budgets(id,scope,key_id,issuer_user_id,period,limit_credits,unlimited,display_limit_credits)
 OVERRIDING SYSTEM VALUE VALUES({bid},'key',{cid},{uid},'lifetime',{q['limit_credits']},{b(q['unlimited'])},{display});
 INSERT INTO core_charging.budget_opening_consumption(scope,key_id,period,start_at,end_at,consumed_credits,import_reference)
 VALUES('key',{cid},'lifetime',{Q.LIFETIME_START},{Q.LIFETIME_END},{q['opening_consumed']},
 {literal('key-quota:' + m['migration_id'] + ':' + str(cid))});
 INSERT INTO core_identity.api_key_settings(credential_id,name,enabled,one_time_reveal,encrypted_secret,
 model_limits_enabled,model_limits,allow_ips,group_name,auto_groups,cross_group_retry,account_balance_read,accessed_at)
 VALUES({cid},{literal(row['name'])},{b(row['enabled'])},{b(row['one_time_reveal'])},{cipher},FALSE,
 {arr(row['model_limits'])},{arr(row['allow_ips'])}::inet[],{literal(row['native_effective_group'])},
 {arr(row['auto_groups']['values'])},{b(row['cross_group_retry'])},{b(row['account_balance_read'])},{access});
""")
    return "\n".join(out)


def _readback_sql(m):
    # Reuse exact field-level checks, but never the old empty-table coverage
    # assertion. That assertion is intentionally left unchanged in C.
    checks = []
    for offset, row in enumerate(m["records"]):
        # C._readback_sql has no per-row-only interface. A one-record material
        # binds its native budget ID then removes only its final total-table
        # check. Refuse unexpected helper structure instead of relaxing checks.
        one = {"records": [row], "migration_id": m["migration_id"], "proposal": m["proposal"],
               "before_state": deepcopy(m["before_state"])}
        seq = one["before_state"]["sequences"][SEQUENCES[1]]
        seq["last_value"] = m["first_budget_id"] + offset
        seq["is_called"] = False
        raw = C._readback_sql(one)
        marker = " IF (SELECT count(*) FROM " + F.relation(TABLES[0]) + ")<>1"
        prefix, separator, suffix = raw.partition(marker)
        need(bool(separator) and "credential_row_coverage_mismatch" in suffix
             and suffix.count("credential_row_coverage_mismatch") == 1,
             "current_keys_shared_readback_interface_changed")
        checks.append(prefix)
    return "\n".join(checks)


def _test_fixture_guard(m, nonce):
    """Not an apply bypass: SQL is constrained to a marked synthetic database."""
    instance = m["snapshot"]["instance"]
    need(type(nonce) is str and HEX.fullmatch(nonce), "current_keys_fixture_nonce_invalid")
    need(re.fullmatch(r"lmm_runtime_keys_test_[a-z0-9_]{1,30}", instance["database"]),
         "current_keys_synthetic_database_required")
    return f"""
 IF current_database()<>{literal(instance['database'])}
 OR shobj_description((SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database')
  IS DISTINCT FROM {literal('lmm-runtime-keys-synthetic:' + nonce)}
 THEN RAISE EXCEPTION 'current_keys_synthetic_fixture_identity_changed'; END IF;
"""


def _mechanics_transaction_sql(m, fixture_guard):
    validate_mechanics(m)
    inv = m["before_state"]["inventory"]
    state = state_sql(inv, m["migration_id"])
    preserved = state_sql(inv, m["migration_id"], inserted=m)
    locks = ",".join(F.relation(n) for n in inv["tables"])
    mapped_ids = ",".join(str(r["source_token_id"]) for r in m["records"])
    expected_sequences = deepcopy(m["before_state"]["sequences"])
    for sequence, first in zip(SEQUENCES, (m["records"][0]["target_credential_id"], m["first_budget_id"])):
        expected_sequences[sequence]["last_value"] = first + len(m["records"]) - 1
        expected_sequences[sequence]["is_called"] = True
    body = f"""
DECLARE m jsonb := {literal(m)}::jsonb; previous core_migration.import_runs%ROWTYPE;
 before_state jsonb; post_state jsonb; preserved_state jsonb;
BEGIN
 {fixture_guard}
 {_guard_sql(m)}
 LOCK TABLE {locks} IN ACCESS EXCLUSIVE MODE;
 {_guard_sql(m)}
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->>'source_archive_sha256'
   OR previous.mapping_sha256<>m->>'mapping_sha256'
   OR previous.manifest->>'scope'<>{literal(SCOPE)}
   OR previous.manifest->'material' IS DISTINCT FROM m
   OR previous.manifest->'mapping' IS DISTINCT FROM m->'credential_receipt'->'records'
   OR previous.manifest->>'source_namespace_sha256' IS DISTINCT FROM m->>'source_namespace_sha256'
  THEN RAISE EXCEPTION 'current_keys_replay_conflict'; END IF;
  {_readback_sql(m)}
  IF previous.manifest->'post_state' IS DISTINCT FROM ({state})
  THEN RAISE EXCEPTION 'current_keys_replay_state_changed'; END IF;
 ELSE
  SELECT ({state}) INTO before_state;
  IF before_state IS DISTINCT FROM m->'before_state' THEN
   RAISE EXCEPTION 'current_keys_target_state_changed'; END IF;
  IF ({C.native_users_sql()}) IS DISTINCT FROM m->'target_projection'->'users'
   OR ({C.LINEAGE.evidence_array_sql(m['target_projection'].get(C.LINEAGE.KEY, []))})
    IS DISTINCT FROM {literal(m['snapshot'].get('admin_activation_evidence', []))}::jsonb THEN
   RAISE EXCEPTION 'current_keys_native_authority_changed'; END IF;
  IF EXISTS(SELECT 1 FROM core_migration.import_runs WHERE manifest->>'scope'={literal(C.SCOPE)}) THEN
   RAISE EXCEPTION 'current_keys_prior_legacy_import_requires_namespace_reconciliation'; END IF;
  IF EXISTS(SELECT 1 FROM core_migration.import_runs r,
   LATERAL jsonb_array_elements(CASE WHEN r.manifest->>'scope'={literal(SCOPE)}
    THEN r.manifest->'mapping' ELSE '[]'::jsonb END) x
   WHERE r.manifest->>'scope'={literal(SCOPE)}
   AND r.manifest->>'source_namespace_sha256'=m->>'source_namespace_sha256'
   AND (x->>'source_key_id')::bigint IN ({mapped_ids})) THEN
   RAISE EXCEPTION 'current_keys_source_identity_already_imported'; END IF;
  IF EXISTS(SELECT 1 FROM core_identity.credentials c JOIN jsonb_array_elements(m->'records') r
   ON c.digest=decode(r->>'digest_sha256','hex')) THEN
   RAISE EXCEPTION 'current_keys_existing_secret_collision'; END IF;
  IF (SELECT coalesce(max(id),0) FROM core_identity.credentials)>={m['records'][0]['target_credential_id']}
   OR (SELECT coalesce(max(id),0) FROM core_charging.budgets)>={m['first_budget_id']} THEN
   RAISE EXCEPTION 'current_keys_sequence_behind_existing_rows'; END IF;
  -- All source/native CAS, namespace, collision, role, catalog and fixture
  -- guards above run before the first non-transactional sequence allocation.
  {_insert_sql(m)}
  SET CONSTRAINTS ALL IMMEDIATE;
  {_readback_sql(m)}
  SELECT ({preserved}) INTO preserved_state;
  IF preserved_state->'tables' IS DISTINCT FROM before_state->'tables'
   OR preserved_state->'inventory' IS DISTINCT FROM before_state->'inventory'
   OR preserved_state->'sequences' IS DISTINCT FROM {literal(expected_sequences)}::jsonb THEN
   RAISE EXCEPTION 'current_keys_preexisting_state_not_preserved'; END IF;
  SELECT ({state}) INTO post_state;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->>'source_archive_sha256',m->>'mapping_sha256',
   jsonb_build_object('scope',{literal(SCOPE)},'source_namespace_sha256',m->>'source_namespace_sha256',
    'mapping',m->'credential_receipt'->'records','material',m,'post_state',post_state));
 END IF;
 {_guard_sql(m)}
END
"""
    tag = "$lmm_current_keys_" + hashlib.sha256(body.encode()).hexdigest()[:24] + "$"
    need(tag not in body, "current_keys_sql_delimiter_collision")
    return f"""SET standard_conforming_strings=on;
SET timezone='UTC'; SET lock_timeout='5s'; SET statement_timeout='120s';
DO $fence$ BEGIN IF NOT pg_try_advisory_lock({FENCE}) THEN
 RAISE EXCEPTION 'current_keys_import_busy'; END IF; END $fence$;
BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on; SET LOCAL row_security=off;
SET LOCAL search_path=pg_catalog,public;
DO {tag}{body}{tag};
COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{literal(m['migration_id'])},
 'material_sha256',{literal(digest(m))});
"""


def build_synthetic_transaction_sql(material, fixture_nonce):
    """For the separate PG test driver, never a production CLI option."""
    return _mechanics_transaction_sql(material, _test_fixture_guard(material, fixture_nonce))


def build_production_transaction_sql(material, evidence):
    # MUST remain before invoking the allocator/generator. No partial receipt,
    # arbitrary digest, reviewed="approved", or synthetic nonce can satisfy it.
    require_current_reconciliation(material, evidence)
    raise Error("current_keys_production_execution_not_connected")


def build_resolve_sql(m, original_application_name):
    """Read-only unknown-commit resolution. Receipt absence remains UNKNOWN.

    Resolving COMMITTED acknowledges the immutable transaction and exact key
    mapping. Later legitimate use may change unrelated data; the resolver does
    not require the whole target to still equal the transaction's post-state.
    A changed imported-key policy is reported as conflict for manual review.
    """
    validate_mechanics(m)
    need(type(original_application_name) is str and re.fullmatch(r"lmm-current-keys-[0-9a-f]{32}",
         original_application_name), "current_keys_attempt_id_invalid")
    instance = m["snapshot"]["instance"]
    body = f"""
DECLARE previous core_migration.import_runs%ROWTYPE;
BEGIN
 IF ({F.instance_sql()}) IS DISTINCT FROM {literal(instance)}::jsonb THEN
  RAISE EXCEPTION 'current_keys_resolve_instance_changed'; END IF;
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())
  AND application_name={literal(original_application_name)} AND pid<>pg_backend_pid()) THEN
  RAISE EXCEPTION 'current_keys_original_attempt_still_active'; END IF;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id={literal(m['migration_id'])};
 IF FOUND THEN
  IF previous.manifest->'material' IS DISTINCT FROM {literal(m)}::jsonb
   OR previous.manifest->>'scope' IS DISTINCT FROM {literal(SCOPE)}
   OR previous.source_archive_sha256<>{literal(m['source_archive_sha256'])}
   OR previous.mapping_sha256<>{literal(m['mapping_sha256'])}
   OR previous.manifest->'mapping' IS DISTINCT FROM {literal(m['credential_receipt']['records'])}::jsonb
   OR previous.manifest->>'source_namespace_sha256' IS DISTINCT FROM {literal(m['source_namespace_sha256'])}
  THEN RAISE EXCEPTION 'current_keys_resolve_receipt_conflict'; END IF;
  {_readback_sql(m)}
 END IF;
END
"""
    tag = "$lmm_resolve_" + hashlib.sha256(body.encode()).hexdigest()[:24] + "$"
    need(tag not in body, "current_keys_sql_delimiter_collision")
    return f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC'; SET LOCAL row_security=off;
DO {tag}{body}{tag};
SELECT jsonb_build_object('status',CASE WHEN EXISTS(SELECT 1 FROM core_migration.import_runs
 WHERE migration_id={literal(m['migration_id'])}) THEN 'COMMITTED' ELSE 'UNKNOWN' END,
 'migration_id',{literal(m['migration_id'])},'material_sha256',{literal(digest(m))},
 'automatic_retry_allowed',false);
COMMIT;
"""


def public_summary(m):
    return {"status": "BLOCKED", "production_authorized": False, "blockers": [BLOCKER],
            "scope": SCOPE, "migration_id": m["migration_id"], "keys": len(m["records"]),
            "source_token_id_reused": False, "native_credentials_preserved": m["before_state"]["tables"][TABLES[0]]["rows"],
            "native_budgets_preserved": m["before_state"]["tables"][TABLES[3]]["rows"],
            "material_sha256": digest(m), "required_evidence": list(REQUIRED_EVIDENCE)}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="command", required=True)
    sub.add_parser("readiness", help="Print the unfulfilled production authority contract; no I/O")
    preview = sub.add_parser("preview", help="Read private source evidence and a frozen read-only target snapshot")
    for name in ("source-receipt", "export-proof", "runtime-observation", "target-projection", "source-review",
                 "target-snapshot", "core78-manifest", "source-namespace", "runtime-roles", "output"):
        preview.add_argument("--" + name, required=True, type=Path)
    preview.add_argument("--snapshot-at", required=True, type=int)
    preview.add_argument("--migration-id", required=True)
    preview.add_argument("--encryption-key", type=Path)
    args = p.parse_args(argv)
    try:
        if args.command == "readiness":
            print(json.dumps({"status": "BLOCKED", "production_authorized": False,
                              "blockers": [BLOCKER], "required_evidence": REQUIRED_EVIDENCE}))
            return 2
        source, _proof = P.source_export_provenance_cli(args.source_receipt, args.export_proof,
            args.runtime_observation, snapshot_at=args.snapshot_at)
        target, review = C.load_json(args.target_projection), C.load_json(args.source_review)
        snapshot = C.load_json(args.target_snapshot)
        contract = core78_contract(C.private_bytes(args.core78_manifest))
        m = prepare_mechanics(source, target, review, snapshot=snapshot, contract=contract,
            migration_id=args.migration_id, source_namespace=C.load_json(args.source_namespace),
            runtime_roles=json.loads(C.private_bytes(args.runtime_roles)),
            encryption_key=C.private_bytes(args.encryption_key) if args.encryption_key else None)
        args.output.mkdir(mode=0o700, parents=True, exist_ok=True)
        F.write_private(args.output, "current-runtime-keys.private.json", canonical(m) + "\n")
        summary = public_summary(m)
        F.write_private(args.output, "current-runtime-keys.public.json", canonical(summary) + "\n")
        print(json.dumps(summary))
        return 2  # A useful preview is not permission to apply.
    except (Error, C.Error, P.ProposalError, I.MappingError, Q.IDENTITY.MappingError, OSError, ValueError, KeyError) as exc:
        # Do not echo malformed private inputs, bearer bytes, or connection details.
        code = str(exc) if re.fullmatch(r"[A-Za-z0-9_]+", str(exc)) else "current_keys_private_input_invalid"
        print(json.dumps({"status": "BLOCKED", "blockers": [code], "production_authorized": False}))
        return 2


if __name__ == "__main__":
    sys.exit(main())

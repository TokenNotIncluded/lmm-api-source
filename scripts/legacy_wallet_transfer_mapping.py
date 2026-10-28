"""Exact legacy transfer facts for offline review; never an import or approval.

Call build(F, ...) with the existing financial planner and its verified frozen
export, proof, plan and identity records. The caller must verify the export bytes
and identity provenance first; supplied hashes are bindings, not attestations.
verify(F, mapping, ...) rebuilds against those same source inputs after readback.
No SQL, connections, secret sidecar, ledger keys or target transfer IDs are made.
"""

from __future__ import annotations

import hashlib


SCOPE = "legacy-wallet-transfer-source-mapping-only-v1"
TABLE = "wallet_transfers"
# Unicode White_Space used by Go strings.TrimSpace and Rust str::trim.
# Python str.strip additionally removes U+001C..U+001F and would change the gate.
SOURCE_SPACE = (
    "\t\n\v\f\r \u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005"
    "\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000"
)
ACTIVATION_BLOCKERS = (
    "legacy_transfer_import_contract_missing",
    "source_unit_and_economic_approval_missing",
    "stable_source_namespace_authority_unverified",
)


def _utf8(F, value, code):
    F.require(type(value) is str, code)
    try:
        return value.encode("utf-8", errors="strict")
    except UnicodeEncodeError:
        raise F.MappingError(code) from None


def _hash(F, value, code):
    F.require(type(value) is str and F.IDENTITY.HEX.fullmatch(value) is not None, code)
    return value


def _source(F, *, tables, plan, source_proof):
    """Rebind every transfer, including terminal rows and an explicitly empty table."""
    F.verify_plan_mapping(plan)
    archive = _hash(F, plan.get("source_archive_sha256"), "wallet_transfer_archive_hash_invalid")
    F.require(type(source_proof) is dict
              and source_proof.get("source_archive_sha256") == archive,
              "wallet_transfer_source_archive_mismatch")
    _hash(F, source_proof.get("source_export_sha256"), "wallet_transfer_export_hash_invalid")
    F.require(source_proof.get("source_column_coverage_exact") is True
              and source_proof.get("source_row_baseline_exact") is True,
              "wallet_transfer_export_coverage_unproven")
    F.require(type(tables) is dict and TABLE in tables and type(tables[TABLE]) is list
              and all(type(row) is dict for row in tables[TABLE]),
              "wallet_transfer_source_table_missing_or_invalid")
    rows = tables[TABLE]
    evidence = plan.get("source_evidence", {}).get(TABLE)
    F.require(type(evidence) is dict, "wallet_transfer_plan_table_missing")
    count = source_proof.get("selected_table_counts", {}).get(TABLE)
    try:
        row_hashes = [F.digest(row) for row in rows]
    except UnicodeEncodeError:
        raise F.MappingError("wallet_transfer_source_utf8_invalid") from None
    F.require(type(count) is int and count == len(rows) == evidence.get("row_count")
              and row_hashes == evidence.get("row_hashes"),
              "wallet_transfer_source_rows_changed")
    F.require(source_proof.get("source_primary_keys", {}).get(TABLE) == ["id"]
              and evidence.get("primary_key_columns") == ["id"],
              "wallet_transfer_primary_key_contract_missing")
    columns = source_proof.get("source_column_schema", {}).get(TABLE)
    F.require(type(columns) is list and F.digest(columns)
              == F.digest(evidence.get("source_column_schema")),
              "wallet_transfer_source_catalog_mismatch")
    records = [{"source_primary_key": {"id": row.get("id")}, "source_export_ordinal": ordinal,
                "source_row_sha256": F.digest(row)} for ordinal, row in enumerate(rows)]
    F.require(F.digest(records) == F.digest(evidence.get("source_records")),
              "wallet_transfer_source_record_binding_mismatch")
    return rows


def build(F, *, tables, identity_records, plan, source_proof, source_namespace):
    """Return all source mappings or reject; never silently skip a bad row.

    Invalid amounts, keys, credentials, duplicate identities and broken source
    bindings raise MappingError with a value-free code. Lifecycle contradictions
    remain visible as row blockers and block the whole mapping. Even a complete
    mapping has can_apply=False: namespace authority, units, schema, receipts and
    economic approval are outside this source-only contract.
    """
    namespace = _utf8(F, source_namespace, "wallet_transfer_source_namespace_invalid")
    F.require(1 <= len(namespace) <= 128 and "\0" not in source_namespace
              and source_namespace.strip(SOURCE_SPACE) == source_namespace,
              "wallet_transfer_source_namespace_invalid")
    rows = _source(F, tables=tables, plan=plan, source_proof=source_proof)
    F.require(type(identity_records) is list and all(
        type(row) is dict and set(row) == {
            "source_user_id", "target_user_id", "target_personal_account_id"}
        for row in identity_records), "wallet_transfer_identity_records_invalid")
    identities = F.identity_index(identity_records)
    F.require(F.digest(identities) == plan.get("identity_map_sha256"),
              "wallet_transfer_identity_plan_mismatch")
    projected = F.map_wallet_transfers(rows, identities)
    F.require(F.digest(projected) == F.digest(plan.get("wallet_transfers")),
              "wallet_transfer_plan_projection_mismatch")
    facts = {row["source_transfer_id"]: row for row in projected}
    records, seen_keys, seen_tokens = [], set(), set()
    for ordinal, row in enumerate(rows):
        fact = facts[row["id"]]
        amount = F.integer(row["quota"], "wallet_transfer_amount_out_of_range",
                           minimum=1, maximum=F.JS_SAFE_MAX)
        key = _utf8(F, row.get("request_key"), "wallet_transfer_request_key_invalid")
        F.require(16 <= len(key) <= 64 and b"\0" not in key
                  and row["request_key"].strip(SOURCE_SPACE) == row["request_key"],
                  "wallet_transfer_request_key_invalid")
        token = _utf8(F, row.get("token"), "wallet_transfer_token_invalid")
        F.require(len(token) == 64 and all(byte in b"0123456789abcdefABCDEF" for byte in token),
                  "wallet_transfer_token_invalid")
        token_hash = hashlib.sha256(token).hexdigest()
        owner_key = (row["sender_id"], key)
        F.require(owner_key not in seen_keys, "wallet_transfer_owner_request_key_duplicate")
        F.require(token_hash not in seen_tokens, "wallet_transfer_token_duplicate")
        seen_keys.add(owner_key)
        seen_tokens.add(token_hash)
        for field in ("created_at", "claimed_at", "cancelled_at"):
            F.integer(row[field], "wallet_transfer_time_out_of_range",
                      minimum=int(field == "created_at"), maximum=F.JS_SAFE_MAX)
        blockers = list(fact["blockers"])
        for state, field in (("claimed", "claimed_at"), ("cancelled", "cancelled_at")):
            if row["status"] == state and 0 < row[field] < row["created_at"]:
                blockers.append("wallet_transfer_terminal_time_precedes_creation")
        origin = {"source_namespace": source_namespace, "source_table": TABLE,
                  "source_primary_key": {"id": row["id"]}}
        records.append({
            "source_origin": origin, "source_origin_sha256": F.digest(origin),
            "source_export_ordinal": ordinal, "source_row_sha256": F.digest(row),
            "source_sender_id": row["sender_id"], "owner_user": row["sender_id"],
            "payer_account": identities.get(row["sender_id"]),
            "source_recipient_id": row["recipient_id"],
            "recipient_user": row["recipient_id"] or None,
            "recipient_account": identities.get(row["recipient_id"]),
            "request_key_utf8_hex": key.hex(), "request_key_utf8_length": len(key),
            "token_digest_sha256": token_hash, "amount_units": amount,
            "source_status": row["status"], "created_at": row["created_at"],
            "claimed_at": row["claimed_at"], "cancelled_at": row["cancelled_at"],
            "source_pending_units": amount if row["status"] == "pending" else 0,
            "sender_available_already_debited": True,
            "wallet_debit_to_apply": 0, "wallet_credit_to_apply": 0, "reserved_units_to_apply": 0,
            "original_bearer_activation_allowed": False,
            "disposition": ("rejected-source-row" if blockers else
                            "pending-right-review-only" if row["status"] == "pending" else
                            "terminal-history-review-only"),
            "blockers": blockers,
        })
    records.sort(key=lambda row: row["source_origin"]["source_primary_key"]["id"])
    source_blockers = sorted({code for row in records for code in row["blockers"]})
    activation_blockers = list(ACTIVATION_BLOCKERS)
    if any(row["source_status"] == "pending" for row in records):
        activation_blockers.append("pending_transfer_claim_contract_missing")
    material = {
        "format_version": 1, "scope": SCOPE,
        "source_archive_sha256": plan["source_archive_sha256"],
        "mapping_sha256": plan["mapping_sha256"],
        "source_export_sha256": source_proof["source_export_sha256"],
        "source_proof_sha256": F.digest(source_proof),
        "identity_records_sha256": F.digest(sorted(identity_records, key=lambda row: row["source_user_id"])),
        "source_namespace": source_namespace,
        "amount_policy": "preserve-source-integer-no-conversion",
        "source_mapping_complete": not source_blockers,
        "source_blockers": source_blockers, "activation_blockers": activation_blockers,
        "can_apply": False, "economic_approval_verified": False,
        "source_unit_provenance_verified": False, "source_namespace_authority_verified": False,
        "runtime_activation_authorized": False, "original_bearer_activation_allowed": False,
        "target_writes": 0, "ledger_writes": 0,
        "records": records,
    }
    material["source_mapping_sha256"] = F.digest(material)
    return material


def verify(F, mapping, **source_inputs):
    """Compare the complete persisted artifact to a fresh source derivation.

    Rehashing an altered artifact cannot approve it or remove a refusal. This
    proves equality to supplied source material, never source/target authority.
    """
    expected = build(F, **source_inputs)
    F.require(F.digest(mapping) == F.digest(expected), "wallet_transfer_mapping_mismatch")
    return True

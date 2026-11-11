#!/usr/bin/env python3
"""Offline historical balance-refund candidate / reviewed SQL, never DB access.

The source export must contain the COMPLETE order, transfer and refund tables
from the reviewed archive. Approval of that completeness belongs to the owner;
this tool never produces its own approval. Existing financial opening SQL must
have committed before the generated grant SQL can be accepted by Core.
"""
import argparse
import hashlib
import json
from pathlib import Path
import stat

import financial_import as F
import legacy_escrow_mapping as ESCROW
import legacy_extensions_import as STORE

ORDER_FIELDS = ("id", "buyer_id", "seller_id", "quantity", "unit_price_quota", "price_quota",
                "payment_method", "status", "paid_at", "amount_minor", "fee_quota", "fee_held")
SALE_FIELDS = ("id", "order_id", "kind", "from_user_id", "to_user_id", "quota")
SCOPE = "legacy-store-full-refund-candidate-only-v1"


def candidate(source, material, identities, input_hashes):
    F.require(set(source) == {"source_archive_sha256", "mapping_sha256", "orders", "transfers", "refunds"},
              "store_source_shape")
    F.require(material.get("scope") in (F.SCOPE, F.ESCROW_SCOPE), "store_financial_scope")
    for key in ("source_archive_sha256", "mapping_sha256"):
        F.checked_hash(source[key]); F.require(source[key] == material[key], "store_opening_source_mismatch")
    F.require(material["source_review"]["credits_per_usd"] == 500_000, "store_original_credit_unit_required")
    mapping = ESCROW._identities(F.PLANNER, identities, material)
    openings = {r["account_id"]: r for r in material["opening"]}
    observed = {r["observed_at"] for r in material["opening"]}
    F.require(len(observed) == 1, "store_opening_observation_mismatch")
    observed = observed.pop()
    F.PLANNER.integer(observed, "store_observed_invalid", minimum=1, maximum=253402300799)
    for table in ("orders", "transfers", "refunds"):
        F.require(type(source[table]) is list, "store_source_table_missing")
        ids = [r["id"] for r in source[table]]
        F.require(len(ids) == len(set(ids)), "store_source_duplicate")
    grants = []
    for order in sorted(source["orders"], key=lambda r: r["id"]):
        F.require(order["status"] in ("paid", "refunded"), "store_source_unsettled_order")
        if order["status"] == "refunded":
            continue  # Preserve history only: no fresh authorization or money.
        F.checked_hash(order["id"])
        projection = {key: order[key] for key in ORDER_FIELDS}
        F.require(order["payment_method"] == "balance" and order["amount_minor"] == 0
                  and order["fee_held"] is False, "store_balance_sale_required")
        amount = F.PLANNER.integer(order["price_quota"], "store_principal_invalid", minimum=1, maximum=F.PLANNER.JS_SAFE_MAX)
        qty = F.PLANNER.integer(order["quantity"], "store_quantity_invalid", minimum=1, maximum=F.PLANNER.JS_SAFE_MAX)
        unit = F.PLANNER.integer(order["unit_price_quota"], "store_unit_invalid", minimum=1, maximum=F.PLANNER.JS_SAFE_MAX)
        F.require(amount == unit * qty, "store_original_principal_mismatch")
        F.PLANNER.integer(order["paid_at"], "store_paid_time_invalid", minimum=1, maximum=observed)
        F.PLANNER.integer(order["fee_quota"], "store_fee_invalid", minimum=0, maximum=amount)
        parties = []
        for field in ("buyer_id", "seller_id"):
            uid = F.PLANNER.integer(order[field], "store_original_user_invalid", minimum=1)
            F.require(uid in mapping, "store_original_identity_missing")
            identity = mapping[uid]
            opening = openings.get(identity["target_personal_account_id"])
            F.require(opening is not None and opening["source_entity_key"] in
                      (f"user:{uid}", f"legacy.users/{uid}"), "store_original_opening_missing")
            parties.append(identity["target_user_id"])
        sales = [r for r in source["transfers"] if r["order_id"] == order["id"] and r["kind"] == "sale"]
        F.require(len(sales) == 1, "store_exact_original_sale_required")
        sale = sales[0]
        F.require((sale["from_user_id"], sale["to_user_id"], sale["quota"]) ==
                  (order["buyer_id"], order["seller_id"], amount), "store_original_sale_mismatch")
        refunds = [r for r in source["refunds"] if r["order_id"] == order["id"]]
        F.require(all(r["status"] in ("rejected", "cancelled") for r in refunds), "store_original_refund_cap_occupied")
        F.require(not any(r["order_id"] == order["id"] and r["kind"].startswith("refund")
                          for r in source["transfers"]), "store_original_refund_transfer_exists")
        grants.append({"migration_id": material["migration_id"], "source_archive_sha256": source["source_archive_sha256"],
                       "mapping_sha256": source["mapping_sha256"], "observed_at": observed,
                       "shop_id": STORE.legacy_shop_id(order["seller_id"]),
                       "source_row_sha256": hashlib.sha256(STORE.jdump(order).encode()).hexdigest(),
                       "source_sale_sha256": hashlib.sha256(STORE.jdump(sale).encode()).hexdigest(),
                       "source_order": projection, "source_sale": {k: sale[k] for k in SALE_FIELDS},
                       "source_refunds": [{"order_id": r["order_id"], "status": r["status"],
                                           "source_row_sha256": hashlib.sha256(STORE.jdump(r).encode()).hexdigest()} for r in refunds],
                       "buyer_user_id": parties[0], "seller_user_id": parties[1]})
    return {"scope": SCOPE, "inputs": input_hashes, "grants": grants,
            "history_only_refunded": sum(r["status"] == "refunded" for r in source["orders"])}


def reviewed_sql(value, review):
    F.require(set(review) == {"scope", "decision", "candidate_sha256", "complete_source_tables"}, "store_review_shape")
    F.require(review["scope"] == "legacy-store-full-refund-review-v1" and review["decision"] == "approved-owner-import"
              and review["complete_source_tables"] is True and review["candidate_sha256"] == F.digest(value),
              "store_review_not_bound")
    proof = F.digest(review)
    statements = ["BEGIN;", "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE;", "SET LOCAL synchronous_commit=on;",
                  f"SELECT pg_advisory_xact_lock({F.FENCE});"]
    for grant in value["grants"]:
        request = {**grant, "review_proof_sha256": proof}
        statements.append("SELECT core_commerce.import_legacy_refund_grant(" + STORE.lit(STORE.jdump(request)) + "::jsonb);")
    return "\n".join([*statements, "COMMIT;", ""])


def private_json(path, *, json_lines=False):
    info = path.lstat()
    F.require(stat.S_ISREG(info.st_mode) and info.st_mode & 0o077 == 0, "store_private_regular_input_required")
    raw = path.read_bytes()
    if json_lines and not raw.lstrip().startswith(b"["):
        rows = [F.PLANNER.parse_json(line) for line in raw.decode("utf-8").splitlines() if line]
        return rows, hashlib.sha256(raw).hexdigest()
    return json.loads(raw), hashlib.sha256(raw).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ("source-export", "financial-material", "identities", "output"):
        p.add_argument("--" + name, type=Path, required=True)
    p.add_argument("--review", type=Path)
    args = p.parse_args()
    source, source_hash = private_json(args.source_export)
    material, material_hash = private_json(args.financial_material)
    identities, identities_hash = private_json(args.identities, json_lines=True)
    value = candidate(source, material, identities, {"source_export_sha256": source_hash,
        "financial_material_sha256": material_hash, "identities_sha256": identities_hash})
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    target = args.output / "candidate.json"
    target.write_text(STORE.jdump(value) + "\n"); target.chmod(0o600)
    if args.review:
        review, _ = private_json(args.review)
        sql = args.output / "owner-import.sql"
        sql.write_text(reviewed_sql(value, review)); sql.chmod(0o600)
    # Candidate identity only: never print private source rows, tokens or SQL.
    print(STORE.jdump({"candidate_sha256": F.digest(value), "grants": len(value["grants"]),
                      "history_only_refunded": value["history_only_refunded"], "applied": False}))


if __name__ == "__main__":
    main()

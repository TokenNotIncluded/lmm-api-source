#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Offline, reviewed channel display openings. Never a wallet/charging writer.

Default operation is a file-only preview. Transport, private files, frozen source
parsing, catalog/state hashes and the offline owner fence reuse the existing
financial import tools. No SQL is submitted without an explicit apply/resolve.
An uncertain attempt stays UNKNOWN; resolution never resubmits its write.
"""

from __future__ import annotations

import argparse
from decimal import Decimal
from fractions import Fraction
from pathlib import Path
import re
import sys
import uuid

import financial_import as F
import legacy_site_statistics as S

IDENTITY, P = F.IDENTITY, F.PLANNER
require = F.require
CONTRACT = "legacy-channel-opening-v1"
DESTINATIONS = ("core_channels.opening_sources", "core_channels.opening_baselines")
BALANCES = "core_channels.balance_observations"
SAFE = (1 << 53) - 1
I64_MIN, I64_MAX = -(1 << 63), (1 << 63) - 1
MAX_TIME = 253402300799
FILES = {
    "archive",
    "source_receipt",
    "source_export",
    "source_export_proof",
    "mapping",
    "unit_evidence",
}
SOURCE_FIELDS = (
    "source_key",
    "archive_sha256",
    "export_sha256",
    "mapping_sha256",
    "unit_evidence_sha256",
    "source_revision",
    "observed_at",
    "native_overlap",
    "native_request_ids",
    "native_seal_sha256",
    "rows_sha256",
    "plan_sha256",
)
BASE_FIELDS = (
    "channel_id",
    "source_key",
    "source_channel_id",
    "raw_used_quota",
    "credits_per_usd",
    "offset_numerator",
    "offset_denominator",
    "historical_coverage",
    "kind",
    "parent_channel_id",
)


def integer(value, minimum=I64_MIN, maximum=I64_MAX):
    if type(value) is str and re.fullmatch(r"-?(0|[1-9][0-9]*)", value):
        value = int(value)
    require(
        type(value) is int and minimum <= value <= maximum, "channel_integer_invalid"
    )
    return value


def observed(value):
    if value is None:
        return None
    value = integer(value, 0, MAX_TIME)
    return None if value == 0 else value


def ids(values):
    require(
        type(values) is list
        and all(type(v) is str and 0 < len(v) <= 128 and "\0" not in v for v in values),
        "channel_request_ids_invalid",
    )
    require(
        values == sorted(set(values), key=lambda v: v.encode()),
        "channel_request_ids_not_canonical",
    )
    return values


def decimal_text(value):
    if value is None:
        return None
    require(type(value) in (int, str, Decimal), "channel_balance_not_exact")
    text = str(value)
    require(
        len(text) <= 256
        and Decimal(text).is_finite()
        and abs(Decimal(text)) <= Decimal("1.7976931348623157e308"),
        "channel_balance_invalid",
    )
    return text


def label(value):
    require(
        value is None
        or (
            type(value) is str
            and len(value) <= 32
            and not any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value)
        ),
        "channel_unit_label_invalid",
    )
    return value


def tool_hashes():
    paths = (
        "scripts/legacy_channel_accounting.py",
        "scripts/legacy_site_statistics.py",
        "services/core/schema/channel_openings.sql",
        "services/core/schema/channel_projection.sql",
        "services/core/src/core_schema.rs",
        "services/core/src/usage/channel_projection.rs",
    )
    return {
        **F.source_hashes(),
        **{p: IDENTITY.sha256_file(IDENTITY.ROOT / p) for p in paths},
    }


def load_source(files, *, inspect_only=False):
    """Read existing frozen owner exports, never reconnect to the source."""
    required = FILES - {"mapping", "unit_evidence"} if inspect_only else FILES
    require(
        type(files) is dict
        and required <= set(files)
        and all(k in FILES or k.startswith("evidence:") for k in files),
        "channel_source_files_invalid",
    )
    loaded, proofs = {}, set()
    for name, ref in files.items():
        require(
            type(ref) is dict and set(ref) == {"path", "sha256"},
            "channel_file_reference_invalid",
        )
        F.checked_hash(ref["sha256"])
        path = IDENTITY.private_backup_path(Path(ref["path"]))
        require(
            IDENTITY.sha256_file(path) == ref["sha256"], "channel_source_file_changed"
        )
        if name.startswith("evidence:"):
            require(path.stat().st_size > 0, "channel_evidence_empty")
            proofs.add(ref["sha256"])
        elif name not in ("archive", "source_export"):
            loaded[name] = P.parse_json(F.read_private(path))
    receipt, proof = loaded["source_receipt"], loaded["source_export_proof"]
    columns, counts, metadata, _, _ = P.metadata(receipt)
    require(
        receipt.get("status")
        == "snapshot-restored-baseline-verified-frozen-and-classified"
        and receipt["archive"]["sha256"] == files["archive"]["sha256"],
        "channel_source_not_frozen",
    )
    require(
        proof.get("source_archive_sha256") == files["archive"]["sha256"]
        and proof.get("source_export_sha256") == files["source_export"]["sha256"]
        and proof.get("source_column_coverage_exact") is True
        and proof.get("source_row_baseline_exact") is True
        and F.digest(proof.get("metadata_hashes")) == F.digest(metadata),
        "channel_export_unproven",
    )
    selected = sorted(proof["selected_table_counts"])
    require(
        "channels" in selected
        and proof["selected_table_counts"] == {n: counts[n] for n in selected},
        "channel_export_scope_invalid",
    )
    tables = P.read_frozen_export(
        Path(files["source_export"]["path"]),
        receipt,
        columns,
        counts,
        selected,
        proof["source_instance"],
    )
    return (
        tables["channels"],
        loaded.get("mapping"),
        loaded.get("unit_evidence"),
        proofs,
    )


def inspect_source(files):
    raw, mapping, _, _ = load_source(files, inspect_only=True)
    rows = [
        {
            "source_channel_id": integer(r.get("id"), 1, SAFE),
            "source_row_sha256": F.digest(r),
            "raw_used_quota": str(integer(r.get("used_quota"))),
            "balance": decimal_text(r.get("balance")),
            "balance_updated_time": observed(r.get("balance_updated_time")),
        }
        for r in raw
    ]
    rows.sort(key=lambda r: r["source_channel_id"])
    require(
        len({r["source_channel_id"] for r in rows}) == len(rows),
        "channel_duplicate_source_id",
    )
    return {
        "contract": CONTRACT,
        "archive_sha256": files["archive"]["sha256"],
        "export_sha256": files["source_export"]["sha256"],
        "recorded_counters": rows,
        "rows_sha256": F.digest(source_rows(raw, mapping))
        if mapping is not None
        else None,
        "mapping_sha256": files["mapping"]["sha256"] if mapping is not None else None,
        "target_writes": 0,
    }


def source_rows(raw_rows, mapping):
    require(
        type(mapping) is dict
        and set(mapping)
        == {
            "format_version",
            "archive_sha256",
            "export_sha256",
            "source_revision",
            "observed_at",
            "channels",
        }
        and type(mapping["format_version"]) is int
        and mapping["format_version"] == 1,
        "channel_mapping_invalid",
    )
    require(
        type(mapping["source_revision"]) is str
        and 1 <= len(mapping["source_revision"]) <= 128,
        "channel_source_revision_invalid",
    )
    require(
        mapping["observed_at"] is None or type(mapping["observed_at"]) is int,
        "channel_source_observation_invalid",
    )
    require(mapping["observed_at"] != 0, "channel_archive_observation_zero_use_null")
    observed(mapping["observed_at"])
    raw = {}
    for row in raw_rows:
        sid = integer(row.get("id"), 1, SAFE)
        require(sid not in raw, "channel_duplicate_source_id")
        raw[sid] = row
    require(
        type(mapping["channels"]) is list and bool(mapping["channels"]),
        "channel_mapping_empty",
    )
    result, seen, targets = [], set(), set()
    for entry in mapping["channels"]:
        require(
            type(entry) is dict
            and set(entry)
            == {
                "source_channel_id",
                "channel_id",
                "source_row_sha256",
                "historical_coverage",
            }
            and type(entry["historical_coverage"]) is bool,
            "channel_mapping_row_invalid",
        )
        sid, cid = (
            integer(entry["source_channel_id"], 1, SAFE),
            integer(entry["channel_id"], 1, SAFE),
        )
        require(
            sid in raw and sid not in seen and cid not in targets,
            "channel_identity_mapping_conflict",
        )
        row = raw[sid]
        require(
            entry["source_row_sha256"] == F.digest(row), "channel_source_row_changed"
        )
        seen.add(sid)
        targets.add(cid)
        result.append(
            {
                **entry,
                "raw_used_quota": str(integer(row.get("used_quota"))),
                "balance": decimal_text(row.get("balance")),
                "balance_updated_time": observed(row.get("balance_updated_time")),
            }
        )
    return sorted(result, key=lambda r: r["channel_id"])


def build_plan(
    raw_rows, mapping, evidence, proofs, files, snapshot, runtime_roles=None
):
    """Pure exact projection. Unknown evidence yields preview only, never a writer."""
    rows = source_rows(raw_rows, mapping)
    require(
        mapping["archive_sha256"] == files["archive"]["sha256"]
        and mapping["export_sha256"] == files["source_export"]["sha256"],
        "channel_mapping_source_mismatch",
    )
    scope = F.digest(rows)
    require(
        type(evidence) is dict
        and evidence.get("contract") == CONTRACT
        and evidence.get("archive_sha256") == mapping["archive_sha256"]
        and evidence.get("export_sha256") == mapping["export_sha256"]
        and evidence.get("mapping_sha256") == files["mapping"]["sha256"]
        and evidence.get("rows_sha256") == scope,
        "channel_unit_evidence_scope_mismatch",
    )
    overlap = evidence.get("native_overlap")
    require(
        overlap in ("disjoint", "included_at_seal", "unknown"),
        "channel_overlap_invalid",
    )
    require(type(evidence.get("channels")) is list, "channel_unit_rows_invalid")
    units = {}
    for entry in evidence["channels"]:
        sid = integer(entry.get("source_channel_id"), 1, SAFE)
        require(sid not in units, "channel_unit_scope_duplicate")
        units[sid] = entry
    require(
        set(units) == {r["source_channel_id"] for r in rows},
        "channel_unit_scope_incomplete",
    )
    stable = {
        "archive_sha256": mapping["archive_sha256"],
        "export_sha256": mapping["export_sha256"],
        "mapping_sha256": files["mapping"]["sha256"],
        "unit_evidence_sha256": files["unit_evidence"]["sha256"],
        "source_revision": mapping["source_revision"],
        "observed_at": mapping["observed_at"],
        "native_overlap": overlap,
        "rows_sha256": scope,
    }
    source_key = F.digest(
        stable
    )  # Deliberately excludes all mutable target/seal facts.
    reasons, baselines, patches = [], [], []
    if snapshot["openings"]["sources"] or snapshot["openings"]["rows"]:
        # Do not even preview a recalculated offset/seal for an occupied
        # namespace. Reconciliation requires the original approved material.
        return {
            "contract": CONTRACT,
            "status": "UNKNOWN",
            "source_key": source_key,
            "recorded_counters": rows,
            "stored_openings": snapshot["openings"],
            "reasons": ["existing_namespace_use_original_material_only"],
        }
    if overlap == "unknown" or evidence.get("overlap_proof_sha256") not in proofs:
        reasons.append("overlap_unproven")
    native = {integer(r["channel_id"], 1, SAFE): r for r in snapshot["native"]}
    identities = {integer(r["id"], 1, SAFE): r for r in snapshot["identities"]}
    for row in rows:
        cid, sid = row["channel_id"], row["source_channel_id"]
        require(
            cid in identities
            and cid in snapshot["active_channel_ids"]
            and identities[cid]["retired_at"] is None
            and identities[cid]["history_ambiguous"] is False,
            "channel_target_identity_unproven",
        )
        unit = units[sid]
        require(
            unit.get("source_row_sha256") == row["source_row_sha256"],
            "channel_unit_row_mismatch",
        )
        if (
            row["historical_coverage"]
            and unit.get("coverage_proof_sha256") not in proofs
        ):
            reasons.append(f"historical_coverage_unproven:{cid}")
        k = unit.get("credits_per_usd")
        proven = (
            unit.get("basis_kind")
            in ("captured_counter_unit", "audited_counter_unit_history")
            and unit.get("proof_sha256") in proofs
        )
        if k is None or not proven:
            reasons.append(f"basis_unproven:{cid}")
            continue
        try:
            k = integer(k, 1, SAFE)
        except F.Error:
            reasons.append(f"basis_outside_installed_schema:{cid}")
            continue
        n = native[cid]
        if n["source_channel_id"] is not None and n["historical_coverage"] is not True:
            reasons.append(f"copied_native_history_unknown:{cid}")
        offset = Fraction(int(row["raw_used_quota"]), k)
        if overlap == "included_at_seal":
            if (
                integer(n["pending"], 0)
                or integer(n["unknown"], 0)
                or integer(snapshot["global_unknown"], 0)
            ):
                reasons.append(f"included_native_unsettled_or_unknown:{cid}")
            gross = integer(n["gross"], 0, 10**38 - 1) + integer(
                n["baseline_gross"], 0, 10**38 - 1
            )
            refunds = integer(n["refunded"], 0, 10**38 - 1) + integer(
                n["baseline_refunded"], 0, 10**38 - 1
            )
            require(refunds <= gross, "channel_native_amount_invalid")
            offset -= Fraction(gross - refunds, 500000)
            if offset < 0 and unit.get("negative_offset_proof_sha256") not in proofs:
                reasons.append(f"included_negative_offset_unproven:{cid}")
        require(
            len(str(abs(offset.numerator))) <= 78
            and len(str(offset.denominator)) <= 78,
            "channel_rational_out_of_domain",
        )
        baselines.append(
            {
                "channel_id": cid,
                "source_key": source_key,
                "source_channel_id": sid,
                "raw_used_quota": row["raw_used_quota"],
                "credits_per_usd": k,
                "offset_numerator": str(offset.numerator),
                "offset_denominator": str(offset.denominator),
                "historical_coverage": row["historical_coverage"],
                "kind": "imported",
                "parent_channel_id": None,
            }
        )
        if row["balance"] is not None:
            patches.append(
                {
                    "channel_id": cid,
                    "balance": row["balance"],
                    "unit_label": label(unit.get("balance_unit_label")),
                    "balance_updated_time": row["balance_updated_time"],
                }
            )
    if (
        snapshot["projection_ready"] is not True
        or integer(snapshot["dirty"], 0)
        or snapshot["projection_consistent"] is not True
    ):
        reasons.append("native_projection_unavailable")
    require(len(native) == len(snapshot["native"]), "channel_native_duplicate")
    sealed_ids = ids(snapshot["native_request_ids"])
    source = {
        "source_key": source_key,
        **stable,
        "native_request_ids": sealed_ids,
        "native_seal_sha256": F.digest(
            {
                "native": snapshot["native"],
                "native_request_ids": sealed_ids,
                "state": snapshot["state"],
            }
        ),
    }
    plan = {
        "contract": CONTRACT,
        "source": source,
        "rows": baselines,
        "balance_patches": patches,
        "recorded_counters": rows,
        "target_sha256": F.digest(snapshot),
        "tool_hashes": tool_hashes(),
        "source_files": files,
        "runtime_roles": runtime_roles or [],
        "status": "UNKNOWN" if reasons else "READY",
        "reasons": sorted(set(reasons)),
    }
    source["plan_sha256"] = F.digest(plan)
    return plan


def opening_sql():
    return "SELECT jsonb_build_object('sources',coalesce((SELECT jsonb_agg(to_jsonb(s)-'captured_at' ORDER BY source_key) FROM core_channels.opening_sources s),'[]'::jsonb),'rows',coalesce((SELECT jsonb_agg(to_jsonb(b)-'captured_at' || jsonb_build_object('raw_used_quota',b.raw_used_quota::text,'offset_numerator',b.offset_numerator::text,'offset_denominator',b.offset_denominator::text) ORDER BY channel_id) FROM core_channels.opening_baselines b),'[]'::jsonb))"


def balances_sql():
    return "SELECT coalesce(jsonb_agg(to_jsonb(b)-'balance' || jsonb_build_object('balance',b.balance::text) ORDER BY channel_id),'[]'::jsonb) FROM core_channels.balance_observations b"


def target_sql(inventory):
    F.validate_inventory(inventory)
    require(
        set(DESTINATIONS)
        | {
            BALANCES,
            "core_channels.usage_rollups",
            "core_channels.projection_state",
            "core_channels.projection_dirty",
            "core_channels.usage_contributions",
        }
        <= set(inventory["tables"]),
        "channel_schema_missing",
    )
    return (
        "SELECT jsonb_build_object("
        + ",".join(
            (
                "'instance',(" + F.instance_sql() + ")",
                "'ownership_sha256',(" + F.ownership_sql() + ")",
                "'catalog_sha256',("
                + IDENTITY.catalog_hash_sql(S.catalog_contract())
                + ")",
                "'state',(" + F.state_sql(inventory, "") + ")",
                "'openings',(" + opening_sql() + ")",
                "'balances',(" + balances_sql() + ")",
                "'identities',(SELECT coalesce(jsonb_agg(to_jsonb(i) ORDER BY id),'[]'::jsonb) FROM core_channels.identities i)",
                "'active_channel_ids',(SELECT coalesce(jsonb_agg((u->>'id')::bigint ORDER BY (u->>'id')::bigint),'[]'::jsonb) FROM public.routing_active_release a JOIN public.routing_config_versions v ON v.version=a.config_version CROSS JOIN LATERAL jsonb_array_elements(v.document->'upstreams') u)",
                "'native_request_ids',(SELECT coalesce(jsonb_agg(id ORDER BY id COLLATE \"C\"),'[]'::jsonb) FROM core_charging.requests)",
                "'projection_ready',coalesce((SELECT ready FROM core_channels.projection_state WHERE singleton),false)",
                "'dirty',(SELECT count(*) FROM core_channels.projection_dirty)",
                "'projection_consistent',NOT EXISTS(SELECT FROM (SELECT channel_id,sum(gross_credits) g,sum(refunded_credits) f,sum(settled_requests) s,sum(pending_requests) p,sum(unknown_requests) u FROM core_channels.usage_contributions GROUP BY channel_id) c FULL JOIN (SELECT channel_id,sum(gross_credits) g,sum(refunded_credits) f,sum(settled_requests) s,sum(pending_requests) p,sum(unknown_requests) u FROM core_channels.usage_rollups WHERE day=-1 GROUP BY channel_id) r USING(channel_id) WHERE (coalesce(c.g,0),coalesce(c.f,0),coalesce(c.s,0),coalesce(c.p,0),coalesce(c.u,0)) IS DISTINCT FROM (coalesce(r.g,0),coalesce(r.f,0),coalesce(r.s,0),coalesce(r.p,0),coalesce(r.u,0)))",
                "'global_unknown',(SELECT coalesce(sum(unknown_requests),0)::text FROM core_channels.usage_rollups WHERE channel_id=0 AND day=-1)",
                "'native',(SELECT coalesce(jsonb_agg(jsonb_build_object('channel_id',i.id,'gross',r.g::text,'refunded',r.f::text,'pending',r.p::text,'unknown',r.u::text,'baseline_gross',coalesce(b.gross_credits,0)::text,'baseline_refunded',coalesce(b.refunded_credits,0)::text,'historical_coverage',coalesce(b.historical_coverage,false),'source_channel_id',b.source_channel_id) ORDER BY i.id),'[]'::jsonb) FROM core_channels.identities i LEFT JOIN core_channels.counter_baselines b ON b.channel_id=i.id CROSS JOIN LATERAL (SELECT coalesce(sum(gross_credits),0) g,coalesce(sum(refunded_credits),0) f,coalesce(sum(pending_requests),0) p,coalesce(sum(unknown_requests),0) u FROM core_channels.usage_rollups WHERE channel_id=i.id AND day=-1) r)",
            )
        )
        + ")"
    )


def owner_guard(runtime_roles):
    # Reuse the existing offline owner/socket/role fence, not a weaker importer role.
    base = S._owner_guard(runtime_roles)
    tables = ",".join(F.literal(n) for n in (*DESTINATIONS, BALANCES))
    roles = ",".join(F.literal(r) for r in runtime_roles)
    return (
        base
        + f"""
 IF EXISTS(SELECT FROM pg_namespace WHERE nspname='core_channels' AND pg_get_userbyid(nspowner)<>current_user)
 THEN RAISE EXCEPTION 'channel_opening_schema_owner_mismatch'; END IF;
 IF EXISTS(SELECT FROM pg_class WHERE oid IN (SELECT x::regclass FROM unnest(ARRAY[{tables}]) x)
   AND pg_get_userbyid(relowner)<>current_user) THEN RAISE EXCEPTION 'channel_opening_owner_mismatch'; END IF;
 IF EXISTS(SELECT FROM pg_roles r CROSS JOIN (VALUES
   ('core_channels.opening_sources',false),('core_channels.opening_baselines',true)) t(name,can_insert)
   WHERE r.rolname IN ({roles}) AND (has_table_privilege(r.oid,t.name,'UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')
     OR has_any_column_privilege(r.oid,t.name,'UPDATE,REFERENCES')
     OR (NOT t.can_insert AND (has_table_privilege(r.oid,t.name,'INSERT') OR has_any_column_privilege(r.oid,t.name,'INSERT')))))
 THEN RAISE EXCEPTION 'channel_opening_runtime_writes_forbidden'; END IF;
 IF NOT EXISTS(SELECT FROM pg_trigger WHERE tgrelid='core_channels.opening_baselines'::regclass
   AND tgname='channel_opening_copy_source' AND tgenabled='O'
   AND tgfoid='core_channels.check_opening_copy()'::regprocedure)
 THEN RAISE EXCEPTION 'channel_opening_copy_guard_missing'; END IF;
"""
    )


def snapshot_sql(inventory, runtime_roles):
    return (
        "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC'; SET LOCAL row_security=off;\nDO $channel_snapshot$ BEGIN "
        + owner_guard(runtime_roles)
        + " END $channel_snapshot$;\n"
        + target_sql(inventory)
        + ";\nCOMMIT;\n"
    )


def receipt_check(plan):
    source, rows = plan["source"], plan["rows"]
    return f"""
 IF actual->'openings'->'sources' IS DISTINCT FROM {F.literal([source])}::jsonb OR
 (SELECT coalesce(jsonb_agg(r ORDER BY (r->>'channel_id')::bigint),'[]'::jsonb)
  FROM jsonb_array_elements(actual->'openings'->'rows') r WHERE r->>'kind'='imported')
 IS DISTINCT FROM {F.literal(rows)}::jsonb THEN RAISE EXCEPTION 'channel_opening_receipt_conflict'; END IF;
"""


def validate_material(material):
    require(
        type(material) is dict and material.get("contract") == CONTRACT,
        "channel_material_invalid",
    )
    require(
        material["plan"]["status"] == "READY", "channel_preview_unknown_cannot_apply"
    )
    require(material["tool_hashes"] == tool_hashes(), "channel_tool_changed")
    source = material["plan"]["source"]
    require(set(source) == set(SOURCE_FIELDS), "channel_source_fields_invalid")
    without_hash = {
        **material["plan"],
        "source": {k: v for k, v in source.items() if k != "plan_sha256"},
    }
    require(source["plan_sha256"] == F.digest(without_hash), "channel_plan_changed")
    raw, mapping, evidence, proofs = load_source(material["files"])
    rebuilt = build_plan(
        raw,
        mapping,
        evidence,
        proofs,
        material["files"],
        material["snapshot"],
        material["runtime_roles"],
    )
    require(rebuilt == material["plan"], "channel_source_material_changed")


def reconcile(material, snapshot):
    """Preview an original plan against a later snapshot without rebasing it."""
    validate_material(material)
    original, plan = material["snapshot"], material["plan"]
    require(
        all(
            original[k] == snapshot[k]
            for k in ("instance", "ownership_sha256", "catalog_sha256")
        ),
        "channel_target_identity_changed",
    )
    openings = snapshot["openings"]
    if openings["sources"]:
        imported = sorted(
            (r for r in openings["rows"] if r["kind"] == "imported"),
            key=lambda r: r["channel_id"],
        )
        require(
            openings["sources"] == [plan["source"]] and imported == plan["rows"],
            "channel_opening_receipt_conflict",
        )
        return "NOOP"
    require(
        snapshot == original and not openings["rows"], "channel_opening_target_changed"
    )
    return "READY"


def build_apply_sql(material, *, resolve=None):
    validate_material(material)
    plan, snapshot, roles = (
        material["plan"],
        material["snapshot"],
        material["runtime_roles"],
    )
    inventory = snapshot["state"]["inventory"]
    query = target_sql(inventory)
    immutable = {
        k: snapshot[k] for k in ("instance", "ownership_sha256", "catalog_sha256")
    }
    sources = plan["source"]
    insert = []
    for table, fields, rows in (
        (DESTINATIONS[0], SOURCE_FIELDS, [sources]),
        (DESTINATIONS[1], BASE_FIELDS, plan["rows"]),
    ):
        insert.append(
            f"INSERT INTO {F.relation(table)} ({','.join(fields)}) SELECT {','.join('r.' + f for f in fields)} FROM jsonb_populate_recordset(NULL::{F.relation(table)},{F.literal(rows)}::jsonb) r;"
        )
    balance_update = f"""
 UPDATE core_channels.balance_observations b SET balance=p.balance,unit_label=p.unit_label,
 observed_at=CASE WHEN p.balance_updated_time IS NULL THEN NULL ELSE to_timestamp(p.balance_updated_time) END,
 kind='copied',credential_id=NULL,credential_revision=NULL,source_channel_id=NULL,recorded_at=transaction_timestamp()
 FROM jsonb_to_recordset({F.literal(plan["balance_patches"])}::jsonb)
 AS p(channel_id bigint,balance double precision,unit_label text,balance_updated_time bigint)
 WHERE b.channel_id=p.channel_id AND b.kind='unknown' AND b.balance IS NULL;
"""
    # Poststate is computed through the table's types, including float8's own
    # round-trip text. Only previously-unknown mapped balances may change.
    expected_balances = f"""SELECT coalesce(jsonb_agg(CASE WHEN b->>'kind'='unknown' AND b->'balance'='null'::jsonb AND p.channel_id IS NOT NULL THEN
 b || jsonb_build_object('balance',p.balance::text,'unit_label',p.unit_label,'observed_at',CASE WHEN p.balance_updated_time IS NULL THEN NULL ELSE to_jsonb(to_timestamp(p.balance_updated_time)) END,
 'kind','copied','credential_id',NULL,'credential_revision',NULL,'source_channel_id',NULL,'recorded_at',transaction_timestamp()) ELSE b END ORDER BY (b->>'channel_id')::bigint),'[]'::jsonb)
 FROM jsonb_array_elements(before_state->'balances') b LEFT JOIN jsonb_to_recordset({F.literal(plan["balance_patches"])}::jsonb)
 AS p(channel_id bigint,balance double precision,unit_label text,balance_updated_time bigint) ON p.channel_id=(b->>'channel_id')::bigint"""
    unchanged = ",".join(F.literal(n) for n in (*DESTINATIONS, BALANCES))
    resolution_guard = ""
    if resolve is not None:
        require(
            type(resolve) is str
            and re.fullmatch(r"lmm-channel-opening-[0-9a-f]{32}", resolve),
            "channel_original_attempt_invalid",
        )
        resolution_guard = f"IF EXISTS(SELECT FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid() AND application_name={F.literal(resolve)}) THEN RAISE EXCEPTION 'channel_original_session_active'; END IF;"
    first_write = (
        "PERFORM set_config('lmm.channel_opening_result','UNKNOWN',false);"
        if resolve
        else f"""
 IF actual IS DISTINCT FROM {F.literal(snapshot)}::jsonb THEN RAISE EXCEPTION 'channel_opening_target_changed'; END IF;
 IF actual->'openings'->'rows'<>'[]'::jsonb THEN RAISE EXCEPTION 'channel_opening_orphan_rows'; END IF;
 {chr(10).join(insert)}
 {balance_update}
 actual:=({query});
 {receipt_check(plan)}
 IF actual->'balances' IS DISTINCT FROM ({expected_balances}) THEN RAISE EXCEPTION 'channel_opening_balance_readback_changed'; END IF;
 IF (actual-'openings'-'balances'-'state') IS DISTINCT FROM (before_state-'openings'-'balances'-'state')
 OR ((actual->'state')-'tables') IS DISTINCT FROM ((before_state->'state')-'tables')
 OR ((actual->'state'->'tables')-ARRAY[{unchanged}]) IS DISTINCT FROM ((before_state->'state'->'tables')-ARRAY[{unchanged}])
 THEN RAISE EXCEPTION 'channel_opening_unrelated_state_changed'; END IF;
 PERFORM set_config('lmm.channel_opening_result','COMMITTED',false);
"""
    )
    body = f"""
DECLARE actual jsonb; before_state jsonb;
BEGIN
 {owner_guard(roles)}
 {resolution_guard}
 actual:=({query}); before_state:=actual;
 IF jsonb_build_object('instance',actual->'instance','ownership_sha256',actual->'ownership_sha256','catalog_sha256',actual->'catalog_sha256') IS DISTINCT FROM {F.literal(immutable)}::jsonb THEN RAISE EXCEPTION 'channel_opening_target_identity_changed'; END IF;
 IF actual->'openings'->'sources'<>'[]'::jsonb THEN
   {receipt_check(plan)}
   PERFORM set_config('lmm.channel_opening_result',{"'COMMITTED'" if resolve else "'NOOP'"},false);
 ELSE
   {first_write}
 END IF;
END;
"""
    tag = "$channel_opening_" + F.sha(body.encode())[:24] + "$"
    all_tables = ",".join(
        F.relation(n) for n in inventory["tables"] if n not in (*DESTINATIONS, BALANCES)
    )
    start = f"""SET standard_conforming_strings=on; SET timezone='UTC'; SET lock_timeout='5s'; SET statement_timeout='120s';
DO $channel_fence$ BEGIN IF NOT pg_try_advisory_lock({F.FENCE}) THEN RAISE EXCEPTION 'channel_opening_busy'; END IF; END $channel_fence$;
BEGIN ISOLATION LEVEL {"REPEATABLE READ READ ONLY" if resolve else "SERIALIZABLE"};
SET LOCAL row_security=off; SET LOCAL synchronous_commit=on; SET LOCAL search_path=pg_catalog,public;
"""
    locks = (
        ""
        if resolve
        else f"LOCK TABLE {all_tables} IN SHARE MODE;\nLOCK TABLE {','.join(F.relation(n) for n in (*DESTINATIONS, BALANCES))} IN EXCLUSIVE MODE;\n"
    )
    return (
        start
        + locks
        + f"DO {tag}{body}{tag};\nCOMMIT;\nSELECT jsonb_build_object('status',current_setting('lmm.channel_opening_result'),'source_key',{F.literal(sources['source_key'])},'plan_sha256',{F.literal(sources['plan_sha256'])});\n"
    )


def approval_for(material, sql):
    return {
        "decision": "approve-offline-channel-opening",
        "material_sha256": F.digest(material),
        "sql_sha256": F.sha(sql.encode()),
        "source_key": material["plan"]["source"]["source_key"],
        "plan_sha256": material["plan"]["source"]["plan_sha256"],
    }


def execute(material, *, approval, connection, output, original_attempt=None):
    """Single submission or original-attempt resolution; no automatic retries."""
    IDENTITY.private_backup_path(output, directory=True)
    require(type(material.get("apply_output")) is str, "channel_apply_slot_missing")
    if original_attempt is None:
        require(output == Path(material["apply_output"]), "channel_apply_slot_mismatch")
    original_sql = build_apply_sql(material)
    require(
        approval == approval_for(material, original_sql),
        "channel_owner_approval_mismatch",
    )
    owner, login = IDENTITY.connection_roles(connection)
    instance = material["snapshot"]["instance"]
    require(
        owner == instance["role"]
        and login == instance["session_user"]
        and connection.get("database") == instance["database"]
        and connection.get("port") == instance["port"],
        "channel_connection_binding_mismatch",
    )
    IDENTITY.socket_connection(connection)
    source = material["plan"]["source"]
    attempt = {
        "operation": "apply" if original_attempt is None else "resolve",
        "application_name": "lmm-channel-opening-" + uuid.uuid4().hex,
        "material_sha256": F.digest(material),
        "sql_sha256": F.sha(original_sql.encode()),
        "source_key": source["source_key"],
        "plan_sha256": source["plan_sha256"],
    }
    sql = original_sql
    if original_attempt is not None:
        require(
            original_attempt
            == P.parse_json(
                F.read_private(Path(material["apply_output"]) / "attempt.private.json")
            ),
            "channel_original_attempt_changed",
        )
        require(
            type(original_attempt) is dict
            and original_attempt.get("operation") == "apply"
            and all(
                original_attempt.get(k) == attempt[k]
                for k in ("material_sha256", "sql_sha256", "source_key", "plan_sha256")
            ),
            "channel_original_material_mismatch",
        )
        sql = build_apply_sql(
            material, resolve=original_attempt.get("application_name")
        )
    F.write_private(output, "attempt.private.json", F.canonical(attempt) + "\n")
    F.write_private(output, "executed.private.sql", sql)
    outcome = {
        "status": "UNKNOWN",
        "source_key": source["source_key"],
        "plan_sha256": source["plan_sha256"],
    }
    try:
        code, stdout = F.run_sql(
            connection,
            sql,
            output,
            attempt["application_name"],
            readonly=original_attempt is not None,
            label="channel-opening",
        )
        if code == 0:
            validate_material(material)
            receipt = P.parse_json(stdout)
            allowed = (
                ("COMMITTED", "UNKNOWN") if original_attempt else ("COMMITTED", "NOOP")
            )
            require(
                receipt in [{**outcome, "status": status} for status in allowed],
                "channel_receipt_mismatch",
            )
            outcome = receipt
    except Exception as error:
        # Transport stderr is private. Even a validation/receipt error after
        # submission cannot establish rollback or authorize another write.
        outcome["error_type"] = type(error).__name__
    try:
        F.write_private(output, "result.private.json", F.canonical(outcome) + "\n")
    except Exception:
        outcome = {**outcome, "status": "UNKNOWN"}
    return outcome


def prepare(files, snapshot, runtime_roles):
    raw, mapping, evidence, proofs = load_source(files)
    plan = build_plan(raw, mapping, evidence, proofs, files, snapshot, runtime_roles)
    return {
        "contract": CONTRACT,
        "files": files,
        "snapshot": snapshot,
        "runtime_roles": runtime_roles,
        "tool_hashes": tool_hashes(),
        "plan": plan,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "mode",
        choices=("inspect", "preview", "snapshot", "apply", "resolve"),
        nargs="?",
        default="preview",
    )
    for name in ("files", "target", "material", "approval", "connection", "attempt"):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--runtime-role", action="append", default=[])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    def load(path):
        return P.parse_json(F.read_private(path))

    try:
        IDENTITY.private_backup_path(args.output, directory=True)
        if args.mode == "inspect":
            require(
                args.files and not args.connection,
                "channel_inspection_arguments_invalid",
            )
            F.write_private(
                args.output,
                "source-inspection.private.json",
                F.canonical(inspect_source(load(args.files))) + "\n",
            )
            print('{"status":"SOURCE_INSPECTED","target_writes":0}')
        elif args.mode == "snapshot":
            require(
                args.connection and args.runtime_role,
                "channel_snapshot_arguments_missing",
            )
            connection = load(args.connection)
            code, stdout = F.run_sql(
                connection,
                "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"
                + F.inventory_sql()
                + ";\nCOMMIT;",
                args.output,
                "lmm-channel-inventory",
                readonly=True,
                label="inventory",
            )
            require(code == 0, "channel_inventory_failed")
            inventory = P.parse_json(stdout)
            code, stdout = F.run_sql(
                connection,
                snapshot_sql(inventory, args.runtime_role),
                args.output,
                "lmm-channel-snapshot",
                readonly=True,
                label="snapshot",
            )
            require(code == 0, "channel_snapshot_failed")
            F.write_private(
                args.output,
                "target.private.json",
                F.canonical(P.parse_json(stdout)) + "\n",
            )
            print('{"status":"READ_ONLY_SNAPSHOT","target_writes":0}')
        elif args.mode == "preview":
            require(args.target, "channel_preview_target_missing")
            if args.material:
                require(
                    not args.files and not args.runtime_role,
                    "channel_original_preview_arguments_conflict",
                )
                status = reconcile(load(args.material), load(args.target))
                F.write_private(
                    args.output,
                    "reconciliation.private.json",
                    F.canonical({"status": status, "target_writes": 0}) + "\n",
                )
                print(F.canonical({"status": status, "target_writes": 0}))
                return 0
            require(
                args.files and args.runtime_role, "channel_preview_arguments_missing"
            )
            material = prepare(load(args.files), load(args.target), args.runtime_role)
            material["apply_output"] = str(args.output / "apply")
            F.write_private(
                args.output, "material.private.json", F.canonical(material) + "\n"
            )
            if material["plan"]["status"] == "READY":
                sql = build_apply_sql(material)
                F.write_private(args.output, "review.private.sql", sql)
                F.write_private(
                    args.output,
                    "approval-request.private.json",
                    F.canonical(
                        {**approval_for(material, sql), "decision": "REVIEW_REQUIRED"}
                    )
                    + "\n",
                )
            print(
                F.canonical(
                    {
                        "status": material["plan"]["status"],
                        "target_writes": 0,
                        "reason_count": len(material["plan"]["reasons"]),
                    }
                )
            )
        else:
            require(
                args.material and args.approval and args.connection,
                "channel_apply_arguments_missing",
            )
            require(
                (args.mode == "resolve") == bool(args.attempt),
                "channel_resolution_attempt_required",
            )
            outcome = execute(
                load(args.material),
                approval=load(args.approval),
                connection=load(args.connection),
                output=args.output,
                original_attempt=load(args.attempt) if args.attempt else None,
            )
            print(F.canonical(outcome))
            return 0 if outcome["status"] in ("COMMITTED", "NOOP") else 2
    except (F.Error, OSError, ValueError, KeyError, TypeError, ArithmeticError):
        print(
            '{"status":"REFUSED","reason":"invalid_or_conflicting_channel_opening_material"}'
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

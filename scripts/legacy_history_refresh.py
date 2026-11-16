#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Prepare an append-only refresh of an already imported legacy log snapshot.

The input is a complete read-only JSONL capture ({kind, data} per line).
This command never connects to a database. It writes private SQL for an offline
owner transaction, preserving existing logs and the installation's deliberate
no-legacy-key-binding policy. It does not import identities, credentials,
balances, financial counters, or current prices.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
import json
import os
from pathlib import Path

import legacy_cutover_history as H
from legacy_history_import import captured_basis
from legacy_sql import literal as lit

SCOPE = "legacy-history-refresh.v1"
KEY_POLICY = "all_legacy_keys_invalid-no-native-credential-binding"
LOG_COLUMNS = (
    "source_log_id", "canonical_event_key", "source_row_sha256", "user_id",
    "owner_account_id", "token_id", "created_at", "log_type", "quota",
    "pricing_credits_per_usd", "prompt_tokens", "completion_tokens", "username",
    "token_name", "model_name", "channel_id", "group_name", "request_id",
    "upstream_request_id",
)
RECEIPT_COLUMNS = (
    "source_id", "user_id", "owner_account_id", "token_id",
    "source_archive_sha256", "mapping_sha256", "source_scope_sha256", "complete",
)
PROJECTIONS = {
    "sources": ("core_history.sources", (
        "source_id", "contract_version", "source_archive_sha256",
        "mapping_sha256", "source_namespace", "observed_at", "complete",
    ), ("source_id",)),
    "users": ("core_identity.users", ("id", "personal_account_id"), ("id",)),
    "log_records": ("core_history.log_records", ("source_id", *LOG_COLUMNS),
                    ("source_id", "source_log_id")),
    "scope_receipts": ("core_history.scope_receipts", RECEIPT_COLUMNS,
                       ("source_id", "user_id", "owner_account_id", "token_id")),
}


def require(value, code):
    if not value:
        raise ValueError(code)


def file_rows(path):
    with Path(path).open("rb") as source:
        for line in source:
            require(line.endswith(b"\n"), "truncated_export_line")
            yield line, json.loads(line)


def projection_row(row, columns):
    # Matches PostgreSQL jsonb_build_array(...)::text for these scalar columns.
    return json.dumps([row[c] for c in columns], ensure_ascii=False,
                      allow_nan=False, separators=(", ", ": "))


def read_target(path):
    """Keep only bounded mapping data and per-log hashes, never raw metadata."""
    target = {"sources": {}, "users": {}, "logs": {}, "scopes": {},
              "source_users": defaultdict(set), "import_runs": [], "metadata": None}
    fingerprints = defaultdict(list)
    export_hash = hashlib.sha256()
    for line, record in file_rows(path):
        export_hash.update(line)
        kind, row = record["kind"], record["data"]
        if kind == "metadata":
            require(target["metadata"] is None, "duplicate_target_metadata")
            target["metadata"] = row
        elif kind == "sources":
            require(row["source_id"] not in target["sources"], "duplicate_target_source")
            target["sources"][row["source_id"]] = row
        elif kind == "users":
            require(row["id"] not in target["users"], "duplicate_target_user")
            target["users"][row["id"]] = row["personal_account_id"]
        elif kind == "log_records":
            key = (row["source_id"], row["source_log_id"])
            require(key not in target["logs"], "duplicate_target_log")
            target["logs"][key] = (
                row["source_row_sha256"], row["user_id"], row["owner_account_id"],
                row["token_id"],
            )
            target["source_users"][row["source_id"]].add(row["user_id"])
        elif kind == "scope_receipts":
            key = tuple(row[k] for k in ("source_id", "user_id", "owner_account_id", "token_id"))
            require(key not in target["scopes"], "duplicate_target_scope")
            target["scopes"][key] = row
        elif kind == "import_runs":
            target["import_runs"].append(row)
        if kind in PROJECTIONS:
            _, columns, order = PROJECTIONS[kind]
            key = tuple((row[c] is not None, row[c]) for c in order)
            row_hash = hashlib.md5(projection_row(row, columns).encode()).hexdigest()
            fingerprints[kind].append((key, row_hash))
    require(target["metadata"] is not None and target["sources"], "target_history_missing")
    target["fingerprints"] = {}
    for kind in PROJECTIONS:
        rows = sorted(fingerprints[kind])
        target["fingerprints"][kind] = {
            "rows": len(rows),
            "md5": hashlib.md5("".join(h for _, h in rows).encode()).hexdigest(),
        }
    target["export_sha256"] = export_hash.hexdigest()
    return target


def history_provenance(target, namespace):
    """Validate the committed map and explicit excluded-row evidence."""
    maps, proofs, orphan_ids = {}, {}, set()
    for sid, source in target["sources"].items():
        require(source["complete"] and source["contract_version"] == "core.history-source.v1",
                "incomplete_target_history")
        require(source["source_namespace"] == namespace, "unreviewed_history_namespace")
        matches = [r for r in target["import_runs"]
                   if r["source_archive_sha256"] == source["source_archive_sha256"]
                   and r["mapping_sha256"] == source["mapping_sha256"]
                   and r["manifest"].get("scope") in (
                       "live-owner-history-no-legacy-key-binding.v1", SCOPE)]
        require(len(matches) == 1, "history_provenance_missing_or_ambiguous")
        run = matches[0]
        manifest = run["manifest"]
        identity_map = manifest["identity_map"]
        if manifest["scope"] == SCOPE:
            mapping = manifest["mapping"]
            require(mapping["identities"] == identity_map, "history_identity_manifest_mismatch")
            require(mapping["key_policy"] == KEY_POLICY, "history_key_policy_mismatch")
        else:
            mapping = {"identities": identity_map, "key_policy": KEY_POLICY,
                       "source_archive": source["source_archive_sha256"]}
        require(H.sha(mapping) == source["mapping_sha256"], "history_mapping_hash_mismatch")
        require(manifest["source"] == {k: v for k, v in source.items() if k != "source_id"},
                "history_source_receipt_mismatch")
        for row in identity_map:
            src, uid, aid = (row[k] for k in (
                "source_user_id", "target_user_id", "target_personal_account_id"))
            require(target["users"].get(uid) == aid, "historical_identity_changed")
            require(src not in maps or maps[src] == (uid, aid), "identity_lineage_conflict")
            maps[src] = (uid, aid)
        excluded = manifest.get("orphans", [])
        # Original recovery stored both log and quota-data orphan observations.
        for row in excluded:
            if row.get("table") in (None, "logs"):
                uid = row.get("user_id", row.get("source_user_id"))
                if uid is not None:
                    orphan_ids.add(uid)
        proofs[sid] = {"migration_id": run["migration_id"], "manifest": manifest,
                       "identity_map": identity_map, "orphans": excluded}
    require(all(row[3] is None for row in target["logs"].values()), "legacy_key_binding_present")
    return maps, proofs, orphan_ids


def normalize(raw, identity):
    uid, aid = identity
    row = {
        "source_log_id": H._int(raw["id"], "invalid_log_id", 1, H.MAX_ID, False),
        "canonical_event_key": "", "source_row_sha256": H.sha(raw),
        "user_id": uid, "owner_account_id": aid, "token_id": None,
        "created_at": H._int(raw.get("created_at"), "invalid_log_time", 0, H.MAX_TIME, False),
        "log_type": H._int(raw.get("type"), "invalid_log_type", 0, 7, False),
        "quota": H._int(raw.get("quota"), "invalid_log_quota"),
        "pricing_credits_per_usd": captured_basis(raw.get("other")),
        "prompt_tokens": H._int(raw.get("prompt_tokens"), "invalid_prompt_tokens"),
        "completion_tokens": H._int(raw.get("completion_tokens"), "invalid_completion_tokens"),
        "channel_id": H._int(raw.get("channel_id") or 0, "invalid_channel", 0, H.MAX_ID, False),
    }
    row.update({key: H._text(raw.get(source)) for key, source in H.TEXT_FIELDS})
    return row


def prepare(source_path, target, binding, additional_identities=()):
    require(binding["database"] == target["metadata"]["database"], "target_database_mismatch")
    source_hash = hashlib.sha256()
    namespace = next(iter(target["sources"].values()))["source_namespace"]
    identities, proofs, approved_orphans = history_provenance(target, namespace)
    for row in additional_identities:
        sid, uid, aid = (H._int(row[k], "invalid_additional_identity", 1, H.MAX_ID, False)
                         for k in ("source_user_id", "target_user_id", "target_personal_account_id"))
        require(sid not in identities or identities[sid] == (uid, aid), "additional_identity_conflict")
        require(target["users"].get(uid) == aid, "additional_identity_target_mismatch")
        identities[sid] = (uid, aid)
    require(len({v[0] for v in identities.values()}) == len(identities), "identity_map_not_one_to_one")
    require(set(identities.values()) == set(target["users"].items()), "target_identity_coverage_incomplete")
    existing = {}
    for (source_id, log_id), row in target["logs"].items():
        require(log_id not in existing, "duplicate_historical_log_across_sources")
        existing[log_id] = row
    rows, excluded, missing = [], [], Counter()
    counts, seen, source_users, matched = Counter(), set(), set(), set()
    metadata = completion = None
    for line, record in file_rows(source_path):
        source_hash.update(line)
        kind, raw = record["kind"], record["data"]
        if kind == "metadata":
            require(metadata is None, "duplicate_source_metadata")
            metadata = raw
        elif kind == "capture_complete":
            require(completion is None, "duplicate_capture_completion")
            completion = raw
        else:
            require(completion is None, "rows_after_capture_completion")
            counts[kind] += 1
        if kind == "users":
            require(raw["id"] not in source_users, "duplicate_source_user")
            source_users.add(raw["id"])
        elif kind == "logs":
            lid = H._int(raw.get("id"), "invalid_log_id", 1, H.MAX_ID, False)
            require(lid not in seen, "duplicate_source_log")
            seen.add(lid)
            old = existing.get(lid)
            if old is not None:
                require(old[0] == H.sha(raw), f"existing_log_changed:{lid}")
                require(identities.get(raw.get("user_id")) == (old[1], old[2]),
                        f"existing_log_identity_changed:{lid}")
                matched.add(lid)
            elif raw.get("user_id") in identities:
                rows.append(normalize(raw, identities[raw["user_id"]]))
            else:
                missing[raw.get("user_id")] += 1
                excluded.append({"table": "logs", "source_log_id": lid,
                                 "user_id": raw.get("user_id"), "type": raw.get("type"),
                                 "quota": raw.get("quota"), "row_sha256": H.sha(raw)})
    require(metadata is not None and completion is not None, "source_capture_incomplete")
    require(namespace == "legacy.logs:" + metadata["schema"], "source_namespace_mismatch")
    require(all(counts[k] == v for k, v in completion.items()), "source_capture_counts_mismatch")
    require(not (set(missing) & source_users),
            "source_users_need_identity_mapping:" + ",".join(map(str, sorted(set(missing) & source_users))))
    require(set(missing) <= approved_orphans, "unreviewed_orphan_log_user")
    require(set(identities) <= source_users, "mapped_identity_missing_from_source")
    snapshot_sha = source_hash.hexdigest()
    rows.sort(key=lambda row: row["source_log_id"])
    for row in rows:
        row["canonical_event_key"] = f"legacy.logs:{snapshot_sha}:{row['source_log_id']}"
    identity_map = [{"source_user_id": sid, "target_user_id": uid,
                     "target_personal_account_id": aid}
                    for sid, (uid, aid) in sorted(identities.items())]
    mapping = {"identities": identity_map, "key_policy": KEY_POLICY,
               "source_archive": snapshot_sha, "orphans": excluded}
    source = {
        "contract_version": "core.history-source.v1", "source_archive_sha256": snapshot_sha,
        "mapping_sha256": H.sha(mapping), "source_namespace": namespace,
        "observed_at": H._int(metadata["observed_at"], "invalid_observation_time", 1, H.MAX_TIME, False),
        "complete": True,
    }
    require(source["observed_at"] >= max(s["observed_at"] for s in target["sources"].values()),
            "source_snapshot_older_than_target")
    by_user = defaultdict(list)
    for row in rows:
        by_user[row["user_id"]].append([row["source_log_id"], row["source_row_sha256"]])
    scopes = []
    for uid, aid in sorted(target["users"].items()):
        scope = {"source_id": None, "user_id": uid, "owner_account_id": aid, "token_id": None,
                 "source_archive_sha256": snapshot_sha, "mapping_sha256": source["mapping_sha256"],
                 "complete": True}
        scope["source_scope_sha256"] = H.sha({"scope": {k: v for k, v in scope.items() if k != "source_id"},
                                            "rows": by_user[uid]})
        scopes.append(scope)
    prior_scopes = []
    for source_id, prior in target["sources"].items():
        proof = proofs[source_id]
        prior_targets = {m["target_user_id"] for m in proof["identity_map"]}
        orphan_users = {r.get("user_id", r.get("source_user_id")) for r in proof["orphans"]}
        for sid, (uid, aid) in sorted(identities.items()):
            key = (source_id, uid, aid, None)
            if key in target["scopes"]:
                receipt = target["scopes"][key]
                require(receipt["complete"]
                        and receipt["source_archive_sha256"] == prior["source_archive_sha256"]
                        and receipt["mapping_sha256"] == prior["mapping_sha256"],
                        "incomplete_existing_user_scope")
                continue
            require(uid not in prior_targets and sid not in orphan_users
                    and uid not in target["source_users"][source_id],
                    "prior_source_absence_unproven")
            scope = {"source_id": source_id, "user_id": uid, "owner_account_id": aid,
                     "token_id": None, "source_archive_sha256": prior["source_archive_sha256"],
                     "mapping_sha256": prior["mapping_sha256"], "complete": True}
            scope["source_scope_sha256"] = H.sha({
                "scope": scope, "rows": [], "absence_proof": proof["migration_id"],
                "absence_manifest_sha256": H.sha(proof["manifest"]),
                "source_user_id": sid, "current_source_archive_sha256": snapshot_sha,
            })
            prior_scopes.append(scope)
    summary = {
        "source_logs": counts["logs"], "existing_logs_matched": len(matched),
        "retained_logs_not_in_current_source": len(existing) - len(matched),
        "new_logs": len(rows), "orphan_logs_excluded": len(excluded),
        "orphan_user_counts": dict(missing), "new_user_scopes": len(scopes),
        "prior_empty_scopes": len(prior_scopes), "legacy_keys_migrated": 0,
        "delta_quota": sum(r["quota"] or 0 for r in rows),
        "delta_consume_quota": sum(r["quota"] or 0 for r in rows if r["log_type"] == 2),
        "delta_prompt_tokens": sum(r["prompt_tokens"] or 0 for r in rows),
        "delta_completion_tokens": sum(r["completion_tokens"] or 0 for r in rows),
        "latest_source_log_id": max(seen, default=None),
        "latest_imported_timestamp": max((r["created_at"] for r in rows), default=None),
        "financial_writes": False,
    }
    manifest = {
        "scope": SCOPE, "source": source, "mapping": mapping, "identity_map": identity_map,
        "orphans": excluded, "summary": summary, "target": binding,
        "target_export_sha256": target["export_sha256"],
        "before": target["fingerprints"], "delta_rows_sha256": H.sha(rows),
        "new_scopes_sha256": H.sha(scopes), "prior_scopes_sha256": H.sha(prior_scopes),
        "prior_proofs": {str(s): {"migration_id": p["migration_id"],
                                 "manifest_sha256": H.sha(p["manifest"])} for s, p in proofs.items()},
    }
    return {"migration_id": "history-refresh-" + snapshot_sha[:24], "manifest": manifest,
            "rows": rows, "scopes": scopes, "prior_scopes": prior_scopes}


def fingerprint_sql(kind):
    table, columns, order = PROJECTIONS[kind]
    values = ",".join(columns)
    ordering = ",".join(c + " NULLS FIRST" for c in order)
    return ("SELECT count(*) AS rows,md5(COALESCE(string_agg(md5(jsonb_build_array("
            + values + ")::text),'' ORDER BY " + ordering + "),'')) AS md5 FROM " + table)


def build_sql(plan):
    m, mid = plan["manifest"], plan["migration_id"]
    binding, source = m["target"], m["source"]
    require(all(k in binding for k in ("database", "database_oid", "schema_version",
                                      "schema_fingerprint", "owner")), "target_binding_incomplete")
    if not plan["rows"] and not plan["prior_scopes"]:
        return "SELECT 'HISTORY_ALREADY_CURRENT' AS result;\n"
    sql = [
        "\\set ON_ERROR_STOP on", "BEGIN ISOLATION LEVEL SERIALIZABLE;",
        "SET LOCAL lock_timeout='5s';", "SET LOCAL statement_timeout='120s';",
        "DO $$BEGIN "
        f"IF current_database()<>{lit(binding['database'])} OR "
        f"(SELECT oid FROM pg_database WHERE datname=current_database())<>{int(binding['database_oid'])} "
        f"OR current_user<>{lit(binding['owner'])} OR "
        "current_user<>pg_get_userbyid((SELECT relowner FROM pg_class "
        "WHERE oid='core_history.sources'::regclass)) THEN "
        "RAISE EXCEPTION 'history_target_binding_mismatch'; END IF; "
        "IF NOT EXISTS(SELECT 1 FROM core_meta.schema_contract WHERE singleton "
        f"AND version={int(binding['schema_version'])} AND encode(fingerprint,'hex')="
        f"{lit(binding['schema_fingerprint'])}) THEN RAISE EXCEPTION "
        "'history_schema_changed'; END IF; "
        "IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() "
        "AND pid<>pg_backend_pid() AND backend_type='client backend') THEN "
        "RAISE EXCEPTION 'history_target_not_offline'; END IF; END$$;",
        "LOCK TABLE core_identity.users,core_identity.accounts,core_history.sources,"
        "core_history.log_records,core_history.scope_receipts,core_migration.import_runs "
        "IN SHARE ROW EXCLUSIVE MODE;",
        "CREATE TEMP TABLE refresh_logs ON COMMIT DROP AS "
        "SELECT * FROM core_history.log_records WITH NO DATA;",
        "CREATE TEMP TABLE refresh_scopes ON COMMIT DROP AS "
        "SELECT * FROM core_history.scope_receipts WITH NO DATA;",
        "CREATE TEMP TABLE refresh_prior_scopes ON COMMIT DROP AS "
        "SELECT * FROM core_history.scope_receipts WITH NO DATA;",
        H.copy_block("refresh_logs", LOG_COLUMNS, plan["rows"]),
        H.copy_block("refresh_scopes", tuple(c for c in RECEIPT_COLUMNS if c != "source_id"), plan["scopes"]),
        H.copy_block("refresh_prior_scopes", RECEIPT_COLUMNS, plan["prior_scopes"]),
        f"SELECT EXISTS(SELECT 1 FROM core_migration.import_runs WHERE migration_id={lit(mid)}) "
        "AS refresh_replay \\gset",
        "\\if :refresh_replay",
        "DO $$BEGIN IF (SELECT manifest FROM core_migration.import_runs WHERE migration_id="
        + lit(mid) + ") IS DISTINCT FROM " + H.jlit(m)
        + " THEN RAISE EXCEPTION 'history_refresh_replay_conflict'; END IF; END$$;",
        "\\else",
    ]
    for kind, expected in m["before"].items():
        sql.append("DO $$BEGIN IF (SELECT to_jsonb(s) FROM (" + fingerprint_sql(kind)
                   + ") s) IS DISTINCT FROM " + H.jlit(expected)
                   + " THEN RAISE EXCEPTION 'history_target_state_changed:" + kind + "'; END IF; END$$;")
    source_cols = tuple(source)
    sql.append("INSERT INTO core_history.sources(" + ",".join(source_cols) + ") VALUES("
               + ",".join(lit(source[c]) for c in source_cols) + ");")
    sql += ["\\endif",
            "CREATE TEMP TABLE refresh_source ON COMMIT DROP AS SELECT source_id FROM core_history.sources WHERE "
            "source_archive_sha256=" + lit(source["source_archive_sha256"]) + " AND mapping_sha256="
            + lit(source["mapping_sha256"]) + " AND source_namespace=" + lit(source["source_namespace"]) + ";",
            "DO $$BEGIN IF (SELECT count(*) FROM refresh_source)<>1 THEN "
            "RAISE EXCEPTION 'history_refresh_source_missing'; END IF; END$$;",
            "\\if :refresh_replay", "\\else"]
    for temp, table, columns in (
        ("refresh_logs", "core_history.log_records", LOG_COLUMNS),
        ("refresh_scopes", "core_history.scope_receipts",
         tuple(c for c in RECEIPT_COLUMNS if c != "source_id")),
    ):
        sql.append(f"INSERT INTO {table}(source_id,{','.join(columns)}) "
                   f"SELECT s.source_id,{','.join('r.'+c for c in columns)} "
                   f"FROM {temp} r CROSS JOIN refresh_source s;")
    sql += ["INSERT INTO core_history.scope_receipts SELECT * FROM refresh_prior_scopes;",
            "INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES("
            + ",".join((lit(mid), lit(source["source_archive_sha256"]),
                        lit(source["mapping_sha256"]), H.jlit(m))) + ");", "\\endif"]
    for temp, table, columns in (
        ("refresh_logs", "core_history.log_records", LOG_COLUMNS),
        ("refresh_scopes", "core_history.scope_receipts",
         tuple(c for c in RECEIPT_COLUMNS if c != "source_id")),
    ):
        cols = ",".join(columns)
        actual = f"SELECT {cols} FROM {table} WHERE source_id=(SELECT source_id FROM refresh_source)"
        expected = f"SELECT {cols} FROM {temp}"
        sql.append("DO $$BEGIN IF EXISTS((" + expected + ") EXCEPT ALL (" + actual
                   + ")) OR EXISTS((" + actual + ") EXCEPT ALL (" + expected
                   + ")) THEN RAISE EXCEPTION 'history_refresh_readback_mismatch'; END IF; END$$;")
    sql += [
        "DO $$BEGIN IF EXISTS((SELECT * FROM refresh_prior_scopes) EXCEPT ALL "
        "(SELECT * FROM core_history.scope_receipts)) THEN "
        "RAISE EXCEPTION 'history_prior_scope_readback_mismatch'; END IF; END$$;",
        "SET CONSTRAINTS ALL IMMEDIATE;", "COMMIT;",
        "SELECT 'HISTORY_REFRESH_COMMITTED' AS result;",
    ]
    return "\n".join(sql) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--target-binding", type=Path, required=True)
    parser.add_argument("--additional-identity-map", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    binding = json.loads(args.target_binding.read_text())
    extra = ([json.loads(s) for s in args.additional_identity_map.read_text().splitlines() if s]
             if args.additional_identity_map else [])
    target = read_target(args.target)
    plan = prepare(args.source, target, binding, extra)
    sql = build_sql(plan)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    for name, text in (
        ("plan.private.json", H.canonical(plan) + "\n"),
        ("apply.private.sql", sql),
        ("summary.json", H.canonical(plan["manifest"]["summary"]) + "\n"),
    ):
        (args.output / name).write_text(text)
    print(H.canonical({"output": str(args.output), **plan["manifest"]["summary"]}))


if __name__ == "__main__":
    main()

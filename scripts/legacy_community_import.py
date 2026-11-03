#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Offline copy of legacy community rows into a fresh lmm_community database.

Reads the restored legacy PostgreSQL schema (read-only) and writes the community
extension database installed by `lmm-extensions install-db --component community`.
Legacy user IDs are Core user IDs and are copied unchanged. Run it as the
community database owner (it sets sequences); it refuses a target that already
contains imported rows, and the whole import is one transaction.

Not imported, by design:
- unified_todo_reads: Core serves /api/todos from its own inbox; legacy read
  markers refer to legacy todo items that do not exist there.
- perf_metrics: Core records its own request performance observations.
- user_ranking_revisions: a legacy cache-invalidation counter, not data.

Default mode prints the plan (row counts) without writing; pass --execute.
Connection strings are libpq conninfo strings passed to psql; keep credentials
in PGPASSFILE/peer auth rather than on the command line.
"""
from __future__ import annotations

import argparse
import json
import secrets
import subprocess
import sys

SOURCE_QUERIES = {
    "directory_ads": """SELECT id,owner_user_id,name,url,summary,description,bid_cents,charged_quota,request_id,status,
        paid_at,expires_at,hidden_at,refunded_at FROM {s}.ai_directory_ads ORDER BY id""",
    "profile_shares": """SELECT user_id,token,created_at,model_usage_enabled,aggregate_usage_enabled,
        COALESCE(NULLIF(linked_profiles,''),'[]') AS linked_profiles FROM {s}.profile_shares ORDER BY user_id""",
    "onboarding_todos": """SELECT id,user_id,client_installed_at,client_configured_at,completed_at,created_at,updated_at
        FROM {s}.l1_onboarding_todos ORDER BY id""",
    "signal_attempts": """SELECT id,token_hash,seed,size,day,actor,COALESCE(model_id,'') AS model_id,COALESCE(harness,'') AS harness,
        COALESCE(agent_name,'') AS agent_name,started_at,COALESCE(finished_at,0) AS finished_at,COALESCE(moves,0) AS moves,
        COALESCE(elapsed_ms,0) AS elapsed_ms,COALESCE(actions,'') AS actions,created_at FROM {s}.signal_game_attempts ORDER BY id""",
    # Names were public only on public challenge records; snapshot just those.
    "signal_records": """SELECT r.id,r.user_id,r.attempt_id,r.mode,COALESCE(r.day,'') AS day,r.size,r.seed,r.rules_version,r.moves,
        r.elapsed_ms,r.hints,r.actor,COALESCE(r.model_id,'') AS model_id,COALESCE(r.harness,'') AS harness,
        COALESCE(r.agent_name,'') AS agent_name,r.participant_key,COALESCE(r.email,'') AS email,COALESCE(r.note,'') AS note,
        r.actions,r.digest,r.public,r.created_at,
        CASE WHEN r.public AND r.mode='challenge' THEN left(COALESCE(NULLIF(u.display_name,''),u.username,''),80) ELSE '' END AS player_name
        FROM {s}.signal_game_records r LEFT JOIN {s}.users u ON u.id=r.user_id ORDER BY r.id""",
}

COLUMNS = {
    "directory_ads": "id bigint,owner_user_id bigint,name text,url text,summary text,description text,bid_cents bigint,charged_quota bigint,request_id text,status text,paid_at bigint,expires_at bigint,hidden_at bigint,refunded_at bigint",
    "profile_shares": "user_id bigint,token text,created_at bigint,model_usage_enabled boolean,aggregate_usage_enabled boolean,linked_profiles text",
    "onboarding_todos": "id bigint,user_id bigint,client_installed_at bigint,client_configured_at bigint,completed_at bigint,created_at bigint,updated_at bigint",
    "signal_attempts": "id bigint,token_hash text,seed bigint,size integer,day text,actor text,model_id text,harness text,agent_name text,started_at bigint,finished_at bigint,moves integer,elapsed_ms bigint,actions text,created_at bigint",
    "signal_records": "id bigint,user_id bigint,attempt_id bigint,mode text,day text,size integer,seed bigint,rules_version integer,moves integer,elapsed_ms bigint,hints integer,actor text,model_id text,harness text,agent_name text,participant_key text,email text,note text,actions text,digest text,public boolean,created_at bigint,player_name text",
}

INSERTS = {
    "directory_ads": """INSERT INTO lmm_community.directory_ads(id,owner_user_id,name,url,summary,description,bid_cents,charged_quota,request_id,status,paid_at,expires_at,hidden_at,refunded_at,refund_status)
        SELECT id,owner_user_id,name,url,summary,description,bid_cents,charged_quota,request_id,status,paid_at,expires_at,hidden_at,refunded_at,
        CASE WHEN refunded_at>0 THEN 'refunded' ELSE 'none' END FROM rows""",
    "profile_shares": """INSERT INTO lmm_community.profile_shares(user_id,token,created_at,model_usage_enabled,aggregate_usage_enabled,linked_profiles)
        SELECT user_id,token,created_at,model_usage_enabled,aggregate_usage_enabled,linked_profiles::jsonb FROM rows""",
    # key_proven_at stays 0: the legacy key step was derived from live key state.
    "onboarding_todos": """INSERT INTO lmm_community.onboarding_todos(id,user_id,client_installed_at,client_configured_at,completed_at,created_at,updated_at)
        SELECT id,user_id,client_installed_at,client_configured_at,completed_at,created_at,updated_at FROM rows""",
    "signal_attempts": """INSERT INTO lmm_community.signal_attempts(id,token_hash,seed,size,day,actor,model_id,harness,agent_name,started_at,finished_at,moves,elapsed_ms,actions,created_at)
        SELECT id,token_hash,seed,size,day,actor,model_id,harness,agent_name,started_at,finished_at,moves,elapsed_ms,actions,created_at FROM rows""",
    # A record whose attempt was already purged keeps its result without a link.
    "signal_records": """INSERT INTO lmm_community.signal_records(id,user_id,attempt_id,mode,day,size,seed,rules_version,moves,elapsed_ms,hints,actor,model_id,harness,agent_name,participant_key,email,note,actions,digest,public,created_at,player_name)
        SELECT id,user_id,CASE WHEN EXISTS(SELECT FROM lmm_community.signal_attempts a WHERE a.id=rows.attempt_id) THEN attempt_id END,
        mode,day,size,seed,rules_version,moves,elapsed_ms,hints,actor,model_id,harness,agent_name,participant_key,email,note,actions,digest,public,created_at,player_name FROM rows""",
}

SEQUENCES = {"directory_ads": "id", "onboarding_todos": "id", "signal_attempts": "id", "signal_records": "id"}


def psql(conninfo: str, sql: str) -> str:
    result = subprocess.run(["psql", "-X", "-q", "-At", "-v", "ON_ERROR_STOP=1", "-d", conninfo],
                            input=sql, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        sys.exit(f"psql failed: {result.stderr.strip()}")
    return result.stdout


def read_source(conninfo: str, schema: str) -> dict[str, list]:
    if not schema.replace("_", "").isalnum():
        sys.exit("invalid source schema")
    data = {}
    for table, query in SOURCE_QUERIES.items():
        sql = "BEGIN READ ONLY;\nSELECT COALESCE(json_agg(t),'[]') FROM (" + query.format(s=schema) + ") t;\nCOMMIT;\n"
        data[table] = json.loads(psql(conninfo, sql).strip() or "[]")
    return data


def import_sql(data: dict[str, list]) -> str:
    out = ["BEGIN;", "SET LOCAL lock_timeout='5s';",
           "LOCK TABLE lmm_community.directory_ads,lmm_community.profile_shares,lmm_community.onboarding_todos,"
           "lmm_community.signal_attempts,lmm_community.signal_records IN EXCLUSIVE MODE;",
           "DO $$BEGIN IF EXISTS(SELECT FROM lmm_community.directory_ads) OR EXISTS(SELECT FROM lmm_community.profile_shares)"
           " OR EXISTS(SELECT FROM lmm_community.onboarding_todos) OR EXISTS(SELECT FROM lmm_community.signal_attempts)"
           " OR EXISTS(SELECT FROM lmm_community.signal_records) THEN RAISE EXCEPTION 'target already contains community rows'; END IF; END$$;"]
    for table in SOURCE_QUERIES:
        payload = json.dumps(data[table], ensure_ascii=False, allow_nan=False)
        tag = "j" + secrets.token_hex(8)
        while f"${tag}$" in payload:
            tag = "j" + secrets.token_hex(8)
        out.append(f"WITH rows AS (SELECT * FROM jsonb_to_recordset(${tag}${payload}${tag}$::jsonb) AS x({COLUMNS[table]})) "
                   + INSERTS[table] + ";")
    for table, column in SEQUENCES.items():
        out.append(f"SELECT setval(pg_get_serial_sequence('lmm_community.{table}','{column}'),"
                   f"GREATEST((SELECT max({column}) FROM lmm_community.{table}),1),"
                   f"(SELECT max({column}) IS NOT NULL FROM lmm_community.{table}));")
    out.append("SELECT json_build_object("
               + ",".join(f"'{t}',(SELECT count(*) FROM lmm_community.{t})" for t in SOURCE_QUERIES)
               + ",'signal_records_unlinked',(SELECT count(*) FROM lmm_community.signal_records WHERE mode='challenge' AND attempt_id IS NULL));")
    out.append("COMMIT;")
    return "\n".join(out) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--source", required=True, help="libpq conninfo of the restored legacy database (read-only use)")
    parser.add_argument("--source-schema", required=True, help="legacy schema, e.g. lmm_prod_20260802")
    parser.add_argument("--target", required=True, help="libpq conninfo of the fresh lmm_community database (owner role)")
    parser.add_argument("--execute", action="store_true", help="write rows; default only prints the plan")
    args = parser.parse_args()
    data = read_source(args.source, args.source_schema)
    plan = {table: len(rows) for table, rows in data.items()}
    print(json.dumps({"mode": "execute" if args.execute else "plan", "source_rows": plan}))
    if args.execute:
        print(psql(args.target, import_sql(data)).strip().splitlines()[-1])


if __name__ == "__main__":
    main()

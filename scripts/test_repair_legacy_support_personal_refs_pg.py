#!/usr/bin/env python3
"""Owned real-PG clone: complete CAS, replay, conflict and author preservation."""
import argparse
import copy
import json
import os
from pathlib import Path
import time

import migrate_legacy_available_wallet as W
import repair_legacy_support_personal_refs as M


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for k in ('source', 'orders', 'live', 'wallet-material', 'socket', 'output'):
        p.add_argument('--' + k, type=Path, required=True)
    p.add_argument('--port', type=int, required=True); p.add_argument('--admin', required=True)
    a = p.parse_args(); os.umask(0o077); a.output.mkdir(mode=0o700)
    c = {'socket_directory': str(a.socket), 'port': a.port, 'database': 'postgres', 'role': a.admin}

    def query(label, sql, fail=False):
        r = W.psql(c, sql, a.output, label)
        assert (r.returncode != 0) == fail, label + ': ' + r.stderr[-1000:]
        return r.stdout

    db = 'qa_support_refs_' + str(os.getpid()) + '_' + str(int(time.time()))
    query('clone', f'CREATE DATABASE {db} TEMPLATE rights_support_26752cda OWNER {a.admin};')
    c['database'] = db; live = W.load(a.live)
    for t in M.TABLES:
        query('seed-' + t, f'TRUNCATE lmm_support.{t};' + ''.join(
            f"INSERT INTO lmm_support.{t}(shop_id,id,body) VALUES({W.literal(r['shop_id'])},{W.literal(r['id'])},{W.literal(r['body'])}::jsonb);"
            for r in live[t]))
    physical = json.loads(query('physical', "SELECT jsonb_build_object('database',current_database(),'database_oid',(SELECT oid FROM pg_database WHERE datname=current_database()),'system_identifier',(pg_control_system()).system_identifier::text);"))
    m = M.prepare(W.load(a.source), W.load(a.orders), live, W.load(a.wallet_material), physical)
    def state(label):
        return json.loads(query(label, 'BEGIN READ ONLY;SELECT ' + M.snapshot_sql() + ';COMMIT;'))
    assert state('before') == m['before']
    # A concurrent destination must abort without removing either conversation.
    row = m['after']['conversations'][0]
    query('conflict-seed', f"INSERT INTO lmm_support.conversations VALUES({W.literal(row['shop_id'])},{W.literal(row['id'])},{W.literal(row['body'])}::jsonb);")
    conflict = state('conflict-before'); query('conflict-reject', M.build_sql(m), True)
    assert state('conflict-after') == conflict
    query('conflict-remove', f"DELETE FROM lmm_support.conversations WHERE shop_id={W.literal(row['shop_id'])} AND id={W.literal(row['id'])};")
    bad = copy.deepcopy(m); bad['physical']['database_oid'] = int(physical['database_oid']) + 1
    query('physical-reject', M.build_sql(bad), True); assert state('physical-unchanged') == m['before']
    query('repair', M.build_sql(m)); after = state('after'); assert after == m['after']
    b = m['before']['messages'][0]['body']; n = after['messages'][0]['body']
    assert {**b, 'id': n['id'], 'conversation_id': n['conversation_id']} == n
    assert n['author_user_id'] == 1 and n['author_account'] == {'kind': 'personal', 'id': 1}
    assert after['conversations'][0]['body']['seller'] == {'kind': 'personal', 'id': 1}
    assert after['conversations'][0]['body']['buyer'] == {'kind': 'personal', 'id': 380}
    assert after['conversations'][0]['id'] == 'e2dda02fafedbd0a41671b7b43d2dec381c91736599321076d3364ab3c4734ea'
    assert after['customers'][0]['id'] == '53c3f5ca69ca0d01efb0d971ca659a9ae91b002ee91528eda6928b11ca0f613d'
    query('replay', M.build_sql(m)); assert state('after-replay') == after
    # An additional replay dependency may not be silently reassigned.
    query('command-seed', f"INSERT INTO lmm_support.commands VALUES({W.literal(row['shop_id'])},'new-unknown-command','{{}}'::jsonb);")
    changed = state('command-before'); query('command-reject', M.build_sql(m), True)
    assert state('command-after') == changed
    W.save(a.output, 'result.private.json', {'status': 'PASS_REAL_PG_SUPPORT_COMPLETE_CAS',
        'exact_three_rows': True, 'seller_author_text_preserved': True, 'replay_noop': True,
        'destination_and_physical_and_replay_dependency_rejected': True,
        'database_retained': db, 'production_writes': 0})
    print('PASS_REAL_PG_SUPPORT_COMPLETE_CAS')


if __name__ == '__main__':
    main()

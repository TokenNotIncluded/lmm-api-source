#!/usr/bin/env python3
"""Prepare a frozen-source Support reference-chain CAS; never connect or apply."""
import argparse
import copy
import json
import os
from pathlib import Path

import legacy_extensions_import as E
import migrate_legacy_available_wallet as W

TABLES = ('customers', 'conversations', 'messages', 'commands')


def snapshot_sql():
    return 'jsonb_build_object(' + ','.join(
        W.literal(t) + f",(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY shop_id,id),'[]'::jsonb) FROM lmm_support.{t} r)"
        for t in TABLES) + ')'


def prepare(source, orders, live, wallet, physical):
    # Deliberately bounded to the measured imported conversation. New messages,
    # replay records or notes need a separately reviewed preimage, never guessing.
    src = {k: [json.loads(r) if isinstance(r, str) else r for r in source[k]]
           for k in ('customers', 'conversations', 'messages')}
    W.require(not src['customers'] and len(src['conversations']) == len(src['messages']) == 1,
              'support_source_cohort_changed')
    W.require(not live['commands'] and all(len(live[t]) == 1 for t in TABLES[:-1]),
              'support_live_cohort_or_replay_dependencies_changed')
    c = src['conversations'][0]; msg = src['messages'][0]
    owners = {r['source_user_id']: r for r in wallet['positions']}
    buyer = owners[c['buyer_id']]; seller = owners[c['seller_id']]
    W.require(buyer['target_user_id'] == c['buyer_id'] and seller['target_user_id'] == c['seller_id'],
              'support_requires_preserved_source_user_ids')
    W.require(msg['conversation_id'] == c['id'] and msg['sender_id'] == c['seller_id'],
              'support_source_message_lineage_changed')
    order = next(o for o in orders['orders'] if o['id'] == c['order_id'])
    W.require(all(order[k] == c[k] for k in ('buyer_id', 'seller_id', 'product_id')),
              'support_source_order_parties_changed')
    before = {t: copy.deepcopy(live[t]) for t in TABLES}
    after = copy.deepcopy(before)
    cr = before['conversations'][0]; cu = before['customers'][0]; mr = before['messages'][0]
    cb = cr['body']; mb = mr['body']; shop = E.go_id('legacy-web-shop-v1', E.personal(c['seller_id']))
    W.require(all(r['shop_id'] == shop for r in (cr, cu, mr)), 'support_shop_changed')
    key = E.go_id('web-order-support', c['order_id'])

    def derived(uid):
        # Account and OpenRequest JSON order is Go struct declaration order;
        # JSONB's reordered object keys must not be used for Go ID derivation.
        account = E.personal(uid)
        request = {'shop_id': shop, 'buyer': account, 'order_id': c['order_id'],
                   'subject': order['product_title'], 'key': key}
        return (E.go_id('conversation', c['buyer_id'], account, key),
                E.go_id('customer', account), E.go_id(request))

    uid = cb['buyer']['id']
    W.require(cb['buyer'] == E.personal(uid) and uid in (buyer['account_id'], buyer['target_user_id']),
              'support_buyer_not_proven_user_or_ledger_alias')
    oldcid, oldcustomer, oldhash = derived(uid)
    W.require(cr['id'] == cb['id'] == oldcid and cb['customer_id'] == oldcustomer
              and cb['request_hash'] == oldhash, 'support_old_derived_chain_changed')
    W.require(cb['seller'] == E.personal(c['seller_id']) and cb['shop_id'] == shop
              and cb['product_id'] == c['product_id'] and cb['order_id'] == c['order_id']
              and cb['subject'] == order['product_title'] and cb['closed'] == (c['status'] != 'open')
              and cb['created_at'] == E.go_time(c['created_at'])
              and cb['updated_at'] == E.go_time(c['updated_at'])
              and cb['sequence'] == cb['last_message_id'] == 1
              and cb['buyer_read_id'] == int(msg['id'] <= c['buyer_read_id'])
              and cb['seller_read_id'] == int(msg['id'] <= c['seller_read_id']),
              'support_source_conversation_metadata_changed')
    W.require(cu['id'] == cu['body']['id'] == oldcustomer and cu['body']['buyer'] == E.personal(uid)
              and cu['body']['notes'] == '' and cu['body']['tags'] == []
              and cu['body']['revision'] == 1 and cu['body']['created_at'] == E.go_time(c['created_at']),
              'support_customer_preimage_changed')
    W.require(mr['id'] == mb['id'] == oldcid + ':00000000000000000001'
              and mb['conversation_id'] == oldcid and mb['numeric_id'] == 1
              and mb['author_user_id'] == msg['sender_id']
              and mb['author_account'] == E.personal(msg['sender_id'])
              and mb['created_at'] == E.go_time(msg['created_at']), 'support_message_lineage_changed')
    cid, customer, request_hash = derived(buyer['target_user_id'])
    ac = after['conversations'][0]; ac['id'] = cid
    ac['body'].update(id=cid, buyer=E.personal(buyer['target_user_id']),
                      customer_id=customer, request_hash=request_hash)
    au = after['customers'][0]; au['id'] = customer
    au['body'].update(id=customer, buyer=E.personal(buyer['target_user_id']))
    am = after['messages'][0]; am['id'] = cid + ':00000000000000000001'
    am['body'].update(id=am['id'], conversation_id=cid)
    return {'scope': 'frozen-support-personal-user-reference-chain-v1', 'physical': physical,
            'source_archive_sha256': wallet['source_archive_sha256'],
            'current_production_source_state_attested': False,
            'source_buyer_user_id': c['buyer_id'], 'ledger_account_alias': buyer['account_id'],
            'preserved_seller_and_message_author': c['seller_id'], 'before': before, 'after': after,
            'money_written': False, 'message_text_changed': False}


def build_sql(m):
    L = W.literal; p = m['physical']; updates = ''
    for t in TABLES:
        for old, new in zip(m['before'][t], m['after'][t]):
            if old == new:
                continue
            updates += f"UPDATE lmm_support.{t} SET id={L(new['id'])},body={L(new['body'])}::jsonb WHERE shop_id={L(old['shop_id'])} AND id={L(old['id'])} AND body={L(old['body'])}::jsonb;\nGET DIAGNOSTICS affected = ROW_COUNT; IF affected<>1 THEN RAISE EXCEPTION 'support CAS row count changed'; END IF;\n"
    tag = '$support_repair_' + W.digest(m) + '$'
    W.require(tag not in W.canonical(m), 'support_sql_delimiter_collision')
    snapshot = snapshot_sql()
    return f"""\\set ON_ERROR_STOP on
BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on; SET LOCAL standard_conforming_strings=on;
SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s';
LOCK TABLE {','.join('lmm_support.'+t for t in TABLES)} IN SHARE ROW EXCLUSIVE MODE;
DO {tag}
DECLARE actual jsonb; affected bigint;
BEGIN
 IF current_database()<>{L(p['database'])} OR (SELECT oid FROM pg_database WHERE datname=current_database())<>{int(p['database_oid'])} OR (pg_control_system()).system_identifier::text<>{L(p['system_identifier'])} THEN RAISE EXCEPTION 'support physical database mismatch'; END IF;
 IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=current_user AND rolsuper) AND current_user<>(SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname='lmm_support') THEN RAISE EXCEPTION 'support metadata owner required'; END IF;
 SELECT {snapshot} INTO actual;
 IF actual={L(m['after'])}::jsonb THEN RAISE NOTICE 'support already exact; no writes';
 ELSIF actual={L(m['before'])}::jsonb THEN
 {updates}
 ELSE RAISE EXCEPTION 'support complete preimage changed or destination conflict'; END IF;
 SELECT {snapshot} INTO actual;
 IF actual<>{L(m['after'])}::jsonb THEN RAISE EXCEPTION 'support exact chain readback mismatch'; END IF;
END {tag};
COMMIT;
SELECT jsonb_build_object('status','COMMITTED_EXACT_SUPPORT_REFERENCE_CHAIN','checked_rows',3,'money_written',false,'message_text_changed',false,'exact_match',true);
"""


def main():
    os.umask(0o077); p = argparse.ArgumentParser(description=__doc__)
    for k in ('source', 'orders', 'live', 'wallet-material', 'physical', 'output'):
        p.add_argument('--' + k, type=Path, required=True)
    a = p.parse_args(); a.output.mkdir(mode=0o700)
    m = prepare(W.load(a.source), W.load(a.orders), W.load(a.live), W.load(a.wallet_material), W.load(a.physical))
    W.save(a.output, 'material.private.json', m)
    W.save(a.output, 'repair.private.sql', build_sql(m))
    W.save(a.output, 'readback.private.sql', 'BEGIN READ ONLY;SELECT ' + snapshot_sql() + ';COMMIT;\n')
    print('PREPARED_NOT_APPLIED_SUPPORT_REFERENCE_CHAIN')


if __name__ == '__main__':
    main()

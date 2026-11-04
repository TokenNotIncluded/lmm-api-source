# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import copy
import hashlib
import json
from decimal import Decimal
from types import SimpleNamespace
import unittest

import legacy_discount_facts as D
import legacy_cutover_history as H
import legacy_cutover_import as I
from owner_history_receipt import owner_history_import_sql


class DiscountFactsTest(unittest.TestCase):
    def setUp(self):
        self.args = SimpleNamespace(source_schema='lmm_prod_20260802', archive_sha256='a'*64, observed_at=123)
        self.tables = dict(discount_codes=[dict(id=1, code=' secret-Code ', owner_user_id=999,
                           min_amount=Decimal('12.3400'), used_count=7, created_time=3,
                           starts_time=4, expired_time=5, deleted_at='old')],
                           discount_code_reservations=[], discount_top_ups=[])

    def order(self, status='pending', provider='waffo_pancake', failure=None):
        return dict(id=20, user_id=2, trade_no='trade', discount_code_id=1,
                    status=status, payment_provider=provider, failure_reason_code=failure)

    def reservation(self, status='reserved'):
        return dict(id=10, user_id=2, top_up_trade_no='trade', discount_code_id=1,
                    status=status, expires_time=0, created_time=1, updated_time=2)

    def test_eight_capacity_boundaries(self):
        cases = [
            ('reserved', None, 1),
            ('reserved', self.order('expired'), 0),
            ('reserved', self.order('failed', 'other'), 0),
            ('reserved', self.order('failed', failure='declined'), 0),
            ('reserved', self.order('failed', failure=None), 1),
            ('released', self.order('failed', failure='checkout_timeout'), 1),
            (None, self.order(), 1),
            ('consumed', self.order(), 0),
        ]
        for status, order, count in cases:
            with self.subTest(status=status, order=order):
                t = copy.deepcopy(self.tables)
                t['discount_code_reservations'] = [self.reservation(status)] if status else []
                t['discount_top_ups'] = [order] if order else []
                holds = D.capacity_holds(t)
                self.assertEqual(len(holds), count)
                if count:
                    h = holds[0]
                    self.assertEqual(h['source_order_id'], 20 if order else None)
                    self.assertEqual(h['source_reservation_id'], 10 if status else None)
                    self.assertEqual(h['source_user_id'], 2)
                    self.assertTrue(h['reason'])
        for status, order, count in [('reserved', self.order(), 1),
                                     ('released', None, 0),
                                     ('released', self.order('failed'), 0),
                                     ('released', self.order('success'), 0),
                                     (None, self.order('failed', failure='checkout_timeout'), 1)]:
            t = copy.deepcopy(self.tables)
            t['discount_code_reservations'] = [self.reservation(status)] if status else []
            t['discount_top_ups'] = [order] if order else []
            self.assertEqual(len(D.capacity_holds(t)), count)

    def test_reject_inconsistent_duplicate_unknown(self):
        for kind in ('code', 'user', 'duplicate', 'unknown', 'missing_code', 'order_duplicate'):
            t = copy.deepcopy(self.tables)
            t['discount_top_ups'] = [self.order()]
            t['discount_code_reservations'] = [self.reservation()]
            if kind == 'code': t['discount_code_reservations'][0]['discount_code_id'] = 9
            if kind == 'user': t['discount_code_reservations'][0]['user_id'] = 3
            if kind == 'duplicate': t['discount_code_reservations'] *= 2
            if kind == 'unknown': t['discount_code_reservations'][0]['status'] = 'unknown'
            if kind == 'missing_code': t['discount_codes'] = []
            if kind == 'order_duplicate': t['discount_top_ups'] *= 2
            with self.subTest(kind=kind), self.assertRaises(ValueError): D.capacity_holds(t)

    def test_manifest_preserves_raw_facts_hash_and_redacts(self):
        m = D.material(self.args, self.tables, [], [2])
        c = m['tables']['discount_codes'][0]
        self.assertEqual(c['min_amount'], '12.3400')
        self.assertEqual(c['used_count'], 7)
        self.assertEqual(c['owner_user_id'], 999)
        for key in ('created_time', 'starts_time', 'expired_time', 'deleted_at'):
            self.assertEqual(c[key], self.tables['discount_codes'][0][key])
        self.assertEqual(c['code_sha256'], hashlib.sha256(b'SECRET-CODE').hexdigest())
        self.assertNotIn('code', c)
        self.assertNotIn('secret-Code', json.dumps(m))
        self.assertEqual(m['source_tables']['discount_codes']['rows_sha256'], D.sha(self.tables['discount_codes']))
        self.assertEqual(m['mapping_sha256'], D.sha([[2, 2]]))
        self.assertEqual(m['source_namespace'], D.NAMESPACE)
        options = [dict(key='DisplayInCurrencyEnabled', value='false')]
        self.assertEqual(D.material(self.args, self.tables, options, [2])['options'], options)
        for invalid in ('schema', 'archive', 'options'):
            args = copy.copy(self.args)
            if invalid == 'schema': args.source_schema = 'other'
            if invalid == 'archive': args.archive_sha256 = 'bad'
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                D.material(args, self.tables, [dict(key='secret', value='x')] if invalid == 'options' else [], [2])

    def test_receipt_body_delimiter_cannot_be_closed_by_a_source_name(self):
        self.tables['discount_codes'][0]['name'] = "offer $owner_history$ $owner_history_1$ 'quoted'"
        m = D.material(self.args, self.tables, [], [2])
        sql = owner_history_import_sql(m, scope=D.SCOPE, error_prefix='discount_facts')
        self.assertEqual(sql.count('$owner_history_2$'), 2)
        self.assertIn('DO $owner_history_2$', sql)
        self.assertIn("''quoted''", sql)

    def test_local_queries_phase_and_sql_contract(self):
        tables = self.tables
        class Db:
            def __init__(self): self.calls = []
            def run(self, sql):
                self.calls.append(sql)
                if 'FROM options ' in sql: return '[]'
                if 'FROM users ' in sql: return '[{"id":2}]'
                if 'SELECT * FROM discount_codes ' in sql: return D.canonical(tables['discount_codes'])
                return '[]'
        src, tgt = Db(), Db()
        sql, stats = D.plan(self.args, src, tgt)
        text = '\n'.join(sql)
        self.assertFalse(tgt.calls)
        self.assertEqual(len(src.calls), 5)
        self.assertTrue(all('SELECT' in q and 'INSERT' not in q for q in src.calls))
        self.assertIn('id,user_id,trade_no,discount_code_id,status,payment_provider,failure_reason_code', src.calls[2])
        self.assertIn('discount_code_id>0 OR trade_no IN', src.calls[2])
        self.assertNotIn('SELECT * FROM options', ' '.join(src.calls))
        self.assertLess(I.PHASES.index('wallet'), I.PHASES.index('discounts'))
        self.assertLess(I.PHASES.index('discounts'), I.PHASES.index('history'))
        self.assertIn(D.SCOPE, H.GUARDS['discounts'])
        self.assertNotIn('BEGIN ISOLATION', text)
        self.assertNotIn('COMMIT;', text)
        for constraint in ('installation_owner_required', 'opening_source_mismatch', 'identity_mapping_mismatch',
                           'other_source_installed', 'import_conflict', 'scope_not_empty', 'LOCK TABLE'):
            self.assertIn(constraint, text)
        self.assertNotIn('UPDATE ', text)
        self.assertNotIn('secret-Code', text)
        self.assertFalse(stats['capacity_reserved_by_import'])
        self.assertFalse(stats['native_discount_enabled_by_import'])
        m = D.material(self.args, tables, [], [2])
        default = owner_history_import_sql(m, scope=D.SCOPE, error_prefix='discount_facts')
        self.assertTrue(default.startswith('BEGIN ISOLATION LEVEL SERIALIZABLE;'))
        self.assertTrue(default.endswith('COMMIT;\n'))
        self.assertIn('previous.manifest IS DISTINCT FROM m', default)


if __name__ == '__main__':
    unittest.main()

#!/usr/bin/env python3
"""Hand-labelled legacy facts and exact arithmetic; no product imports.

Business authority: parent traditional 3948a0a1, INDEPENDENT_CONTRACT.md.
This module never obtains expected values from the owner importer or reader.
"""
import copy
from fractions import Fraction
import json
import unittest

LEGACY_REVISION = '3948a0a1ee1cefdd016a7aa45a11851545bb4b85'
SAFE = 2**53 - 1


def normalized(raw, baseline, divisor, rounding, reversed_raw=0, reversed_normalized=0):
    """Independent rational arithmetic for the explicitly supplied audit facts."""
    if not all(type(n) is int and 0 <= n <= SAFE for n in
               (raw, baseline, reversed_raw, reversed_normalized)):
        raise ValueError('invalid independent usage fact')
    ratio = Fraction(baseline) / Fraction(divisor)
    if Fraction(divisor) <= 1 or rounding not in ('half-away-from-zero', 'toward-zero'):
        raise ValueError('unproved normalization basis')
    rounded = ratio.numerator // ratio.denominator
    if rounding == 'half-away-from-zero' and ratio - rounded >= Fraction(1, 2):
        rounded += 1
    historical = rounded - reversed_normalized
    delta = raw - baseline + reversed_raw
    if min(historical, delta) < 0 or historical + delta > SAFE:
        raise ValueError('unproved counter reduction or overflow')
    return historical + delta


def fixture():
    """Original synthetic raw rows, with manually assigned expected categories."""
    users = [
        dict(id=8, quota=70, used_quota=1155, request_count=2, aff_history=0,
             status=1, deleted_at=None),
        dict(id=12, quota=40, used_quota=42, request_count=1, aff_history=0,
             status=2, deleted_at=None),
        dict(id=9, quota=-5, used_quota=0, request_count=0, aff_history=0,
             status=1, deleted_at='2026-01-02T00:00:00Z'),
    ]
    # Expected classes are independent labels, not computed by porting the writer.
    inputs = [
        ('cash', 'stripe', 'stripe', 'USD', 1200001, 200001, 0, 7777777),
        ('cash', 'creem', 'creem', ' usd ', 800000, 800000, 0, 0),
        ('cash', 'epay', 'alipay', 'CNY', 2300005, 3, 0, 0),
        ('virtual', 'epay', 'ldc', 'LDC', 3300000, 1000001, 3300000, 9000000),
        ('unconfirmed', 'unverified', 'stripe', 'USD', -1, 0, 0, 0),
        ('unconfirmed', 'stripe', 'stripe', '', 1, 0, 0, 0),
        ('unconfirmed', 'stripe', 'stripe', 'USD', 0, 0, 999999, 0),
        ('unconfirmed', 'epay', 'ldc', 'LDC', 4, 0, 0, 0),
        ('invalid', 'stripe', 'stripe', 'USD', 5, 6, 0, 0),
        ('invalid', 'stripe', 'stripe', 'USD', -2, 0, 0, 0),
        ('excluded', 'admin', 'gift', 'USD', 999999, 0, 0, 0),
        ('excluded', 'stripe', 'balance', 'USD', 999999, 0, 0, 0),
        ('excluded', 'stripe', 'stripe', 'USD', 999999, 0, 0, 0),
    ]
    rows, labels = [], {}
    for number, (classification, provider, method, currency, gross, refund, expected, credited) in enumerate(inputs, 1):
        rows.append(dict(id=number, user_id=8, trade_no=f'independent-{number}',
                         status='success', payment_provider=provider, payment_method=method,
                         settlement_currency=currency, settled_amount_micros=gross,
                         refunded_amount_micros=refund, expected_amount_micros=expected,
                         credited_quota=credited, money='999999.999999'))
        labels[number] = classification
    for number, status in [(14, 'pending'), (15, 'failed')]:
        row = {**rows[0], 'id': number, 'trade_no': f'independent-{number}', 'status': status}
        rows.append(row)
    original_fee = dict(id=55, user_id=8, charged_quota=200, status='charged',
                        reversed_at=0, created_at=100, error_code='synthetic.fee')
    audit = dict(version=1, kind='offline_credit_balance_rebase_preview',
                 migration_id='independent-unit-basis', user_ids=[8],
                 has_complete_history=True, usd_credit_conversion=500000,
                 divisor='2', rounding='half-away-from-zero', snapshot_at=150,
                 user_sources=[dict(id=8, used_quota=1205)], token_sources=[],
                 other_credit_bases=[dict(kind='violation_fee_refund', source_id='55',
                                         user_id=8, original_quota=200, rebased_quota=100,
                                         source=original_fee)])
    tables = dict(users=users, top_ups=rows,
                  subscription_orders=[dict(id=1, trade_no='independent-13', status='pending')],
                  wallet_credit_rebases=[dict(migration_id='independent-unit-basis', plan=json.dumps(audit))],
                  tokens=[], violation_fee_records=[{**original_fee, 'status': 'reversed',
                                                     'reversed_at': 175, 'reversed_by': 7}])
    return dict(tables=tables, labels=labels,
                opening={8: dict(available=70, reserved=20, affiliate=700),
                         12: dict(available=40, reserved=0, affiliate=0),
                         9: dict(available=-5, reserved=0, affiliate=0)},
                normalized={8: normalized(1155, 1205, '2', 'half-away-from-zero', 200, 100),
                            12: 42, 9: 0})


def expected_statistics(native_paid_minor=0, native_refunded_minor=0):
    """Literal old-contract totals plus an explicit USD synthetic native fact."""
    if not 0 <= native_refunded_minor <= native_paid_minor:
        raise ValueError('unresolved native refund')
    data = dict(credits_per_usd=500000, total_used_credits='695', total_balance_credits='105',
                recharge=dict(currencies=[
                    dict(currency='CNY', gross_amount_micros='2300005', refunded_amount_micros='3', net_amount_micros='2300002', orders=1),
                    dict(currency='USD', gross_amount_micros='2000001', refunded_amount_micros='1000001', net_amount_micros='1000000', orders=2)],
                    virtual_units=[dict(currency='LDC', gross_amount_micros='3300000', refunded_amount_micros='1000001', net_amount_micros='2299999', orders=1)],
                    confirmed_orders=4, unconfirmed_orders=4, invalid_orders=2))
    if native_paid_minor:
        usd = data['recharge']['currencies'][1]
        usd['gross_amount_micros'] = str(2000001 + native_paid_minor * 10000)
        usd['refunded_amount_micros'] = str(1000001 + native_refunded_minor * 10000)
        usd['net_amount_micros'] = str(int(usd['gross_amount_micros']) - int(usd['refunded_amount_micros']))
        usd['orders'] += 1
        data['recharge']['confirmed_orders'] += 1
        data['total_balance_credits'] = str(105 + (native_paid_minor-native_refunded_minor)*5000)
    return data


def assert_response(body, expected, before, after):
    if set(body) != {'success', 'data'} or body['success'] is not True:
        raise AssertionError('unexpected successful finance envelope')
    actual = copy.deepcopy(body['data'])
    timestamp = actual.pop('as_of', None)
    if type(timestamp) is not int or not before <= timestamp <= after:
        raise AssertionError('as_of outside independently observed request interval')
    if actual != expected:
        raise AssertionError(f'finance DTO mismatch: actual={actual!r}; expected={expected!r}')


class IndependentArithmeticTests(unittest.TestCase):
    def test_frozen_hand_examples(self):
        self.assertEqual(normalized(1155,1205,'2','half-away-from-zero',200,100),653)
        self.assertEqual(normalized(1155,1205,'2','toward-zero',200,100),652)
        self.assertEqual(653 + (70-10) + 42,755)
        self.assertEqual(1031*SAFE,9286422431637961721)

    def test_unproved_reductions_fail(self):
        for arguments in [(900,1205,'2','toward-zero',200,100),
                          (1155,1205,'0','toward-zero',200,100)]:
            with self.assertRaises((ValueError,ZeroDivisionError)):
                normalized(*arguments)

    def test_exact_expected_currencies(self):
        fact=fixture()
        self.assertEqual(sum(fact['normalized'].values()),695)
        self.assertEqual(sum(u['quota'] for u in fact['tables']['users']),105)
        self.assertEqual(list(fact['labels'].values()).count('unconfirmed'),4)
        expected=expected_statistics(123,23)
        self.assertEqual(expected['recharge']['currencies'][1]['net_amount_micros'],'2000000')
        self.assertEqual(expected['total_balance_credits'],'500105')


if __name__ == '__main__':
    unittest.main()

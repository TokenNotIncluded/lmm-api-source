# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import copy
import unittest
import prepare_checkin_history_snapshot as h


class CheckinHistoryTests(unittest.TestCase):
    def setUp(self):
        self.export = {'source_namespace': h.NAMESPACE, 'observed_at': 1793674367,
            'source_row_count': 2, 'source_user_ids': [1],
            'settings': {'checkin_setting.enabled': 'true', 'checkin_setting.min_quota': '1',
                'checkin_setting.max_quota': '500000', 'checkin_setting.level_multipliers': '[0,0.2,0.7,1,1.5]'},
            'rows': [{'id': 1, 'user_id': 1, 'checkin_date': '2026-11-02', 'quota_awarded': 7, 'created_at': 1793500000},
                     {'id': 2, 'user_id': 9, 'checkin_date': '2026-11-02', 'quota_awarded': 0, 'created_at': 1793500001}]}

    def build(self, data=None):
        return h.prepare(data or self.export, expected_rows=2, archive_sha256='a'*64)

    def test_exact_rows_options_orphans_and_zero_awards_are_preserved_without_credit(self):
        m = self.build()
        self.assertEqual([r['record'] for r in m['rows']], self.export['rows'])
        self.assertEqual(m['settings'], self.export['settings'])
        self.assertEqual(m['orphan_rows'], 1)
        self.assertEqual(m['balance_effect'], 'none')
        self.assertFalse(m['native_checkin_enabled_by_import'])
        self.assertIsNone(m['legacy_calendar_timezone'])

    def test_duplicate_day_bad_dates_wrong_count_and_boolean_money_are_rejected(self):
        for mutate in [lambda d:d['rows'][1].update(user_id=1),
                       lambda d:d['rows'][0].update(checkin_date='2026-02-30'),
                       lambda d:d['rows'][0].update(quota_awarded=True),
                       lambda d:d.update(source_row_count=1),
                       lambda d:d['rows'][0].update(created_at=1793674368)]:
            d = copy.deepcopy(self.export); mutate(d)
            with self.assertRaises(ValueError): self.build(d)

    def test_record_order_is_canonical_but_source_export_changes_are_bound(self):
        a=self.build(); d=copy.deepcopy(self.export); d['rows'].reverse(); b=self.build(d)
        self.assertEqual(a['rows'], b['rows'])
        self.assertNotEqual(a['source_export_sha256'], b['source_export_sha256'])


if __name__ == '__main__':
    unittest.main()

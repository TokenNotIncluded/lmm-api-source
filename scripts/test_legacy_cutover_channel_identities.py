"""Fresh identity plans bind every legacy row without publishing routes or keys."""
import copy
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import legacy_cutover_history as H
import legacy_cutover_import as I


class ChannelIdentitiesTest(unittest.TestCase):
    def setUp(self):
        self.args = SimpleNamespace(archive_sha256='a' * 64, observed_at=1000,
                                    source_schema='legacy', migration_id='opening-1')
        self.rows = [dict(id=1, status=1, type=1, created_time=10, key='secret-live'),
                     dict(id=17, status=2, type=3, created_time=20, key='secret-disabled')]
        self.mapping = dict(format_version=1, archive_sha256='a' * 64,
                            observed_at=1000, source_schema='legacy', channels=[
                                dict(source_channel_id=r['id'], channel_id=r['id'],
                                     source_row_sha256=H.sha(r)) for r in self.rows])

    def material(self, mapping=None, rows=None):
        return H.channel_identity_material(self.args, self.rows if rows is None else rows,
                                           self.mapping if mapping is None else mapping)

    def test_all_statuses_preserved_without_credentials(self):
        result = self.material()
        self.assertEqual([r['status'] for r in result['channels']], [1, 2])
        self.assertNotIn('secret-', json.dumps(result))
        self.assertEqual(result['mapping_sha256'], H.sha(self.mapping))
        self.assertEqual(result['channels'][1]['source_row_sha256'], H.sha(self.rows[1]))

    def test_reject_partial_duplicate_remapped_invalid_and_stale_rows(self):
        cases = []
        m = copy.deepcopy(self.mapping)
        m['channels'].pop()
        cases.append(m)
        m = copy.deepcopy(self.mapping)
        m['channels'][1] = m['channels'][0]
        cases.append(m)
        for field, value in [('channel_id', 18), ('channel_id', True),
                             ('source_row_sha256', 'b' * 64)]:
            m = copy.deepcopy(self.mapping)
            m['channels'][1][field] = value
            cases.append(m)
        for field, value in [('archive_sha256', 'b' * 64), ('observed_at', 1001),
                             ('source_schema', 'other')]:
            m = copy.deepcopy(self.mapping)
            m[field] = value
            cases.append(m)
        for m in cases:
            with self.subTest(mapping=m), self.assertRaises(SystemExit):
                self.material(m)
        with self.assertRaises(SystemExit):
            self.material(rows=self.rows + [self.rows[0]])
        with self.assertRaises(SystemExit):
            self.material(rows=[{**self.rows[0], 'key': 'changed'}, self.rows[1]])

    def plan(self, identities):
        with tempfile.TemporaryDirectory() as d:
            self.args.channel_identity_mapping = Path(d) / 'mapping.json'
            self.args.channel_identity_mapping.write_text(json.dumps(self.mapping))
            with patch.object(H, 'query', side_effect=[self.rows, identities]):
                return H.plan('channel_identities', self.args, object(), object())

    def test_any_existing_identity_is_rejected(self):
        for origin, ambiguous, retired in [('bootstrap', False, None), ('native', False, None),
                                            ('migrated', True, None), ('migrated', False, 1)]:
            with self.subTest(origin=origin), self.assertRaisesRegex(SystemExit, 'target_not_empty'):
                self.plan([dict(id=1, origin=origin, history_ambiguous=ambiguous, retired_at=retired)])

    def test_sql_fences_and_insert_scope(self):
        sql, stats = self.plan([])
        script = '\n'.join(sql)
        self.assertEqual(stats['identities'], 2)
        self.assertIn("current_setting('transaction_isolation')<>'serializable'", script)
        self.assertIn('rolcanlogin OR rolsuper', script)
        self.assertIn('relowner<>', script)
        self.assertIn('LOCK TABLE core_channels.identities', script)
        self.assertIn("migration_id='opening-1' AND source_archive_sha256='" + 'a' * 64, script)
        self.assertIn('AND observed_at=1000', script)
        self.assertIn("jsonb_build_object('opening_source'", script)
        self.assertIn('EXISTS(SELECT 1 FROM public.routing_active_release WHERE config_version IS NOT NULL)', script)
        self.assertNotIn('EXISTS(SELECT 1 FROM public.routing_active_release)', script)
        for table in ('identities', 'counter_baselines', 'balance_observations', 'opening_sources',
                      'opening_baselines'):
            self.assertIn('EXISTS(SELECT 1 FROM core_channels.' + table + ')', script)
        self.assertIn("VALUES(17,'migrated',FALSE)", script)
        self.assertIn('VALUES(17,FALSE)', script)
        self.assertIn("VALUES(17,'unknown')", script)
        inserts = [s.split('(', 1)[0] for s in sql if s.startswith('INSERT')]
        self.assertEqual(set(inserts), {'INSERT INTO core_channels.identities',
                                       'INSERT INTO core_channels.counter_baselines',
                                       'INSERT INTO core_channels.balance_observations'})
        self.assertNotIn('secret-', script)
        self.assertNotIn('ON CONFLICT', script)
        self.assertEqual(I.PHASES[I.PHASES.index('channels_history') - 1], 'channel_identities')
        self.assertEqual(I.GUARDS['channel_identities'], H.GUARDS['channel_identities'])

    def test_explicit_mapping_required(self):
        with self.assertRaisesRegex(SystemExit, 'mapping_required'):
            H.plan_channel_identities(self.args, object(), object())


if __name__ == '__main__':
    unittest.main()

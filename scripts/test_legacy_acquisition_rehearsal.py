# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import json
import unittest
from legacy_acquisition_rehearsal import decode_snapshot_documents


class SnapshotDocumentsTests(unittest.TestCase):
    def test_accepts_multiline_postgres_composite_json_aggregates(self):
        data = {'self_reports': [{'detail': '中文\u2028test\nsecond line'}]}
        audit = {'projection': {'stored': [{'user_id': 1}, {'user_id': 2}]}}
        wire = '\n' + json.dumps(data, ensure_ascii=False) + '\n' + json.dumps(audit, indent=2) + '\n'
        self.assertEqual(decode_snapshot_documents(wire), (data, audit))

    def test_rejects_unexpected_third_result(self):
        with self.assertRaisesRegex(ValueError, 'unexpected source result'):
            decode_snapshot_documents('{}\n{}\n{}')

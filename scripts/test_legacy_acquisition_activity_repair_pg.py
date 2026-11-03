# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import json
import os
from pathlib import Path
import subprocess
import unittest
from legacy_acquisition_activity_repair import repair_sql


def projection():
    saved = {'user_id': 1, 'day': 0, 'first_at': 110, 'last_at': 110}
    return {'contract': 'legacy-acquisition-activity-projection.v1',
            'window': {'started_at': 100, 'scanned_through': 200000},
            'stored': [saved], 'reconstructed': [saved, {'user_id': 1, 'day': 86400, 'first_at': 86410, 'last_at': 86410}]}


@unittest.skipUnless(os.environ.get('LMM_ACQUISITION_REPAIR_TEST_DSN'), 'isolated PostgreSQL DSN required')
class ActivityRepairTests(unittest.TestCase):
    def run_sql(self, sql, check=True):
        return subprocess.run(['psql', os.environ['LMM_ACQUISITION_REPAIR_TEST_DSN'], '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=sqlstate'], input=sql, text=True, capture_output=True, check=check)

    def setUp(self):
        self.assertEqual(self.run_sql("SELECT to_regnamespace('lmm_acquisition') IS NULL").stdout.strip(), 't', 'requires an empty dedicated database')
        schema = Path(__file__).resolve().parents[1] / 'services/extensions/internal/modules/acquisition/schema.sql'
        self.run_sql(schema.read_text())
        self.addCleanup(lambda: self.run_sql('DROP SCHEMA lmm_acquisition CASCADE'))
        self.run_sql("""INSERT INTO lmm_acquisition.accounts(user_id,consent_version) VALUES(1,2);
INSERT INTO lmm_acquisition.consents(user_id,allowed,version,updated_at) VALUES(1,true,2,110);
INSERT INTO lmm_acquisition.cohort(user_id,registered_at,origin) VALUES(1,105,'legacy_import');
INSERT INTO lmm_acquisition.activities(user_id,day,first_at,last_at) VALUES(1,0,110,110);
INSERT INTO lmm_acquisition.activity_state(singleton,started_at,scanned_through,updated_at,status,incomplete) VALUES(true,100,200000,200000,'legacy_mismatch',true);""")

    def test_repairs_missing_rows_and_preserves_incomplete_coverage(self):
        result = json.loads(self.run_sql(repair_sql(projection())).stdout)
        self.assertEqual((result['repaired_missing_rows'], result['daily_rows'], result['activity_status'], result['coverage_complete']), (1, 2, 'legacy_rebuilt', False))
        self.assertNotEqual(self.run_sql(repair_sql(projection()), check=False).returncode, 0)
        self.assertEqual(self.run_sql('SELECT count(*) FROM lmm_acquisition.activities').stdout.strip(), '2')

    def test_changed_consent_or_cache_aborts_without_partial_repair(self):
        for change in ("UPDATE lmm_acquisition.consents SET allowed=false", "UPDATE lmm_acquisition.activities SET first_at=109"):
            self.run_sql('UPDATE lmm_acquisition.consents SET allowed=true; UPDATE lmm_acquisition.activities SET first_at=110; ' + change)
            self.assertNotEqual(self.run_sql(repair_sql(projection()), check=False).returncode, 0)
            self.assertEqual(self.run_sql("SELECT count(*),(SELECT status FROM lmm_acquisition.activity_state) FROM lmm_acquisition.activities").stdout.strip(), '1|legacy_mismatch')

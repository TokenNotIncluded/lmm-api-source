# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Synthetic read-only reconstruction against an explicitly isolated PostgreSQL DB."""
import json
import os
import subprocess
import unittest
from legacy_acquisition_activity_audit import audit_sql


@unittest.skipUnless(os.environ.get('LMM_ACQUISITION_AUDIT_TEST_DSN'), 'isolated PostgreSQL DSN required')
class ActivityAuditTests(unittest.TestCase):
    def psql(self, sql):
        result = subprocess.run(['psql', os.environ['LMM_ACQUISITION_AUDIT_TEST_DSN'], '-X', '-qAt', '-v', 'ON_ERROR_STOP=1'], input=sql, text=True, capture_output=True, check=True)
        return result.stdout

    def test_deduplicates_requests_and_preserves_only_consented_successes(self):
        self.psql('''CREATE SCHEMA acquisition_fixture;
CREATE TABLE acquisition_fixture.users(id bigint,created_at bigint,role int,deleted_at bigint);
INSERT INTO acquisition_fixture.users VALUES(1,10,1,NULL),(2,10,10,NULL),(3,10,1,NULL);
CREATE TABLE acquisition_fixture.acquisition_accounts(user_id bigint,consent_version int);
INSERT INTO acquisition_fixture.acquisition_accounts VALUES(1,2),(2,2),(3,2);
CREATE TABLE acquisition_fixture.acquisition_consents(user_id bigint,allowed bool,version int);
INSERT INTO acquisition_fixture.acquisition_consents VALUES(1,true,2),(2,true,2),(3,false,2);
CREATE TABLE acquisition_fixture.acquisition_activity_states(id int,started_at bigint,scanned_through bigint,incomplete bool);
INSERT INTO acquisition_fixture.acquisition_activity_states VALUES(1,10,200000,false);
CREATE TABLE acquisition_fixture.acquisition_activities(user_id bigint,day bigint,first_at bigint,last_at bigint);
INSERT INTO acquisition_fixture.acquisition_activities VALUES(1,0,100,100);
CREATE TABLE acquisition_fixture.acquisition_activity_gaps(from_at bigint,through_at bigint);
CREATE TABLE acquisition_fixture.logs(user_id bigint,request_id text,created_at bigint,type int,token_id bigint,other text);
INSERT INTO acquisition_fixture.logs VALUES
(1,'same-request',100,2,1,'{"acquisition_success_v1":true}'),
(1,'same-request',86410,2,1,'{"acquisition_success_v1":true}'),
(1,'second-request',86420,2,1,'{"acquisition_success_v1":true}'),
(1,'bad-json',86500,2,1,'not json'),
(1,'string-marker',86501,2,1,'{"acquisition_success_v1":"true"}'),
(1,'no-key',86502,2,0,'{"acquisition_success_v1":true}'),
(1,'',86503,2,1,'{"acquisition_success_v1":true}'),
(2,'staff',86504,2,1,'{"acquisition_success_v1":true}'),
(3,'withdrawn',86505,2,1,'{"acquisition_success_v1":true}');''')
        try:
            result = json.loads(self.psql(audit_sql('acquisition_fixture', True)))
            self.assertEqual((result['reconstructed_rows'], result['stored_rows'], result['missing_or_changed_rows'], result['extra_or_changed_rows']), (2, 1, 1, 0))
            self.assertEqual(result['projection']['reconstructed'], [
                {'user_id': 1, 'day': 0, 'first_at': 100, 'last_at': 100},
                {'user_id': 1, 'day': 86400, 'first_at': 86420, 'last_at': 86420},
            ])
            for row in result['projection']['reconstructed']:
                self.assertEqual(set(row), {'user_id', 'day', 'first_at', 'last_at'})
            # The auditor is read-only: the source cache still has one row.
            self.assertEqual(self.psql('SELECT count(*) FROM acquisition_fixture.acquisition_activities').strip(), '1')
        finally:
            self.psql('DROP SCHEMA acquisition_fixture CASCADE')

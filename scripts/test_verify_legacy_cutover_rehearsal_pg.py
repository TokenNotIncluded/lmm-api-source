# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Run directly to test payable orphan gaps on a private Unix-only PostgreSQL."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from types import SimpleNamespace
import unittest

import verify_legacy_cutover_rehearsal as audit


class PayableOrphanPostgreSQLTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        root = Path.home() / '.local/state/lmm-api/payable-orphan-tests'
        root.mkdir(parents=True, exist_ok=True, mode=0o700)
        cls.output = Path(tempfile.mkdtemp(prefix='run-', dir=root))
        cls.data = cls.output / 'data'
        cls.socket = cls.output / 'sock'
        cls.socket.mkdir(mode=0o700)
        cls.env = {k: v for k, v in os.environ.items()
                   if not k.startswith('PG') and k not in ('DATABASE_URL', 'SQL_DSN')}
        cls.env.update(PGHOST=str(cls.socket), PGPORT='55479', PGUSER='orphan_admin',
                       PGDATABASE='postgres', PGPASSFILE='/dev/null', PGSERVICEFILE='/dev/null')
        cls.command_number = 0
        print('PostgreSQL evidence: ' + str(cls.output), flush=True)
        cls.command(['initdb', '-D', str(cls.data), '--auth-local=trust',
                     '--auth-host=reject', '--username=orphan_admin', '--no-locale', '--encoding=UTF8'])
        cls.command(['pg_ctl', '-D', str(cls.data), '-l', str(cls.output / 'postgres.log'),
                     '-o', f"-k {cls.socket} -p 55479 -c listen_addresses='' -c max_connections=10",
                     '-w', 'start'])
        cls.addClassCleanup(cls.stop)
        cls.sql('''CREATE SCHEMA legacy;
CREATE TABLE legacy.users(id bigint PRIMARY KEY);
CREATE TABLE legacy.top_ups(id bigint PRIMARY KEY, user_id bigint,
 status text, payment_provider text, failure_reason_code text);
INSERT INTO legacy.users VALUES (1);''')

    @classmethod
    def stop(cls):
        cls.command(['pg_ctl', '-D', str(cls.data), '-m', 'fast', '-w', 'stop'])
        shutil.rmtree(cls.data)
        cls.socket.rmdir()

    @classmethod
    def command(cls, argv, sql=None):
        cls.command_number += 1
        prefix = cls.output / f'command-{cls.command_number}'
        if sql is not None:
            prefix.with_suffix('.sql').write_text(sql)
        result = subprocess.run(argv, input=sql, text=True, capture_output=True,
                                env=cls.env, timeout=60, umask=0o077)
        prefix.with_suffix('.json').write_text(json.dumps({
            'argv': argv, 'returncode': result.returncode,
            'stdout': result.stdout, 'stderr': result.stderr}, indent=2))
        if result.returncode:
            raise AssertionError(result.stderr)
        return result.stdout

    @classmethod
    def sql(cls, sql):
        return cls.command(['psql', '-X', '--no-password', '-qAt', '-v', 'ON_ERROR_STOP=1'], sql)

    def check_gap(self, rows, expected):
        self.sql('TRUNCATE legacy.top_ups; INSERT INTO legacy.top_ups VALUES ' + rows)
        args = SimpleNamespace(query_number=0, socket=self.socket, port=55479,
                               source_db='postgres', source_user='orphan_admin',
                               report=self.output / 'report.json')
        def query(sql, source=False):
            self.assertTrue(source)
            (self.output / (self._testMethodName + '.sql')).write_text(sql)
            return audit.read_rows(args, sql, source)
        gaps = audit.payable_topup_orphan_gaps(query, 'legacy')
        (self.output / (self._testMethodName + '.json')).write_text(json.dumps(gaps))
        self.assertEqual(gaps, {'payable_topup_orphan_rows_not_mapped': expected})
        self.assertEqual(audit.reconciliation_status({}, gaps),
                         'SCOPED_PASS_WITH_GAPS' if expected else 'SCOPED_PASS')

    def test_pending_orphan_blocks_scoped_pass(self):
        self.check_gap("(1,99,'pending',NULL,NULL)", 1)

    def test_late_payable_orphan_blocks_scoped_pass(self):
        self.check_gap("(1,99,'failed','waffo_pancake','checkout_timeout')", 1)

    def test_ordinary_failed_orphans_are_closed(self):
        self.check_gap("(1,99,'failed','stripe','checkout_timeout'),"
                       "(2,99,'failed','waffo_pancake','payment_declined')", 0)

    def test_payable_orders_with_owner_are_covered(self):
        self.check_gap("(1,1,'pending',NULL,NULL),"
                       "(2,1,'failed','waffo_pancake','checkout_timeout')", 0)

    def test_mixed_rows_count_both_payable_orphans_only(self):
        self.check_gap("(1,99,'pending',NULL,NULL),"
                       "(2,99,'failed','waffo_pancake','checkout_timeout'),"
                       "(3,99,'failed','stripe','checkout_timeout'),"
                       "(4,1,'pending',NULL,NULL),"
                       "(5,1,'failed','waffo_pancake','checkout_timeout')", 2)


if __name__ == '__main__':
    unittest.main()

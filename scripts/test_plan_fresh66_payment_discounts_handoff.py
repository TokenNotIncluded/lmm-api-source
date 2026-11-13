# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Synthetic local evidence only; these tests execute no migration or upgrade."""
import argparse
import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import plan_fresh66_payment_discounts_handoff as P


class HandoffTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.args = argparse.Namespace()
        review = P.load(P.ROOT / 'scripts/reviewed-fresh66-module-split.json')
        pin = P.load(P.ROOT / 'services/core/src/core_schema/upgrade/payment_discounts_target.json')
        archive = self.root / 'source.snapshot'
        archive.write_bytes(b'SYNTHETIC archive; never a real database')
        self.args.archive = archive
        self.archive_hash = P.digest(archive)
        self.write('source_receipt', {'status': 'snapshot-restored-baseline-verified-frozen-and-classified',
                                     'archive': {'path': str(archive), 'sha256': self.archive_hash}})
        self.instance = {'database': 'synthetic_target', 'database_oid': 42,
                         'server_instance_nonce': 'e' * 64, 'server_version_num': '180006',
                         'system_identifier': '123456789', 'postmaster_started_at': 'synthetic-time',
                         'role': 'synthetic_owner', 'session_user': 'synthetic_installer'}
        self.contract = {'import_profile': 'fresh66-module-split-identity6-complete-seeds-v1',
                         'compiled_manifest_sha256': review['compiled_manifest_sha256'],
                         'fresh_catalog_sha256': review['fresh_catalog_sha256'],
                         'component_fingerprints': {f'fixture_{i}': 'c' * 64 for i in range(66)}}
        self.write('target_receipt', {'status': 'initialized-empty-identity-target',
                                    'source_archive_sha256': self.archive_hash,
                                    'contract': self.contract, 'instance_identity': self.instance,
                                    'connection': {'database': self.instance['database'],
                                                   'role': self.instance['role'], 'login_role': self.instance['session_user']}})
        self.write('target_snapshot', {'instance': self.instance,
                                      'catalog_fingerprint_sha256': review['fresh_catalog_sha256'],
                                      'component_fingerprints': self.contract['component_fingerprints']})
        credential = self.root / 'credential-mapping-receipt.private.json'
        credential.write_text(json.dumps({'fixture': 'credential mapping receipt'}))
        self.write('aliases', {'version': 1, 'scope': 'legacy-api-key-wire-aliases-v1',
                              'source_archive_sha256': self.archive_hash,
                              'credential_receipt_sha256': P.digest(credential),
                              'target': {'database': self.instance['database'], 'database_oid': 42,
                                         'instance_nonce': 'e' * 64}})
        checks = {name: {'source_rows': 2, 'target_rows': 2, 'missing': 0, 'extra': 0,
                         'different': 0, 'passed': True} for name in P.REQUIRED_CHECKS}
        self.write('reconciliation', {'scope': 'read-only-available-core-phase-reconciliation',
                                     'status': 'SCOPED_PASS_WITH_GAPS', 'checks': checks,
                                     'source_gaps': {'legacy_late_payment_callback_not_verified': 1},
                                     'inputs': {'source_archive_sha256': self.archive_hash, 'observed_at': 12345,
                                                'aliases_sha256': P.digest(self.args.aliases),
                                                'verifier_sha256': P.digest(P.ROOT / 'scripts/verify_legacy_cutover_rehearsal.py')}})
        sql = self.root / 'identity.sql'
        sql.write_text('-- SYNTHETIC plan, not executable import evidence\n')
        report = self.write('phase', {'phase': 'identity', 'migration_id': 'fixture-import',
                                    'plan_sha256': P.digest(sql), 'applied': True})
        self.args.phase_report = [report]
        self.write('upgrade_request', {'version': 1, 'procedure': P.PROCEDURE,
                                      'upgrade_id': 'fixture-upgrade-00001',
                                      'database': {'name': self.instance['database'], 'oid': 42,
                                                   'system_identifier': self.instance['system_identifier'],
                                                   'server_version_num': 180006},
                                      'roles': {'installer': 'synthetic_installer', 'owner': 'synthetic_owner',
                                                'runtime': 'synthetic_runtime', 'extensions': 'synthetic_extensions'},
                                      'predecessor_manifest_sha256': review['compiled_manifest_sha256'],
                                      'predecessor_catalog_sha256': review['fresh_catalog_sha256'],
                                      'successor_manifest_sha256': pin['manifest_sha256']})

    def write(self, name, value):
        path = self.root / (name + '.json')
        path.write_text(json.dumps(value))
        setattr(self.args, name, path)
        return path

    def change(self, name, update):
        path = getattr(self.args, name)
        value = P.load(path)
        update(value)
        path.write_text(json.dumps(value))

    def test_rerun_is_identical_and_does_not_turn_gaps_or_booleans_into_acceptance(self):
        files = {p: p.read_bytes() for p in self.root.iterdir()}
        with patch('subprocess.run', side_effect=AssertionError('no processes allowed')):
            first = P.plan(self.args)
            self.assertEqual(first, P.plan(self.args))
        self.assertEqual(first['status'], 'PLAN_ONLY_NOT_EXECUTED')
        self.assertEqual(first['source_gaps'], {'legacy_late_payment_callback_not_verified': 1})
        self.assertNotIn('passed', first)
        self.assertTrue(any('COMMIT' in gap for gap in first['remaining']))
        self.assertEqual({p: p.read_bytes() for p in self.root.iterdir()}, files)
        for binding in first['artifacts'].values():
            self.assertEqual(binding['sha256'], P.digest(binding['path']))

    def test_numeric_mismatch_rejected_even_when_report_claims_passed(self):
        name = sorted(P.REQUIRED_CHECKS)[0]
        self.change('reconciliation', lambda r: r['checks'][name].update(different=1))
        with self.assertRaisesRegex(P.Invalid, 'numeric_mismatch'):
            P.plan(self.args)

    def test_boolean_only_and_missing_comparisons_are_not_accepted(self):
        name = sorted(P.REQUIRED_CHECKS)[0]
        self.change('reconciliation', lambda r: r['checks'].update({name: {'passed': True}}))
        with self.assertRaisesRegex(P.Invalid, 'boolean_only'):
            P.plan(self.args)

    def test_actual_archive_and_phase_sql_bytes_are_bound(self):
        original = self.args.archive.read_bytes()
        self.args.archive.write_bytes(b'changed archive')
        with self.assertRaisesRegex(P.Invalid, 'archive_binding'):
            P.plan(self.args)
        self.args.archive.write_bytes(original)
        (self.root / 'identity.sql').write_text('changed plan')
        with self.assertRaisesRegex(P.Invalid, 'phase_sql_hash'):
            P.plan(self.args)

    def test_wrong_target_incarnation_roles_and_successor_are_rejected(self):
        path = self.args.upgrade_request
        original = path.read_bytes()
        for update, error in [
            (lambda r: r['database'].update(oid=43), 'database_binding'),
            (lambda r: r['database'].update(system_identifier='987654321'), 'database_binding'),
            (lambda r: r['roles'].update(owner='other_owner'), 'role_binding'),
            (lambda r: r.update(successor_manifest_sha256='0' * 64), 'pin_mismatch'),
        ]:
            with self.subTest(error=error):
                path.write_bytes(original)
                self.change('upgrade_request', update)
                with self.assertRaisesRegex(P.Invalid, error):
                    P.plan(self.args)

    def test_old_target_receipt_cannot_be_relabelled_as_fresh67(self):
        self.change('target_receipt', lambda r: r['contract'].update(import_profile='fresh67-payment-discounts'))
        with self.assertRaisesRegex(P.Invalid, 'not_reviewed_fresh66'):
            P.plan(self.args)

    def test_reconciliation_input_hashes_and_alias_instance_are_bound(self):
        self.change('aliases', lambda r: r['target'].update(database_oid=43))
        with self.assertRaisesRegex(P.Invalid, 'aliases_binding'):
            P.plan(self.args)

    def test_unknown_or_unapplied_report_rejected(self):
        self.change('phase', lambda r: r.update(applied=True, status='UNKNOWN'))
        with self.assertRaisesRegex(P.Invalid, 'phase_commit_unknown'):
            P.plan(self.args)
        self.change('phase', lambda r: r.update(applied=False, status='prepared'))
        with self.assertRaisesRegex(P.Invalid, 'phase_not_reported_applied'):
            P.plan(self.args)

    def test_duplicate_json_field_is_rejected(self):
        self.args.upgrade_request.write_text('{"version":1,"version":1}')
        with self.assertRaisesRegex(P.Invalid, 'duplicate_json_key'):
            P.plan(self.args)

    def test_symlink_input_is_rejected(self):
        link = self.root / 'linked.json'
        link.symlink_to(self.args.target_receipt)
        self.args.target_receipt = link
        with self.assertRaisesRegex(P.Invalid, 'regular_evidence'):
            P.plan(self.args)

    def test_cli_emits_plan_only_and_fixed_error_without_receipt_contents(self):
        argv = []
        for name in ('source_receipt', 'archive', 'target_receipt', 'target_snapshot', 'aliases', 'reconciliation', 'upgrade_request'):
            argv += ['--' + name.replace('_', '-'), str(getattr(self.args, name))]
        argv += ['--phase-report', str(self.args.phase_report[0])]
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            self.assertEqual(P.main(argv), 0)
        self.assertEqual(json.loads(stdout.getvalue())['status'], 'PLAN_ONLY_NOT_EXECUTED')
        self.args.source_receipt.write_text('private invalid data')
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            self.assertEqual(P.main(argv), 2)
        self.assertEqual(stdout.getvalue(), '')
        self.assertNotIn('private invalid data', stderr.getvalue())


if __name__ == '__main__':
    unittest.main()

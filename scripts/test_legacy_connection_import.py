import copy
import json
import unittest
import legacy_connection_import as imp


def fixture():
    doc = {'contract': 'legacy.connection-source.v1', 'source_schema': 'synthetic',
           'observed_at': 1000, 'coverage': imp.COVERAGE,
           'users': [{'id': 101, 'created_at': 100}],
           'tokens': [{'id': 8, 'user_id': 101, 'oauth_managed': False,
                       'created_time': 110, 'deleted_at': 'synthetic-deleted'}],
           'oauth_server_grants': [{'id': 'synthetic-grant', 'user_id': 101,
                                    'created_at_ms': 120999, 'revoked_at_ms': 500000}],
           'developer_access_requests': []}
    doc['counts'] = {k: len(doc[k]) for k in imp.TABLES}
    raw = json.dumps(doc).encode()
    metadata = {'contract': 'legacy.connection-export-binding.v1',
                'source_archive_sha256': 'a'*64, 'source_snapshot_id': 'synthetic',
                'source_schema': 'synthetic', 'snapshot_observed_at': 900,
                'export_file_sha256': imp.sha(raw), 'coverage': imp.COVERAGE}
    mapping = {'contract': 'legacy.connection-mapping.v1',
               'users': [{'source_user_id': 101, 'user_id': 11, 'account_id': 1,
                          'opening_entity_key': 'legacy.users/101'}], 'omitted_users': []}
    target = {'contract': 'legacy.connection-target.v1', 'component_count': 55,
              'target_database': 'synthetic_core55', 'target_database_oid': 123,
              'target_nonce': 'd'*64, 'catalog_sha256': 'e'*64, 'native_credential_floor': 8}
    return raw, metadata, json.dumps(mapping).encode(), target


class ConnectionImport(unittest.TestCase):
    def plan(self, args):
        return imp.plan(*args, opening_migration_id='synthetic-opening',
                        opening_mapping_sha256='b'*64, identity_migration_id='synthetic-identity')

    def test_exact_source_hash_and_complete_mapping_required(self):
        for mutation in ('hash', 'count', 'mapping', 'coverage', 'core54', 'duplicate', 'secret-column'):
            with self.subTest(mutation=mutation):
                args = list(fixture())
                doc = json.loads(args[0])
                if mutation == 'hash': args[0] += b'\n'
                elif mutation == 'count': doc['counts']['tokens'] = 2
                elif mutation == 'mapping': args[2] = b'{"contract":"legacy.connection-mapping.v1","users":[],"omitted_users":[]}'
                elif mutation == 'coverage': args[1]['coverage'] = 'active-only'
                elif mutation == 'core54': args[3]['component_count'] = 54
                elif mutation == 'duplicate': doc['tokens'].append(copy.deepcopy(doc['tokens'][0])); doc['counts']['tokens'] = 2
                elif mutation == 'secret-column': doc['tokens'][0]['key'] = 'never-import-a-secret'
                if mutation in ('count', 'duplicate', 'secret-column'):
                    args[0] = json.dumps(doc).encode(); args[1]['export_file_sha256'] = imp.sha(args[0])
                with self.assertRaises(ValueError): self.plan(args)

    def test_missing_facts_and_explicit_omissions_remain_importable(self):
        args = list(fixture()); doc = json.loads(args[0])
        doc['tokens'][0].update(oauth_managed=None, created_time=None)
        args[0] = json.dumps(doc).encode(); args[1]['export_file_sha256'] = imp.sha(args[0])
        self.assertTrue(self.plan(args))
        args[2] = json.dumps({'contract': 'legacy.connection-mapping.v1', 'users': [],
                             'omitted_users': [{'source_user_id': 101, 'reason': 'synthetic unmapped owner'}]}).encode()
        self.assertTrue(self.plan(args))

    def test_duplicate_json_keys_reject(self):
        with self.assertRaises(ValueError): imp.decode(b'{"id":1,"id":2}')


if __name__ == '__main__': unittest.main()

import copy
import tempfile
from pathlib import Path
import unittest

import current_affiliate_aliases as A

REFS = {'source_archive_sha256':'1'*64, 'source_export_sha256':'2'*64,
        'source_export_proof_sha256':'3'*64}


def user(uid, code, **extra):
    return {'id':uid,'aff_code':code,'role':1,'status':1,'deleted_at':None,**extra}


class SourceAliasTests(unittest.TestCase):
    def test_case_distinct_codes_are_preserved_exactly(self):
        p=A.project_users([user(1,'Ab12'),user(2,'ab12')],REFS)
        self.assertEqual([r['code'] for r in p['rows']],['Ab12','ab12'])
        self.assertEqual(A.source_summary(p)['casefold_collision_groups'],1)

    def test_exact_duplicate_code_rejects_the_complete_batch(self):
        with self.assertRaisesRegex(A.Error,'exact_code_collision'):
            A.project_users([user(1,'Ab12'),user(2,'Ab12')],REFS)

    def test_duplicate_owner_cannot_hide_a_null_or_second_code(self):
        with self.assertRaisesRegex(A.Error,'duplicate_source_user'):
            A.project_users([user(1,'Ab12'),user(1,None)],REFS)

    def test_disabled_deleted_and_privileged_ownership_is_not_filtered(self):
        p=A.project_users([user(1,'Ab12',status=2),user(2,'ZZ99',deleted_at='2026-01-01T00:00:00Z'),
                          user(3,'AdM1',role=10,status=2)],REFS)
        self.assertEqual(p['user_count'],3)
        self.assertEqual(p['alias_count'],3)
        self.assertEqual([r['source_status'] for r in p['rows']],[2,1,2])

    def test_null_codes_remain_rows_and_do_not_generate_codes(self):
        p=A.project_users([user(1,None),user(2,None),user(3,'Ab12')],REFS)
        self.assertEqual(p['user_count'],3)
        self.assertEqual(p['alias_count'],1)
        self.assertEqual([r['code'] for r in p['rows']],[None,None,'Ab12'])

    def test_ascii_exact_shape_no_normalization(self):
        for code in ['', 'abc', 'abcde', 'Ab_1', ' Ab1', 'Ab1 ', 'é123', 'Ａb12', 'a'*32]:
            with self.subTest(code=code), self.assertRaisesRegex(A.Error,'source_code_shape'):
                A.project_users([user(1,code)],REFS)

    def test_missing_source_column_is_not_null(self):
        u=user(1,'Ab12');del u['aff_code']
        with self.assertRaisesRegex(A.Error,'columns_missing'):
            A.project_users([u],REFS)

    def test_full_row_hash_binds_non_alias_identity_fields(self):
        first=A.project_users([user(1,'Ab12',auth_version=1)],REFS)
        second=A.project_users([user(1,'Ab12',auth_version=2)],REFS)
        self.assertNotEqual(first['source_users_sha256'],second['source_users_sha256'])
        self.assertNotEqual(first['rows'][0]['source_row_sha256'],second['rows'][0]['source_row_sha256'])

    def test_code_change_changes_full_and_projected_hashes(self):
        first=A.project_users([user(1,'Ab12')],REFS)
        second=A.project_users([user(1,'ab12')],REFS)
        self.assertNotEqual(first['source_codes_sha256'],second['source_codes_sha256'])
        self.assertNotEqual(first['source_users_sha256'],second['source_users_sha256'])

    def test_summary_does_not_publish_invitation_codes(self):
        p=A.project_users([user(1,'Ab12')],REFS)
        s=A.source_summary(p)
        self.assertNotIn('Ab12',A.canonical(s))
        self.assertFalse(s['production_authorization'])
        self.assertFalse(s['source_fence_observed'])

    def test_synthetic_rows_cannot_be_mislabeled_by_same_digest(self):
        real=A.project_users([user(1,'Ab12')],REFS)
        synth=A.project_users([user(1,'Ab12')],REFS,synthetic=True)
        self.assertNotEqual(real['source_kind'],synth['source_kind'])
        self.assertEqual(real['source_users_sha256'],synth['source_users_sha256'])

    def test_missing_actual_identity_receipt_rejects_mapping(self):
        projection=A.project_users([user(1,'Ab12')],REFS)
        with self.assertRaisesRegex(A.Error,'actual_committed_identity_required'):
            A.actual_mapping(None,[],[user(1,'Ab12')],projection)

    def test_different_target_binding_is_rejected_before_manifest_use(self):
        with self.assertRaisesRegex(A.Error,'reviewed_target_binding_changed'):
            A.contract_from_manifest(b'{}',{})

    def test_approved_word_does_not_replace_review_binding(self):
        p={'material':{'source_kind':'VERIFIED_FULL_CANONICAL_SOURCE',
                      'migration_actor_user_id':1,'source_fence_sha256':'a'*64,'target':{}}}
        with self.assertRaisesRegex(A.Error,'exact_review_document_required'):
            A.reviewed(p,{'approved':True})

    def test_source_fence_query_is_read_only_and_complete(self):
        query=A.alias_fence_sql({'system_identifier':'1','database':'lmm_api','schema':'lmm_prod_20260802'})
        self.assertIn('REPEATABLE READ READ ONLY',query)
        self.assertIn('ORDER BY id',query)
        self.assertIn('aff_code',query)
        self.assertNotIn('WHERE status=',query)
        self.assertNotIn('UPDATE ',query)
        self.assertNotIn('ALTER ',query)

    def test_file_bytes_change_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'source.json';p.write_text('{}');p.chmod(0o600)
            ref=A.file_ref(p);p.write_text('[]')
            with self.assertRaisesRegex(A.Error,'file_bytes_changed'):
                A.read_ref(ref)


if __name__=='__main__':
    unittest.main()

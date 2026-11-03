# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
import base64
import copy
import os
from pathlib import Path
import tempfile
import unittest
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
import verify_hero_sms_payloads as h


class PayloadTests(unittest.TestCase):
    def setUp(self):
        self.key=h.legacy_payload_key('synthetic-hero-payload-interop-20261104-ABCxyz-0123456789')
        nonce=b'0123456789ab'
        body=AESGCM(self.key).encrypt(nonce,'合成测试'.encode(),None)
        self.cipher='v1:'+base64.urlsafe_b64encode(nonce+body).decode().rstrip('=')
        self.manifest={'scope':'hero-sms-history.v1','tables':{'hero_sms_sms_orders':[{field:self.cipher for field in h.FIELDS}]}}

    def test_authenticated_payloads_only_produce_counts_not_values_or_plaintext_hashes(self):
        self.assertEqual(h.decrypt_payload(self.cipher,self.key),'合成测试')
        r=h.verify(self.manifest,self.key)
        self.assertEqual((r['status'],r['encrypted_cells'],r['authenticated_cells']),('VERIFIED',4,4))
        self.assertFalse(r['plaintexts_written']);self.assertFalse(r['plaintext_hashes_written'])
        self.assertEqual(h.verify(self.manifest)['status'],'KEY_REQUIRED')

    def test_wrong_key_tampering_and_wrong_envelopes_never_verify(self):
        r=h.verify(self.manifest,b'0'*32)
        self.assertEqual(r['status'],'FAILED');self.assertEqual(r['authenticated_cells'],0)
        for value in ['not-encrypted','v2:AAAA','v1:AA','v1:%%%']:
            m=copy.deepcopy(self.manifest);m['tables']['hero_sms_sms_orders'][0]['phone_ciphertext']=value
            self.assertEqual(h.verify(m,self.key)['status'],'FAILED')
        nonce,body=h.envelope(self.cipher)
        altered=body[:-1]+bytes([body[-1]^1])
        bad='v1:'+base64.urlsafe_b64encode(nonce+altered).decode().rstrip('=')
        with self.assertRaisesRegex(h.PayloadError,'authentication_failed'):h.decrypt_payload(bad,self.key)

    def test_malformed_manifest_is_rejected_without_dumping_payload_data(self):
        for value in [None, [], {'scope':'other'}, {'scope':'hero-sms-history.v1','tables':[]},
                      {'scope':'hero-sms-history.v1','tables':{'hero_sms_sms_orders':['invalid']}}]:
            with self.assertRaises(h.PayloadError):h.verify(value,self.key)
        missing=copy.deepcopy(self.manifest)
        del missing['tables']['hero_sms_sms_orders'][0]['phone_ciphertext']
        self.assertEqual(h.verify(missing,self.key)['status'],'FAILED')

    def test_secret_file_permissions_and_weak_material_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            p=Path(directory)/'key';p.write_text('synthetic-key');p.chmod(0o600)
            self.assertEqual(h.private_bytes(p,4096),b'synthetic-key')
            p.chmod(0o644)
            with self.assertRaises(h.PayloadError):h.private_bytes(p,4096)
        for secret in ['short','x'*64,'replace_with_a_real_key_material_0123456789']:
            with self.assertRaises(h.PayloadError):h.legacy_payload_key(secret)


if __name__=='__main__':unittest.main()

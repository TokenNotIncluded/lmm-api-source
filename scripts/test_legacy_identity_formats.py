"""Legacy-format migration regressions; all credentials are synthetic."""
import base64
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import cbor2
from cryptography.hazmat.primitives.asymmetric import ec, ed25519, rsa
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

import legacy_cutover_history as H
import legacy_cutover_keys as K
from legacy_identity_formats import legacy_cose_key
from test_legacy_cutover_mapping import source_data, token


class LegacyFormats(unittest.TestCase):
    def test_cose_curves_rsa_and_ed25519_keep_key_material(self):
        for alg, crv, curve, size, name in (
            (-7, 1, ec.SECP256R1(), 32, "ES256"),
            (-35, 2, ec.SECP384R1(), 48, "ES384"),
            (-36, 3, ec.SECP521R1(), 66, "ES512"),
        ):
            numbers = ec.generate_private_key(curve).public_key().public_numbers()
            # Deliberately different map order from the old byte-slicing parser.
            key = {-3: numbers.y.to_bytes(size, "big"), -2: numbers.x.to_bytes(size, "big"),
                   -1: crv, 3: alg, 1: 2}
            converted = legacy_cose_key(cbor2.dumps(key))
            self.assertEqual(converted["type_"], name)
            self.assertEqual(converted["key"]["EC_EC2"]["x"], H.b64url(key[-2]))
        numbers = rsa.generate_private_key(public_exponent=65537, key_size=2048).public_key().public_numbers()
        converted = legacy_cose_key(cbor2.dumps({1: 3, 3: -257,
            -1: numbers.n.to_bytes(256, "big"), -2: b"\x01\x00\x01"}))
        self.assertEqual(converted["key"]["RSA"]["e"], [1, 0, 1])
        ed = ed25519.Ed25519PrivateKey.generate().public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        converted = legacy_cose_key(cbor2.dumps({1: 1, 3: -8, -1: 6, -2: ed}))
        self.assertEqual(converted["key"]["EC_OKP"]["x"], H.b64url(ed))

    def test_fixtures_match_the_exact_owner_converter_output(self):
        path = Path(__file__).resolve().parents[1] / "services/core/tests/fixtures/legacy-passkey-formats.json"
        for fixture in json.loads(path.read_text()):
            self.assertEqual(H.passkey_json(fixture["source"])[1], fixture["passkey"])

    def test_unsupported_mismatched_trailing_and_duplicate_cose_refuse(self):
        for raw in (b"", cbor2.dumps({1: 2, 3: -7, -1: 2, -2: bytes(48), -3: bytes(48)}),
                    cbor2.dumps({1: 3, 3: -37, -1: bytes(256), -2: b"\x01\x00\x01"}),
                    cbor2.dumps({1: 1, 3: -8, -1: 6, -2: bytes(31)}),
                    b"\xa2\x01\x02\x01\x03", cbor2.dumps({}) + b"\x00"):
            with self.subTest(raw=raw[:8]), self.assertRaisesRegex(SystemExit, "passkey_cose_key_unsupported"):
                legacy_cose_key(raw)

    def plan_mfa(self, codes):
        factor = {"user_id": 7, "secret": base64.b32encode(b"12345678901234567890").decode(),
                  "is_enabled": True, "deleted_at": None, "last_used": 90}
        with patch.object(H, "query", side_effect=[[factor], codes, [{"user_id": 7}]]), \
                patch.object(H, "load_mfa_key", return_value=("test", bytes(32))):
            return H.plan_mfa(SimpleNamespace(source_schema="synthetic", mfa_keyring="synthetic"), None, None)

    def test_only_live_codes_for_enabled_factors_are_imported(self):
        code = {"id": 1, "user_id": 7, "code_hash": "$2a$10$" + "." * 53,
                "is_used": False, "deleted_at": None}
        sql, stats = self.plan_mfa([code, {**code, "is_used": True},
                                  {**code, "deleted_at": "2026-01-01"}, {**code, "user_id": 8}])
        self.assertEqual(stats["backup_codes_imported"], 1)
        self.assertEqual(stats["backup_codes_not_imported"], 3)
        self.assertEqual(sum("legacy-bcrypt:" in s for s in sql), 1)
        self.assertIn("last_used_step", sql[0])
        for codes, error in (([{**code, "code_hash": "$2a$31$" + "." * 53}], "verifier"),
                             ([code] * 5, "count")):
            with self.assertRaisesRegex(SystemExit, "mfa_backup_.*" + error):
                self.plan_mfa(codes)

    def test_special_keys_produce_bound_rekey_actions_without_secrets(self):
        data = source_data({"accounts": {2: 12, 3: 13}})
        special = token()
        special["key"] = "lmm_" + "a" * 40
        delegated = token(41, 3)
        delegated["oauth_managed"] = True
        deleted = {**token(42), "deleted_at": "2026-01-01", "key": "lmm_" + "c" * 40}
        data["tokens"] = [special, delegated, deleted]
        args = SimpleNamespace(api_key_encryption_key_file=None, observed_at=1000,
                               archive_sha256="a" * 64, migration_id="synthetic")
        info = {"versions": {"2": 1, "3": 1}, "nonce": "b" * 64,
                "database": "synthetic", "database_oid": 1}
        with patch.object(K, "target_info", return_value=info):
            sql, stats = K.plan_keys(data, args, None)
        artifacts = K._pending.pop("keys")
        plan = json.loads(artifacts[K.REKEY_FILE])
        self.assertEqual(plan["replacement_credentials_issued"], 0)
        self.assertEqual([r["action"] for r in plan["records"]],
                         ["authenticated_key_creation", "issuer_reauthorization"])
        self.assertEqual(stats["rekey_owner_session_required"], 1)
        for raw in (t["key"] for t in data["tokens"]):
            self.assertNotIn(raw, "".join(artifacts.values()) + "".join(sql))


if __name__ == "__main__":
    unittest.main()

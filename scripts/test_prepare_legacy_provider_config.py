"""Owner-only provider export planning; no live credentials or HTTP."""
from copy import deepcopy
import base64
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from cryptography.hazmat.primitives.asymmetric import rsa
import prepare_legacy_provider_config as P


def source():
    def b64(number):
        return base64.urlsafe_b64encode(number.to_bytes((number.bit_length() + 7) // 8, "big")).rstrip(b"=").decode()
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048).public_key().public_numbers()
    return {"source_revision": P.SOURCE_REVISION, "source_archive_sha256": "a" * 64,
            "oidc": {"enabled": True, "display_name": "Legacy OIDC", "client_id": "synthetic",
                     "client_secret": "synthetic-private-client-secret"},
            "expected_oidc_issuer": "https://issuer.example.test",
            "oidc_metadata": {"issuer": "https://issuer.example.test",
                "jwks_uri": "https://issuer.example.test/keys",
                "id_token_signing_alg_values_supported": ["RS256"],
                "authorization_endpoint": "https://issuer.example.test/authorize",
                "token_endpoint": "https://issuer.example.test/token",
                "userinfo_endpoint": "https://issuer.example.test/userinfo",
                "jwks": {"keys": [{"kty": "RSA", "alg": "RS256", "n": b64(key.n), "e": b64(key.e)}]}},
            "custom_oauth_providers": [{"id": 9, "slug": "synthetic-custom", "name": "Custom",
                "enabled": True, "client_id": "synthetic-custom-id", "client_secret": "synthetic-custom-secret",
                "authorization_endpoint": "https://custom.example.test/auth",
                "token_endpoint": "https://custom.example.test/token",
                "user_info_endpoint": "https://custom.example.test/userinfo",
                "user_id_field": "data.id", "auth_style": 2}]}


class ProviderPlanTests(unittest.TestCase):
    def test_exact_provider_write_requests_are_disabled_and_bound(self):
        data = source()
        before = deepcopy(data)
        result = P.plan(data)
        self.assertEqual(data, before)
        self.assertEqual(result["blocked"], [])
        self.assertEqual(len(result["requests"]), 2)
        oidc, custom = [r["body"] for r in result["requests"]]
        self.assertEqual(oidc["kind"], "oidc")
        self.assertEqual(oidc["issuer"], data["expected_oidc_issuer"])
        self.assertEqual(oidc["id_token_algorithms"], ["RS256"])
        self.assertEqual(custom["user_id_field"], "data.id")
        self.assertEqual(custom["auth_style"], 2)
        self.assertTrue(all(not r["body"]["enabled"] for r in result["requests"]))
        self.assertEqual(result["requests"][0]["body"]["operation_key"],
                         P.plan(data)["requests"][0]["body"]["operation_key"])
        self.assertEqual(result["target_writes"], 0)

    def test_missing_capture_or_secret_remains_explicitly_blocked(self):
        data = source()
        del data["oidc_metadata"]
        del data["custom_oauth_providers"][0]["client_secret"]
        result = P.plan(data)
        self.assertEqual(len(result["blocked"]), 2)
        self.assertEqual(result["requests"], [])

    def test_issuer_jwks_algorithm_and_endpoint_mismatches_reject(self):
        base = source()
        for field, value in (("issuer", "https://other.example.test"),
                             ("jwks_uri", "http://issuer.example.test/keys"),
                             ("id_token_signing_alg_values_supported", ["none"]),
                             ("id_token_signing_alg_values_supported", ["ES256"]),
                             ("jwks", {"keys": [{"kty": "RSA", "n": "bad", "e": "bad"}]})):
            data = deepcopy(base)
            data["oidc_metadata"][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                P.plan(data)

    def test_cli_enforces_private_files_refuses_overwrite_and_never_prints_secrets(self):
        with tempfile.TemporaryDirectory(prefix="provider-plan-") as folder:
            root = Path(folder)
            src, out = root / "source.json", root / "requests.json"
            data = source()
            src.write_text(json.dumps(data))
            src.chmod(0o600)
            command = [sys.executable, "-B", str(Path(P.__file__)), "--source", str(src), "--output", str(out)]
            for expected in (0, 1):
                result = subprocess.run(command, text=True, capture_output=True)
                self.assertEqual(result.returncode, expected, result.stderr)
                for secret in (data["oidc"]["client_secret"], data["custom_oauth_providers"][0]["client_secret"]):
                    self.assertNotIn(secret, result.stdout + result.stderr)
            self.assertEqual(out.stat().st_mode & 0o777, 0o600)
            self.assertFalse(json.loads(out.read_text())["requests"][0]["body"]["enabled"])


if __name__ == "__main__":
    unittest.main()

"""Captured legacy user-verification policy reaches the normal owner producer."""
import base64
from copy import deepcopy
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import legacy_cutover_history as H


def row():
    cose = H.COSE_ES256 + bytes(32) + bytes.fromhex("225820") + bytes(32)
    return {"id": 1, "user_id": 71, "credential_id": base64.b64encode(b"synthetic-key").decode(),
            "public_key": base64.b64encode(cose).decode(), "sign_count": 137,
            "user_verified": False, "backup_eligible": True, "backup_state": True,
            "transports": '["usb"]', "name": "Synthetic old key",
            "created_at": "2026-01-01T00:00:00Z", "last_used_at": None, "deleted_at": None}


class SourcePasskeyPolicyTests(unittest.TestCase):
    def plan(self, options):
        source = row()
        before = deepcopy(source)
        captured = []
        args = SimpleNamespace(source_schema="synthetic", archive_sha256="a" * 64,
            migration_id="synthetic-passkeys", observed_at=1000,
            identity_binding={"provenance": {"identity_manifest_sha256": "b" * 64}})
        with patch.object(H, "query", side_effect=[[source], [{"id": 71}], options]), \
                patch.object(H, "jlit", side_effect=lambda value: captured.append(value) or "'{}'::jsonb"):
            _, stats = H.plan_passkeys(args, object(), object())
        self.assertEqual(source, before)
        self.assertEqual(stats["passkeys_active"], 1)
        return captured[0]["cred"], captured[-1]

    def test_missing_and_empty_use_exact_cc52_preferred_default(self):
        for options in ([], [{"key": "passkey.user_verification", "value": ""}]):
            with self.subTest(options=options):
                credential, receipt = self.plan(options)
                self.assertEqual(credential["registration_policy"], "preferred")
                self.assertEqual(receipt["source_policy"]["user_verification"], "preferred")
                self.assertEqual(receipt["source_policy"]["captured_options"], options)
                self.assertEqual(receipt["sessions_issued"], 0)

    def test_explicit_policy_and_original_flags_counter_are_preserved(self):
        for policy in ("preferred", "required"):
            with self.subTest(policy=policy):
                credential, receipt = self.plan([{"key": "passkey.user_verification", "value": policy}])
                self.assertEqual(credential["registration_policy"], policy)
                self.assertEqual((credential["user_verified"], credential["backup_eligible"],
                                  credential["backup_state"], credential["counter"]), (False, True, True, 137))
                self.assertEqual(receipt["source_archive_sha256"], "a" * 64)
                self.assertEqual(receipt["identity_manifest_sha256"], "b" * 64)

    def test_unknown_or_null_policy_refuses_before_any_apply(self):
        for policy in (None, "unknown", "discouraged", "Preferred"):
            with self.subTest(policy=policy), self.assertRaisesRegex(SystemExit, "passkey_source_verification_unsupported"):
                self.plan([{"key": "passkey.user_verification", "value": policy}])

    def test_existing_direct_converter_default_stays_required(self):
        _, key = H.passkey_json(row())
        self.assertEqual(key["cred"]["registration_policy"], "required")


if __name__ == "__main__":
    unittest.main()

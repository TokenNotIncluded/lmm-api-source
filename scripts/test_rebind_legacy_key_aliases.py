#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import legacy_credential_import as C
import rebind_legacy_key_aliases as R


class RebindTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=Path.home() / ".local/state/lmm-api")
        self.root = Path(self.temp.name)
        os.chmod(self.root, 0o700)
        self.receipt = {"scope": C.RECEIPT_SCOPE, "source_archive_sha256": "a" * 64,
                        "records": [{"source_key_id": 1, "target_credential_id": 2,
                                     "target_user_id": 3, "target_account_id": 4,
                                     "source_secret_sha256": "c" * 64}]}
        self.alias = {"version": 1, "scope": "legacy-api-key-wire-aliases-v1",
                      "source_archive_sha256": "a" * 64,
                      "credential_receipt_sha256": self.sha(self.receipt),
                      "target": {"database": "original", "database_oid": 1,
                                 "instance_nonce": "d" * 64},
                      "keys": [{**self.receipt["records"][0], "credential_user_version": 5,
                                "sk_alias_sha256": "e" * 64}]}
        self.actual = {"database": "recovered", "database_oid": "9",
                       "server_instance_nonce": "f" * 64}
        self.target = {"source_archive_sha256": "a" * 64,
                       "application_or_worker_started": False,
                       "connection": {"database": "recovered", "role": "recovery_owner"},
                       "instance_identity": self.actual}

    def tearDown(self):
        self.temp.cleanup()

    @staticmethod
    def sha(value):
        raw = C.canonical(value).encode()
        return hashlib.sha256(raw).hexdigest()

    def write(self, name, value):
        path = self.root / name
        path.write_text(C.canonical(value))
        path.chmod(0o600)
        return path

    def args(self, report=None, target=None):
        alias = self.write("alias", self.alias)
        receipt = self.write("receipt", self.receipt)
        report = self.write("report", report if report is not None else {
            "phase": "keys", "applied": True, "migration_id": "rebind-test",
            "stats": {"tokens": 1, "alias_keys": 1},
            "reconciliation": {"legacy_tokens": 1, "core_api_keys": 1,
                               "extra_core_keys": 0, "mismatches": {}},
            "artifacts": {R.RECEIPT_FILE: {"path": str(receipt), "sha256": self.sha(self.receipt)},
                          R.ALIAS_FILE: {"path": str(alias), "sha256": self.sha(self.alias)}}})
        target = self.write("target", target if target is not None else self.target)
        return SimpleNamespace(alias_manifest=alias, credential_receipt=receipt,
            commit_report=report, target_receipt=target,
            target_receipt_sha256=hashlib.sha256(target.read_bytes()).hexdigest(),
            output=self.root / "out")

    def test_rebind_preserves_keys_and_receipt_bytes(self):
        args = self.args()
        with patch.object(R.H, "psql", return_value=json.dumps(self.actual)), \
                patch.object(R.H, "verify_instance"):
            result = R.run(args)
        rebound = json.loads((args.output / R.ALIAS_FILE).read_text())
        self.assertEqual(result["credential_count"], 1)
        self.assertEqual(rebound["keys"], self.alias["keys"])
        self.assertEqual(rebound["target"], {"database": "recovered", "database_oid": 9,
                                             "instance_nonce": "f" * 64})
        self.assertEqual((args.output / R.RECEIPT_FILE).read_text(), C.canonical(self.receipt))
        receipt = json.loads((args.output / "rebinding-receipt.private.json").read_text())
        self.assertFalse(receipt["database_writes"])
        self.assertFalse(receipt["key_mappings_changed"])

    def test_uncommitted_or_changed_mapping_is_rejected(self):
        args = self.args(report={"phase": "keys", "applied": False})
        with self.assertRaises(R.H.MappingError):
            R.run(args)
        args = self.args()
        args.alias_manifest.write_text(C.canonical({**self.alias, "keys": []}))
        args.alias_manifest.chmod(0o600)
        with patch.object(R.H, "psql", return_value=json.dumps(self.actual)), \
                patch.object(R.H, "verify_instance"):
            with self.assertRaises(R.H.MappingError):
                R.run(args)


if __name__ == "__main__":
    unittest.main()

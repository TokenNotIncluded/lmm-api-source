"""Targeted restart binding tests. Actual PostgreSQL coverage lives in the PG suite."""
import copy
from pathlib import Path
import sys
import shutil
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import financial_import as F


class RestartBindingTests(unittest.TestCase):
    def setUp(self):
        self.instance = {"postmaster_started_at": "2026-11-11T12:00:00+00:00",
            "system_identifier": "123", "database_oid": 42, "server_instance_nonce": "nonce",
            "role": "owner", "database_owner": "owner", "session_user": "installer", "port": 65447}
        self.material = {"scope": F.SCOPE, "source_archive_sha256": "a" * 64,
                         "target": {"instance": self.instance}, "tool_source_hashes": F.source_hashes()}
        self.files = {"material": "b" * 64, "sql": "c" * 64, "approval": "d" * 64, "attempt": "e" * 64}
        self.proof = {"format_version": 1, "scope": "financial-same-data-directory-restart-v1",
            "source_archive_sha256": "a" * 64, "files": self.files,
            "restarted_instance": {**self.instance, "postmaster_started_at": "2026-11-11T13:00:00+00:00"}}

    def test_only_newer_start_can_differ(self):
        for field in ("system_identifier", "database_oid", "server_instance_nonce", "role", "database_owner", "session_user", "port"):
            with self.subTest(field=field):
                proof = copy.deepcopy(self.proof)
                proof["restarted_instance"][field] = "wrong"
                with self.assertRaisesRegex(F.Error, "restart_instance_fields_mismatch"):
                    F.validate_restart_proof(proof, self.material, self.files)
        self.proof["restarted_instance"]["postmaster_started_at"] = "2026-11-11T11:00:00+00:00"
        with self.assertRaisesRegex(F.Error, "restart_start_not_newer"):
            F.validate_restart_proof(self.proof, self.material, self.files)

    def test_source_and_all_original_file_bytes_are_bound(self):
        for field in self.files:
            with self.subTest(field=field):
                proof = copy.deepcopy(self.proof)
                proof["files"][field] = "0" * 64
                with self.assertRaisesRegex(F.Error, "restart_evidence_binding_mismatch"):
                    F.validate_restart_proof(proof, self.material, self.files)
        self.proof["source_archive_sha256"] = "0" * 64
        with self.assertRaisesRegex(F.Error, "restart_evidence_binding_mismatch"):
            F.validate_restart_proof(self.proof, self.material, self.files)

    def test_continuity_is_mandatory(self):
        with self.assertRaisesRegex(F.Error, "restart_continuity_missing"):
            F.validate_restart_proof(self.proof, self.material, self.files)

    def test_original_writer_hash_mismatch_rejected_before_import(self):
        self.material["tool_source_hashes"]["financial_writer_sha256"] = "0" * 64
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "original-tools"
            root.mkdir(mode=0o700)
            scripts = root / "scripts"
            scripts.mkdir(mode=0o700)
            for filename in ("financial_import.py", "migrate-legacy-financial.py", "migrate-legacy-snapshot.py",
                             "legacy_financial_preflight.py", "legacy_cutover_money.py"):
                shutil.copyfile(Path(__file__).with_name(filename), scripts / filename)
            with patch.object(F.IDENTITY, "BACKUP_ROOT", Path(temporary)):
                with self.assertRaisesRegex(F.Error, "restart_canonical_source_mismatch"):
                    F.restart_canonical(root, self.material)

    def test_restart_flags_never_enable_apply(self):
        argv = ["financial_import.py", "--apply", "--restart-proof", "proof", "--canonical-source-root", "root", "--output", "unused"]
        with patch.object(sys, "argv", argv), patch.object(F, "execute") as execute:
            self.assertEqual(F.main(), 1)
            execute.assert_not_called()

    def test_foreign_cached_dependency_is_replaced_only_with_bound_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            scripts = root / "scripts"
            scripts.mkdir()
            for filename in ("financial_import.py", "migrate-legacy-financial.py", "migrate-legacy-snapshot.py",
                             "legacy_financial_preflight.py", "legacy_cutover_money.py"):
                shutil.copyfile(Path(__file__).with_name(filename), scripts / filename)
            for profile in Path(__file__).parent.glob("reviewed-fresh*.json"):
                shutil.copyfile(profile, scripts / profile.name)
            foreign = SimpleNamespace(__file__=__file__)
            previous_path = list(sys.path)
            try:
                with patch.dict(sys.modules, {"legacy_financial_preflight": foreign}), \
                        patch.object(F.IDENTITY, "private_backup_path", side_effect=lambda path, **kw: path):
                    original = F.restart_canonical(root, self.material)
                    self.assertEqual(original.source_hashes(), self.material["tool_source_hashes"])
                    self.assertEqual(Path(original.PREFLIGHT.__file__), scripts / "legacy_financial_preflight.py")
            finally:
                sys.path[:] = previous_path

    def test_foreign_mapping_error_retains_fixed_refusal_code(self):
        ForeignError = type("MappingError", (Exception,), {})
        with patch.object(F, "_resolve_restarted", side_effect=ForeignError("source_file_changed")):
            with self.assertRaisesRegex(F.Error, "source_file_changed"):
                F.resolve_restarted(None, None)


if __name__ == "__main__":
    unittest.main()

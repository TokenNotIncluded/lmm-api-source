import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import prepare_legacy_admin_key_continuity as P


def fixture():
    users = [dict(id=37, role=100, status=1, auth_version=6, deleted_at=None,
                  password="SyntheticPasswordHashOnly", access_token="SyntheticConsoleBearerOnly"),
             dict(id=8, role=1, status=1, auth_version=1, deleted_at=None)]
    tokens = []
    for key in range(1, 7):
        tokens.append(dict(id=key, user_id=8 if key == 6 else 37, key=f"SyntheticPrivateToken{key:020d}",
                           status=1, oauth_managed=key == 3, creation_source="assistant_runtime" if key == 4 else "manual",
                           deleted_at="2026-01-01T00:00:00Z" if key == 2 else None,
                           used_quota=None, remain_quota=None, unlimited_quota=False, expired_time=-1,
                           extra_original_column={"nested": True}))
    tokens[4]["key"] = "oauth_managed_" + "x" * 32
    raw = dict(users=users, tokens=tokens)
    mapping = [dict(source_user_id=8, target_user_id=8, target_personal_account_id=99),
               dict(source_user_id=37, target_user_id=37, target_personal_account_id=8)]
    records, aliases = [], []
    for token in tokens:
        key = P.key_projection(token)
        record = dict(source_key_id=token["id"], target_credential_id=token["id"],
                      target_user_id=token["user_id"], target_account_id=99 if token["user_id"] == 8 else 8,
                      source_secret_sha256=key["source_secret_sha256"])
        records.append(record)
        aliases.append(dict(record, credential_user_version=6,
                            sk_alias_sha256=hashlib.sha256(("sk-" + token["key"]).encode()).hexdigest(),
                            **({"revoked_rekey_required": True} if key["managed"] else {})))
    documents = dict(raw_export=raw, source_receipt=dict(archive=dict(sha256="a" * 64),
                      rehearsal_only=True, future_cutover_valid=False), identity_map=mapping,
                     identity_manifest=dict(format_version=2, scope="identity-only", source_archive_sha256="a" * 64,
                                            mapping_sha256="b" * 64),
                     credential_receipt=dict(scope="a02-credential-mapping-receipt", source_archive_sha256="a" * 64,
                                             records=records),
                     alias_manifest=dict(version=1, scope="legacy-api-key-wire-aliases-v1", source_archive_sha256="a" * 64,
                                         keys=aliases), key_import_sql="-- Synthetic original committed SQL\n")
    documents["export_receipt"] = dict(source_archive_sha256="a" * 64, users=2, tokens=6,
                                        readonly_source_transaction=True, source_instance_verified=True)
    return documents


def encode(documents):
    encoded = {name: (P.F.canonical(value) + "\n").encode() for name, value in documents.items()
               if name not in ("identity_map", "key_import_sql", "export_receipt", "identity_manifest", "alias_manifest")}
    encoded["identity_map"] = "".join(P.F.canonical(row) + "\n" for row in documents["identity_map"]).encode()
    documents["identity_manifest"]["source_target_id_map_sha256"] = hashlib.sha256(encoded["identity_map"]).hexdigest()
    documents["alias_manifest"]["credential_receipt_sha256"] = hashlib.sha256(encoded["credential_receipt"]).hexdigest()
    documents["export_receipt"].update(raw_export_sha256=hashlib.sha256(encoded["raw_export"]).hexdigest(),
        source_receipt_sha256=hashlib.sha256(encoded["source_receipt"]).hexdigest())
    for name in ("identity_manifest", "alias_manifest", "export_receipt"):
        encoded[name] = (P.F.canonical(documents[name]) + "\n").encode()
    encoded["key_import_sql"] = documents["key_import_sql"].encode()
    return encoded


PARAMETERS = dict(source_user_id=37, observed_at=1_800_000_000,
                  identity_migration_id="synthetic-identity-01", key_import_migration_id="synthetic-keys-01")


def prepare(documents):
    hashes = {name: hashlib.sha256(data).hexdigest() for name, data in encode(documents).items()}
    return P.prepare(documents, hashes, **PARAMETERS)


class ContinuityTests(unittest.TestCase):
    def test_complete_cohort_and_original_hashes(self):
        documents = fixture()
        result = prepare(documents)
        self.assertEqual([key["source_key_id"] for key in result["keys"]], [1, 2, 3, 4, 5])
        self.assertEqual([key["managed"] for key in result["keys"]], [False, False, True, True, True])
        self.assertTrue(result["keys"][1]["deleted"])
        self.assertEqual(result["source_user_sha256"], P.LCI.digest(documents["raw_export"]["users"][0]))
        self.assertEqual(result["keys"][0]["source_row_sha256"], P.LCI.digest(documents["raw_export"]["tokens"][0]))
        self.assertNotEqual(result["keys"][0]["source_row_sha256"], P.F.digest(documents["raw_export"]["tokens"][0]))
        self.assertEqual(result["keys"][0]["used_quota"], 0)
        self.assertEqual(result["keys"][0]["remain_quota"], 0)
        self.assertEqual(result["keys"][0]["expires_at_unix"], 253402300799)

    def test_inclusive_expiry_and_signed_quota_preserved(self):
        documents = fixture()
        token = documents["raw_export"]["tokens"][0]
        token.update(expired_time=10, used_quota=17, remain_quota=-9)
        key = prepare(documents)["keys"][0]
        self.assertEqual((key["expires_at_unix"], key["used_quota"], key["remain_quota"]), (11, 17, -9))
        token["expired_time"] = None
        self.assertEqual(prepare(documents)["keys"][0]["expires_at_unix"], P.LCI.KEYS.NEVER)

    def test_raw_extra_column_changes_row_hash(self):
        documents = fixture()
        before = prepare(documents)["keys"][0]["source_row_sha256"]
        documents["raw_export"]["tokens"][0]["extra_original_column"]["nested"] = False
        self.assertNotEqual(before, prepare(documents)["keys"][0]["source_row_sha256"])

    def test_wrong_owner_account_and_cross_user_collision_rejected(self):
        documents = fixture()
        documents["credential_receipt"]["records"][0]["target_account_id"] = 37
        documents["alias_manifest"]["keys"][0]["target_account_id"] = 37
        with self.assertRaises(Exception):
            prepare(documents)

    def test_missing_or_duplicate_cohort_receipt_and_alias_rejected(self):
        for name, field in (("raw_export", "tokens"), ("credential_receipt", "records"), ("alias_manifest", "keys")):
            for duplicate in (False, True):
                documents = fixture()
                rows = documents[name][field]
                if duplicate:
                    rows[-1] = copy.deepcopy(rows[0])
                else:
                    rows.pop()
                with self.subTest(name=name, duplicate=duplicate), self.assertRaises(Exception):
                    prepare(documents)

    def test_bad_digest_managed_flag_and_alias_rejected(self):
        for field, value in (("source_secret_sha256", "e" * 64), ("sk_alias_sha256", "f" * 64),
                             ("target_user_id", True), ("revoked_rekey_required", True)):
            documents = fixture()
            documents["alias_manifest"]["keys"][0][field] = value
            with self.subTest(field=field), self.assertRaises(Exception):
                prepare(documents)

    def test_source_and_credential_hash_bindings_rejected_when_changed(self):
        for name in ("raw_export", "source_receipt", "identity_map", "credential_receipt"):
            documents = fixture()
            hashes = {key: hashlib.sha256(data).hexdigest() for key, data in encode(documents).items()}
            hashes[name] = "0" * 64
            with self.subTest(name=name), self.assertRaises(Exception):
                P.prepare(documents, hashes, **PARAMETERS)

    def test_bad_present_scalar_is_not_defaulted(self):
        for field in ("expired_time", "used_quota", "remain_quota", "status", "unlimited_quota", "oauth_managed"):
            documents = fixture()
            documents["raw_export"]["tokens"][0][field] = "bad-present"
            with self.subTest(field=field), self.assertRaises(Exception):
                prepare(documents)
        documents = fixture()
        documents["raw_export"]["tokens"][0]["expired_time"] = -1.0
        with self.assertRaises(Exception):
            prepare(documents)

    def test_duplicate_json_and_nonfinite_rejected(self):
        for data in (b'{"x":1,"x":2}', b'{"x":NaN}'):
            with self.assertRaises(Exception):
                P.read_json(data)

    def test_private_files_five_paths_no_secret_and_replay_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            backup = Path(temporary)
            root = backup / "case"
            root.mkdir(mode=0o700)
            documents = fixture()
            paths = {}
            for name, data in encode(documents).items():
                paths[name] = root / (name + ".private.json")
                paths[name].write_bytes(data)
                paths[name].chmod(0o600)
            before = {name: path.read_bytes() for name, path in paths.items()}
            output = root / "prepared"
            with patch.object(P.F.IDENTITY, "BACKUP_ROOT", backup):
                receipt = P.run(paths, output, **PARAMETERS)
                with self.assertRaises(FileExistsError):
                    P.run(paths, output, **PARAMETERS)
            self.assertFalse(receipt["source_future_cutover_valid"])
            self.assertFalse(receipt["activation_created"])
            materials = json.loads((output / "materials.private.json").read_bytes())
            self.assertEqual(set(materials), set(P.MATERIALS) | {"source_projection"})
            projection = json.loads((output / "source-projection.private.json").read_bytes())
            for field, source in (("source_receipt_file", "source_receipt"),
                                  ("key_import_sql_file", "key_import_sql")):
                self.assertEqual(projection[field], str(paths[source].resolve()))
                self.assertEqual(hashlib.sha256(Path(projection[field]).read_bytes()).hexdigest(),
                                 receipt["input_sha256"][source])
            normalized = output / "source-rows.normalized.private.json"
            self.assertEqual(projection["source_rows_file"], str(normalized))
            self.assertEqual(json.loads(normalized.read_bytes()), documents["raw_export"])
            self.assertEqual(hashlib.sha256(normalized.read_bytes()).hexdigest(),
                             receipt["normalized_source_rows_sha256"])
            self.assertEqual(output.stat().st_mode & 0o777, 0o700)
            for path in output.iterdir():
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
                if path == normalized:
                    continue
                for token in documents["raw_export"]["tokens"]:
                    self.assertNotIn(token["key"].encode(), path.read_bytes())
                for field in ("password", "access_token"):
                    self.assertNotIn(documents["raw_export"]["users"][0][field].encode(), path.read_bytes())
            self.assertEqual(before, {name: path.read_bytes() for name, path in paths.items()})

    def test_normalized_decimal_unicode_and_integer_quota(self):
        raw = b'{"users":[{"score":0.000000,"name":"\\u4e2d"}],"tokens":[{"quota":9007199254740993}]}'
        parsed = P.read_json(raw)
        normalized = json.dumps(parsed, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        self.assertIn('"score":0.0', normalized)
        self.assertIn('中', normalized)
        self.assertEqual(json.loads(raw, parse_float=P.Decimal),
                         json.loads(normalized, parse_float=P.Decimal))
        self.assertEqual(parsed["tokens"][0]["quota"], 9007199254740993)

    def test_symlink_and_public_input_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "input"
            path.write_text("{}")
            with patch.object(P.F.IDENTITY, "BACKUP_ROOT", root):
                with self.assertRaises(Exception):
                    P.PRIVATE.private_bytes(path)
                path.chmod(0o600)
                link = root / "link"
                link.symlink_to(path)
                with self.assertRaises(Exception):
                    P.PRIVATE.private_bytes(link)

    def test_normal_cli_success_failure_and_no_secret_diagnostics(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / ".local/state/lmm-api/fixture"
            root.mkdir(mode=0o700, parents=True)
            root.parent.chmod(0o700)
            documents = fixture()
            arguments = [sys.executable, str(Path(P.__file__).resolve())]
            for name, data in encode(documents).items():
                path = root / (name + ".private.json")
                path.write_bytes(data)
                path.chmod(0o600)
                arguments.extend(["--" + name.replace("_", "-"), str(path)])
            for name, value in dict(PARAMETERS, output=root / "output").items():
                arguments.extend(["--" + name.replace("_", "-"), str(value)])
            environment = dict(os.environ, HOME=temporary)
            success = subprocess.run(arguments, env=environment, capture_output=True, check=False)
            self.assertEqual((success.returncode, success.stderr), (0, b""))
            self.assertEqual(success.stdout, b"legacy-admin-key-continuity-prepared\n")
            (root / "raw_export.private.json").write_text('{"secret":"SyntheticPrivateToken00000000000000000001",BROKEN')
            failure = subprocess.run(arguments, env=environment, capture_output=True, check=False)
            self.assertEqual(failure.returncode, 1)
            self.assertEqual(failure.stderr, b"error: legacy_admin_key_continuity_prepare_failed\n")
            self.assertEqual(failure.stdout, b"")


if __name__ == "__main__":
    unittest.main()

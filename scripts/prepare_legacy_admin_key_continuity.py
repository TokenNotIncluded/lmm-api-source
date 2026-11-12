#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Offline source projection for the owner-only admin-key continuity planner.

Consumes complete raw SELECT * exports. Never connects, activates credentials,
or creates an approval. Core independently verifies the five material files.
"""
import argparse
from decimal import Decimal
import hashlib
import json
from pathlib import Path
import sys

import financial_import as F
import legacy_credential_import as PRIVATE
import legacy_cutover_import as LCI

SCOPE = "legacy-admin-key-continuity-source.v1"
INPUTS = ("raw_export", "export_receipt", "source_receipt", "identity_manifest",
          "identity_map", "credential_receipt", "alias_manifest", "key_import_sql")
MATERIALS = ("identity_manifest", "identity_map", "credential_receipt", "alias_manifest")


def integer(value, minimum=-(2**63), maximum=2**63 - 1):
    F.require(type(value) is int and minimum <= value <= maximum, "continuity_integer_invalid")
    return value


def objects(rows):
    F.require(type(rows) is list and all(type(row) is dict for row in rows), "continuity_rows_invalid")
    result = {}
    for row in rows:
        key = integer(row.get("id"), 1)
        F.require(key not in result, "continuity_duplicate_row")
        result[key] = row
    return result


def key_projection(row):
    raw = row["key"]
    F.require(type(raw) is str and raw, "continuity_secret_invalid")
    F.require(type(row["oauth_managed"]) in (bool, type(None))
              and type(row["unlimited_quota"]) is bool, "continuity_boolean_invalid")
    expiry = row["expired_time"]
    if expiry is not None:
        expiry = integer(expiry)
    expires = LCI.KEYS.NEVER if expiry is None or expiry == -1 else integer(expiry + 1)
    return dict(source_key_id=integer(row["id"], 1), source_user_id=integer(row["user_id"], 1),
                source_secret_sha256=hashlib.sha256(raw.encode("utf-8")).hexdigest(),
                source_row_sha256=LCI.digest(row), deleted=row["deleted_at"] is not None,
                managed=bool(row["oauth_managed"]) or row["creation_source"] == "assistant_runtime"
                or not LCI.KEYS.ordinary(raw), status=integer(row["status"], -32768, 32767),
                used_quota=integer(0 if row["used_quota"] is None else row["used_quota"]),
                remain_quota=integer(0 if row["remain_quota"] is None else row["remain_quota"]),
                unlimited_quota=row["unlimited_quota"], expires_at_unix=expires)


def prepare(documents, hashes, *, source_user_id, observed_at, identity_migration_id, key_import_migration_id):
    """Validate immutable file bindings and preserve every source key row."""
    integer(source_user_id, 1)
    integer(observed_at, 1, LCI.KEYS.NEVER)
    for migration in (identity_migration_id, key_import_migration_id):
        F.require(type(migration) is str and F.IDENTITY.MIGRATION_ID.fullmatch(migration),
                  "continuity_migration_id_invalid")
    for name in INPUTS:
        F.checked_hash(hashes[name])
    raw, export = documents["raw_export"], documents["export_receipt"]
    F.require(type(raw) is dict and set(raw) == {"users", "tokens"}, "continuity_export_shape")
    users, tokens = objects(raw["users"]), objects(raw["tokens"])
    archive = F.checked_hash(export["source_archive_sha256"])
    F.require(export["raw_export_sha256"] == hashes["raw_export"]
              and export["source_receipt_sha256"] == hashes["source_receipt"]
              and documents["source_receipt"]["archive"]["sha256"] == archive,
              "continuity_export_binding_mismatch")
    F.require(type(export["users"]) is int and export["users"] == len(users)
              and type(export["tokens"]) is int and export["tokens"] == len(tokens)
              and export["source_instance_verified"] is True
              and export["readonly_source_transaction"] is True, "continuity_export_incomplete")
    mapping = documents["identity_map"]
    accounts = F.PLANNER.identity_index(mapping)
    F.PLANNER.bind_identity_provenance(documents["identity_manifest"],
        map_file_sha256=hashes["identity_map"], source_archive_sha256=archive)
    F.require(set(accounts) == set(users) and source_user_id in users, "continuity_identity_coverage")
    receipt, alias = documents["credential_receipt"], documents["alias_manifest"]
    F.require(receipt["scope"] == "a02-credential-mapping-receipt"
              and alias["scope"] == "legacy-api-key-wire-aliases-v1"
              and type(alias["version"]) is int and alias["version"] == 1
              and receipt["source_archive_sha256"] == archive == alias["source_archive_sha256"]
              and alias["credential_receipt_sha256"] == hashes["credential_receipt"],
              "continuity_credential_binding_mismatch")
    # Normal LCI imports all tokens, including permanently revoked managed rows.
    records = receipt["records"]
    aliases = alias["keys"]
    F.require(type(records) is list and type(aliases) is list
              and len(records) == len(aliases) == len(tokens), "continuity_key_coverage")
    alias_by_id = {}
    for row in aliases:
        key = integer(row["source_key_id"], 1)
        F.require(key not in alias_by_id, "continuity_duplicate_alias")
        alias_by_id[key] = row
    seen, credentials, secrets = set(), set(), set()
    projected = {key: key_projection(row) for key, row in tokens.items()}
    for row in records:
        key = integer(row["source_key_id"], 1)
        credential = integer(row["target_credential_id"], 1)
        F.require(key in tokens and key not in seen and credential not in credentials,
                  "continuity_duplicate_or_unknown_key")
        source = projected[key]
        owner = source["source_user_id"]
        F.require(owner in accounts and row["target_user_id"] == owner
                  and type(row["target_user_id"]) is int
                  and type(row["target_account_id"]) is int
                  and row["target_account_id"] == accounts[owner]
                  and row["source_secret_sha256"] == source["source_secret_sha256"],
                  "continuity_key_owner_or_digest_mismatch")
        binding = alias_by_id.get(key)
        F.require(binding is not None and all(type(binding.get(k)) is type(v) and binding[k] == v
                                             for k, v in row.items())
                  and binding["sk_alias_sha256"] == hashlib.sha256(("sk-" + tokens[key]["key"]).encode()).hexdigest()
                  and binding.get("revoked_rekey_required", False) is source["managed"],
                  "continuity_alias_mismatch")
        F.require(source["source_secret_sha256"] not in secrets, "continuity_duplicate_secret")
        seen.add(key)
        credentials.add(credential)
        secrets.add(source["source_secret_sha256"])
    user = users[source_user_id]
    return dict(scope=SCOPE, source_archive_sha256=archive, source_receipt_sha256=hashes["source_receipt"],
                observed_at=observed_at, identity_migration_id=identity_migration_id,
                key_import_migration_id=key_import_migration_id, key_import_sql_sha256=hashes["key_import_sql"],
                source_user_id=source_user_id, source_user_sha256=LCI.digest(user),
                source_status=integer(user["status"], -32768, 32767), source_deleted=user["deleted_at"] is not None,
                source_role=integer(user["role"], -32768, 32767), source_auth_version=integer(user["auth_version"], 1),
                keys=[projected[key] for key in sorted(tokens) if projected[key]["source_user_id"] == source_user_id])


def read_json(data):
    return json.loads(data, object_pairs_hook=PRIVATE.PROPOSAL.unique_object,
                      parse_constant=lambda _: F.require(False, "continuity_json_constant_invalid"))


def run(paths, output, **parameters):
    contents = {name: PRIVATE.private_bytes(paths[name]) for name in INPUTS}
    hashes = {name: hashlib.sha256(data).hexdigest() for name, data in contents.items()}
    documents = {name: read_json(data) for name, data in contents.items()
                 if name not in ("identity_map", "key_import_sql")}
    documents["identity_map"] = [read_json(line) for line in contents["identity_map"].splitlines() if line]
    projection = prepare(documents, hashes, **parameters)
    # Match the existing LCI parsed-row digest contract while leaving the
    # receipt-bound raw export intact. Core hashes this separate private file.
    normalized_rows = json.dumps(documents["raw_export"], ensure_ascii=False,
                                 sort_keys=True, separators=(",", ":")) + "\n"
    semantic_decode = lambda data: json.loads(data, parse_float=Decimal,
                                              object_pairs_hook=PRIVATE.PROPOSAL.unique_object)
    F.require(semantic_decode(contents["raw_export"]) == semantic_decode(normalized_rows),
              "continuity_normalization_changes_semantics")
    projection.update(source_receipt_file=str(paths["source_receipt"].resolve()),
                      source_rows_file=str(output / "source-rows.normalized.private.json"),
                      key_import_sql_file=str(paths["key_import_sql"].resolve()))
    parent = F.IDENTITY.private_backup_path(output.parent, directory=True)
    F.require(output.is_absolute() and output.parent == parent, "continuity_output_path_invalid")
    materials = {name: str(paths[name].resolve()) for name in MATERIALS}
    materials["source_projection"] = str(output / "source-projection.private.json")
    files = {"source-projection.private.json": projection, "materials.private.json": materials}
    receipt = dict(scope="legacy-admin-key-continuity-prepare.v1", input_sha256=hashes,
                   input_paths={name: str(path.resolve()) for name, path in paths.items()},
                   source_projection_sha256=hashlib.sha256((F.canonical(projection) + "\n").encode()).hexdigest(),
                   materials_sha256=hashlib.sha256((F.canonical(materials) + "\n").encode()).hexdigest(),
                   source_rehearsal_only=documents["source_receipt"].get("rehearsal_only"),
                   source_future_cutover_valid=documents["source_receipt"].get("future_cutover_valid"),
                   normalized_source_rows_sha256=hashlib.sha256(normalized_rows.encode()).hexdigest(),
                   normalized_source_rows_semantic_equality=True,
                   normalization_contract="LCI parsed JSON sorted UTF-8 compact v1",
                   target_verified=False, activation_created=False, database_connections=0)
    files["prepare-receipt.private.json"] = receipt
    # Recheck all inputs before publishing a new private directory. Never overwrite.
    F.require(all(PRIVATE.private_bytes(paths[name]) == data for name, data in contents.items()),
              "continuity_input_changed")
    encoded = {name: F.canonical(value) + "\n" for name, value in files.items()}
    F.require(all(token["key"] not in text for token in documents["raw_export"]["tokens"]
                  for text in encoded.values()), "continuity_secret_echo")
    encoded["source-rows.normalized.private.json"] = normalized_rows
    output.mkdir(mode=0o700)
    for name, text in encoded.items():
        F.write_private(output, name, text)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in INPUTS:
        parser.add_argument("--" + name.replace("_", "-"), required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-user-id", required=True, type=int)
    parser.add_argument("--observed-at", required=True, type=int)
    parser.add_argument("--identity-migration-id", required=True)
    parser.add_argument("--key-import-migration-id", required=True)
    args = vars(parser.parse_args())
    paths = {name: args.pop(name) for name in INPUTS}
    output = args.pop("output")
    try:
        run(paths, output, **args)
    except Exception:
        # Input rows contain bearers, passwords and personal data. No exception repr.
        print("error: legacy_admin_key_continuity_prepare_failed", file=sys.stderr)
        return 1
    print("legacy-admin-key-continuity-prepared")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

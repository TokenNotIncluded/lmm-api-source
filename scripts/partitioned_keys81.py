#!/usr/bin/env python3
"""Ordinary-key partition adapter for the existing Core81 owner transaction.

The complete source, identity, wallet, fence and restore checks stay in
final81_execution.inspect/execute. This component only supplies the derived
ordinary-key view to the unchanged key writer. It does not fence, approve or
connect to a database.
"""
from collections import Counter
from copy import deepcopy
import argparse
import hashlib
import importlib
import json
from pathlib import Path
import re
import sys

import current_key_source_partition as S
import current_connection_history as H

ADAPTER_VERSION = "lmm.partitioned-keys81-owner-adapter.v1"
SCOPE = "current-core81-runtime-key-append-v1"
PARTITION_BINDING = "lmm.core81-key-source-partition-binding.v1"
PARTITION_SHA = "6adee89b30c98bc66b5037075b9f234053f04f42cb8c2d6ffd5929d6dd2a1efe"
WRITER_SHA = "51313c45f0f6fc9f614b8f70deb6d3540d2d9625cca16f600029a7604e81a7e0"
EXECUTION_SHA = "b47735d642a2efdbd24c9c7c68b9a5ac1f3f47067fc04ab6029e1f8aaa3655dc"
HISTORY_LOADER_SHA = "45d13a84800a8c14e2d9171bfcd9d417e7f18705b54beb8d764c874e4ec34562"
SOURCE_RESOURCE_PINS = {
    "reviewed-fresh66-credit-quotes.json": "bb76fe088c7925370043246a64bbd179f1642f520d4ba9ee9c96753920a7152e",
    "reviewed-fresh66-module-split.json": "0f62bc54c39562414224fc821becf90029f2becb9757b4bcb0018cc7fcb703e4",
    "reviewed-fresh67-pancake-language.json": "9d071b265ccf7ce0a557c73b8b0f91bcf8bf4eb1982a8fb93869c42679fa066a",
    "reviewed-fresh68-checkins.json": "1133243d5015487863ecd3d9a74e0f489047cf5724c2104d4db9f7753ca38634"
}
A = L = E = P = F = C = K = None
_expected_adapter_sha256 = None


def _verify_history_loader():
    # This check must not delegate to the loader whose bytes it authenticates.
    if hashlib.sha256(Path(H.__file__).read_bytes()).hexdigest() != HISTORY_LOADER_SHA:
        raise ValueError("partition81_history_loader_changed")


_verify_history_loader()
need, digest = H.need, H.digest


def load_support(support_scripts, *, expected_adapter_sha256):
    """Load explicit reviewed bundles; the caller pins this adapter's release bytes.

    Reuse the repository's complete frozen-helper validator. No private bundle
    or evidence path is embedded in this module, and no database is opened.
    """
    global A, L, E, P, F, C, K, need, digest, _expected_adapter_sha256
    _verify_history_loader()
    need(isinstance(expected_adapter_sha256, str)
         and re.fullmatch(r"[a-f0-9]{64}", expected_adapter_sha256) is not None
         and hashlib.sha256(Path(__file__).read_bytes()).hexdigest() == expected_adapter_sha256,
         "partition81_adapter_bytes_changed")
    paths = [Path(path).resolve() for path in support_scripts]
    need(paths and all(path.is_dir() for path in paths), "partition81_support_directory_missing")
    for path in reversed(paths):
        sys.path.insert(0, str(path))
    writer, lineage = H.frozen_helpers()
    execution = importlib.import_module("final81_execution")
    for module, expected in ((S, PARTITION_SHA), (writer, WRITER_SHA), (execution, EXECUTION_SHA)):
        need(hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest() == expected,
             "partition81_dependency_changed")
    need(execution.T is writer.T and execution.K is writer.K78 and execution.L is lineage
         and writer.SCOPE == SCOPE, "partition81_support_objects_changed")
    A, L, E = writer, lineage, execution
    P, F, C, K = A.P, A.F, A.C, E.K
    need, digest = A.need, A.digest
    _expected_adapter_sha256 = expected_adapter_sha256
    return dependencies()


def dependencies():
    _verify_history_loader()
    need(A is not None and _expected_adapter_sha256 is not None,
         "partition81_support_not_loaded")
    need(hashlib.sha256(Path(__file__).read_bytes()).hexdigest() == _expected_adapter_sha256,
         "partition81_adapter_bytes_changed")
    writer, lineage = H.frozen_helpers()
    need(writer is A and lineage is L and E.T is A.T and E.K is A.K78 and E.L is L,
         "partition81_support_objects_changed")
    for module, expected in ((S, PARTITION_SHA), (A, WRITER_SHA), (E, EXECUTION_SHA)):
        need(hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest() == expected,
             "partition81_dependency_changed")
    E.dependencies()
    A.dependencies()
    for name, expected in SOURCE_RESOURCE_PINS.items():
        resource = Path(A.I.__file__).with_name(name)
        need(hashlib.sha256(resource.read_bytes()).hexdigest() == expected,
             "partition81_source_resource_changed")
    return {"adapter_version": ADAPTER_VERSION, "adapter_sha256": _expected_adapter_sha256,
            "frozen_helpers": {**H.FROZEN_KEY_TOOLS, **H.FROZEN_SOURCE_EXTRAS,
                "current_key_source_partition.py": PARTITION_SHA,
                "current_connection_history.py": HISTORY_LOADER_SHA,
                **SOURCE_RESOURCE_PINS,
                "final81_execution.py": EXECUTION_SHA},
            "full_execution_qualified": False}


def handoff(parent, provenance, canonical_tables, partition, view, archive,
            observation, acquisition, fence, *, origin):
    """Pure source boundary. Synthetic fixtures never supply execution authority."""
    context = S.parent_context(P, F, parent, provenance,
        canonical_tables["oauth_server_tokens"], partition["source_context"]["verified_files"], origin=origin)
    S.validate_saved(P, F, C, parent, provenance, context, observation, acquisition,
        partition, view, archive, partition["oauth"]["explicit_terminal_evidence"])
    need(not partition["oauth"]["unresolved"]
         and not partition["oauth"]["pending_authorization_sha256"],
         "partition81_oauth_continuation_unresolved")
    # All OAuth authorities, including grants without billing rows, come from
    # the same complete canonical source that the final fence checked.
    for table in S.TABLES:
        need(table in canonical_tables
             and Counter(F.digest(r) for r in canonical_tables[table]) ==
                 Counter(F.digest(r) for r in observation[table]),
             "partition81_oauth_not_final_canonical_source")
    namespace = fence["namespace"]
    need(acquisition["instance"]["database"] == namespace["database"]
         and acquisition["instance"]["system_identifier"] == namespace["system_identifier"],
         "partition81_source_instance_changed")
    need(S.ms(observation["observed_at"]) <= S.ms(fence["observed_at"]),
         "partition81_oauth_observation_after_final_fence")
    need(fence["transaction_read_only"] == "on" and fence["database_allows_connections"] is False
         and type(fence["other_database_backends"]) is int and fence["other_database_backends"] == 0
         and type(fence["prepared_transactions"]) is int and fence["prepared_transactions"] == 0,
         "partition81_source_fence_not_held")
    return deepcopy(view)


def checked_source(context, inputs):
    dependencies()
    values = {k: K.read_ref(inputs[k]) for k in
        ("partition", "ordinary_view", "internal_archive", "oauth_observation", "oauth_capture")}
    material = values["partition"]
    need(material["source_context"]["origin"] == "FULL_CANONICAL_EXPORT_REBUILT",
         "partition81_synthetic_source_not_production")
    for path, sha in material["source_context"]["verified_files"].items():
        K.read_ref({"path": path, "sha256": sha}, parse=False)
    for key in ("oauth_sql", "oauth_collector"):
        K.read_ref(inputs[key], parse=False)
    need(inputs["oauth_observation"]["sha256"] == values["oauth_capture"]["observation_sha256"],
         "partition81_oauth_capture_file_changed")
    acquisition = S.validate_capture(P, F, values["oauth_observation"], values["oauth_capture"],
        Path(inputs["oauth_sql"]["path"]), Path(inputs["oauth_collector"]["path"]))
    view = handoff(context["source"], context["key_projection"], context["tables"], material,
        values["ordinary_view"], values["internal_archive"], values["oauth_observation"], acquisition,
        context["facts"]["source_fence_observation"], origin="FULL_CANONICAL_EXPORT_REBUILT")
    return view, material


def retain_ciphertexts(rebuilt, saved, source, encryption_key):
    """Preserve the reviewed nonce after independently checking its plaintext."""
    fresh_rows, saved_rows = rebuilt["records"], saved["records"]
    need(len(fresh_rows) == len(saved_rows), "partition81_key_records_changed")
    tokens = {row["id"]: row for row in source["tokens"]}
    for fresh, original in zip(fresh_rows, saved_rows):
        need({k: v for k, v in fresh.items() if k != "encrypted_secret_hex"} ==
             {k: v for k, v in original.items() if k != "encrypted_secret_hex"},
             "partition81_key_records_changed")
        if not fresh["one_time_reveal"]:
            need(encryption_key is not None, "partition81_encryption_key_required")
            C.verify_encrypted_secret(encryption_key, original, tokens[fresh["source_token_id"]]["key"])
            fresh["encrypted_secret_hex"] = original["encrypted_secret_hex"]
        else:
            need(original["encrypted_secret_hex"] is None, "partition81_one_time_secret_changed")


def prepare(context, inputs, *, migration_id, source_namespace, runtime_roles,
            migration_actor_user_id, encryption_key=None, saved=None):
    source, partition = checked_source(context, inputs)
    target = K.read_ref(inputs["target_projection"])
    review = K.read_ref(inputs["credential_review"])
    need(target["source_target_id_map"] == context["mapping"]
         and source_namespace == context["facts"]["source_fence_observation"]["namespace"],
         "partition81_owner_mapping_or_namespace_changed")
    identity = context["identity"]
    bindings = L.validate(context["tables"]["users"], context["source"], target, source_namespace,
                          identity["run"], identity["authority"])
    dormant = L.dormant_bindings(context["tables"], source, target, identity["run"],
                                 context["snapshot"]["reconciliation_observation"])
    keys = A.prepare_mechanics(source, target, review, snapshot=context["snapshot"],
        contract=context["contract"], migration_id=migration_id, source_namespace=source_namespace,
        runtime_roles=runtime_roles, current_bindings=bindings, dormant_bindings=dormant,
        migration_actor_user_id=migration_actor_user_id, encryption_key=encryption_key)
    if saved is not None:
        retain_ciphertexts(keys, saved, source, encryption_key)
        A.validate_mechanics(keys)
    # Keep the frozen inner and outer Key receipt contract identical. The
    # existing connection-history reader consumes this same material and scope.
    material = keys
    material["source_partition"] = {"format": PARTITION_BINDING,
        "material": partition, "inputs": deepcopy(inputs),
        "source_fence_sha256": context["facts"]["source_fence_sha256"]}
    need(not any(row["key"] in A.canonical(material) for row in context["source"]["tokens"]),
         "partition81_secret_echo")
    return material


def observation_sql(target):
    return "SELECT (" + A.native_evidence_sql(target["source_target_id_map"]) + \
        ")::jsonb || jsonb_build_object('key_append',(" + \
        A.key_projection_sql(target.get(C.LINEAGE.KEY, [])) + "))"


def inspect(evidence, inputs, *, identity_sql):
    """Enter through the same full evidence validator used during execution."""
    dependencies()
    target = K.read_ref(inputs["target_projection"])
    return E.inspect(evidence, affected_tables=A.affected_tables(), identity_sql=identity_sql,
        component_scope=SCOPE, reconciliation_observation_sql=observation_sql(target))


def bind_review(evidence, context, component, *, outer_migration_id, identity_sql):
    """Bind an already-issued review; this function never creates its authority."""
    dependencies()
    keys = component
    need(component["source_partition"]["source_fence_sha256"] == context["facts"]["source_fence_sha256"],
         "partition81_execution_evidence_changed")
    review = E.validate_review(evidence, context, component)
    body = A.append_body_sql(keys)
    readback = A.component_readback_sql(keys, outer_migration_id)
    return E.T.scope_material(snapshot=context["snapshot"], c=context["contract"],
        runtime_roles=evidence["runtime_roles"], migration_actor_user_id=evidence["migration_actor_user_id"],
        migration_id=outer_migration_id, component_material=component,
        component_module_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        body_sql=body, allowed_insert_counts=A.insert_counts(keys), sequence_increments=A.sequence_increments(keys),
        evidence_sha256=context["evidence_sha256"], source_archive_sha256=keys["source_archive_sha256"],
        mapping_sha256=keys["mapping_sha256"], identity_observation_sql=identity_sql,
        reviewed_document=review, reviewed_document_sha256=evidence["review"]["sha256"],
        reconciliation_observation_sql=observation_sql(keys["target_projection"]),
        component_readback_sql=readback)


def production_rebuilder(saved_material, *, encryption_key=None):
    """Supply this callback to the unchanged final81_execution.execute."""
    saved = deepcopy(saved_material)
    need(saved.get("scope") == SCOPE and saved.get("format_version") == 1
         and saved.get("source_partition", {}).get("format") == PARTITION_BINDING,
         "partition81_component_shape_invalid")
    def rebuild(context):
        keys = saved
        binding = saved["source_partition"]
        need(binding["source_fence_sha256"] == context["facts"]["source_fence_sha256"],
             "partition81_execution_evidence_changed")
        rebuilt = prepare(context, binding["inputs"], migration_id=keys["migration_id"],
            source_namespace=keys["source_namespace"], runtime_roles=keys["runtime_roles"],
            migration_actor_user_id=keys["migration_actor_user_id"],
            encryption_key=encryption_key, saved=saved)
        return rebuilt, A.append_body_sql(rebuilt), observation_sql(keys["target_projection"])
    return rebuild


def execute(outer, *, evidence, approval, connection, output, encryption_key=None):
    """Execute once through the unchanged owner transaction and approval gate."""
    saved = outer["component_material"]
    return E.execute(outer, component_body_sql=A.append_body_sql(saved),
        evidence=evidence, approval=approval, connection=connection, output=output,
        rebuild_component=production_rebuilder(saved, encryption_key=encryption_key))


def resolution_sql(outer, original_application_name):
    """Inspect an uncertain commit through the unchanged read-only resolver."""
    return E.T.resolve_sql(outer, original_application_name,
        component_readback_sql=A.component_readback_sql(outer["component_material"],
                                                       outer["migration_id"]))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["verify-support"])
    parser.add_argument("--support-scripts", type=Path, action="append", required=True)
    parser.add_argument("--expected-adapter-sha256", required=True)
    args = parser.parse_args(argv)
    result = load_support(args.support_scripts,
                          expected_adapter_sha256=args.expected_adapter_sha256)
    print(json.dumps({"status": "SUPPORT_VERIFIED", **result}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Opt-in PG18 owner CLI acceptance on Main's synthetic Core44 template.

Default prints a plan without connections or subprocesses. --execute is only
for Main's separately authorized private local fixture. No Cargo, cluster
startup, production inputs, source/schema edits, or financial source seeding.
"""

from __future__ import annotations

import argparse
import importlib
import json
import os
from pathlib import Path
import secrets
import sys
from types import SimpleNamespace
import uuid

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "scripts"))
sys.path.insert(0, str(REPO / "tests/site_statistics"))
C = importlib.import_module("legacy_channel_accounting")
SITE = importlib.import_module("importer_selftest")

require, literal, identifier = SITE.require, SITE.literal, SITE.identifier
CHANNELS = (880001, 880002, 880003)
GROUPS = (
    "core44-template",
    "real-dump-restore-export",
    "metadata-only-fixture",
    "disjoint-apply-resolve-noop",
    "included-seal-lost-ack-resolve-noop",
    "target-drift-refusal-resolve-unknown",
    "unknown-basis-no-apply",
    "unknown-overlap-no-apply",
)


class Run(SITE.Run):
    """Reuse siteFinance command logs, independent capture and group reporting."""

    def __init__(self, args):
        self.args = args
        self.root = C.IDENTITY.private_backup_path(args.output, directory=True)
        require(
            not any(self.root.iterdir()), "output must be a new empty private directory"
        )
        self.env = SITE.clean_environment()
        self.logs = self.root / "logs"
        self.logs.mkdir(mode=0o700)
        self.seq = 0
        self.results = dict.fromkeys(GROUPS, "NOT_RUN")
        self.admin = C.P.parse_json(C.F.read_private(args.connection))
        require(
            self.admin.get("synthetic_only") is True,
            "Main must attest synthetic-only fixture",
        )
        require(
            "login_role" not in self.admin and "set_role" not in self.admin,
            "fixture administration requires its explicit direct local login",
        )
        self.env.update(C.IDENTITY.socket_connection(self.admin))
        self.socket, self.port = (
            Path(self.admin["socket_directory"]),
            self.admin["port"],
        )
        self.template = self.admin["database"]
        self.base = None
        self.prefix = "co_" + uuid.uuid4().hex[:10]
        self.operator, self.reader, self.source_owner, self.runtime = (
            self.prefix + suffix
            for suffix in ("_op", "_reader", "_source_owner", "_runtime")
        )
        self.databases = []
        self.created_roles = []
        self.writer_path = REPO / "scripts/legacy_channel_accounting.py"
        self.writer = SimpleNamespace(
            F=C.F, I=C.IDENTITY, catalog_contract=C.S.catalog_contract
        )
        self.source_dir = self.root / "source-material"
        self.source_dir.mkdir(mode=0o700)
        self.binding = {
            "core": str(args.core),
            "core_sha256": C.IDENTITY.sha256_file(args.core),
            "tool_hashes": C.tool_hashes(),
            "harness_sha256": C.IDENTITY.sha256_file(Path(__file__)),
        }
        SITE.save(self.root / "binding.json", self.binding)

    def psql(self, database, role=None):
        return [
            "/usr/bin/psql",
            "-X",
            "--no-password",
            "-qAt",
            "-v",
            "ON_ERROR_STOP=1",
            "-h",
            self.socket,
            "-p",
            self.port,
            "-U",
            role or self.admin["role"],
            "-d",
            database,
        ]

    def sql(self, query, database=None, *, owner=False, expected=0, label="sql"):
        database = database or self.base
        require(
            database == self.template or database in self.databases,
            "database outside this fixture",
        )
        if owner:
            require(database in self.databases, "never write the supplied template")
            query = "SET ROLE " + identifier(self.owner) + ";\n" + query
        return self.command(
            label,
            self.psql(database, self.operator if owner else None),
            data=query,
            expected=expected,
        ).stdout.strip()

    def json_sql(self, query, database=None, **kwargs):
        return json.loads(self.sql(query, database, **kwargs))

    def write(self, directory, name, value):
        text = value if type(value) is str else C.F.canonical(value) + "\n"
        path = C.F.write_private(directory, name, text)
        return {"path": str(path), "sha256": C.IDENTITY.sha256_file(path)}

    def create_database(self, suffix, *, template=None, owner=None):
        name = self.prefix + "_" + suffix
        sql = (
            "CREATE DATABASE "
            + identifier(name)
            + " OWNER "
            + identifier(owner or self.owner)
        )
        if template:
            sql += " TEMPLATE " + identifier(template)
        self.sql(sql, self.template, label="create-owned-fixture-database")
        self.databases.append(name)
        SITE.save(
            self.root / "owned-resources.json",
            {"databases": self.databases, "roles": self.created_roles},
        )
        self.sql(
            "REVOKE CONNECT,CREATE,TEMPORARY ON DATABASE "
            + identifier(name)
            + " FROM PUBLIC; "
            "GRANT CONNECT ON DATABASE "
            + identifier(name)
            + " TO "
            + identifier(self.operator),
            name,
        )
        return name

    def setup(self):
        require(
            int(self.sql("SHOW server_version_num", self.template)) // 10000 == 18,
            "PG18 required",
        )
        require(
            self.json_sql(
                "SELECT to_jsonb(rolsuper) FROM pg_roles WHERE rolname=current_user",
                self.template,
            )
            is True,
            "resource-owner synthetic administration login required",
        )
        require(
            self.sql("SHOW listen_addresses", self.template) == "",
            "Unix-only synthetic PG required",
        )
        manifest = json.loads(
            self.command(
                "native-schema-manifest", [self.args.core, "schema-manifest"]
            ).stdout
        )
        require(len(manifest["components"]) == 44, "precompiled Core44 required")
        actual = self.json_sql(
            "SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract",
            self.template,
        )
        require(
            actual
            == {r["name"]: r["raw_schema_sha256"] for r in manifest["components"]},
            "template differs from compiled Core44; do not install/patch schema here",
        )
        self.owner = self.sql(
            "SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database()",
            self.template,
        )
        require(
            self.json_sql(
                "SELECT to_jsonb(NOT rolcanlogin) FROM pg_roles WHERE rolname="
                + literal(self.owner),
                self.template,
            ),
            "NOLOGIN installation owner required",
        )
        require(
            self.sql(
                "SELECT count(*) FROM core_channels.opening_sources", self.template
            )
            == "0",
            "template already contains legacy openings",
        )
        native_ids = self.json_sql(
            "SELECT coalesce(jsonb_agg(id ORDER BY id COLLATE \"C\"),'[]'::jsonb) FROM core_charging.requests",
            self.template,
        )
        require(
            bool(native_ids),
            "DEPENDENCY: template needs canonical synthetic charging requests for nonempty opaque ID seal",
        )
        require(
            self.sql(
                "SELECT ready AND NOT EXISTS(SELECT FROM core_channels.projection_dirty) FROM core_channels.projection_state WHERE singleton",
                self.template,
            )
            == "t",
            "DEPENDENCY: canonical charging/projection fixture must be ready and clean",
        )
        self.template_before = self.capture(self.template, "template-before")
        for program in ("psql", "pg_dump", "pg_restore"):
            self.command(program + "-version", ["/usr/bin/" + program, "--version"])
        passwords = {
            self.operator: secrets.token_hex(24),
            self.reader: secrets.token_hex(24),
        }
        roles = [
            "CREATE ROLE " + identifier(self.source_owner) + " NOLOGIN",
            "CREATE ROLE " + identifier(self.runtime) + " NOLOGIN",
            "GRANT "
            + identifier(self.args.runtime_role)
            + " TO "
            + identifier(self.runtime),
        ]
        for role, password in passwords.items():
            roles.append(
                "CREATE ROLE "
                + identifier(role)
                + " LOGIN NOINHERIT PASSWORD "
                + literal(password)
            )
        roles.append(
            "GRANT "
            + identifier(self.owner)
            + " TO "
            + identifier(self.operator)
            + " WITH INHERIT FALSE,SET TRUE"
        )
        self.sql(";".join(roles), self.template, label="create-owned-fixture-roles")
        self.created_roles = [
            self.operator,
            self.reader,
            self.source_owner,
            self.runtime,
        ]
        original_pgpass = C.F.read_private(Path(self.admin["pgpass_file"]))

        def escape(value):
            return str(value).replace("\\", "\\\\").replace(":", "\\:")

        entries = [
            ":".join(escape(v) for v in (self.socket, self.port, "*", role, password))
            for role, password in passwords.items()
        ]
        pgpass = self.write(
            self.root,
            "fixture-pgpass.private",
            original_pgpass.rstrip("\n") + "\n" + "\n".join(entries) + "\n",
        )
        self.env["PGPASSFILE"] = pgpass["path"]
        self.pgpass = pgpass["path"]
        self.native_ids = native_ids

    def source(self):
        source = self.create_database("source", owner=self.source_owner)
        frozen = self.create_database("frozen", owner=self.source_owner)
        declaration = """CREATE TABLE public.channels(id bigint PRIMARY KEY,used_quota bigint NOT NULL,
balance double precision,balance_updated_time bigint,key text);
CREATE TABLE public.options(key text PRIMARY KEY,value text);
INSERT INTO public.channels VALUES(17,7,0.125,1700000000,'SYNTHETIC-KEY'),
(18,-1,0,0,'SYNTHETIC-KEY'),(19,0,12,1690000000,'SYNTHETIC-KEY');
INSERT INTO public.options VALUES('QuotaPerUnit','3');"""
        self.sql(
            "ALTER SCHEMA public OWNER TO "
            + identifier(self.source_owner)
            + "; SET ROLE "
            + identifier(self.source_owner)
            + ";"
            + declaration,
            source,
            label="synthetic-source-only-ddl-and-data",
        )
        archive = self.source_dir / "synthetic-source.dump"
        common = [
            "--no-password",
            "-h",
            self.socket,
            "-p",
            self.port,
            "-U",
            self.admin["role"],
        ]
        self.command(
            "real-pg-dump",
            ["/usr/bin/pg_dump", *common, "-d", source, "-Fc", "-f", archive],
        )
        self.command("real-archive-toc", ["/usr/bin/pg_restore", "--list", archive])
        self.command(
            "real-pg-restore",
            [
                "/usr/bin/pg_restore",
                *common,
                "-d",
                frozen,
                "--clean",
                "--if-exists",
                "--exit-on-error",
                archive,
            ],
        )
        for table in ("channels", "options"):
            query = (
                'SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text COLLATE "C") FROM public.'
                + identifier(table)
                + " t"
            )
            require(
                self.json_sql(query, source) == self.json_sql(query, frozen),
                "actual dump/restore row mismatch",
            )
        self.sql(
            "GRANT CONNECT ON DATABASE "
            + identifier(frozen)
            + " TO "
            + identifier(self.reader)
            + "; GRANT USAGE ON SCHEMA public TO "
            + identifier(self.reader)
            + "; GRANT SELECT ON ALL TABLES IN SCHEMA public TO "
            + identifier(self.reader),
            frozen,
        )
        privileges = self.json_sql(
            "SELECT jsonb_build_object('writes',EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p') AND (has_table_privilege("
            + literal(self.reader)
            + ",c.oid,'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') OR has_any_column_privilege("
            + literal(self.reader)
            + ",c.oid,'INSERT,UPDATE,REFERENCES'))),'definer_execute',EXISTS(SELECT FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.prosecdef AND has_function_privilege("
            + literal(self.reader)
            + ",p.oid,'EXECUTE')))",
            frozen,
        )
        require(
            privileges == {"writes": False, "definer_execute": False},
            "restored source reader has unsafe authority",
        )
        columns = self.json_sql(
            "SELECT jsonb_agg(jsonb_build_object('schema','public','table_name',table_name,'column_name',column_name) ORDER BY table_name,ordinal_position) FROM information_schema.columns WHERE table_schema='public'",
            frozen,
        )
        constraints = self.json_sql(
            "SELECT jsonb_agg(jsonb_build_object('schema','public','table_name',c.relname,'contype',p.contype,'definition',pg_get_constraintdef(p.oid))) FROM pg_constraint p JOIN pg_class c ON c.oid=p.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND p.contype='p'",
            frozen,
        )
        catalog = self.write(
            self.source_dir,
            "native-catalog-metadata.jsonl",
            "\n".join(
                C.F.canonical(v) for v in ([], [], columns, constraints, [], [], [])
            )
            + "\n",
        )
        # Narrow synthetic adapter: the general financial exporter also demands
        # unrelated required financial tables. Keep its actual framing/reader.
        query = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC'; SET LOCAL row_security=off;\n"
        query += (
            "SELECT jsonb_build_object('kind','identity','row',("
            + C.IDENTITY.IDENTITY_SQL.rstrip(";")
            + "));\n"
        )
        query += "SELECT jsonb_build_object('kind','schema','row',jsonb_build_object('schema','public','schema_owner',pg_get_userbyid(nspowner))) FROM pg_namespace WHERE nspname='public';\n"
        for table in ("channels", "options"):
            query += (
                "SELECT jsonb_build_object('kind','row','table',"
                + literal(table)
                + ",'row',to_jsonb(t)) FROM public."
                + identifier(table)
                + ' t ORDER BY to_jsonb(t)::text COLLATE "C";\n'
            )
        query += "COMMIT;"
        env = {**self.env, "PGOPTIONS": "-c default_transaction_read_only=on"}
        raw = self.command(
            "actual-readonly-restored-export",
            self.psql(frozen, self.reader),
            data=query,
            env=env,
        ).stdout
        exported = self.write(self.source_dir, "source-export.private.jsonl", raw)
        tagged = [C.P.parse_json(line) for line in raw.splitlines() if line]
        instance = tagged[0]["row"]
        self.tables = {
            table: [r["row"] for r in tagged if r.get("table") == table]
            for table in ("channels", "options")
        }
        counts = {
            table: int(
                self.sql("SELECT count(*) FROM public." + identifier(table), frozen)
            )
            for table in self.tables
        }
        baseline = self.write(
            self.source_dir,
            "native-table-row-count-baseline.jsonl",
            "\n".join(
                C.F.canonical({"table": table, "row_count": count})
                for table, count in counts.items()
            )
            + "\n",
        )
        archive_ref = {"path": str(archive), "sha256": C.IDENTITY.sha256_file(archive)}
        reader = {
            "socket_directory": str(self.socket),
            "port": self.port,
            "database": frozen,
            "role": self.reader,
            "pgpass_file": self.pgpass,
        }
        receipt = {
            "status": "snapshot-restored-baseline-verified-frozen-and-classified",
            "archive": archive_ref,
            "private_metadata_files": [catalog["path"], baseline["path"]],
            "source": {"schema": "public"},
            "readonly_reader": reader,
            "source_instance_identity": {
                **instance,
                "listen_addresses": "",
                "schema_owner": self.source_owner,
            },
        }
        metadata = C.P.metadata(receipt)[2]
        checked = C.P.read_frozen_export(
            Path(exported["path"]),
            receipt,
            C.P.metadata(receipt)[0],
            counts,
            sorted(counts),
            instance,
        )
        require(
            checked == self.tables,
            "shared financial frozen reader differs from actual source",
        )
        proof = {
            "source_archive_sha256": archive_ref["sha256"],
            "source_export_sha256": exported["sha256"],
            "source_column_coverage_exact": True,
            "source_row_baseline_exact": True,
            "metadata_hashes": metadata,
            "selected_table_counts": counts,
            "source_instance": instance,
        }
        self.files = {
            "archive": archive_ref,
            "source_export": exported,
            "source_receipt": self.write(
                self.source_dir, "receipt.private.json", receipt
            ),
            "source_export_proof": self.write(
                self.source_dir, "export-proof.private.json", proof
            ),
        }

    def metadata_fixture(self):
        self.base = self.create_database("base", template=self.template)
        before = self.capture(self.base, "before-metadata")
        old_upstreams = self.json_sql("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY version,id),'[]'::jsonb) FROM public.routing_upstreams t")
        old_events = self.json_sql("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM core_notifications.price_events t")
        require(
            self.sql(
                "SELECT count(*) FROM core_channels.identities WHERE id IN (880001,880002,880003)"
            )
            == "0",
            "fixture channel IDs already exist",
        )
        current = self.json_sql(
            "SELECT jsonb_build_object('config',a.config_version,'price',v.price_version) FROM public.routing_active_release a LEFT JOIN public.routing_config_versions v ON v.version=a.config_version"
        )
        version = int(
            self.sql(
                "SELECT greatest(coalesce((SELECT max(version) FROM public.routing_config_versions),0),coalesce((SELECT max(version) FROM public.routing_price_versions),0))+1"
            )
        )
        credential = int(
            self.sql("SELECT coalesce(max(id),0)+1 FROM public.routing_credential_refs")
        )
        config = {
            "version": version,
            "groups": [],
            "routes": [],
            "aliases": [],
            "upstreams": [
                {
                    "id": cid,
                    "endpoint": "https://synthetic.invalid",
                    "protocol": "openai_chat",
                    "enabled": False,
                    "credential": {"id": credential, "revision": 1},
                }
                for cid in CHANNELS
            ],
        }
        query = (
            "BEGIN; INSERT INTO public.routing_credential_refs(id,revision) VALUES("
            + str(credential)
            + ",1);"
        )
        query += (
            "SELECT public.routing_stage_prices("
            + literal({"version": version, "rules": []})
            + "::jsonb);"
        )

        def old(value):
            return "NULL" if value is None else str(value)

        query += (
            "SELECT public.routing_publish_config("
            + literal(config)
            + "::jsonb,"
            + str(version)
            + ","
            + old(current["config"])
            + ","
            + old(current["price"])
            + ");"
        )
        for cid in CHANNELS:
            query += f"INSERT INTO core_channels.identities(id,origin) VALUES({cid},'native');"
            query += f"INSERT INTO core_channels.counter_baselines(channel_id,gross_credits,refunded_credits) VALUES({cid},{200000 if cid == CHANNELS[0] else 0},{50000 if cid == CHANNELS[0] else 0});"
            query += f"INSERT INTO core_channels.balance_observations(channel_id) VALUES({cid});"
        query += f"UPDATE core_channels.balance_observations SET kind='provider',balance=999.25,unit_label='provider-unit',observed_at=to_timestamp(1719999999),credential_id={credential},credential_revision=1 WHERE channel_id={CHANNELS[2]}; COMMIT;"
        self.sql(query, owner=True, label="channel-display-metadata-only")
        after = self.capture(self.base, "after-metadata")
        upstreams = self.json_sql("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY version,id),'[]'::jsonb) FROM public.routing_upstreams t")
        events = self.json_sql("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM core_notifications.price_events t")
        require(upstreams[:len(old_upstreams)] == old_upstreams and len(upstreams) == len(old_upstreams) + len(CHANNELS), "existing upstreams changed")
        require([row['id'] for row in upstreams[len(old_upstreams):]] == list(CHANNELS) and all(row['version'] == version for row in upstreams[len(old_upstreams):]), "unexpected published upstreams")
        require(events[:len(old_events)] == old_events and all(row['release_version'] == version and row['price_version'] == version for row in events[len(old_events):]), "unexpected price publication events")
        allowed = {
            "public.routing_upstreams",
            "core_notifications.price_events",
            "core_notifications.price_events_id_seq",
            "public.routing_credential_refs",
            "public.routing_price_versions",
            "public.routing_config_versions",
            "public.routing_active_release",
            "core_channels.identities",
            "core_channels.counter_baselines",
            C.BALANCES,
        }
        self.preserved(before, after, allowed)

    @staticmethod
    def preserved(before, after, allowed=()):
        require(before.keys() == after.keys(), "inventory changed")
        require(
            {k for k in before if before[k] != after[k]} <= set(allowed),
            "unexpected ledger/table/sequence/catalog/ACL change",
        )

    def case(self, name):
        database = self.create_database(name, template=self.base)
        directory = self.root / name
        directory.mkdir(mode=0o700)
        connection = {
            "socket_directory": str(self.socket),
            "port": self.port,
            "database": database,
            "role": self.owner,
            "login_role": self.operator,
            "set_role": self.owner,
            "pgpass_file": self.pgpass,
        }
        self.write(directory, "connection.private.json", connection)
        return database, directory

    def cli(self, directory, mode, output, *, extra=(), expected=0, env=None):
        out = directory / output
        out.mkdir(mode=0o700)
        command = [
            sys.executable,
            "-B",
            self.writer_path,
            mode,
            "--output",
            out,
            *extra,
        ]
        result = self.command(
            directory.name + "-" + mode, command, expected=expected, env=env
        )
        return out, json.loads(result.stdout)

    def prepare(self, directory, overlap, *, unknown_basis=False):
        before_files = self.binding["tool_hashes"]
        require(
            C.tool_hashes() == before_files,
            "owner sources changed since harness freeze",
        )
        snap, _ = self.cli(
            directory,
            "snapshot",
            "snapshot",
            extra=(
                "--connection",
                directory / "connection.private.json",
                "--runtime-role",
                self.runtime,
            ),
        )
        snapshot = C.P.parse_json((snap / "target.private.json").read_text())
        require(
            snapshot["native_request_ids"] == self.native_ids,
            "global exact opaque IDs changed before seal",
        )
        selected = (
            self.tables["channels"]
            if overlap != "included_at_seal"
            else [r for r in self.tables["channels"] if r["id"] == 17]
        )
        mapping = {
            "format_version": 1,
            "archive_sha256": self.files["archive"]["sha256"],
            "export_sha256": self.files["source_export"]["sha256"],
            "source_revision": "synthetic-real-dump-K3",
            "observed_at": None,
            "channels": [
                {
                    "source_channel_id": r["id"],
                    "channel_id": CHANNELS[r["id"] - 17],
                    "source_row_sha256": C.F.digest(r),
                    "historical_coverage": False,
                }
                for r in selected
            ],
        }
        files = {
            **self.files,
            "mapping": self.write(directory, "mapping.private.json", mapping),
        }
        scope = C.F.digest(C.source_rows(self.tables["channels"], mapping))
        basis = {
            "synthetic_only": True,
            "archive_sha256": mapping["archive_sha256"],
            "export_sha256": mapping["export_sha256"],
            "rows_sha256": scope,
            "captured_options": self.tables["options"],
            "counter_unit_at_construction": 3,
        }
        files["evidence:basis"] = self.write(
            directory, "basis-proof.private.json", basis
        )
        files["evidence:overlap"] = self.write(
            directory,
            "overlap-proof.private.json",
            {
                **basis,
                "mode": overlap,
                "target_sha256": C.F.digest(snapshot),
                "native_request_ids": snapshot["native_request_ids"],
                "native": snapshot["native"],
            },
        )
        evidence = {
            "contract": C.CONTRACT,
            "archive_sha256": mapping["archive_sha256"],
            "export_sha256": mapping["export_sha256"],
            "mapping_sha256": files["mapping"]["sha256"],
            "rows_sha256": scope,
            "native_overlap": overlap,
            "overlap_proof_sha256": files["evidence:overlap"]["sha256"],
            "channels": [
                {
                    "source_channel_id": r["id"],
                    "source_row_sha256": C.F.digest(r),
                    "credits_per_usd": None if unknown_basis else 3,
                    "basis_kind": "captured_counter_unit",
                    "proof_sha256": files["evidence:basis"]["sha256"],
                    "balance_unit_label": "provider-USD",
                }
                for r in selected
            ],
        }
        files["unit_evidence"] = self.write(
            directory, "unit-evidence.private.json", evidence
        )
        manifest = self.write(directory, "files.private.json", files)
        return self.cli(
            directory,
            "preview",
            "review",
            extra=(
                "--files",
                manifest["path"],
                "--target",
                snap / "target.private.json",
                "--runtime-role",
                self.runtime,
            ),
        )

    def approve(self, review):
        request = json.loads((review / "approval-request.private.json").read_text())
        require(
            request["decision"] == "REVIEW_REQUIRED", "owner CLI must not self approve"
        )
        self.write(
            review,
            "approved.private.json",
            {**request, "decision": "approve-offline-channel-opening"},
        )

    def apply(self, directory, review, *, expected=0, env=None):
        return self.cli(
            review,
            "apply",
            "apply",
            expected=expected,
            env=env,
            extra=(
                "--material",
                review / "material.private.json",
                "--approval",
                review / "approved.private.json",
                "--connection",
                directory / "connection.private.json",
            ),
        )

    def resolve(self, directory, review, *, expected=0):
        return self.cli(
            directory,
            "resolve",
            "resolve",
            expected=expected,
            extra=(
                "--material",
                review / "material.private.json",
                "--approval",
                review / "approved.private.json",
                "--connection",
                directory / "connection.private.json",
                "--attempt",
                review / "apply/attempt.private.json",
            ),
        )

    def happy(self, overlap, lost_ack=False):
        database, directory = self.case("included" if lost_ack else "disjoint")
        before = self.capture(database, directory.name + "-before")
        review, status = self.prepare(directory, overlap)
        require(status["status"] == "READY", "evidenced opening must be READY")
        self.preserved(
            before, self.capture(database, directory.name + "-preview-after")
        )
        self.approve(review)
        env = None
        if lost_ack:
            shim = directory / "ack-loss-bin"
            shim.mkdir(mode=0o700)
            proxy = Path(
                self.write(
                    shim,
                    "psql",
                    "#!/usr/bin/python3\nimport subprocess,sys\nr=subprocess.run(['/usr/bin/psql',*sys.argv[1:]],input=sys.stdin.buffer.read(),capture_output=True)\nsys.stderr.buffer.write(r.stderr)\nif r.returncode==0: sys.exit(42)\nsys.stdout.buffer.write(r.stdout)\nsys.exit(r.returncode)\n",
                )["path"]
            )
            proxy.chmod(0o700)
            env = {**self.env, "PATH": str(shim) + ":" + self.env["PATH"]}
        _, result = self.apply(
            directory, review, expected=2 if lost_ack else 0, env=env
        )
        require(
            result["status"] == ("UNKNOWN" if lost_ack else "COMMITTED"),
            "wrong transport outcome",
        )
        after = self.capture(database, directory.name + "-after")
        self.preserved(before, after, {*C.DESTINATIONS, C.BALANCES})
        expected = (
            [[CHANNELS[0], "7", "3", "61", "30"]]
            if lost_ack
            else [
                [CHANNELS[0], "7", "3", "7", "3"],
                [CHANNELS[1], "-1", "3", "-1", "3"],
                [CHANNELS[2], "0", "3", "0", "1"],
            ]
        )
        actual = self.json_sql(
            "SELECT jsonb_agg(jsonb_build_array(channel_id,raw_used_quota::text,credits_per_usd::text,offset_numerator::text,offset_denominator::text) ORDER BY channel_id) FROM core_channels.opening_baselines",
            database,
        )
        require(actual == expected, "persisted signed K3 rational differs")
        require(
            self.sql(
                f"SELECT extract(epoch FROM observed_at)::bigint FROM core_channels.balance_observations WHERE channel_id={CHANNELS[0]}",
                database,
            )
            == "1700000000",
            "original provider observation time lost",
        )
        if not lost_ack:
            require(
                self.sql(
                    f"SELECT observed_at IS NULL FROM core_channels.balance_observations WHERE channel_id={CHANNELS[1]}",
                    database,
                )
                == "t",
                "zero source time must remain unknown",
            )
        require(
            self.sql(
                f"SELECT balance=999.25 AND unit_label='provider-unit' AND kind='provider' FROM core_channels.balance_observations WHERE channel_id={CHANNELS[2]}",
                database,
            )
            == "t",
            "fresh provider observation overwritten",
        )
        _, resolved = self.resolve(directory, review)
        require(
            resolved["status"] == "COMMITTED",
            "actual read-only resolver missed committed receipt",
        )
        sealed = self.sql(
            "SELECT to_jsonb(s)-'captured_at' FROM core_channels.opening_sources s",
            database,
        )
        require(
            self.json_sql(
                "SELECT to_jsonb(native_request_ids) FROM core_channels.opening_sources",
                database,
            )
            == self.native_ids,
            "persisted opaque seal differs",
        )
        # Explicit display metadata delta, NOT a claimed real settlement/refund.
        self.sql(
            f"UPDATE core_channels.counter_baselines SET gross_credits=gross_credits+500000,refunded_credits=refunded_credits+250000 WHERE channel_id={CHANNELS[0]}",
            database,
            owner=True,
            label="native-display-baseline-delta-only",
        )
        noop_before = self.capture(database, directory.name + "-noop-before")
        snapshot, _ = self.cli(
            directory,
            "snapshot",
            "after-growth-snapshot",
            extra=(
                "--connection",
                directory / "connection.private.json",
                "--runtime-role",
                self.runtime,
            ),
        )
        _, noop = self.cli(
            directory,
            "preview",
            "noop-review",
            extra=(
                "--material",
                review / "material.private.json",
                "--target",
                snapshot / "target.private.json",
            ),
        )
        require(
            noop["status"] == "NOOP",
            "original material must reconcile after Native display changes",
        )
        material = C.P.parse_json((review / "material.private.json").read_text())
        # Known-committed branch: execute generated NOOP SQL, without replaying
        # an uncertain attempt or bypassing the CLI's one-attempt claim.
        result = self.json_sql(
            C.build_apply_sql(material),
            database,
            owner=True,
            label="actual-generated-noop-sql",
        )
        require(
            result["status"] == "NOOP", "generated SQL did not execute its NOOP branch"
        )
        require(
            self.sql(
                "SELECT to_jsonb(s)-'captured_at' FROM core_channels.opening_sources s",
                database,
            )
            == sealed,
            "NOOP replaced original request IDs/seal/plan",
        )
        require(
            self.json_sql(
                "SELECT jsonb_agg(jsonb_build_array(channel_id,raw_used_quota::text,credits_per_usd::text,offset_numerator::text,offset_denominator::text) ORDER BY channel_id) FROM core_channels.opening_baselines",
                database,
            )
            == expected,
            "NOOP recalculated legacy offsets",
        )
        self.preserved(
            noop_before, self.capture(database, directory.name + "-noop-after")
        )

    def drift(self):
        database, directory = self.case("drift")
        review, status = self.prepare(directory, "disjoint")
        require(status["status"] == "READY", "drift test needs a valid approved plan")
        self.approve(review)
        self.sql(
            f"UPDATE core_channels.balance_observations SET unit_label='synthetic-drift' WHERE channel_id={CHANNELS[0]}",
            database,
            owner=True,
        )
        before = self.capture(database, "drift-before-submit")
        out, result = self.apply(directory, review, expected=2)
        require(
            result["status"] == "UNKNOWN",
            "post-submission refusal must not claim rollback",
        )
        require(
            "channel_opening_target_changed"
            in (out / "channel-opening.stderr.private.txt").read_text(),
            "did not reach target drift guard",
        )
        _, result = self.resolve(directory, review, expected=2)
        require(result["status"] == "UNKNOWN", "absent receipt must not infer rollback")
        self.preserved(before, self.capture(database, "drift-after-resolve"))

    def unknown(self, basis):
        database, directory = self.case("unknown_basis" if basis else "unknown_overlap")
        before = self.capture(database, directory.name + "-before")
        review, status = self.prepare(
            directory, "disjoint" if basis else "unknown", unknown_basis=basis
        )
        require(
            status["status"] == "UNKNOWN", "unknown source facts became confirmable"
        )
        require(
            not (review / "review.private.sql").exists()
            and not (review / "approval-request.private.json").exists(),
            "unknown preview emitted financial confirmation material",
        )
        self.write(
            review, "approved.private.json", {}
        )  # Deliberately not a valid financial approval.
        out, result = self.apply(directory, review, expected=1)
        require(
            result["status"] == "REFUSED"
            and not (out / "attempt.private.json").exists(),
            "unknown material reached SQL dispatch",
        )
        self.preserved(before, self.capture(database, directory.name + "-after"))

    def execute(self):
        try:
            actions = (
                self.setup,
                self.source,
                self.metadata_fixture,
                lambda: self.happy("disjoint"),
                lambda: self.happy("included_at_seal", True),
                self.drift,
                lambda: self.unknown(True),
                lambda: self.unknown(False),
            )
            for name, action in zip(GROUPS, actions):
                self.group(name, action)
            self.preserved(
                self.template_before, self.capture(self.template, "template-after")
            )
            require(
                C.tool_hashes() == self.binding["tool_hashes"]
                and C.IDENTITY.sha256_file(self.args.core)
                == self.binding["core_sha256"],
                "frozen owner/Core inputs drifted",
            )
        finally:
            SITE.save(
                self.root / "result.json",
                {
                    "groups": self.results,
                    "pg_executed": True,
                    "real_settlement_refund_acceptance": "NOT_RUN; metadata-only baseline delta",
                    "owned_resources": {
                        "databases": self.databases,
                        "roles": self.created_roles,
                    },
                    "cleanup": "Main owns these disposable resources; preserved for inspection",
                },
            )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--connection", type=Path)
    parser.add_argument("--core", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--runtime-role", default="lmm_core_runtime")
    args = parser.parse_args()
    if not args.execute:
        print(
            json.dumps(
                {
                    "status": "PLAN_ONLY; PG NOT RUN",
                    "groups": GROUPS,
                    "requires": "Main-authorized Unix-only PG18 synthetic Core44 template with nonempty canonical Native IDs, clean projection; direct resource-owner connection; prebuilt matching Core44; new private Backups output",
                    "source_adapter": "real pg_dump/restore; narrow channels+options readonly export using existing financial framing/reader",
                    "large_output": "Main runs space-first before authorizing database clones",
                },
                indent=2,
            )
        )
        return
    if not all((args.connection, args.core, args.output)):
        parser.error("--execute requires --connection --core --output")
    os.umask(0o077)
    Run(args).execute()


if __name__ == "__main__":
    main()

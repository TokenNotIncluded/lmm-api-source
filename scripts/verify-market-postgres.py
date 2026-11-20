#!/usr/bin/env python3
"""Run three existing PG cases; this is not Core funds or provider acceptance."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import time
from urllib.parse import quote


TOOL_CASES = (
    "TestPostgresStorePersistenceConcurrentRollbackAndRestart",
    "TestPostgresMarketAIReviewLostCommitACKRestoresPreviousLiveVersion",
)
MERCHANT_CASE = "TestPostgresMerchantHomeHTTP"


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def git(root, *args):
    return subprocess.run(["git", "-C", str(root), *args], check=True,
                          capture_output=True, timeout=30).stdout


def source_binding(root, revision):
    if git(root, "rev-parse", "HEAD").decode().strip() != revision:
        raise RuntimeError("product HEAD differs from the requested immutable input")
    if git(root, "status", "--porcelain=v1", "--untracked-files=all").strip():
        raise RuntimeError("product checkout is not clean")
    files = []
    for entry in git(root, "ls-files", "--stage", "-z").split(b"\0"):
        if not entry:
            continue
        metadata, name = entry.split(b"\t", 1)
        mode, blob, stage = metadata.decode().split()
        if stage != "0":
            raise RuntimeError("unmerged input")
        path = name.decode()
        item = {"path": path, "mode": mode, "blob": blob}
        if mode != "160000":
            p = root / path
            raw = os.readlink(p).encode() if mode == "120000" else p.read_bytes()
            item.update(sha256=digest(raw), bytes=len(raw))
        files.append(item)
    return {"revision": revision, "tree": git(root, "rev-parse", "HEAD^{tree}").decode().strip(),
            "files": files}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--product-sha", required=True)
    parser.add_argument("--postgres-container", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--private-dir", type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.product_sha):
        raise RuntimeError("full product SHA required")
    if not re.fullmatch(r"[0-9a-f]{12,64}", args.postgres_container):
        raise RuntimeError("exact job-owned PostgreSQL container ID required")
    root = args.source.resolve(strict=True)
    out, private = args.output, args.private_dir
    if not out.is_absolute() or not private.is_absolute() or out == private:
        raise RuntimeError("separate absolute output and private directories required")
    if out.parent.resolve(strict=True) != out.parent or private.parent.resolve(strict=True) != private.parent:
        raise RuntimeError("symlink-free output parents required")
    os.umask(0o077)
    out.mkdir(mode=0o700)
    private.mkdir(mode=0o700)
    hidden = []
    databases, roles, credential_files = [], [], []
    receipt = {"schema": "lmm.market-pg-check.v1", "started_at": utc(),
               "product_sha": args.product_sha, "expected_parent_cases": [*TOOL_CASES, MERCHANT_CASE],
               "core_authority": "synthetic adapters", "core_funds": "not exercised",
               "provider": "counted local synthetic endpoint", "tests": [], "cleanup": []}

    def write(name, value):
        raw = json.dumps(value, indent=2, sort_keys=True) + "\n"
        for secret in hidden:
            raw = raw.replace(secret, "[fixture-secret]")
        (out / name).write_text(raw)

    def admin(label, sql, database="postgres"):
        command = ["docker", "exec", "-i", args.postgres_container, "psql", "-X", "-qAt",
                   "-U", "postgres", "-d", database, "-v", "ON_ERROR_STOP=1", "-f", "-"]
        result = subprocess.run(command, input=sql.encode(), capture_output=True, timeout=30)
        if result.returncode:
            error = result.stderr.decode(errors="replace")
            for secret in hidden:
                error = error.replace(secret, "[fixture-secret]")
            write("postgres-error-" + label + ".json", {"operation": label, "exit_code": result.returncode,
                                                       "stderr": error})
            raise RuntimeError("PostgreSQL fixture operation failed: " + label)
        return result.stdout.decode().strip()

    def create_role(name):
        password = secrets.token_urlsafe(32)
        hidden.append(password)
        admin("create-role", f"CREATE ROLE {name} LOGIN PASSWORD '{password}' NOSUPERUSER "
              "NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;")
        roles.append(name)
        return password

    def credential(label, role, password, database):
        dsn = f"postgres://{role}:{quote(password, safe='')}@127.0.0.1:5432/{database}?sslmode=disable"
        hidden.append(dsn)
        path = private / (label + ".dsn")
        path.write_text(dsn + "\n")
        path.chmod(0o600)
        credential_files.append(path)
        return path, dsn

    def fixture(label, runtime):
        prefix = "market_" + secrets.token_hex(6) + "_" + label
        owner, database = prefix + "_owner", prefix + "_test"
        password = create_role(owner)
        admin("create-database", f"CREATE DATABASE {database} OWNER {owner} TEMPLATE template0;")
        databases.append(database)
        admin("restrict-database", f"REVOKE ALL ON DATABASE {database} FROM PUBLIC;")
        owner_file, owner_dsn = credential(label + "-owner", owner, password, database)
        value = {"database": database, "owner": owner, "owner_file": owner_file, "owner_dsn": owner_dsn}
        if runtime:
            role = prefix + "_runtime"
            password = create_role(role)
            admin("runtime-connect", f"GRANT CONNECT ON DATABASE {database} TO {role};")
            path, dsn = credential(label + "-runtime", role, password, database)
            value.update(runtime=role, runtime_file=path, runtime_dsn=dsn)
        return value

    def runtime_proof(label, value):
        role, owner = value["runtime"], value["owner"]
        sql = f"""SELECT json_build_object(
          'same_database_owner', d.datdba=(SELECT oid FROM pg_roles WHERE rolname='{owner}'),
          'runtime_owns_database', d.datdba=r.oid,
          'runtime_superuser', r.rolsuper, 'runtime_createdb', r.rolcreatedb,
          'runtime_createrole', r.rolcreaterole, 'runtime_replication', r.rolreplication,
          'runtime_bypassrls', r.rolbypassrls,
          'runtime_owner_member', pg_has_role('{role}','{owner}','MEMBER'),
          'schema_usage', has_schema_privilege('{role}','lmm_toolmarket','USAGE'),
          'schema_create', has_schema_privilege('{role}','lmm_toolmarket','CREATE'),
          'snapshot_select', has_table_privilege('{role}','lmm_toolmarket.snapshot','SELECT'),
          'snapshot_update', has_table_privilege('{role}','lmm_toolmarket.snapshot','UPDATE'),
          'snapshot_insert', has_table_privilege('{role}','lmm_toolmarket.snapshot','INSERT'),
          'snapshot_delete', has_table_privilege('{role}','lmm_toolmarket.snapshot','DELETE'),
          'snapshot_truncate', has_table_privilege('{role}','lmm_toolmarket.snapshot','TRUNCATE'))
          FROM pg_roles r JOIN pg_database d ON d.datname=current_database()
          WHERE r.rolname='{role}';"""
        proof = json.loads(admin("runtime-proof-" + label, sql, value["database"]))
        expected_true = {"same_database_owner", "schema_usage", "snapshot_select", "snapshot_update"}
        if len(proof) != 15 or any(v is not (k in expected_true) for k, v in proof.items()):
            raise RuntimeError("runtime permissions exceed the fixture contract")
        write("runtime-permissions-" + label + ".json", proof)

    allowed_env = ("PATH", "HOME", "LANG", "LC_ALL", "TMPDIR", "RUNNER_TEMP", "GOCACHE", "GOPATH",
                   "GOMODCACHE", "GOPROXY", "GOSUMDB", "SSL_CERT_FILE", "SSL_CERT_DIR")
    env = {k: os.environ[k] for k in allowed_env if k in os.environ}
    env.update(GOWORK="off", GOTOOLCHAIN="local", GOMAXPROCS="2", CGO_ENABLED="1", GOFLAGS="-mod=readonly -p=1")

    def run_cases(label, package, names, variables):
        pattern = "^(" + "|".join(names) + ")$"
        command = ["go", "test", "-mod=readonly", "-race", "-count=1", "-json", package, "-run", pattern]
        began = time.monotonic()
        info = {"label": label, "command": command, "started_at": utc(), "cwd": "services/extensions"}
        try:
            result = subprocess.run(command, cwd=root / "services/extensions", env={**env, **variables},
                                    capture_output=True, timeout=780)
            code, timed_out = result.returncode, False
            raw_stdout, raw_stderr = result.stdout, result.stderr
        except subprocess.TimeoutExpired as error:
            code, timed_out = None, True
            raw_stdout, raw_stderr = error.stdout or b"", error.stderr or b""
        info.update(exit_code=code, command_timed_out=timed_out,
                    elapsed_seconds=time.monotonic() - began, finished_at=utc())
        stdout, stderr = raw_stdout.decode(errors="replace"), raw_stderr.decode(errors="replace")
        for secret in hidden:
            stdout, stderr = stdout.replace(secret, "[fixture-secret]"), stderr.replace(secret, "[fixture-secret]")
        (out / (label + ".jsonl")).write_text(stdout)
        (out / (label + ".stderr")).write_text(stderr)
        events, parse_error = [], False
        for line in stdout.splitlines():
            if line.strip():
                try:
                    events.append(json.loads(line))
                except json.JSONDecodeError:
                    parse_error = True
        selected = [e for e in events if e.get("Test") and e.get("Action") in ("run", "pass", "fail", "skip")]
        observed = {e["Test"] for e in selected}
        started = [e["Test"] for e in selected if e["Action"] == "run"]
        passed = [e["Test"] for e in selected if e["Action"] == "pass"]
        refused = [e for e in selected if e["Action"] in ("skip", "fail")]
        ok = code == 0 and not parse_error and observed == set(names) and sorted(started) == sorted(names) and sorted(passed) == sorted(names) and not refused
        info.update(passed=passed, skipped_or_failed=[{"test": e["Test"], "action": e["Action"]} for e in refused],
                    expected=list(names), observed=sorted(observed), json_parse_error=parse_error, qualified=ok)
        receipt["tests"].append(info)
        write("test-results.json", receipt["tests"])
        return ok

    before = None
    try:
        receipt["phase"] = "control-binding"
        control = Path(__file__).resolve().parent.parent
        control_revision = git(control, "rev-parse", "HEAD").decode().strip()
        if control_revision != os.environ.get("GITHUB_SHA"):
            raise RuntimeError("control checkout differs from its workflow head")
        write("control.json", {"revision": control_revision,
                              "tree": git(control, "rev-parse", "HEAD^{tree}").decode().strip(),
                              "runner_sha256": digest(Path(__file__).read_bytes()),
                              "workflow_sha256": digest((control / ".github/workflows/core-protocol.yml").read_bytes())})
        receipt["phase"] = "product-binding"
        before = source_binding(root, args.product_sha)
        write("source-before.json", before)
        receipt["phase"] = "toolchain"
        version = subprocess.run(["go", "version"], env=env, check=True, capture_output=True, text=True, timeout=10).stdout.strip()
        if version != "go version go1.27.2 linux/amd64":
            raise RuntimeError("unexpected Go toolchain")
        pg_version = admin("version", "SHOW server_version;")
        if not pg_version.startswith("18."):
            raise RuntimeError("unexpected PostgreSQL major version")
        write("tools.json", {"go": version, "postgres": pg_version})
        receipt["phase"] = "private-fixtures"
        persistence, review, merchant = fixture("persistence", True), fixture("review", True), fixture("merchant", False)
        receipt["phase"] = "toolmarket-cases"
        good = run_cases("toolmarket", "./internal/modules/toolmarket", TOOL_CASES, {
            "LMM_TOOLMARKET_TEST_DSN_FILE": str(persistence["runtime_file"]),
            "LMM_TOOLMARKET_TEST_OWNER_DSN_FILE": str(persistence["owner_file"]),
            "LMM_MARKET_REVIEW_TEST_DSN_FILE": str(review["runtime_file"]),
            "LMM_MARKET_REVIEW_TEST_OWNER_DSN_FILE": str(review["owner_file"]),
        })
        receipt["phase"] = "merchant-home-case"
        merchant_good = run_cases("merchant-home", "./internal/webcompat", (MERCHANT_CASE,), {
            "LMM_STORE_TEST_DSN": merchant["owner_dsn"],
        })
        receipt["phase"] = "runtime-permissions"
        runtime_proof("persistence", persistence)
        runtime_proof("review", review)
        if not good or not merchant_good:
            raise RuntimeError("the three original parent cases did not all pass without skips")
        receipt["result"] = "PASS"
    except Exception as error:
        # Only fixed diagnostic labels reach the receipt, never repr/subprocess argv/env.
        receipt["result"] = "FAIL"
        receipt["error_class"] = type(error).__name__
    finally:
        if before is not None:
            try:
                after = source_binding(root, args.product_sha)
                write("source-after.json", after)
                receipt["source_unchanged"] = before == after
                if before != after:
                    receipt["result"] = "FAIL"
            except Exception:
                receipt["source_unchanged"] = False
                receipt["result"] = "FAIL"
        for database in reversed(databases):
            try:
                admin("drop-database", f"DROP DATABASE {database};")
                receipt["cleanup"].append({"kind": "database", "removed": True})
            except Exception:
                receipt["cleanup"].append({"kind": "database", "removed": False})
                receipt["result"] = "FAIL"
        for role in reversed(roles):
            try:
                admin("drop-role", f"DROP ROLE {role};")
                receipt["cleanup"].append({"kind": "role", "removed": True})
            except Exception:
                receipt["cleanup"].append({"kind": "role", "removed": False})
                receipt["result"] = "FAIL"
        try:
            for path in credential_files:
                path.unlink()
            private.rmdir()
            receipt["credential_files_removed"] = True
        except OSError:
            receipt["credential_files_removed"] = False
            receipt["result"] = "FAIL"
        receipt["finished_at"] = utc()
        write("receipt.json", receipt)
        write("artifact-manifest.json", {p.name: {"sha256": digest(p.read_bytes()), "bytes": p.stat().st_size}
                                         for p in sorted(out.iterdir()) if p.is_file()})
    print(json.dumps({"result": receipt["result"], "selected_parent_cases": 3, "evidence": str(out)}))
    return 0 if receipt["result"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())

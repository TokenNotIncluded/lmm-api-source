"""Verify the site relay refund through real Core processes and a private PG cluster."""

import json
import os
import pathlib
import socket
import subprocess
import tempfile
import time
from urllib.parse import quote
from urllib.request import build_opener, ProxyHandler, Request


def main():
    core = pathlib.Path(os.environ["LMM_RELAY_REFUND_CORE_BIN"]).resolve(strict=True)
    source = pathlib.Path(__file__).resolve().parents[1]
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix="rfc-") as path:
        root = pathlib.Path(path)
        if "LMM_RELAY_REFUND_GO_TEST_BIN" in os.environ:
            go = pathlib.Path(os.environ["LMM_RELAY_REFUND_GO_TEST_BIN"]).resolve(
                strict=True
            )
        else:
            go = root / "relay-pgtest"
            subprocess.run(
                ["go", "test", "-c", "-o", go, "./internal/webcompat"],
                cwd=source / "services/extensions",
                check=True,
            )
        data = root / "pg"

        def call(args, *, text=None, env=None):
            r = subprocess.run(
                args, input=text, text=True, capture_output=True, env=env, timeout=90
            )
            if r.returncode != 0:
                raise RuntimeError(
                    f"{pathlib.Path(args[0]).name} {r.returncode}: {r.stderr[-1800:]}"
                )
            return r.stdout.strip()

        call(
            [
                "initdb",
                "-D",
                data,
                "-U",
                "fixture",
                "-A",
                "trust",
                "--encoding=UTF8",
                "--locale=C",
            ]
        )
        call(
            [
                "pg_ctl",
                "-D",
                data,
                "-o",
                f"-c listen_addresses='' -c unix_socket_directories='{root}'",
                "-l",
                root / "pg.log",
                "-w",
                "start",
            ]
        )
        child = None
        try:
            for db in ("core_refund_test", "relay_refund_test"):
                call(["createdb", "-h", root, "-U", "fixture", db])
            psql = [
                "psql",
                "-X",
                "-qAt",
                "-v",
                "ON_ERROR_STOP=1",
                "-h",
                root,
                "-U",
                "fixture",
                "-d",
                "core_refund_test",
            ]

            def sql(stmt):
                return call(psql, text=stmt)

            def priv(name, value):
                f = root / name
                f.write_text(value)
                return f

            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            sockpath = root / "core.sock"
            ident = priv(
                "id-token", "synthetic-identity-relay-refund-service-credential"
            )
            money = priv(
                "money-token", "synthetic-commerce-relay-refund-service-credential"
            )
            perms = priv(
                "commerce.toml",
                "version=1\npay=true\nsettle=true\nrefund=true\nread=true\ncomplete_delivery=true\nreconcile=true\n",
            )

            def config(name, user):
                url = priv(
                    name + ".url",
                    f"postgresql://{user}@localhost/core_refund_test?host={quote(str(root), safe='')}&sslmode=disable",
                )
                return priv(
                    name + ".toml",
                    f"""version=1
[server]
listen='127.0.0.1:{port}'
[database]
url_file='{url}'
[rpc]
socket='{sockpath}'
token_file='{ident}'
[commerce]
config_file='{perms}'
token_file='{money}'
""",
                )

            admin = config("owner", "fixture")
            run = config("runtime", "relay_runtime")
            sql(
                "CREATE ROLE relay_owner NOLOGIN; CREATE ROLE relay_runtime LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; ALTER DATABASE core_refund_test OWNER TO relay_owner; ALTER ROLE fixture IN DATABASE core_refund_test SET role TO relay_owner;"
            )
            call([core, "--config", admin, "init-db"])
            secrets = {}
            for label, user, level in (
                ("BUYER", 7, 1),
                ("MERCHANT", 11, 6),
                ("OTHER", 19, 5),
            ):
                issued = json.loads(
                    call(
                        [core, "--config", admin, "bootstrap-user"],
                        text=json.dumps(dict(user_id=user, platform_level=level)),
                    )
                )
                secrets[label] = issued
            call(
                psql
                + [
                    "-v",
                    "owner_role=relay_owner",
                    "-v",
                    "runtime_role=relay_runtime",
                    "-v",
                    "enable_commerce=true",
                    "-v",
                    "enable_events=true",
                    "-f",
                    source / "deployment/postgres/grant-core-runtime.sql",
                ]
            )
            # Core releases with reset-aware charging check these additional narrow privileges.
            if (
                sql(
                    "SELECT to_regclass('core_charging.subscription_request_resets') IS NOT NULL;"
                )
                == "t"
            ):
                sql(
                    "GRANT USAGE ON SCHEMA core_subscriptions TO relay_runtime; GRANT SELECT ON core_subscriptions.reset_events TO relay_runtime; GRANT SELECT,INSERT ON core_charging.subscription_request_resets TO relay_runtime; GRANT EXECUTE ON FUNCTION core_charging.subscription_base_window(bigint,bigint),core_charging.subscription_consumption(bigint,bigint,text) TO relay_runtime;"
                )
            sql(
                "SELECT core_billing.open_ledger_wallet(personal_account_id) FROM core_identity.users WHERE id IN(7,11);"
            )
            buyer = sql(
                "SELECT personal_account_id FROM core_identity.users WHERE id=7;"
            )
            setup = dict(
                scope="relay-test",
                operation_key="opening",
                actor_user_id=7,
                reason="Disposable isolated fixture opening",
                action=dict(
                    kind="credit",
                    account_id=int(buyer),
                    amount_units=73,
                    payment_reference="synthetic:relay",
                ),
            )
            sql("SELECT core_billing.post_ledger('" + json.dumps(setup) + "'::jsonb);")
            env = os.environ.copy()
            env.update(
                LMM_STORE_TEST_DSN=f"host={root} user=fixture port=5432 dbname=relay_refund_test sslmode=disable",
                LMM_REFUND_TEST_SOCKET=str(sockpath),
            )
            for key, issued in secrets.items():
                env["LMM_REFUND_FIXTURE_" + key] = issued["secret"]
            for phase in ("lost", "restart", "revoked"):
                with (root / "core.log").open("w") as log:
                    child = subprocess.Popen(
                        [core, "--config", run, "serve"], stdout=log, stderr=log
                    )
                    for _ in range(200):
                        if child.poll() is not None:
                            raise RuntimeError(
                                "Core startup: "
                                + (root / "core.log").read_text()[-1800:]
                            )
                        if sockpath.exists():
                            break
                        time.sleep(0.05)
                    env["LMM_REFUND_TEST_PHASE"] = phase
                    cp = subprocess.run(
                        [
                            go,
                            "-test.v",
                            "-test.run=^TestRelayExpiredRefundCoreProcess$",
                            "-test.timeout=60s",
                        ],
                        capture_output=True,
                        text=True,
                        env=env,
                        timeout=65,
                    )
                    if cp.returncode:
                        raise RuntimeError(
                            f"{phase}: {cp.stdout[-2400:]} {cp.stderr[-900:]}"
                        )
                    if phase == "restart":
                        # Exercise the real Core session revocation before the next restart.
                        req = Request(
                            f"http://127.0.0.1:{port}/core/v1/auth/logout",
                            data=b"",
                            method="POST",
                            headers={
                                "Authorization": "Bearer "
                                + secrets["MERCHANT"]["secret"]
                            },
                        )
                        with build_opener(ProxyHandler({})).open(
                            req, timeout=5
                        ) as response:
                            assert response.status == 204
                    child.terminate()
                    child.wait(timeout=12)
                    child = None
                rows = sql(
                    "SELECT kind||'='||amount_units FROM core_billing.ledger_journals ORDER BY id;"
                ).splitlines()
                assert rows == ["credit=73", "reserve=9", "release=9"], rows
                assert sql("SELECT count(*) FROM core_commerce.operations;") == "2"
                assert sql("SELECT count(*) FROM core_commerce.orders;") == "1"
                assert (
                    sql(
                        "SELECT sum(amount_credits-refunded) FROM core_charging.external_consumption;"
                    )
                    == "0"
                )
                assert (
                    sql(
                        "SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) s;"
                    )
                    == "0"
                )
                assert (
                    sql(
                        "SELECT string_agg(u.id||'='||b.balance_units,',' ORDER BY u.id) FROM core_identity.users u JOIN core_billing.ledger_accounts a ON a.owner_account_id=u.personal_account_id AND a.bucket='wallet' JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE u.id IN (7,11);"
                    )
                    == "7=73,11=0"
                )
                print(
                    f"{phase}: private Core + HTTP; buyer=73 merchant=0 reserve=9 release=9 orders=1 operations=2 net consumption=0; journals balanced",
                    flush=True,
                )
        finally:
            if child and child.poll() is None:
                child.kill()
                child.wait()
            subprocess.run(
                ["pg_ctl", "-D", data, "-m", "immediate", "-w", "stop"],
                capture_output=True,
            )


if __name__ == "__main__":
    main()

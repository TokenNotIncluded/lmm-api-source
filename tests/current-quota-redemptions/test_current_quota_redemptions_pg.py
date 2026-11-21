#!/usr/bin/env python3
"""Real Core78 append and HTTP redemption on a disposable marked database.

Use the existing four-role Core78 fixture, private pgpass and BACKUP_ROOT.
Do not rebuild Core. Run `mechanics` with every runtime stopped; it preserves
real Sessions, a native budget, original funds and an unrelated old campaign.
After the controller starts the same binary on loopback, run `runtime` against
the emitted private probes. This never creates production approval evidence.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import Request, build_opener, HTTPRedirectHandler
import uuid

import migrate_current_quota_redemptions as M
import test_migrate_current_runtime_keys_pg as KT
from test_current_quota_redemptions import source_row

check = KT.check
NAMESPACE = {"system_identifier": "991882773665", "database": "synthetic_quota_source", "schema": "synthetic_public"}


class Regression(KT.Regression):
    def seed(self):
        self.seed_sessions_and_budget()
        self.now = self.query("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
        uid = self.owner["user_id"]
        self.mapping = [{"source_user_id": 37, "target_user_id": uid,
                         "target_personal_account_id": self.owner["personal_account_id"]}]
        native_code = uuid.uuid4().hex
        self.native_revoked_code = native_code
        cid, iid = hashlib.sha256(b"synthetic-preexisting-campaign").hexdigest(), hashlib.sha256(b"synthetic-preexisting-issue").hexdigest()
        self.run_sql("BEGIN ISOLATION LEVEL SERIALIZABLE; "
            "INSERT INTO core_promotions.campaigns(id,revision,kind,title,award_units,budget_units,max_claims,max_claims_per_user,starts_at,ends_at,enabled) "
            f"VALUES({M.literal(cid)},1,'redemption','Synthetic old native campaign',7,7,1,1,0,{M.MAX_TIME},true); "
            f"INSERT INTO core_promotions.issues(id,campaign_id,actor_user_id,quantity,created_at) VALUES({M.literal(iid)},{M.literal(cid)},{uid},1,{self.now}); "
            "INSERT INTO core_promotions.codes(digest,issue_id,position,campaign_id,revoked) "
            f"VALUES(decode({M.literal(hashlib.sha256(native_code.encode()).hexdigest())},'hex'),{M.literal(iid)},0,{M.literal(cid)},true); COMMIT;")
        # A second genuine ordinary actor proves a claimed bearer code cannot
        # be credited to another account through a different operation key.
        aid = self.query("INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id")
        other = self.query(f"INSERT INTO core_identity.users(personal_account_id) VALUES({aid}) RETURNING id")
        self.run_sql(f"INSERT INTO core_identity.user_profiles(user_id,username) VALUES({other},'synthetic-quota-other');")
        self.other_bearer = "synthetic-quota-session-" + uuid.uuid4().hex
        credential = self.query("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) "
            f"VALUES(decode({M.literal(hashlib.sha256(self.other_bearer.encode()).hexdigest())},'hex'),'session',{other},1,{aid},clock_timestamp()+interval '1 day') RETURNING id")
        self.run_sql("INSERT INTO core_identity.session_authentication(credential_id,user_id,user_version,factor_revision,factor_level,method,authenticated_at) "
            f"VALUES({credential},{other},1,0,1,'primary',clock_timestamp())")
        self.other_user = other
        self.rows = [source_row(9001 + i, key=uuid.uuid4().hex, created_time=self.now - 30,
                     expired_time=self.now + 3600 if i == 1 else 0, quota=31 + i) for i in range(3)]
        self.rows += [source_row(9100, key=uuid.uuid4().hex, status=2),
                      source_row(9101, key=uuid.uuid4().hex, expired_time=self.now - 1),
                      source_row(9102, key=uuid.uuid4().hex, reward_type="reset_voucher")]
        self.completed.append("seeded_real_sessions_native_budget_funds_and_unrelated_revoked_campaign")

    def prepare_quota(self, migration, *, rows=None, adversarial_duplicate=False):
        snapshot = self.query(M.U.snapshot_sql(self.contract, self.inventory, migration))
        observed = self.query(M.native_sql(self.mapping))
        facts = M.source_facts(self.rows if rows is None else rows, self.mapping, self.now)
        passed = deepcopy(observed)
        if adversarial_duplicate:
            # Pure preflight sees an intentionally incomplete projection; the
            # SQL sees the real observation and must independently reject the
            # stable source identity before any INSERT/PK collision.
            passed["imports"] = []
            for name in M.CHANGED:
                passed[name] = []
        material = M.prepare_mechanics(facts, self.mapping, snapshot, self.contract, NAMESPACE,
            "a" * 64, migration, self.fixture["runtime_roles"], passed)
        material["native_observation"] = observed
        path = M.F.write_private(self.output, migration + "-" + str(self.counter) + ".material.private.json", M.canonical(material) + "\n")
        return material, path

    def proxy(self, m, before_commit):
        conn = deepcopy(self.connection)
        original = Path(conn["socket_directory"]) / (".s.PGSQL." + str(conn["port"]))
        directory = self.output / ("quota-proxy-" + uuid.uuid4().hex[:8])
        app = "lmm-current-quota-" + uuid.uuid4().hex
        with KT.CommitDropProxy(original, directory, conn["port"], before_commit=before_commit):
            conn["socket_directory"] = str(directory)
            code, _, _ = self.run_sql(M.synthetic_sql(m, self.nonce), connection=conn, app=app, expect_ok=False)
            check(code != 0, "lost PG connection was incorrectly acknowledged")
        for _ in range(50):
            active = self.query("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=" + M.literal(app) + ")")
            if not active:
                break
            time.sleep(0.1)
        check(not active, "original quota transaction is still active; do not retry")
        return self.query(M.resolve_sql(m, app))

    def concurrent_fence(self, m):
        env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        env.update(M.I.socket_connection(self.connection))
        env.update(PGAPPNAME="synthetic-quota-fence-" + uuid.uuid4().hex,
                   PGOPTIONS=M.I.psql_options(self.connection, readonly=False))
        proc = subprocess.Popen(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True, env=env)
        try:
            proc.stdin.write(f"SELECT pg_advisory_lock({M.U.FENCE}); SELECT pg_sleep(20);\n")
            proc.stdin.close()
            ready = False
            for _ in range(50):
                ready = self.query(f"SELECT NOT pg_try_advisory_xact_lock({M.U.FENCE})")
                if ready:
                    break
                time.sleep(0.1)
            check(ready, "competing importer did not obtain fence")
            self.reject(M.synthetic_sql(m, self.nonce), "quota_append_import_busy")
        finally:
            if proc.poll() is None:
                proc.send_signal(signal.SIGINT)
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)
        self.completed.append("concurrent_importer_refused_without_any_row_or_sequence_change")

    def run(self):
        self.seed()
        migration = "synthetic-current-quota-" + uuid.uuid4().hex[:10]
        m, path = self.prepare_quota(migration)
        self.concurrent_fence(m)
        self.run_sql("BEGIN ISOLATION LEVEL SERIALIZABLE; "
            f"UPDATE core_charging.budgets SET revision=revision+1,limit_credits=10000 WHERE id={self.native_budget_id}; COMMIT;")
        self.reject(M.synthetic_sql(m, self.nonce), "quota_append_target_state_changed")
        self.completed.append("real_native_budget_drift_rejected_before_any_insert")
        m, path = self.prepare_quota(migration)
        before = self.state(migration)
        lost = self.proxy(m, before_commit=True)
        check(lost["status"] == "UNKNOWN" and lost["automatic_retry_allowed"] is False,
              "receipt absence must remain unknown")
        check(self.state(migration) == before, "precommit disconnect changed rows or sequences")
        self.completed.append("actual_precommit_disconnect_preserves_all_rows_and_every_sequence")
        # The test proxy positively observed that COMMIT was never sent, and
        # the original backend exited. Take a new explicitly reviewed fixture
        # snapshot; this is not the production unknown-result retry behavior.
        m, path = self.prepare_quota(migration)
        committed = self.proxy(m, before_commit=False)
        check(committed["status"] == "COMMITTED", "real committed transaction did not resolve")
        after = self.state(migration)
        check(self.query(M.state_sql(m, preserve=True)) == m["before_state"],
              "existing funds/Sessions/native campaigns/budgets/history/sequences changed")
        replay = self.query(M.synthetic_sql(m, self.nonce))
        check(replay["status"] == "COMMITTED" and self.state(migration) == after, "exact replay changed target")
        self.completed.extend(["actual_commit_reply_loss_resolves_readonly", "exact_replay_preserves_all_state",
            "no_wallet_credit_or_claim_at_import", "all_old_rows_and_sequences_preserved"])
        dupe_rows = [source_row(9001, key=uuid.uuid4().hex, created_time=self.now - 30)]
        duplicate, _ = self.prepare_quota(migration + "-duplicate", rows=dupe_rows, adversarial_duplicate=True)
        self.reject(M.synthetic_sql(duplicate, self.nonce), "quota_append_source_identity_already_imported")
        self.completed.append("database_namespace_and_source_identity_dedup_survives_hidden_python_receipts")
        # A legitimate later native revocation changes mutable campaign
        # counters. Resolve acknowledges the immutable commit and never clears
        # that revocation or restores its reservation.
        rev = next(r for r in m["plan"]["records"] if r["source_redemption_id"] == 9003)
        self.run_sql("BEGIN ISOLATION LEVEL SERIALIZABLE; "
            f"UPDATE core_promotions.codes SET revoked=true WHERE digest=decode({M.literal(rev['code_sha256'])},'hex'); "
            f"UPDATE core_promotions.campaigns SET reserved_units=reserved_units-{rev['award_units']},reserved_claims=reserved_claims-1 WHERE id={M.literal(rev['campaign_id'])}; COMMIT;")
        revoked = self.state(migration)
        result = self.query(M.resolve_sql(m, "lmm-current-quota-" + uuid.uuid4().hex))
        check(result["status"] == "COMMITTED" and self.state(migration) == revoked, "resolve revived or modified a revoked code")
        self.query(M.synthetic_sql(m, self.nonce))
        check(self.state(migration) == revoked, "replay restored a later native revocation")
        self.completed.append("resolve_and_replay_preserve_later_native_revocation")
        claims = self.query("SELECT count(*) FROM core_promotions.claims")
        check(claims == 0, "synthetic append unexpectedly created claims")
        probes = {"scope": "current-quota-redemption-synthetic-runtime", "fixture_nonce": self.nonce,
            "database": self.connection["database"], "instance": m["snapshot"]["instance"],
            "material": {"path": str(path), "sha256": hashlib.sha256(M.C.private_bytes(path)).hexdigest()},
            "owner": self.owner, "session": self.session_bearers[0], "other_session": self.other_bearer,
            "other_user": self.other_user, "codes": self.rows, "preexisting_revoked_code": self.native_revoked_code,
            "production_approval_created": False}
        M.F.write_private(self.output, "runtime-probes.private.json", M.canonical(probes) + "\n")
        public = {"status": "SYNTHETIC_PG_APPEND_VERIFIED", "checks": self.completed,
            "http_runtime": "NOT_RUN", "production_approval_created": False, "product_rebuilt": False,
            "production_writes": 0}
        M.F.write_private(self.output, "result.public.json", M.canonical(public) + "\n")
        print(json.dumps(public, sort_keys=True))


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def runtime(fixture, probes, base_url, output):
    url = urlsplit(base_url)
    check(url.scheme == "http" and url.hostname in {"127.0.0.1", "localhost", "::1"}
          and url.port and url.path in {"", "/"} and not url.query and not url.fragment
          and url.username is None and url.password is None, "synthetic runtime must use explicit loopback HTTP")
    check(probes["scope"] == "current-quota-redemption-synthetic-runtime"
          and probes["fixture_nonce"] == fixture["fixture_nonce"]
          and probes["database"] == fixture["connection"]["database"], "synthetic runtime fixture mismatch")
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    m = M.K.read_ref(probes["material"])
    M.validate_material(m)
    count = 0

    def query(sql):
        nonlocal count
        count += 1
        statement = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC'; SET LOCAL row_security=off;\n" + sql + ";\nCOMMIT;\n"
        code, raw = M.F.run_sql(fixture["connection"], statement, output, "synthetic-quota-http-readback", readonly=True, label="read-" + str(count))
        check(code == 0, "runtime read-only SQL failed")
        return KT.parse_single_result(raw)

    marker = query("SELECT jsonb_build_object('marker',shobj_description((SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database'),'instance',(" + M.F.instance_sql() + "))")
    check(marker == {"marker": "lmm-runtime-keys-synthetic:" + fixture["fixture_nonce"], "instance": probes["instance"]},
          "runtime database physical identity changed")
    account = probes["owner"]["personal_account_id"]
    balance_sql = "SELECT balance_units FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.owner_account_id=" + str(account) + " AND a.bucket='wallet'"
    original_balance = query(balance_sql)
    opener = build_opener(NoRedirect())

    def post(code, operation, bearer=None):
        body = json.dumps({"code": code, "operation_key": operation}).encode()
        req = Request(base_url.rstrip("/") + "/core/v1/promotions/redemptions", data=body,
            headers={"Authorization": "Bearer " + (probes["session"] if bearer is None else bearer), "Content-Type": "application/json"}, method="POST")
        try:
            with opener.open(req, timeout=30) as response:
                return response.status, json.loads(response.read())
        except HTTPError as error:
            return error.code, json.loads(error.read())

    rows = {r["id"]: r for r in probes["codes"]}
    first = post(rows[9001]["key"], "synthetic_quota_http_first")
    check(first[0] == 200 and int(first[1]["amount_units"]) == rows[9001]["quota"], "original 32-hex code did not redeem exact quota")
    replay = post(rows[9001]["key"], "synthetic_quota_http_first")
    second_operation = post(rows[9001]["key"], "synthetic_quota_http_same_code")
    check(replay == first and second_operation == first, "claim replay created a second gift or changed receipt")
    check(post(rows[9001]["key"], "synthetic_quota_http_other_actor", probes["other_session"])[0] == 409,
          "another owner reused claimed code")
    with ThreadPoolExecutor(max_workers=2) as executor:
        results = list(executor.map(lambda n: post(rows[9002]["key"], "synthetic_quota_http_race_" + str(n)), (1, 2)))
    check(any(status == 200 for status, _ in results) and all(status in {200, 409, 503} for status, _ in results),
          "concurrent real claims did not produce a valid outcome")
    for sid, expected in ((9003, 409), (9100, 404), (9101, 404), (9102, 404)):
        check(post(rows[sid]["key"], "synthetic_quota_http_blocked_" + str(sid))[0] == expected,
              "revoked/disabled/expired/reset-purpose code gained quota authority")
    check(post(probes["preexisting_revoked_code"], "synthetic_quota_http_old_revoked")[0] == 409,
          "old native revocation was lost")
    expected_amount = rows[9001]["quota"] + rows[9002]["quota"]
    check(query(balance_sql) == original_balance + expected_amount, "runtime did not post exact once-only wallet delta")
    digests = ",".join(M.literal(hashlib.sha256(rows[sid]["key"].encode()).hexdigest()) for sid in (9001, 9002))
    evidence = query("SELECT jsonb_build_object('claims',(SELECT count(*) FROM core_promotions.claims WHERE encode(code_digest,'hex') IN (" + digests + ")),'amount',(SELECT sum(amount_units) FROM core_promotions.claims WHERE encode(code_digest,'hex') IN (" + digests + ")),'wrong_journals',(SELECT count(*) FROM core_promotions.claims c JOIN core_billing.ledger_journals j ON j.id=c.journal_id WHERE j.kind<>'gift' OR j.scope<>'core-promotion-rewards'),'unbalanced',(SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) x),'wrong_reservations',(SELECT count(*) FROM core_promotions.campaigns c WHERE c.reserved_units<>c.award_units*(SELECT count(*) FROM core_promotions.codes d WHERE d.campaign_id=c.id AND NOT d.revoked AND NOT EXISTS(SELECT 1 FROM core_promotions.claims l WHERE l.code_digest=d.digest))))")
    check(evidence == {"claims": 2, "amount": expected_amount, "wrong_journals": 0, "unbalanced": 0, "wrong_reservations": 0},
          "claim, gift journal, conservation or reservation readback failed")
    # Receipt resolution after real use remains read-only and acknowledges the
    # old append; it cannot reinterpret a consumed code as missing authority.
    code, raw = M.F.run_sql(fixture["connection"], M.resolve_sql(m, "lmm-current-quota-" + uuid.uuid4().hex), output,
                           "synthetic-quota-post-runtime-resolve", readonly=True, label="post-runtime-resolve")
    check(code == 0 and KT.parse_single_result(raw)["status"] == "COMMITTED", "used append receipt did not resolve")
    public = {"status": "SYNTHETIC_PG_HTTP_QUOTA_REDEMPTION_VERIFIED", "checks": [
        "original_32hex_code_redeems_exact_amount", "operation_and_code_replay_post_once", "different_owner_cannot_reuse_claim",
        "concurrent_claims_have_one_journal", "revoked_disabled_expired_and_reset_codes_do_not_gain_quota",
        "preexisting_native_revocation_preserved", "real_gift_journal_and_ledger_conservation",
        "remaining_campaign_reservations_exact", "immutable_receipt_resolves_after_real_runtime_use"],
        "production_writes": 0, "production_approval_created": False, "product_rebuilt": False}
    M.F.write_private(output, "result.public.json", M.canonical(public) + "\n")
    print(json.dumps(public, sort_keys=True))


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("phase", choices=("mechanics", "runtime"))
    p.add_argument("--fixture", required=True, type=Path)
    p.add_argument("--output", required=True, type=Path)
    p.add_argument("--probes", type=Path)
    p.add_argument("--base-url")
    a = p.parse_args(argv)
    M.I.private_backup_path(a.output.parent, directory=True)
    fixture = M.C.load_json(a.fixture)
    if a.phase == "mechanics":
        Regression(fixture, a.output).run()
    else:
        runtime(fixture, M.C.load_json(a.probes), a.base_url, a.output)


if __name__ == "__main__":
    main()

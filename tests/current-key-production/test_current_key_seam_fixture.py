#!/usr/bin/env python3
"""Synthetic HTTP fixture extension; frozen migration writers stay unchanged."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import uuid

import current_key_production as M
import test_current_key_production_pg as G


class Regression(G.Regression):
    def seed_current_source(self):
        super().seed_current_source()
        self.owner_sessions = []
        for user_id in (103, 109):
            owner = self.query("SELECT jsonb_build_object('user_id',id,'account_id',personal_account_id,"
                               "'auth_version',auth_version) FROM core_identity.users WHERE id=" + str(user_id))
            bearer = "synthetic-owner-session-" + uuid.uuid4().hex
            credential = self.query("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,"
                "owner_account_id,expires_at) VALUES(decode('" + hashlib.sha256(bearer.encode()).hexdigest()
                + "','hex'),'session'," + str(user_id) + "," + str(owner['auth_version']) + ","
                + str(owner['account_id']) + ",clock_timestamp()+interval '1 day') RETURNING id")
            self.run_sql("INSERT INTO core_identity.session_authentication(credential_id,user_id,user_version,"
                "factor_revision,factor_level,method,authenticated_at) VALUES("
                + str(credential) + "," + str(user_id) + "," + str(owner['auth_version'])
                + ",0,1,'primary',clock_timestamp())")
            self.owner_sessions.append({**owner, 'credential_id': credential, 'bearer': bearer})
        self.completed.append('synthetic_owner_sessions_seeded_before_key_preservation_baseline')

    def run_current(self):
        super().run_current()
        path = self.output / 'runtime-probes.private.json'
        probe = json.loads(path.read_text())
        probe.update(owner_sessions=self.owner_sessions,
                     preexisting_user_id=self.fixture_user_id,
                     expired_session={'bearer': self.session_bearers[1], 'credential_id': self.session_ids[1]},
                     expected_core78_manifest_sha256=M.U.CORE78_MANIFEST_SHA256)
        # Keep the original probe file immutable and include every migration receipt
        # in the HTTP preservation baseline, using an asserted absent exclusion ID.
        check_id = 'synthetic-http-preservation-no-excluded-import'
        G.check(self.query('SELECT count(*) FROM core_migration.import_runs WHERE migration_id='
                          + M.literal(check_id)) == 0, 'state exclusion ID unexpectedly exists')
        state_sql = M.U.state_sql(self.inventory, check_id)
        before = self.query(state_sql)
        G.check(before['tables']['core_operations.instances']['rows'] == 0, 'fixture already has a live Core')
        before_audit = self.query("SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY id),'[]'::jsonb) FROM core_identity.audit a")
        probe['before_audit'] = before_audit
        M.F.write_private(self.output, 'runtime-seam-probes.private.json', M.canonical(probe) + '\n')
        M.F.write_private(self.output, 'http-preservation-before.private.json', M.canonical(before) + '\n')
        M.F.write_private(self.output, 'http-preservation-readback.sql', state_sql + ';\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fixture', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    M.frozen_dependencies()
    Regression(M.C.load_json(args.fixture), args.output).run_current()


if __name__ == '__main__':
    main()

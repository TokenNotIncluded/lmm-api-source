"""Only synthetic fixtures in the caller's fresh private PG. No real input path."""
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'scripts'))
import legacy_connection_import as imp

out = Path(os.environ['CONNECTION55_EVIDENCE'])
database = sys.argv[1]
registered = int(sys.argv[2])
source_db = 'connection_source_' + imp.sha(database.encode())[:16]
base_env = dict(os.environ)


def query(sql, db=database, role=None):
    env = {**base_env, 'PGDATABASE': db}
    if role:
        sql = 'SET ROLE '+role+';\n'+sql
    p = subprocess.run(['psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1'], input=sql,
                       text=True, capture_output=True, env=env)
    if p.returncode:
        raise RuntimeError(p.stderr)
    return p.stdout


subprocess.run(['createdb', source_db], check=True, env=base_env)
try:
    query('''CREATE TABLE users(id bigint,created_at bigint);
CREATE TABLE tokens(id bigint,user_id bigint,oauth_managed boolean,created_time bigint,deleted_at timestamptz);
CREATE TABLE oauth_server_grants(id text,user_id bigint,created_at_ms bigint,revoked_at_ms bigint);
CREATE TABLE developer_access_requests(id bigint,user_id bigint,created_at bigint,reviewed_at bigint,status text);
''' + f'''
INSERT INTO users SELECT id,{registered} FROM generate_series(101,107) id;
INSERT INTO tokens VALUES
 (1001,101,false,{registered+100},to_timestamp({registered+500})),
 (1002,102,true,{registered+200},NULL),
 (1003,104,false,{registered+100},NULL),
 (1004,104,true,{registered+50},NULL),
 (1005,105,false,0,NULL),
 (1006,105,NULL,{registered+300},NULL),
 (1007,999,false,{registered+50},NULL);
INSERT INTO oauth_server_grants VALUES
 ('synthetic-revoked',102,{(registered+150)*1000+999},{(registered+500)*1000}),
 ('synthetic-both',104,{(registered+75)*1000},0),
 ('synthetic-missing-time',105,NULL,0);
INSERT INTO developer_access_requests VALUES
 (1,101,{registered+10},{registered+20},'approved'),
 (2,102,{registered+30},0,'pending'),
 (3,105,NULL,NULL,'approved');
''', source_db)
    raw = query((ROOT/'scripts/legacy_connection_export.sql').read_text(), source_db).encode()
finally:
    subprocess.run(['dropdb', source_db], check=True, env=base_env)

metadata = {'contract': 'legacy.connection-export-binding.v1', 'source_archive_sha256': 'a'*64,
            'source_snapshot_id': 'SYNTHETIC-all-retained-v1', 'source_schema': 'public',
            'snapshot_observed_at': registered+3600, 'export_file_sha256': imp.sha(raw), 'coverage': imp.COVERAGE}
users = json.loads(query("SELECT json_agg(json_build_object('source_user_id',u.id+90,'user_id',u.id,'account_id',u.personal_account_id,'opening_entity_key','legacy.users/'||(u.id+90)::text) ORDER BY u.id) FROM core_identity.users u WHERE u.id BETWEEN 11 AND 15"))
mapping = {'contract': 'legacy.connection-mapping.v1', 'users': users,
           'omitted_users': [{'source_user_id': 106, 'reason': 'synthetic existing legacy user awaiting mapping'},
                             {'source_user_id': 107, 'reason': 'synthetic omitted/deleted source owner'}]}
mapping_raw = json.dumps(mapping, sort_keys=True).encode()
target = json.loads(query((ROOT/'scripts/legacy_connection_target.sql').read_text()))
for name, value in [('source.json', raw), ('mapping.json', mapping_raw), ('metadata.json', json.dumps(metadata).encode()), ('target.json', json.dumps(target).encode())]:
    (out/name).write_bytes(value)


def plan(meta=metadata, map_raw=mapping_raw, binding=target):
    return imp.plan(raw, meta, map_raw, binding, opening_migration_id='synthetic-connection-opening',
                    opening_mapping_sha256='b'*64, identity_migration_id='synthetic-connection-identity')

valid = plan()
(out/'import.sql').write_text(valid)
# Every plan is independently tested while target still has no snapshot.
variants = {
    'wrong-target': plan(binding={**target, 'target_database': target['target_database']+'_wrong'}),
    'wrong-nonce': plan(binding={**target, 'target_nonce': 'c'*64}),
    'wrong-archive': plan(meta={**metadata, 'source_archive_sha256': 'f'*64}),
    'wrong-migration': valid.replace('synthetic-connection-opening', 'synthetic-missing-opening'),
    'wrong-map': plan(map_raw=json.dumps({**mapping, 'users': [{**u, 'account_id': u['account_id']+1} if u['user_id']==11 else u for u in users]}).encode()),
    'changed-credential-floor': plan(binding={**target, 'native_credential_floor': target['native_credential_floor']+1}),
}

def apply(sql, role):
    env = {**base_env, 'PGDATABASE': database}
    return subprocess.run(['psql','-X','-qAt','-v','ON_ERROR_STOP=1'], input='SET ROLE '+role+';\n'+sql,
                          text=True, capture_output=True, env=env)

for name, sql in variants.items():
    result = apply(sql, 'lmm_core_owner')
    (out/(name+'.stderr.log')).write_text(result.stderr)
    assert result.returncode != 0, name+' unexpectedly accepted'
    assert query('SELECT count(*) FROM core_history.connection_snapshot').strip() == '0', name+' partial import'
result = apply(valid, 'lmm_core_owner')
(out/'import.stderr.log').write_text(result.stderr)
assert result.returncode == 0, result.stderr
before = query("SELECT document_utf8||mapping_utf8 FROM core_history.connection_snapshot")
result = apply(valid, 'lmm_core_owner')
(out/'replay.stderr.log').write_text(result.stderr)
assert result.returncode != 0, 'replay accepted'
assert before == query("SELECT document_utf8||mapping_utf8 FROM core_history.connection_snapshot"), 'replay changed source'
retained = json.loads(query("SELECT document->'counts' FROM core_history.connection_snapshot"))
assert retained == {'users':7,'tokens':7,'oauth_server_grants':3,'developer_access_requests':3}, retained
assert json.loads(query('SELECT document FROM core_history.connection_snapshot')) == json.loads(raw)
print('PASS: synthetic read-only export, exact retained-row readback, mapping/opening/archive/target/nonce/credential-floor rejections, atomic failure, strict replay rejection')

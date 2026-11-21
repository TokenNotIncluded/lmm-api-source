"""Actual Core HTTP checks for the isolated current identity-to-key seam."""
from datetime import datetime, timezone
from copy import deepcopy
import hashlib
import http.client
import json
from pathlib import Path
import re
import secrets
import socket
import subprocess
import time
from urllib.parse import urlencode


def run_probe(H, output, work, sock, db, port, runtime, env, sql):
    probe = json.loads((work / 'regression/runtime-seam-probes.private.json').read_text())
    H.need(probe['database'] == db and probe['expected_core78_manifest_sha256'] == H.MANIFEST_SHA,
           'current key runtime probes belong to another fixture')
    before = json.loads((work / 'regression/http-preservation-before.private.json').read_text())
    state_sql = (work / 'regression/http-preservation-readback.sql').read_text()
    H.need(json.loads(sql('BEGIN READ ONLY; ' + state_sql + ' COMMIT;')) == before,
           'state changed between the committed mechanics test and HTTP startup')
    sql(f'ALTER ROLE {runtime} LOGIN; GRANT CONNECT ON DATABASE {db} TO {runtime};', admin=True)
    listener = socket.socket()
    listener.bind(('127.0.0.1', 0))
    http_port = listener.getsockname()[1]
    listener.close()
    origin = 'http://127.0.0.1:' + str(http_port)
    runtime_url = f'postgresql://{runtime}@localhost:{port}/{db}?' + urlencode({'host': str(sock), 'sslmode': 'disable'})
    (work / 'runtime.url').write_text(runtime_url + '\n')
    (work / 'refresh.key').write_bytes(secrets.token_bytes(32))
    (work / 'api-key.key').write_bytes(bytes.fromhex(probe['encryption_key_hex']))
    (work / 'upstream.key').write_text('synthetic-invalid-provider-' + secrets.token_hex(16) + '\n')
    model = f'''version=1
input_token_ceiling=100
lease_seconds=20
timeout_seconds=3
max_output_tokens=20
credentials=[{{reference={{id=7,revision=1}},secret_file={json.dumps(str(work / 'upstream.key'))}}}]
key_groups=[]
external_models=[{{upstream_id=1,upstream_model='provider-model',protocol='openai_chat',endpoint='https://synthetic-current-key-seam.invalid/v1',max_input_tokens=100,max_output_tokens=20,contract_revision='synthetic-catalog-admission-v1'}}]
[routing]
version=1
groups=[{{name='paid',enabled=true}}]
upstreams=[{{id=1,endpoint='https://synthetic-current-key-seam.invalid/v1',protocol='openai_chat',credential={{id=7,revision=1}},enabled=true}}]
routes=[{{id=1,model='synthetic-model',group='paid',enabled=true,speeds=['standard'],targets=[{{id=1,upstream_id=1,upstream_model='provider-model',enabled=true,priority=0,weight=1,speeds=['standard'],payload='adapt',timeout_ms=5000}}],failover={{max_attempts=1,retry_on=[],allow_replay_after_send=false}}}}]
aliases=[]
[prices]
version=1
rules=[{{id=1,model='synthetic-model',group='paid',speed='standard',rates={{input=2000000,output=4000000,cache_read=500000,cache_write=0,request=0}},group_multiplier_ppm=1000000,speed_multiplier_ppm=1000000}}]
'''
    (work / 'model.toml').write_text(model)
    config = f'''version=1
[server]
listen="127.0.0.1:{http_port}"
worker_threads=1
drain_timeout_seconds=10
[database]
url_file={json.dumps(str(work / 'runtime.url'))}
max_connections=4
[console]
enabled=true
[model]
config_file={json.dumps(str(work / 'model.toml'))}
local_http=false
[identity]
api_key_encryption_key_file={json.dumps(str(work / 'api-key.key'))}
registration_enabled=false
password_register_enabled=false
oauth_register_enabled=false
registration_disabled_methods=[]
email_verification_enabled=false
email_domain_restriction_enabled=false
email_domain_whitelist=[]
[identity.web]
origin={json.dumps(origin)}
refresh_key_file={json.dumps(str(work / 'refresh.key'))}
local_http=true
user_agreement_enabled=false
privacy_policy_enabled=false
'''
    (work / 'runtime.toml').write_text(config)
    log_path = work / 'key-seam-runtime-core.log'
    log = log_path.open('xb')
    started = time.time()
    proc = subprocess.Popen([str(output / 'lmm-core'), '--config', str(work / 'runtime.toml'), 'serve'],
                            stdout=log, stderr=subprocess.STDOUT, env=env)
    lifecycle = {'status': 'STARTED', 'pid': proc.pid, 'production_access': False,
                 'started_utc': datetime.fromtimestamp(started, timezone.utc).isoformat(),
                 'forced_termination': False}
    events = []
    revealed = []
    primary_failed = False
    fixture_requests = []

    def request(method, path, bearer=None):
        H.need((method == 'GET' and path in ('/health/ready', '/core/v1/identity', '/v1/models'))
               or (method == 'GET' and re.fullmatch(r'/api/token/[1-9][0-9]*', path))
               or (method == 'POST' and re.fullmatch(r'/api/token/[1-9][0-9]*/key', path)),
               'key seam fixture attempted an endpoint outside its read/reveal scope')
        fixture_requests.append((method, path))
        headers = {'Origin': origin}
        if bearer is not None:
            headers['Authorization'] = 'Bearer ' + bearer
        client = http.client.HTTPConnection('127.0.0.1', http_port, timeout=5)
        try:
            client.request(method, path, headers=headers)
            response = client.getresponse()
            body = response.read(1024 * 1024)
            H.need(len(body) < 1024 * 1024, 'unexpected oversized native HTTP response')
            return response.status, {k.lower(): v for k, v in response.getheaders()}, json.loads(body)
        finally:
            client.close()

    try:
        for _ in range(200):
            H.need(proc.poll() is None, 'Core exited before key seam readiness')
            try:
                status, _, _ = request('GET', '/health/ready')
                if status == 200:
                    break
            except (OSError, http.client.HTTPException):
                pass
            time.sleep(0.1)
        else:
            raise RuntimeError('key seam readiness timeout')
        lifecycle['actual_core_sha256'] = H.sha(Path(f'/proc/{proc.pid}/exe'))
        H.need(lifecycle['actual_core_sha256'] == H.CORE_SHA, 'running key seam binary differs')
        original = probe['session']
        status, _, body = request('GET', '/core/v1/identity', original['bearer'])
        H.need(status == 200 and body['credential_id'] == original['credential_id']
               and body['user_id'] == probe['preexisting_user_id'], 'preexisting Session changed')
        events.append({'case': 'preserved_session', 'status': status})
        status, _, _ = request('GET', '/core/v1/identity', probe['expired_session']['bearer'])
        H.need(status == 401, 'old expired Session was re-enabled')
        events.append({'case': 'preserved_expired_session', 'status': status})
        sessions = {s['user_id']: s for s in probe['owner_sessions']}
        for session in sessions.values():
            status, _, body = request('GET', '/core/v1/identity', session['bearer'])
            H.need(status == 200 and body['credential_id'] == session['credential_id']
                   and body['user_id'] == session['user_id'], 'owner Session is invalid')
        for key in probe['keys']:
            expected = 200 if key['expected_identity_auth'] == 'allowed' else 401
            status, _, body = request('GET', '/core/v1/identity', key['bearer'])
            H.need(status == expected, 'current imported key identity status differs')
            if status == 200:
                H.need(body['credential_id'] == key['credential_id'] and body['user_id'] == key['user_id'],
                       'current key authenticated with another owner')
            events.append({'case': 'current_key_auth', 'credential_id': key['credential_id'],
                           'user_id': key['user_id'], 'account_id': key['account_id'], 'status': status})
            status, _, body = request('GET', '/v1/models', key['bearer'])
            model_expected = 401 if expected == 401 else 403 if key['expected_model_admission'] == 'denied' else 200
            H.need(status == model_expected, 'paid/dormant model admission differs')
            if model_expected == 200:
                H.need(body['object'] == 'list' and [m['id'] for m in body['data']] == ['synthetic-model'],
                       'paid key lost its exact supported model')
            events.append({'case': 'actual_model_admission', 'credential_id': key['credential_id'], 'status': status})
            session = sessions[key['user_id']]
            H.need(session['account_id'] == key['account_id'], 'recovery Session account mismatch')
            status, _, body = request('GET', '/api/token/' + str(key['credential_id']), session['bearer'])
            H.need(status == 200 and body.get('success') is True, 'owner native key projection failed')
            view = body['data']
            H.need(view['used_quota'] == key['used_units'] and view['remain_quota'] == key['remaining_units']
                   and view['unlimited_quota'] == key['unlimited'], 'current key historical quota changed')
            status, headers, body = request('POST', '/api/token/' + str(key['credential_id']) + '/key', session['bearer'])
            if key['recoverable']:
                H.need(status == 200 and body.get('success') is True and body['data']['key'] == key['bearer']
                       and 'no-store' in headers.get('cache-control', ''), 'native-ID recovery or cache policy changed')
                revealed.append({'actor_user_id': key['user_id'], 'credential_id': session['credential_id'],
                                 'action': 'dashboard.key.reveal', 'target': {'credential_ids': [key['credential_id']]}})
            else:
                H.need(status == 403, 'one-time imported key unexpectedly recoverable')
            events.append({'case': 'quota_and_recovery', 'credential_id': key['credential_id'],
                           'used_units': view['used_quota'], 'remaining_units': view['remain_quota'],
                           'unlimited': view['unlimited_quota'], 'recovery_status': status})
        heartbeat = json.loads(sql("SELECT jsonb_build_object('instances',(SELECT jsonb_agg(to_jsonb(i)) "
                                  "FROM core_operations.instances i),'now',extract(epoch from clock_timestamp()));"))
        H.need(len(heartbeat['instances'] or []) == 1, 'unexpected live Core heartbeat count')
        instance = heartbeat['instances'][0]
        H.need(re.fullmatch('[a-f0-9]{32}', instance['boot_id']) is not None
               and instance['instance_name'] == 'core-' + instance['boot_id'][:12]
               and instance['version'] == '0.1.0' and instance['phase'] == 'ready'
               and instance['models_ready'] is True and instance['rpc_ready'] is False,
               'heartbeat does not describe this exact model-enabled Core')
        # Timestamp text is parsed with an explicit timezone before comparing.
        for field in ('started_at', 'heartbeat_at'):
            observed = datetime.fromisoformat(instance[field]).timestamp()
            H.need(started <= observed <= heartbeat['now'], 'heartbeat lies outside runtime observation')
        lifecycle['heartbeat_rows_running'] = 1
        lifecycle['status'] = 'VERIFIED'
    except BaseException:
        primary_failed = True
        lifecycle['status'] = 'FAILED'
        raise
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                lifecycle['forced_termination'] = True
                proc.kill()
                proc.wait(timeout=10)
        log.close()
        lifecycle.update(core_exit=proc.returncode, core_log_sha256=H.sha(log_path),
                         finished_utc=datetime.now(timezone.utc).isoformat(),
                         fixture_requests=len(fixture_requests), fixture_model_execution_requests=0,
                         provider_execution_exercised=False)
        try:
            after = json.loads(sql('BEGIN READ ONLY; ' + state_sql + ' COMMIT;'))
            lifecycle['heartbeat_rows_after_stop'] = after['tables']['core_operations.instances']['rows']
            audits = json.loads(sql("SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY id),'[]'::jsonb) FROM core_identity.audit a;"))
            audit_sequence = 'core_identity.audit_id_seq'
            original_sequence = before['sequences'][audit_sequence]
            next_id = original_sequence['last_value'] + (original_sequence['increment'] if original_sequence['is_called'] else 0)
            expected_audit_ids = list(range(next_id, next_id + len(revealed)))
            actual_new = [row for row in audits if row['id'] in expected_audit_ids]
            preserved_audits = [row for row in audits if row['id'] not in expected_audit_ids]
            H.need((len(revealed) <= 1 if primary_failed else len(revealed) == 1)
                   and original_sequence['increment'] == 1 and original_sequence['cache'] == 1
                   and len(actual_new) == len(revealed) and preserved_audits == probe['before_audit'],
                   'native key recovery did not preserve old audit rows and append its one expected event')
            for row, fact, audit_id in zip(actual_new, revealed, expected_audit_ids):
                H.need(row['id'] == audit_id and all(row[k] == v for k, v in fact.items())
                       and set(row) == {'id', 'actor_user_id', 'credential_id', 'action', 'target', 'created_at'},
                       'reveal audit authority or target differs')
                created = datetime.fromisoformat(row['created_at']).timestamp()
                H.need(started <= created <= time.time(), 'reveal audit timestamp is outside this actual runtime')
            expected = deepcopy(before)
            # This one table is accepted only after the complete row-by-row check
            # above proves every original row plus the exact native reveal event.
            expected['tables']['core_identity.audit'] = after['tables']['core_identity.audit']
            if expected_audit_ids:
                expected['sequences'][audit_sequence].update(last_value=expected_audit_ids[-1], is_called=True)
            lifecycle['unrelated_application_state_unchanged'] = after == expected
            lifecycle['expected_reveal_audit_ids'] = expected_audit_ids
            lifecycle['old_audit_rows_preserved'] = True
            lifecycle['after_stop_state_sha256'] = hashlib.sha256(json.dumps(after, sort_keys=True).encode()).hexdigest()
            H.need(after == expected, 'key auth/admission/recovery changed rows or sequences beyond its exact reveal audit')
        except Exception as error:
            lifecycle['preservation_check_error'] = str(error)
            if not primary_failed:
                raise
        finally:
            H.save(work / 'key-seam-runtime-lifecycle.public.json', lifecycle)
        if not primary_failed:
            H.need(proc.returncode == 0 and not lifecycle['forced_termination'], 'key seam Core did not stop cleanly')
    result = {'status': 'PASS', 'events': events, 'actual_core_sha256': H.CORE_SHA,
              'unrelated_application_state_unchanged': True, 'expected_reveal_audit_appended': True,
              'fixture_model_execution_requests': 0, 'provider_execution_exercised': False,
              'configured_endpoint': 'reserved .invalid synthetic HTTPS provider', 'production_access': False,
              'production_authorized': False, 'production_closure_tested': False,
              'scope': 'current identity generation, dormant group, key auth/recovery/quota and read-only model admission'}
    H.save(work / 'key-seam-runtime-result.public.json', result)
    return result

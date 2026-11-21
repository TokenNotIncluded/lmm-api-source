"""Start the exact archived Core for synthetic quota redemption only."""
from datetime import datetime, timezone
import http.client
import json
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import time
from urllib.parse import urlencode


def run_probe(H, output, work, sock, db, port, runtime, env, sql, driver, testenv):
    mechanics = json.loads((work / 'regression/result.public.json').read_text())
    probe = json.loads((work / 'regression/runtime-probes.private.json').read_text())
    fixture = json.loads((work / 'fixture.private.json').read_text())
    H.need(mechanics['status'] == 'SYNTHETIC_PG_APPEND_VERIFIED'
           and mechanics['production_approval_created'] is False
           and mechanics['production_writes'] == 0
           and probe['scope'] == 'current-quota-redemption-synthetic-runtime'
           and probe['database'] == db and probe['fixture_nonce'] == fixture['fixture_nonce'],
           'quota runtime does not follow this isolated append')
    sql(f'ALTER ROLE {runtime} LOGIN; GRANT CONNECT ON DATABASE {db} TO {runtime};', admin=True)
    listener = socket.socket()
    listener.bind(('127.0.0.1', 0))
    http_port = listener.getsockname()[1]
    listener.close()
    origin = 'http://127.0.0.1:' + str(http_port)
    url = f'postgresql://{runtime}@localhost:{port}/{db}?' + urlencode({'host': str(sock), 'sslmode': 'disable'})
    (work / 'runtime.url').write_text(url + '\n')
    (work / 'refresh.key').write_bytes(secrets.token_bytes(32))
    (work / 'api-key.key').write_bytes(secrets.token_bytes(32))
    # No model or payment provider is configured. The only HTTP mutations are
    # synthetic native promotion claims against this disposable local database.
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
    log_path = work / 'quota-runtime-core.log'
    log = log_path.open('xb')
    proc = None
    lifecycle = {'status': 'STARTING', 'production_access': False,
                 'forced_termination': False, 'provider_configuration_present': False,
                 'started_utc': datetime.now(timezone.utc).isoformat()}
    try:
        proc = subprocess.Popen([str(output / 'lmm-core'), '--config', str(work / 'runtime.toml'), 'serve'],
                                stdout=log, stderr=subprocess.STDOUT, env=env)
        lifecycle.update(status='STARTED', pid=proc.pid)
        for _ in range(200):
            H.need(proc.poll() is None, 'Core exited before quota readiness')
            conn = http.client.HTTPConnection('127.0.0.1', http_port, timeout=2)
            try:
                conn.request('GET', '/health/ready', headers={'Origin': origin})
                response = conn.getresponse()
                response.read(65536)
                if response.status == 200:
                    break
            except (OSError, http.client.HTTPException):
                pass
            finally:
                conn.close()
            time.sleep(0.1)
        else:
            raise RuntimeError('quota runtime readiness timeout')
        lifecycle['actual_core_sha256'] = H.sha(Path(f'/proc/{proc.pid}/exe'))
        H.need(lifecycle['actual_core_sha256'] == H.CORE_SHA, 'running quota Core differs')
        H.run([sys.executable, '-B', driver, 'runtime', '--fixture', work / 'fixture.private.json',
               '--probes', work / 'regression/runtime-probes.private.json', '--base-url', origin,
               '--output', work / 'quota-runtime'], work, 'quota-http-regression', env=testenv, timeout=180)
        verified = json.loads((work / 'quota-runtime/result.public.json').read_text())
        H.need(verified['status'] == 'SYNTHETIC_PG_HTTP_QUOTA_REDEMPTION_VERIFIED'
               and verified['production_writes'] == 0
               and verified['production_approval_created'] is False
               and verified['product_rebuilt'] is False, 'quota HTTP scope differs')
        lifecycle['status'] = 'VERIFIED'
        return verified
    finally:
        if proc is not None and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                lifecycle['forced_termination'] = True
                proc.kill()
                proc.wait(timeout=10)
        log.close()
        lifecycle.update(core_exit=None if proc is None else proc.returncode,
                         core_process_stopped=proc is not None and proc.poll() is not None,
                         core_log_sha256=H.sha(log_path), finished_utc=datetime.now(timezone.utc).isoformat())
        if proc is not None and proc.returncode == 0:
            heartbeat = json.loads(sql('BEGIN READ ONLY; SELECT to_jsonb(count(*)) FROM core_operations.instances; COMMIT;'))
            lifecycle['heartbeat_rows_after_stop'] = heartbeat
        H.save(work / 'quota-runtime-lifecycle.public.json', lifecycle)
        H.need(lifecycle['core_exit'] == 0 and lifecycle['forced_termination'] is False
               and lifecycle.get('heartbeat_rows_after_stop') == 0, 'quota Core did not stop cleanly')

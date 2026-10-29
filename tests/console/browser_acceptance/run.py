#!/usr/bin/env python3
"""Frozen HEAD browser acceptance. Local fixtures only; never edits product code."""
import argparse
import fcntl
import hashlib
import http.client
import http.server
import importlib.util
import json
import os
import re
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
from unittest.mock import patch


CORE_SHA = '6eca9a6e8b270bc7368057469dbd8185ad287db74589df3f3a223f610a53ec31'
NOTICE_PATH = 'apps/web/src/features/system-settings/maintenance/notice-section.tsx'
CASES = ['freeze', 'dependencies', 'build', 'fixture', 'browser']


def sha(path):
    with Path(path).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def save(path, data):
    Path(path).write_text(json.dumps(data, indent=2, ensure_ascii=False) + '\n')


def pins(root):
    return {str(p.relative_to(root)): sha(p) for p in sorted(root.rglob('*'))
            if p.is_file() and 'node_modules' not in p.parts and 'dist' not in p.parts}


def require(test, message):
    if not test:
        raise RuntimeError(message)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--repo', type=Path, required=True)
    p.add_argument('--commit', default='7c3ebb86cf406c5f0a268ea2eb90edb985271751')
    p.add_argument('--core', type=Path, required=True)
    p.add_argument('--root', type=Path, required=True)
    p.add_argument('--playwright', type=Path, required=True, help='Existing playwright/index.mjs')
    p.add_argument('--chromium', type=Path, default=Path('/usr/bin/chromium'))
    p.add_argument('--reuse-root', type=Path, help='Reuse verified frozen source/dist and clone its stopped private PG; no install/build/init')
    p.add_argument('--diagnose-selector', action='store_true', help='Record exact original selector hits only; no business PASS')
    p.add_argument('--ordinary-only', action='store_true', help='Independent ordinary-user login/route check; no settings writes')
    p.add_argument('--notice-overlay', type=Path, help='HEAD plus exactly the owner-provided synchronous Notice mutate fix; one private Web build')
    p.add_argument('--bun', type=Path, default=Path('/home/lightjunction/.local/share/mise/installs/bun/1.3.14/bin/bun'))
    args = p.parse_args()
    require(not args.notice_overlay or (args.reuse_root and not args.ordinary_only and not args.diagnose_selector), 'notice_retest_requires_reuse')
    require(not args.ordinary_only or (args.reuse_root and not args.diagnose_selector), 'ordinary_only_requires_reuse_and_no_diagnosis')
    require(not args.diagnose_selector or args.reuse_root, 'diagnosis_requires_reused_fixture')
    os.umask(0o077)
    root = args.root.absolute()
    require(root.parent.resolve() == Path('/home/lightjunction/Documents/Codex'), 'private Codex root required')
    require(len(os.fsencode(root / 'tmp')) <= 55, 'short_private_root_required_for_Chromium_Unix_socket')
    root.mkdir(mode=0o700, exist_ok=False)
    env = {'PATH': '/home/lightjunction/.bun/bin:/usr/bin:/bin', 'HOME': str(Path.home()),
           'LANG': 'C.UTF-8', 'TZ': 'UTC', 'NODE_OPTIONS': '--max-old-space-size=1152',
           'GOMAXPROCS': '1', 'RAYON_NUM_THREADS': '1', 'UV_THREADPOOL_SIZE': '1', 'CI': '1'}
    (root / 'tmp').mkdir()
    env['TMPDIR'] = str(root / 'tmp')
    results = dict.fromkeys(CASES, 'NOT_RUN')
    commands = []
    development = {}
    fixture = server = None
    lock = None
    case = 'freeze'
    cgroup = Path('/sys/fs/cgroup') / Path('/proc/self/cgroup').read_text().strip().split('::')[1].lstrip('/')

    def command(label, argv, cwd=None, timeout=60):
        print('START', label, flush=True)
        with (root / (label + '.log')).open('wb') as log:
            result = subprocess.run(list(map(str, argv)), cwd=cwd or root, env=env,
                                    stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
        commands.append({'label': label, 'argv': list(map(str, argv)), 'exit': result.returncode})
        save(root / 'commands.json', commands)
        require(result.returncode == 0, label + '_exit_' + str(result.returncode))

    try:
        lock = open('/tmp/lmm-delegate/heavy-build.lock', 'a')
        deadline = time.monotonic() + 1800
        while True:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                require(time.monotonic() < deadline, 'shared_lock_timeout')
                print('WAIT shared heavy-build lock', flush=True)
                time.sleep(15)
        available = int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines()
                             if x.startswith('MemAvailable:')))
        limits = {n: (cgroup / n).read_text().strip() for n in
                  ['memory.max', 'memory.high', 'memory.swap.max', 'cpu.max', 'pids.max']}
        save(root / 'admission.json', {'available_kib': available, 'limits': limits})
        require(available >= 2097152, '2GiB_admission_closed')
        for key, ceiling in [('memory.max', 2**31), ('memory.high', 1536*1024**2),
                             ('memory.swap.max', 4*1024**3), ('pids.max', 256)]:
            require(limits[key] != 'max' and int(limits[key]) <= ceiling, 'resource_limit_' + key)
        quota, period = limits['cpu.max'].split()
        require(quota != 'max' and int(quota) <= int(period), 'CPU1_required')
        require(sha(args.core) == CORE_SHA, 'actual_Core_SHA_mismatch')
        tests = root / 'runner'
        if args.reuse_root:
            previous = args.reuse_root.resolve(strict=True)
            require(previous.parent == root.parent and previous != root, 'private_reuse_root_required')
            freeze = json.loads((previous / 'freeze.json').read_text())
            require(freeze['commit'] == args.commit and freeze['core_sha256'] == CORE_SHA, 'reuse_binding_mismatch')
            frozen = previous / 'source'
            dist = frozen / 'apps/web/dist'
            source_pins = json.loads((previous / 'source-pins.json').read_text())
            artifacts = json.loads((previous / 'artifact-pins.json').read_text())
            require(all(sha(frozen / k) == v for k, v in source_pins.items()), 'reused_source_drift')
            require(all(sha(dist / k) == v for k, v in artifacts.items()), 'reused_bundle_drift')
            require({str(x.relative_to(dist)) for x in dist.rglob('*') if x.is_file()} == set(artifacts), 'reused_bundle_extra_files')
            require(args.playwright.is_file() and args.chromium.is_file(), 'existing_browser_tools_missing')
            for name in ['freeze.json', 'source-pins.json', 'artifact-pins.json']:
                shutil.copy2(previous / name, root / name)
            save(root / 'reuse-binding.json', {'from': str(previous), 'source': str(frozen), 'dist': str(dist),
                'source_files_verified': len(source_pins), 'artifacts_verified': len(artifacts),
                'source_manifest_sha256': sha(previous / 'source-pins.json'),
                'artifact_manifest_sha256': sha(previous / 'artifact-pins.json'),
                'no_install_no_build_no_init_no_registration': True})
            tests.mkdir()
            for name in ['run.py', 'browser.mjs', 'README.md']:
                shutil.copy2(Path(__file__).parent / name, tests / name)
            save(root / 'runner-pins.json', pins(tests))
            results.update(freeze='PASS', dependencies='REUSED', build='REUSED')
        else:
            require(subprocess.check_output(['git', '-C', str(args.repo), 'rev-parse', 'HEAD']).decode().strip() == args.commit,
                    'HEAD_changed_review_new_commit_first')
            command('git-status', ['git', '-C', args.repo, 'status', '--short'])
            frozen = root / 'source'
            frozen.mkdir()
            archive = root / 'HEAD.tar'
            command('git-archive', ['git', '-C', args.repo, 'archive', '--format=tar', '-o', archive, args.commit])
            with tarfile.open(archive) as tar:
                tar.extractall(frozen, filter='data')
            source_pins = pins(frozen)
            save(root / 'source-pins.json', source_pins)
            save(root / 'freeze.json', {'commit': args.commit, 'git_archive_sha256': sha(archive),
                 'source_files': len(source_pins), 'core_path': str(args.core), 'core_sha256': CORE_SHA,
                 'core_provenance': 'User supplied actual Core; delivery-native-binding.json associates it with 7c3ebb86; not a release-profile claim',
                 'working_tree_changes_included': False})
            tests = root / 'runner'
            tests.mkdir()
            for file in Path(__file__).parent.iterdir():
                if file.is_file():
                    shutil.copy2(file, tests / file.name)
            save(root / 'runner-pins.json', pins(tests))
            results[case] = 'PASS'

            case = 'dependencies'
            require(args.playwright.is_file() and args.chromium.is_file(), 'existing_browser_tools_missing')
            save(root / 'dependency-preflight.json', {
                'repository_rsbuild_present': (args.repo / 'apps/web/node_modules/@rsbuild/core/package.json').exists(),
                'policy': 'one frozen-lockfile install, ignore scripts, one network request at a time, 240 second bound; no repo install',
                'playwright_entry_sha256': sha(args.playwright), 'chromium_executable_sha256': sha(args.chromium),
                'bun_lock_sha256': sha(frozen / 'bun.lock')})
            command('dependencies', ['bun', 'install', '--frozen-lockfile', '--ignore-scripts', '--network-concurrency=1'], frozen, 240)
            results[case] = 'PASS'

            case = 'build'
            env['VITE_REACT_APP_VERSION'] = args.commit[:12]
            command('build-web', ['bun', 'run', 'build:web'], frozen, 600)
            command('bundle-check', ['bun', 'run', '--filter', '@lmm/web', 'bundle:check'], frozen, 60)
            web = frozen / 'apps/web'
            dist = web / 'dist'
            require((dist / 'index.html').is_file(), 'static_index_missing')
            save(root / 'artifact-pins.json', {str(x.relative_to(dist)): sha(x) for x in sorted(dist.rglob('*')) if x.is_file()})
            # Router generators may rewrite their generated file; record every difference.
            save(root / 'postbuild-source-diff.json', {k: sha(frozen / k) for k, v in source_pins.items()
                                                     if sha(frozen / k) != v})
            results[case] = 'PASS'

        if args.notice_overlay:
            case = 'freeze'
            require(args.bun.is_file(), 'pinned_Bun_missing_no_install')
            env['PATH'] = str(args.bun.parent) + ':' + env['PATH']
            command('bun-version', [args.bun, '--version'])
            command('bun-revision', [args.bun, '--revision'])
            require((root / 'bun-version.log').read_text().strip() == '1.3.14', 'pinned_Bun_version_mismatch_no_fallback')
            old_source = frozen
            original_notice = (old_source / NOTICE_PATH).read_bytes()
            fixed_notice = args.notice_overlay.read_bytes()
            expected_notice = original_notice.replace(b'const onSubmit = async (values: NoticeFormValues)', b'const onSubmit = (values: NoticeFormValues)').replace(b'await updateOption.mutateAsync({', b'updateOption.mutate({')
            require(fixed_notice == expected_notice and fixed_notice != original_notice, 'overlay_must_be_exact_owner_two_line_fix')
            frozen = root / 'source'
            frozen.mkdir()
            archive = root / 'HEAD.tar'
            command('git-archive', ['git', '-C', args.repo, 'archive', '--format=tar', '-o', archive, args.commit])
            with tarfile.open(archive) as tar:
                tar.extractall(frozen, filter='data')
            require(pins(frozen) == source_pins, 'base_HEAD_does_not_match_reused_source')
            (frozen / NOTICE_PATH).write_bytes(fixed_notice)
            (root / 'inputs').mkdir()
            (root / 'inputs/notice-section.tsx').write_bytes(fixed_notice)
            source_pins = pins(frozen)
            save(root / 'source-pins.json', source_pins)
            save(root / 'overlay-binding.json', {'base_commit': args.commit, 'file': NOTICE_PATH, 'source': str(frozen), 'dist': str(frozen / 'apps/web/dist'),
                 'base_sha256': hashlib.sha256(original_notice).hexdigest(), 'overlay_sha256': sha(frozen / NOTICE_PATH),
                 'source_manifest_sha256': sha(root / 'source-pins.json'), 'other_source_changes': [],
                 'bun_path': str(args.bun), 'bun_sha256': sha(args.bun), 'bun_version': (root / 'bun-version.log').read_text().strip(),
                 'prior_dependencies_installed_by': (previous / 'dependencies.log').read_text().splitlines()[0],
                 'bun_lock_sha256': sha(frozen / 'bun.lock'), 'core_source_unchanged': True})
            save(root / 'freeze.json', {**freeze, 'git_archive_sha256': sha(archive), 'overlay': {NOTICE_PATH: sha(frozen / NOTICE_PATH)},
                 'working_tree_changes_included': [NOTICE_PATH], 'other_working_tree_changes_included': False})
            command('reuse-dependencies', ['cp', '-a', '--reflink=auto', old_source / 'node_modules', frozen / 'node_modules'], timeout=240)
            case = 'build'
            env['VITE_REACT_APP_VERSION'] = args.commit[:12]
            command('build-web-once', [args.bun, 'run', 'build:web'], frozen, 600)
            command('bundle-check', [args.bun, 'run', '--filter', '@lmm/web', 'bundle:check'], frozen, 60)
            dist = frozen / 'apps/web/dist'
            artifacts = {str(x.relative_to(dist)): sha(x) for x in sorted(dist.rglob('*')) if x.is_file()}
            save(root / 'artifact-pins.json', artifacts)
            save(root / 'postbuild-source-diff.json', {k: sha(frozen / k) for k, v in source_pins.items() if sha(frozen / k) != v})
            results['build'] = 'PASS'
            web = frozen / 'apps/web'
            for label, argv in [
                ('typecheck-web', [args.bun, 'run', 'typecheck']),
                ('lint-notice', [frozen / 'node_modules/.bin/oxlint', '-c', '.oxlintrc.json', 'src/features/system-settings/maintenance/notice-section.tsx'])]:
                try:
                    command(label, argv, web, 600)
                    development[label] = 'PASS'
                except (RuntimeError, subprocess.TimeoutExpired) as error:
                    development[label] = 'FAIL'
                    save(root / (label + '-failure.json'), {'type': type(error).__name__, 'message': str(error)})
            # Match the repository formatter's protected-header contract without
            # rewriting the frozen component or running format across other files.
            label = 'format-notice'
            body = re.sub(rb'^/\*\nCopyright \(C\)[\s\S]*?QuantumNous[\s\S]*?\*/\n+', b'', fixed_notice, count=1)
            argv = [str(frozen / 'node_modules/.bin/oxfmt'), '-c', '.oxfmtrc.json', '--stdin-filepath', 'src/features/system-settings/maintenance/notice-section.tsx']
            formatted = subprocess.run(argv, cwd=web, env=env, input=body, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
            (root / (label + '.log')).write_bytes(formatted.stderr)
            (root / (label + '-output.txt')).write_bytes(formatted.stdout)
            commands.append({'label': label, 'argv': argv, 'exit': formatted.returncode, 'stdin': 'frozen Notice minus protected attribution header', 'unchanged': formatted.stdout == body})
            save(root / 'commands.json', commands)
            development[label] = 'PASS' if formatted.returncode == 0 and formatted.stdout == body else 'FAIL'
            save(root / 'development-checks.json', {'checks': development, 'overall': 'PASS' if all(v == 'PASS' for v in development.values()) else 'FAIL',
                 'scope': 'developer checks on same frozen Web; separate from browser acceptance'})
            require(all(sha(frozen / k) == v for k, v in source_pins.items()), 'frozen_source_changed_during_checks')

        case = 'fixture'
        sys.path.insert(0, str(frozen / 'tests/console/independent_process'))
        spec = importlib.util.spec_from_file_location('independent_fixture', frozen / 'tests/console/independent_process/run.py')
        base = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(base)
        class Fixture(base.Runner):
            def start_core(self, label):
                # Serve static files + transparent API proxy on one browser origin.
                self.config.write_text(self.config.read_text().replace(
                    f'origin="http://127.0.0.1:{self.port}"', f'origin="http://127.0.0.1:{self.webport}"'))
                super().start_core(label)

        fargs = argparse.Namespace(source=frozen, core=args.core, evidence=root / 'fixture-evidence',
                                   pins=root / 'source-pins.json', freeze=root / 'freeze.json', core_sha256=CORE_SHA)
        fixture = Fixture(fargs)
        fixture.env.update({'TMPDIR': str(root / 'tmp')})
        fixture.webport = base.free_port()
        if args.reuse_root:
            original_work = Path(json.loads((previous / 'fixture-evidence/fixture-location.json').read_text())['work'])
            require(original_work.resolve().is_relative_to(previous), 'private_original_PG_required')
            require(not (original_work / 'data/postmaster.pid').exists(), 'original_PG_must_be_stopped')
            require(not any(x.is_symlink() for x in (original_work / 'data').rglob('*')), 'PG_external_paths_forbidden')
            command('original-pg-control', ['pg_controldata', original_work / 'data'])
            require('shut down' in (root / 'original-pg-control.log').read_text(), 'PG_clean_shutdown_required')
            original_pins = pins(original_work)
            save(root / 'original-fixture-pins.json', original_pins)
            fixture.work = root / 'pg-fixture'
            fixture.sockdir = root / 'pg-socket'
            fixture.sockdir.mkdir(mode=0o700)
            command('clone-stopped-pg', ['cp', '-a', '--reflink=auto', original_work, fixture.work])
            require(pins(fixture.work) == original_pins, 'PG_clone_mismatch')
            fixture.save('fixture-location.json', {'work': str(fixture.work), 'socket': str(fixture.sockdir), 'cloned_from': str(original_work)})
            fixture.pg = base.Process(fixture, 'postgres', ['postgres', '-D', fixture.work / 'data',
                '-c', 'listen_addresses=', '-c', f'unix_socket_directories={fixture.sockdir}',
                '-c', 'unix_socket_permissions=0700', '-c', 'shared_buffers=16MB', '-c', 'max_connections=24',
                '-c', 'min_wal_size=2MB', '-c', 'max_wal_size=128MB', '-c', 'log_statement=none',
                '-c', 'log_min_error_statement=panic', '-c', 'log_parameter_max_length_on_error=0'])
            for _ in range(100):
                if (fixture.sockdir / '.s.PGSQL.5432').exists():
                    if subprocess.run(['pg_isready', '-q', '-h', str(fixture.sockdir), '-U', 'fixture_admin'], env=env).returncode == 0:
                        break
                require(fixture.pg.proc.poll() is None, 'reused_PG_exited')
                time.sleep(.1)
            else:
                raise RuntimeError('reused_PG_readiness_timeout')
            identity = json.loads(fixture.sql('pg_identity', "SELECT json_build_object('system_id',system_identifier::text,'data_directory',current_setting('data_directory'),'database',current_database(),'listen',current_setting('listen_addresses')) FROM pg_control_system()"))
            original_identity = json.loads((previous / 'fixture-evidence/postgres-identity.json').read_text())
            require(identity['system_id'] == original_identity['system_id'] and identity['listen'] == '', 'PG_identity_mismatch')
            require(Path(identity['data_directory']).resolve() == (fixture.work / 'data').resolve(), 'PG_clone_directory_mismatch')
            fixture.save('postgres-identity.json', identity)
            fixture.proxy = base.Proxy(fixture.sockdir / '.s.PGSQL.5432', 'console_runtime', 'console_acceptance')
            fixture.file('runtime.url', f'postgresql://console_runtime@127.0.0.1:{fixture.proxy.server_address[1]}/console_acceptance?sslmode=disable')
            fixture.port = base.free_port()
            fixture.config = fixture.work / 'core.toml'
            config_text = fixture.config.read_text().replace(str(original_work), str(fixture.work))
            config_text = re.sub(r'listen="127.0.0.1:[0-9]+"', f'listen="127.0.0.1:{fixture.port}"', config_text)
            config_text = re.sub(r'origin="http://127.0.0.1:[0-9]+"', f'origin="http://127.0.0.1:{fixture.webport}"', config_text)
            fixture.config.write_text(config_text)
            credentials = json.loads((previous / 'browser-private.json').read_text())
            fixture.password = fixture.secret(credentials['adminPassword'])
            fixture.user_password = fixture.secret(credentials['ordinaryPassword'])
            fixture.start_core('reused_formal_main')
        else:
            original_mkdtemp = tempfile.mkdtemp
            def private_temp(*pos, **kw):
                kw['dir'] = root
                return original_mkdtemp(*pos, **kw)
            with patch.object(base.tempfile, 'mkdtemp', private_temp):
                fixture.fresh_cli_and_runtime()
            fixture.user_password = fixture.secret('Us!' + secrets.token_hex(8))
            registration = http.client.HTTPConnection('127.0.0.1', fixture.port, timeout=15)
            registration.request('POST', '/api/user/register', json.dumps(
                {'username': 'consoleordinary', 'password': fixture.user_password, 'accepted_legal': True}),
                {'Content-Type': 'application/json', 'Origin': f'http://127.0.0.1:{fixture.webport}'})
            registration_response = registration.getresponse()
            body = json.loads(registration_response.read())
            save(root / 'ordinary-fixture-registration.json', {'status': registration_response.status, 'response': body})
            require(registration_response.status == 200 and body.get('success') is True, 'ordinary_fixture_registration')
            registration.close()
        control_secret = secrets.token_hex(24)
        requests = []

        class Handler(http.server.SimpleHTTPRequestHandler):
            def __init__(self, *a, **kw):
                super().__init__(*a, directory=str(dist), **kw)

            def log_message(self, *_):
                pass

            def do_GET(self):
                if self.path.startswith(('/api/', '/core/', '/health/')):
                    self.proxy()
                else:
                    if not Path(self.translate_path(self.path)).is_file():
                        self.path = '/index.html'
                    super().do_GET()

            def do_POST(self):
                if self.path == '/__fixture':
                    require(self.headers.get('X-Fixture-Key') == control_secret, 'fixture_control_key')
                    action = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))))['action']
                    if action == 'snapshot':
                        data = fixture.snapshot()
                    elif action == 'revoke-audit':
                        fixture.sql('revoke_audit', 'SET ROLE lmm_core_owner; REVOKE INSERT ON core_identity.audit FROM console_runtime')
                        data = {'ok': True}
                    elif action == 'restore-grants':
                        fixture.grants()
                        data = {'ok': True}
                    elif action == 'arm-commit-loss':
                        fixture.proxy.arm()
                        data = {'ok': True}
                    elif action == 'fault-evidence':
                        data = {'events': fixture.proxy.events, 'errors': fixture.proxy.errors,
                                'dropped': fixture.proxy.dropped.is_set()}
                    else:
                        raise RuntimeError('unknown_fixture_action')
                    raw = json.dumps(data).encode()
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(raw)))
                    self.end_headers()
                    self.wfile.write(raw)
                else:
                    self.proxy()

            def do_PUT(self):
                self.proxy()

            def do_DELETE(self):
                self.proxy()

            def proxy(self):
                payload = self.rfile.read(int(self.headers.get('Content-Length', '0')))
                headers = {k: v for k, v in self.headers.items() if k.lower() not in ('host', 'connection', 'accept-encoding')}
                conn = http.client.HTTPConnection('127.0.0.1', fixture.port, timeout=20)
                conn.request(self.command, self.path, payload, headers)
                response = conn.getresponse()
                raw = response.read()
                record = {'method': self.command, 'path': self.path.split('?')[0], 'status': response.status}
                if self.path.startswith('/api/option'):
                    record['request'] = json.loads(payload) if payload else None
                    record['response'] = json.loads(raw)
                else:
                    try:
                        record['response_keys'] = sorted(json.loads(raw))
                    except (ValueError, TypeError):
                        record['response_bytes'] = len(raw)
                requests.append(record)
                save(root / 'requests.json', requests)
                self.send_response(response.status)
                for k, v in response.getheaders():
                    if k.lower() not in ('transfer-encoding', 'connection', 'content-length'):
                        self.send_header(k, v)
                self.send_header('Content-Length', str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)
                conn.close()

        server = http.server.ThreadingHTTPServer(('127.0.0.1', fixture.webport), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        save(root / 'local-binding.json', {'origin': f'http://127.0.0.1:{fixture.webport}',
             'core_pid': fixture.main.proc.pid, 'running_core_sha256': sha(f'/proc/{fixture.main.proc.pid}/exe'),
             'static_dist': str(dist)})
        results[case] = 'PASS'

        case = 'browser'
        config = root / 'browser-private.json'
        save(config, {'origin': f'http://127.0.0.1:{fixture.webport}', 'root': str(root),
             'playwright': str(args.playwright), 'chromium': str(args.chromium),
             'adminPassword': fixture.password, 'ordinaryPassword': fixture.user_password,
             'fixtureKey': control_secret, 'resume': bool(args.reuse_root), 'diagnoseSelector': args.diagnose_selector, 'ordinaryOnly': args.ordinary_only, 'noticeRetest': bool(args.notice_overlay),
             'reuseRoot': str(args.reuse_root) if args.reuse_root else None,
             'priorCases': json.loads((previous / 'browser-result.json').read_text())['cases'] if args.reuse_root else {},
             'selectorDiagnosis': '/home/lightjunction/Documents/Codex/cb1030a7/selector-hits.json',
             'priorReadonlyCases': json.loads((root.parent / 'cb1030a8/browser-result.json').read_text())['cases'] if args.notice_overlay else {},
             'priorOrdinaryCases': json.loads((root.parent / 'cb1030a9/browser-result.json').read_text())['cases'] if args.notice_overlay else {}})
        command('browser', ['/usr/bin/node', tests / 'browser.mjs', config], timeout=300)
        require(json.loads((root / 'browser-result.json').read_text())['overall'] == ('INDEPENDENT_CASE_PASS' if args.ordinary_only else 'DIAGNOSTIC_ONLY' if args.diagnose_selector else 'PASS'), 'browser_incomplete')
        if args.reuse_root:
            require(pins(original_work) == original_pins, 'original_failure_PG_changed')
            require(all(sha(dist / k) == v for k, v in artifacts.items()), 'bundle_changed')
        require(sha(args.core) == CORE_SHA, 'Core_drift')
        results[case] = 'PASS'
    except Exception as error:
        results[case] = 'FAIL'
        save(root / 'failure.json', {'phase': case, 'type': type(error).__name__, 'message': str(error)})
        print('FAIL', case, str(error), flush=True)
    finally:
        if server:
            server.shutdown()
            server.server_close()
        if fixture:
            # Keep fixture evidence truthful: API-only case matrix was not run.
            fixture.finish()
        ok = all(value in ('PASS', 'REUSED') for value in results.values())
        cleanup_ok = ok and all(v == 'PASS' for v in development.values())
        if cleanup_ok:
            for item in [root / 'source', root / 'HEAD.tar', root / 'tmp', root / 'browser-private.json',
                         fixture.work, fixture.sockdir]:
                if item.is_dir():
                    shutil.rmtree(item)
                elif item.exists():
                    item.unlink()
        save(root / 'results.json', {'overall': ('INDEPENDENT_CASE_PASS' if args.ordinary_only else 'DIAGNOSTIC_ONLY' if args.diagnose_selector else 'PASS') if ok else 'INCOMPLETE', 'phases': results,
             'temporary_inputs_removed': cleanup_ok, 'development_checks': development, 'scope': 'Notice/announcements actual browser only; no project-wide signoff'})
        save(root / 'resource-final.json', {n: (cgroup / n).read_text().strip() for n in
             ['memory.peak', 'memory.events', 'cpu.stat', 'pids.peak', 'pids.events'] if (cgroup / n).exists()})
        if lock:
            lock.close()
    return 0 if ok else 1


if __name__ == '__main__':
    raise SystemExit(main())

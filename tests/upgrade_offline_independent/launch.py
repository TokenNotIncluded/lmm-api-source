#!/usr/bin/env python3
"""Run the independent r3 acceptance under the shared lock and resource slice."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import time

TEST = Path(__file__).resolve().parent
REPO = TEST.parents[1]
LOCK = Path('/tmp/lmm-delegate/heavy-build.lock')
CORE_SHA = '2d982f5368f984241cc26bcbf60c38ddf56cf243c2400387a811b6e0cd4ef9f8'
PREVIOUS_SHA = '6eca9a6e8b270bc7368057469dbd8185ad287db74589df3f3a223f610a53ec31'
PINS_SHA = '20464fa3e5b34566f43f7ca95a2464530d827b2de118475683fd1915f99e470b'
MANIFEST_SHA = '47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078'
CATALOG_SHA = '557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d'
FAULT_SHA = '3798afdab928a579bbf57b08bce1c87703e41a0b56af0ea6860ef528653f4a6c'


def sha(path):
    with Path(path).open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('core', 'previous-core', 'source-pins', 'source', 'reference', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    for name in ('core', 'previous_core', 'source_pins', 'source', 'reference'):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    root = args.output.resolve()
    if root.is_relative_to(REPO) or root.exists():
        raise SystemExit('output must be a new directory outside the source repository')
    if not LOCK.is_file():
        raise SystemExit('shared resource lock is missing')

    actual = {'core': sha(args.core), 'previous_core': sha(args.previous_core),
              'source_pins_sha256': sha(args.source_pins),
              'pg_fault_sha256': sha(REPO / 'tests/console/independent_process/pg_fault.py')}
    if actual != {'core': CORE_SHA, 'previous_core': PREVIOUS_SHA,
                  'source_pins_sha256': PINS_SHA, 'pg_fault_sha256': FAULT_SHA}:
        raise SystemExit('frozen binary, source pins, or fault helper hash mismatch')
    pins = json.loads(args.source_pins.read_text())
    bad = [name for name, expected in pins.items()
           if not (args.source / name).is_file() or sha(args.source / name) != expected]
    if bad or len(pins) != 1308:
        raise SystemExit('source pin verification failed: ' + json.dumps(bad[:8]))
    if json.loads(args.reference.read_text())['target_catalog_sha256'] != CATALOG_SHA:
        raise SystemExit('reference catalog mismatch')
    manifest = subprocess.run([str(args.core), 'schema-manifest'], capture_output=True, check=True)
    if hashlib.sha256(manifest.stdout).hexdigest() != MANIFEST_SHA:
        raise SystemExit('binary schema manifest mismatch')
    root.mkdir(mode=0o700)
    (root / 'frozen-inputs.json').write_text(json.dumps({**actual, 'source_files': len(pins),
        'source_mismatches': bad, 'schema_manifest_sha256': MANIFEST_SHA,
        'reference_catalog_sha256': CATALOG_SHA}, indent=2) + '\n')

    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('PG', 'LMM_', 'PAYMENT_', 'SQLX_'))
           and k not in ('DATABASE_URL', 'SQL_DSN')}
    env.update(XDG_RUNTIME_DIR=f'/run/user/{os.getuid()}',
               DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{os.getuid()}/bus')
    token = secrets.token_hex(6)
    slice_name = 'lmmv3accept_' + token + '.slice'
    scope = 'lmm-v3-acceptance-' + token + '.scope'
    env['INDEPENDENT_SLICE'] = slice_name
    unitdir = Path(env['XDG_RUNTIME_DIR']) / 'systemd/user'
    unitdir.mkdir(parents=True, exist_ok=True)
    unit = unitdir / slice_name
    if unit.exists():
        raise SystemExit('temporary slice name collision')
    unit.write_text('[Unit]\nDescription=Disposable aggregate offline acceptance slice\n'
                    '[Slice]\nMemoryAccounting=yes\nMemoryHigh=1536M\nMemoryMax=2G\n'
                    'MemorySwapMax=4G\nCPUAccounting=yes\nCPUQuota=100%\n')
    result = {'unit': str(unit), 'slice': slice_name, 'scope': scope,
              'started': time.time(), 'test_exit': None, 'acl_exit': None}
    try:
        subprocess.run(['systemctl', '--user', 'daemon-reload'], env=env, check=True, timeout=30)
        argv = ['systemd-run', '--user', '--scope', '--quiet', '--unit=' + scope,
                '--slice=' + slice_name, '--property=MemoryHigh=1536M', '--property=MemoryMax=2G',
                '--property=MemorySwapMax=4G', '--property=CPUQuota=100%', '/usr/bin/flock',
                '--wait=1800', str(LOCK), '/usr/bin/python3', '-B', str(TEST / 'run.py'),
                '--core', str(args.core), '--core-sha256', CORE_SHA, '--previous-core', str(args.previous_core),
                '--source', str(args.source), '--reference', str(args.reference), '--output', str(root / 'run')]
        (root / 'launch-command.json').write_text(json.dumps({'argv': argv, 'slice': slice_name}, indent=2) + '\n')
        result['test_exit'] = subprocess.run(argv, env=env, timeout=2400).returncode
        if result['test_exit'] == 0:
            result['acl_exit'] = subprocess.run(['/usr/bin/python3', '-B', str(TEST / 'check_acl_capture.py'),
                                                  str(root / 'run')], env=env, timeout=30).returncode
    finally:
        unit.unlink(missing_ok=True)
        reload = subprocess.run(['systemctl', '--user', 'daemon-reload'], env=env, capture_output=True, timeout=30)
        result.update(ended=time.time(), slice_file_removed=not unit.exists(), reload_exit=reload.returncode)
        (root / 'launch-result.json').write_text(json.dumps(result, indent=2) + '\n')
    print('launcher', json.dumps(result), flush=True)
    return 0 if result['test_exit'] == 0 and result['acl_exit'] == 0 and result['reload_exit'] == 0 else 1


if __name__ == '__main__':
    sys.exit(main())

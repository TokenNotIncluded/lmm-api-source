#!/usr/bin/env python3
"""Launch only the independent v6 upgraded-runtime authority acceptance."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess

CORE_SHA = 'dea60909b00bed72db09772200db99233b9eb68bdb9a62197aa38288786d71aa'
PREVIOUS_SHA = '6eca9a6e8b270bc7368057469dbd8185ad287db74589df3f3a223f610a53ec31'
FREEZE = Path('/home/lightjunction/Documents/Codex/2026-10-30-runtime-authority-q29f_kip/compile-r2')
COMMIT = 'fcb29cd98950053ab3a76e29548d8752366baa22'


def sha(p):
    with Path(p).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = args.output.absolute()
    assert root.parent == Path('/home/lightjunction/Documents/Codex') and not root.exists()
    previous = Path('/home/lightjunction/Documents/Codex/2026-10-29-upgrade-history-5h209odr/lmm-core')
    assert sha(FREEZE / 'lmm-core') == CORE_SHA
    assert sha(previous) == PREVIOUS_SHA
    pins = json.loads((FREEZE / 'source-pins.json').read_text())
    assert len(pins) == 1308
    assert all(sha(FREEZE / 'source' / name) == digest for name, digest in pins.items())
    names = list(pins)
    objects = subprocess.run(['git','cat-file','--batch'], cwd=Path(__file__).resolve().parents[2],
                             input=''.join(COMMIT+':'+name+'\n' for name in names).encode(), capture_output=True, check=True).stdout
    offset = 0
    for name in names:
        end = objects.index(b'\n', offset)
        _, kind, size = objects[offset:end].split()
        assert kind == b'blob'
        size = int(size); offset = end+1
        assert hashlib.sha256(objects[offset:offset+size]).hexdigest() == pins[name], name
        offset += size+1
    assert offset == len(objects)
    root.mkdir(mode=0o700)
    (root / 'runner').mkdir()
    for name in ('runtime_launch.py', 'runtime_run.py', 'runtime_README.md'):
        shutil.copy2(Path(__file__).parent / name, root / 'runner' / name)
    (root / 'binding.json').write_text(json.dumps({
        'freeze': str(FREEZE), 'core_sha256': CORE_SHA, 'previous_core_sha256': PREVIOUS_SHA,
        'source_pins_sha256': sha(FREEZE / 'source-pins.json'), 'source_count': len(pins),
        'source_commit': COMMIT, 'independent_git_blob_matches': len(pins),
        'runner': {p.name: sha(p) for p in (root / 'runner').iterdir()},
        'scope': 'v6 actual offline upgrade then independent restricted runtime SQL/HTTP; no v5/browser'}, indent=2)+'\n')
    env = {k: v for k, v in os.environ.items() if not k.startswith(('PG', 'LMM_', 'PAYMENT_', 'SQLX_'))
           and k not in ('DATABASE_URL', 'SQL_DSN')}
    env.update(XDG_RUNTIME_DIR=f'/run/user/{os.getuid()}', DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{os.getuid()}/bus')
    suffix = secrets.token_hex(5)
    aggregate = 'lmmruntime_' + suffix + '.slice'
    unitdir = Path(env['XDG_RUNTIME_DIR']) / 'systemd/user'
    unitdir.mkdir(parents=True, exist_ok=True)
    unit = unitdir / aggregate
    unit.write_text('[Unit]\nDescription=Independent upgraded runtime aggregate\n[Slice]\nMemoryAccounting=yes\nMemoryMax=2G\nMemoryHigh=1536M\nMemorySwapMax=4G\nCPUQuota=100%\nTasksMax=256\n')
    env['RUNTIME_ACCEPTANCE_SLICE'] = aggregate
    argv = ['systemd-run', '--user', '--scope', '--quiet', '--unit=lmm-runtime-independent-'+suffix,
            '--slice='+aggregate, '-p', 'MemoryMax=2G', '-p', 'MemoryHigh=1536M', '-p', 'MemorySwapMax=4G',
            '-p', 'CPUQuota=100%', '-p', 'TasksMax=256', 'python3', '-B', str(root/'runner/runtime_run.py'),
            '--root', str(root), '--freeze', str(FREEZE), '--previous-core', str(previous)]
    (root/'launch-command.json').write_text(json.dumps({'argv': argv, 'slice': aggregate}, indent=2)+'\n')
    result = {}
    try:
        subprocess.run(['systemctl', '--user', 'daemon-reload'], env=env, check=True, timeout=30)
        result['exit'] = subprocess.run(argv, env=env, timeout=1800).returncode
    finally:
        unit.unlink(missing_ok=True)
        result['reload_exit'] = subprocess.run(['systemctl', '--user', 'daemon-reload'], env=env, timeout=30).returncode
        result['slice_file_removed'] = not unit.exists()
        (root/'launch-result.json').write_text(json.dumps(result, indent=2)+'\n')
    return result['exit'] or result['reload_exit']


if __name__ == '__main__':
    raise SystemExit(main())

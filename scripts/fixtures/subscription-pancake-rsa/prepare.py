#!/usr/bin/env python3
"""Materialize fixed fixture inputs; never starts PG or builds anything."""
import argparse
import ast
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib

PACKAGE = Path(__file__).resolve().parent
REPO = PACKAGE.parents[2]


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1048576), b''):
            value.update(block)
    return value.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--core', type=Path)
    parser.add_argument('--go', type=Path)
    parser.add_argument('--output-parent', type=Path,
                        default=Path.home()/'.cache/lmm-subscription-rsa-repo')
    parser.add_argument('--check-only', action='store_true')
    args = parser.parse_args()
    pins = json.loads((PACKAGE/'pins.json').read_text())
    for name, expected in pins['harness_files'].items():
        path = PACKAGE/name
        assert digest(path) == expected, (name, 'fixture input drift')
        if path.suffix == '.py':
            ast.parse(path.read_text(), filename=name)
    for tool in ('python', 'git', 'systemd-run', 'unshare', 'mount', 'ip',
                 'nice', 'ionice', 'findmnt', 'df', 'initdb', 'pg_ctl', 'psql'):
        assert shutil.which(tool), ('required tool missing', tool)
    from cryptography.hazmat.primitives.asymmetric import rsa  # noqa: F401
    artifacts = {}
    for name in ('core', 'go'):
        path = (getattr(args, name) or Path.home()/pins['default_artifacts'][name]).resolve()
        expected = pins[name]['sha256' if name == 'core' else 'binary_sha256']
        assert digest(path) == expected, (name, 'fixed binary drift')
        assert os.access(path, os.X_OK), (name, 'binary is not executable')
        artifacts[name] = {'path': str(path), 'sha256': expected}
    provenance = {}
    for name, reference in pins['provenance_files'].items():
        path = Path.home()/reference['path']
        assert digest(path) == reference['sha256'], (name, 'artifact provenance drift')
        provenance[name] = {'path': str(path), 'sha256': reference['sha256']}
    model = tomllib.loads((PACKAGE/'model.fixture.toml').read_text().format(
        secret_file='"/private/synthetic-model.key"', endpoint='"http://127.0.0.1:12345/v1"'))
    assert model['input_token_ceiling'] == 200 and model['lease_seconds'] == 30
    assert model['prices']['rules'][0]['rates']['input'] == 4000000
    assert model['prices']['rules'][0]['rates']['output'] == 8000000
    sql = {}
    for name, expected in pins['sql'].items():
        data = subprocess.check_output(
            ['git', '-C', str(REPO), 'show', pins['sql_revision']+':'+name])
        assert hashlib.sha256(data).hexdigest() == expected, (name, 'fixed SQL drift')
        sql['frozen/'+name] = data
    if args.check_only:
        print(json.dumps({'static_inputs': 'PASS', 'pg': 'NOT_RUN',
                          'pins_sha256': digest(PACKAGE/'pins.json')}))
        return
    # Only small input copies here. RUN.sh checks mount/space/residue before PG.
    os.umask(0o077)
    parent = args.output_parent.expanduser().resolve()
    parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    assert parent.stat().st_uid == os.getuid(), 'output parent must be owned by current user'
    workspace = Path(tempfile.mkdtemp(prefix='run-', dir=parent))
    runtime = ('runner.py', 'namespace-entry.sh', 'host_integrity.py',
               'hosts.fixture', 'nsswitch.fixture', 'model.fixture.toml')
    for name in runtime:
        shutil.copyfile(PACKAGE/name, workspace/name)
    for name, data in sql.items():
        path = workspace/name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    owned = {name: digest(workspace/name) for name in (*runtime, *sql)}
    freeze = {'status': 'PREPARED_REPO_ADAPTATION_NOT_EXECUTED',
              'owned_files': owned, 'artifacts': artifacts, 'limits': pins['limits'],
              'package_pins_sha256': digest(PACKAGE/'pins.json'),
              'source_sql_revision': pins['sql_revision'],
              'provenance_files': provenance, 'scope': pins['scope'],
              'known_exclusions': pins['known_exclusions']}
    (workspace/'freeze.json').write_text(json.dumps(freeze, indent=2)+'\n')
    print(workspace)


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Capture the final fresh Core65 SQL catalog in one empty isolated PG18 DB.

No PG launcher, build, runtime-grants application, native upgrade authorization,
production access, or tracked target writes. Requires frozen source + reviewed
six-component SQL/grants + an independently recorded compiled binary artifact.
--current-ledger-update instead binds the frozen current source and binary,
restores only the three reviewed predecessor ledger-producer digests for historical lineage
checks, and captures a fresh-install successor without upgrade authorization.
Run --help for the one-shot entry; root owns PG lifetime and runtime ACL tests.
All SQL/stdout/stderr (including the deliberate rollback failure) are retained.
"""
import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess

NAMES = ['federated-auth', 'commerce-partial-refunds', 'subscription-stripe',
         'subscription-pancake', 'commerce-legacy-refunds', 'image-native-usage']
AUTH = '0784c5097bf3ed003e7c6d7963c09660845acf6f27779896d858fc356bbef7bc'
STORE = 'd64019bcdba8e0cd5c45d501013276eca062535a4dc96b7e11b30bbcfc0a3100'
STORE_GRANTS = '22671b1dbc95a343509eda8a542d62f5899edbcfc4317d248b44da97c4bd8cec'
# SUB01 static producer freeze; the extensions SQL is provenance only and is
# never executed in the Core PG topology or included in the Core manifest.
STRIPE_RECEIPT_SHA256 = "940b7775d559a00c68dbac1d687f653f68370d4121b071ef3d0c688269bc2f87"
STRIPE_SQL_FREEZE = {'services/core/schema/subscription_stripe.sql': 'a96a29d57e01c4a2656eff9b19980360f50c19578ee92c7c78ddec038cdd3917', 'services/core/schema/subscription_stripe_grants.sql': '08ab9bf0c2d85a8f091f9b535872d1fefe4ee5c5c307921c296b6c613328bada', 'services/extensions/internal/modules/payments/subscription_stripe_schema.sql': '01f966266f339fa634db2f24875ebd6e49a56c91050d1f5df3f6d8369fd40aef'}
APP = "n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'"
IDENT = re.compile(r'[a-z][a-z0-9_]{0,62}\Z')
# Frozen pre-wallet fresh lineage. This mode changes installation evidence only.
PRIOR_LEDGER_SHA256 = 'ce973f6fee25904d4fcdc87895ec49e6c53e10c9fb3508df4d7d72ace16df77f'
PRIOR_LEDGER_COMPONENTS = {
    'ledger': ('services/core/schema/ledger.sql', PRIOR_LEDGER_SHA256),
    'referral-ledger-revision': ('services/core/schema/referral_ledger_revision.sql',
        '4221800369a6ac582b367892b94d9b734a882f7d023de12a8e530bab29c7989e'),
    'commerce-self-purchase': ('services/core/schema/commerce_self_purchase.sql',
        'fae185c575e3b82e634523a95d3453e0c86c4f21f28719f885ac53631c8986e0'),
}
PRIOR_FRESH_MANIFEST_SHA256 = '62c8f7e32b0c831dfea5a11311d94193d6625fc9f7e3564755971882c65cff3a'
HEX = re.compile(r'[0-9a-f]{64}\Z')


def need(value, message):
    if not value:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def digest_file(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(path, value):
    with path.open('x', encoding='utf-8') as stream:
        stream.write(json.dumps(value, indent=2, ensure_ascii=False) + '\n')


def inputs(root, files):
    result = {}
    for name, expected in files.items():
        need(HEX.fullmatch(expected), 'invalid freeze digest: ' + name)
        path = (root / name).resolve()
        need(path.is_relative_to(root), 'freeze input escapes source root')
        result[name] = digest_file(path)
        need(result[name] == expected, 'frozen input changed: ' + name)
    return result


class Pg:
    def __init__(self, args, output):
        self.args, self.output, self.sequence = args, output, 0
        # Do not inherit connection/service/password/options from the workstation.
        self.env = {k: os.environ[k] for k in ('PATH', 'HOME', 'LANG') if k in os.environ}
        self.env.update(PGCONNECT_TIMEOUT='5', PGCLIENTENCODING='UTF8',
                        PGOPTIONS='-c timezone=UTC -c statement_timeout=30000 -c lock_timeout=3000 -c work_mem=4MB -c max_parallel_workers_per_gather=0')
        self.database = args.fresh_db

    def sql(self, body, role=None, reject=False):
        self.sequence += 1
        stem = self.output / f'{self.sequence:03d}-{self.database}'
        statement = (f'SET ROLE "{role}";\n' if role else '') + body
        stem.with_suffix('.sql').write_text(statement)
        command = ['psql', '-X', '-q', '-At', '-v', 'ON_ERROR_STOP=1',
                   '-h', str(self.args.socket_dir), '-p', str(self.args.port),
                   '-U', self.args.user, '-d', self.database]
        save(stem.with_suffix('.command.json'), command)
        result = subprocess.run(command, input=statement, text=True, capture_output=True,
                                env=self.env, timeout=90)
        stem.with_suffix('.stdout').write_text(result.stdout)
        stem.with_suffix('.stderr').write_text(result.stderr)
        save(stem.with_suffix('.result.json'), {'returncode': result.returncode, 'expected_rejection': reject})
        if reject:
            need(result.returncode != 0 and 'division by zero' in result.stderr,
                 'rollback probe failed before its deliberate error; inspect ' + str(stem))
        else:
            need(result.returncode == 0, 'SQL failed; inspect ' + str(stem))
        return result.stdout.strip()


def final_component_count(components):
    # Count the actual source registry; the exact launch suffix is still closed.
    names = [name for name, _ in components]
    need(len(names) == 59 + len(NAMES) and names[59:] == NAMES,
         'not the exact final 59+6 assembly; final image usage component is required')
    return len(names)


def validate_current_ledger_lineage(compiled, predecessor, manifest_hash, component_sha256):
    """Restore exactly the three frozen assembled ledger producers for lineage."""
    need(set(component_sha256) == set(PRIOR_LEDGER_COMPONENTS),
         'current ledger producer set must match the three reviewed components')
    restored = copy.deepcopy(compiled)
    for name, (_, prior) in PRIOR_LEDGER_COMPONENTS.items():
        current = component_sha256[name]
        need(HEX.fullmatch(current), 'invalid current ledger source digest: ' + name)
        components = [item for item in restored['components'] if item['name'] == name]
        need(len(components) == 1 and components[0]['raw_schema_sha256'] == current,
             'current compiled ledger producer differs from frozen SQL: ' + name)
        need(current != prior, 'current-ledger mode requires an actual producer change: ' + name)
        components[0]['raw_schema_sha256'] = prior
    prefix = copy.deepcopy(restored)
    prefix['components'] = prefix['components'][:59]
    need(manifest_hash(prefix) == predecessor['manifest_sha256'],
         '59 prefix changed beyond the authorized ledger producers')
    need(manifest_hash(restored) == PRIOR_FRESH_MANIFEST_SHA256,
         'fresh manifest changed beyond the authorized ledger producers')


def validate(args, output):
    root = args.source_root.resolve()
    need(root.is_relative_to((Path.home()/'.cache').resolve()) or
         root.is_relative_to((Path.home()/'.local/state/lmm-api').resolve()),
         'source root must be a private frozen cache/state snapshot, not the live workspace')
    freeze = json.loads(args.freeze.read_text())
    files = freeze['files']
    if args.current_ledger_update:
        required_capture = (
            'scripts/capture-core62-target.py', 'scripts/test-commerce-self-purchase.py',
            'services/core/src/core_schema.rs', 'services/core/src/core_schema/manifest.rs',
            'services/core/src/core_schema/upgrade/subscription_epay_target.json',
            'services/core/src/core_schema/upgrade/fresh_target.json',
            'services/core/schema/ledger.sql',
        )
        for name in required_capture:
            need(name in files, 'required frozen input absent: ' + name)
    else:
        for name in NAMES:
            need('services/core/schema/' + name.replace('-', '_') + '.sql' in files, 'component SQL not frozen: ' + name)
        need(freeze.get('stripe_component_receipt_sha256') == STRIPE_RECEIPT_SHA256,
             'SUB01 Stripe receipt not bound to the reviewed static freeze')
        for name, digest in STRIPE_SQL_FREEZE.items():
            need(files.get(name) == digest, 'Stripe SQL/grants freeze changed: ' + name)
        need(files['services/core/schema/federated_auth.sql'] == AUTH, 'AUTH freeze changed: review required')
        need(files['services/core/schema/commerce_partial_refunds.sql'] == STORE, 'STORE freeze changed: review required')
        need(files.get('services/core/schema/commerce_partial_refunds_grants.sql') == STORE_GRANTS, 'STORE grants freeze changed')
        for name in ('deployment/postgres/grant-core-runtime.sql',
                     'deployment/postgres/revoke-core-runtime.sql',
                     'services/core/schema/subscription_stripe_grants.sql',
                     'services/core/schema/subscription_pancake_grants.sql',
                     'services/core/schema/commerce_legacy_refunds_grants.sql',
                     'services/core/src/subscriptions.rs',
                     'services/core/src/subscriptions/external_pancake.rs', 'services/core/src/commerce.rs',
                     'scripts/migrate-legacy-snapshot.py', 'scripts/capture-core62-target.py',
                     'services/core/src/core_schema.rs', 'services/core/src/core_schema/manifest.rs',
                     'scripts/test-commerce-self-purchase.py',
                     'services/core/src/core_schema/upgrade/subscription_epay_target.json'):
            need(name in files, 'required frozen input absent: ' + name)
    # Freeze every source/DDL file used by contract reconstruction, including
    # SQL concat inputs and the permission snapshot query; no old SHA relabeling.
    required = list((root/'services/core/schema').glob('*.sql'))
    required += list((root/'services/core/src/core_schema').rglob('*.rs'))
    required += list((root/'services/core/src/core_schema').rglob('*.sql'))
    for path in required:
        need(str(path.relative_to(root)) in files, 'contract input not frozen: ' + str(path))
    before = inputs(root, files)
    need(digest_file(Path(__file__).resolve()) == before['scripts/capture-core62-target.py'],
         'capture entrypoint differs from the frozen source')
    need(digest_file(args.binary) == freeze['binary_sha256'], 'binary artifact digest mismatch')
    receipt = Path(freeze['artifact_receipt']).resolve()
    need(digest_file(receipt) == freeze['artifact_receipt_sha256'], 'artifact receipt changed')
    save(output/'inputs.before.json', before)
    save(output/'freeze.json', freeze)
    # This command has no DB/config I/O. Its actual compiled bytes are compared
    # to the source reconstruction; Python reconstruction alone grants no pin.
    command = [str(args.binary.resolve()), 'schema-manifest']
    save(output/'binary.command.json', command)
    result = subprocess.run(command, capture_output=True, timeout=30)
    (output/'binary.stdout').write_bytes(result.stdout)
    (output/'binary.stderr').write_bytes(result.stderr)
    save(output/'binary.result.json', {'returncode': result.returncode})
    need(result.returncode == 0, 'compiled schema-manifest failed')
    compiled = json.loads(result.stdout)
    spec = importlib.util.spec_from_file_location('fresh_source_contract', root/'scripts/test-commerce-self-purchase.py')
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    components, metadata, manifest = helper.source_contract()
    component_count = final_component_count(components)
    need(component_count == len(compiled['components']), 'registry/compiled component count mismatch')
    need(compiled == manifest, 'actual binary manifest differs from frozen source contract')
    predecessor = json.loads((root/'services/core/src/core_schema/upgrade/subscription_epay_target.json').read_text())
    if args.current_ledger_update:
        validate_current_ledger_lineage(compiled, predecessor, helper.manifest_hash,
                                        {name: before[path] for name, (path, _) in PRIOR_LEDGER_COMPONENTS.items()})
    else:
        prefix = copy.deepcopy(compiled)
        prefix['components'] = prefix['components'][:59]
        need(helper.manifest_hash(prefix) == predecessor['manifest_sha256'], '59 prefix/identity/installer/catalog contract changed')
    save(output/'compiled-manifest.json', compiled)
    return root, freeze, before, helper, components, metadata, manifest, predecessor


def capture(args, output):
    root, freeze, before, f, components, metadata, manifest, predecessor = validate(args, output)
    pg = Pg(args, output)
    f.sql = pg.sql
    owner = args.owner
    facts = json.loads(pg.sql("SELECT json_build_array(current_database(),current_setting('server_version_num')::int,(SELECT system_identifier::text FROM pg_control_system()),(SELECT oid FROM pg_database WHERE datname=current_database()),EXISTS(SELECT FROM pg_namespace n WHERE " + APP + " AND n.nspname<>'public'),EXISTS(SELECT FROM pg_class WHERE relnamespace='public'::regnamespace) OR EXISTS(SELECT FROM pg_proc WHERE pronamespace='public'::regnamespace),pg_get_userbyid((SELECT datdba FROM pg_database WHERE datname=current_database())))"))
    need(facts[0] == args.fresh_db and 180000 <= facts[1] < 190000 and not facts[4] and not facts[5]
         and facts[6] == owner, 'requires an empty owner-controlled private PG18 database')
    role = json.loads(pg.sql(f"SELECT json_build_array(rolsuper,rolcreaterole,rolcreatedb,rolcanlogin,rolreplication,rolbypassrls) FROM pg_roles WHERE rolname='{owner}'"))
    need(role == [False]*6, 'installer owner must be a restricted NOLOGIN role')
    save(output/'database-identity.json', facts)
    query = manifest['catalog']['query_utf8']

    def catalog(label):
        definitions = pg.sql(query, owner).splitlines()
        save(output/(label + '.definitions.json'), definitions)
        return f.framed(definitions)

    empty = catalog('empty')
    ddl = '\n'.join(part for _, part in components) + '\n' + '\n'.join(metadata)
    pg.sql('BEGIN;\n' + ddl + '\nSELECT 1/0;\nCOMMIT;', owner, reject=True)
    need(catalog('after-rollback') == empty, 'failed fresh install left catalog objects')
    pg.sql('BEGIN;\n' + ddl + '\nCOMMIT;', owner)
    fresh = catalog('fresh')
    tables = pg.sql(f"SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {APP} AND c.relkind IN ('r','p') ORDER BY 1", owner).splitlines()
    seqs = pg.sql(f"SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {APP} AND c.relkind='S' ORDER BY 1", owner).splitlines()
    source = (root/'services/core/src/core_schema/upgrade/finance.rs').read_text()
    acl = source.split('pub(super) async fn permissions(', 1)[1].split('r#"', 1)[1].split('"#', 1)[0]
    acl = acl.replace('$1', 'ARRAY(SELECT oid::bigint FROM pg_class)').replace('$2', 'ARRAY(SELECT oid::bigint FROM pg_type)')
    fresh_acl = pg.sql(acl, owner).splitlines()
    save(output/'fresh.permissions.json', fresh_acl)
    save(output/'fresh.raw-sql-data-sha256.json', f.data_snapshot(tables, seqs, owner))
    after = inputs(root, freeze['files'])
    need(before == after and digest_file(args.binary) == freeze['binary_sha256'], 'inputs drifted during capture')
    save(output/'inputs.after.json', after)
    target = dict(component_count=len(components), manifest_sha256=f.manifest_hash(manifest), catalog_sha256=fresh)
    save(output/'fresh_target.proposal.json', target)
    save(output/'report.json', dict(status='fresh_sql_catalog_captured_unadmitted', target=target,
         observed_acl_sha256=f.framed(fresh_acl), native_upgrade=False, importer_admitted=False,
         scope='empty synthetic SQL install/rollback/catalog/owner-ACL observation; no native init or business acceptance',
         capture_mode='current-ledger-update' if args.current_ledger_update else 'original-65-assembly',
         prior_ledger_components={name: {'path': path, 'sha256': prior}
             for name, (path, prior) in PRIOR_LEDGER_COMPONENTS.items()} if args.current_ledger_update else None,
         prior_fresh_manifest_sha256=PRIOR_FRESH_MANIFEST_SHA256 if args.current_ledger_update else None,
         not_run=['actual final Core init/seed receipt', 'runtime grants application/rejections (root-owned)',
                  'final importer admission/legacy migration', 'production deployment'],
         cleanup='no server/roles/databases created by this tool; caller stops its isolated PG; evidence DB retained'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--current-ledger-update', action='store_true',
                        help='Capture only the reviewed fresh ledger replacement; preserve historical upgrade targets')
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--freeze', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--socket-dir', type=Path, required=True)
    parser.add_argument('--port', type=int, required=True)
    parser.add_argument('--user', required=True)
    parser.add_argument('--owner', required=True)
    parser.add_argument('--fresh-db', required=True)
    args = parser.parse_args()
    for value in (args.user, args.owner, args.fresh_db):
        need(IDENT.fullmatch(value), 'invalid isolated fixture identifier')
    need(args.fresh_db.endswith('_test'), 'an isolated *_test database is required')
    args.socket_dir = args.socket_dir.resolve(strict=True)
    private = (Path.home()/'.local/state/lmm-api').resolve(strict=True)
    need(args.socket_dir.is_relative_to(private), 'only explicit private Unix socket under ~/.local/state/lmm-api permitted')
    need(1 <= args.port <= 65535, 'invalid port')
    os.umask(0o077)
    args.out.mkdir(parents=False, exist_ok=False)
    try:
        capture(args, args.out)
    except Exception as error:
        save(args.out/'failure.json', {'error': str(error), 'type': type(error).__name__, 'retry': 'none'})
        raise


if __name__ == '__main__':
    main()

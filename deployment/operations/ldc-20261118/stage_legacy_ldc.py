#!/usr/bin/env python3
"""Reviewed disabled-LDC migration candidate builder. No service/config activation.

Run only on the server. Default is a read-only preflight. --stage-to creates a
fresh root-private candidate directory, never overwrites active configuration,
never restarts services, and never prints merchant IDs, keys or credential URLs.
"""
import argparse
import decimal
import hashlib
import json
import os
import pathlib
import re
import subprocess
import tomllib
import urllib.parse

CORE_MAIN = pathlib.Path('/etc/lmm.conf.d/native69-20261115/core-final.toml')
CORE_SOURCE = pathlib.Path('/etc/lmm.conf.d/core-payments.toml')
GO_SOURCE = pathlib.Path('/etc/lmm.conf.d/extensions-payments.toml')
CORE_TARGET = pathlib.Path('/var/lib/lmm/core/config/payments.toml')
KEY_TARGET = CORE_TARGET.parent / 'legacy-ldc-5004.secret'
CHANNEL = 5004
MINIMUM = 10000
MAXIMUM = 9_000_000_000_000
CREDITS = 500
ORIGIN = 'https://native.api.lmm.best'
CANONICAL_ENDPOINT = 'https://credit.linux.do/epay'
LEGACY_ENDPOINT = CANONICAL_ENDPOINT + '/pay'
SAFE_POLICY_FIELDS = ('type', 'enabled', 'min_topup', 'settlement_unit', 'unit_price', 'audience_mode')


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def run(args, env=None):
    result = subprocess.run(args, env=env, capture_output=True, timeout=30)
    require(result.returncode == 0, 'read-only subprocess failed')
    return result.stdout


def digest(value):
    return hashlib.sha256(value).hexdigest()


def legacy_options():
    pid = int(run(['systemctl', 'show', 'lmm-api', '--property=MainPID', '--value']).strip())
    require(pid > 1, 'legacy process unavailable')
    dsn = next((x.split(b'=', 1)[1].decode() for x in
                pathlib.Path(f'/proc/{pid}/environ').read_bytes().split(b'\0')
                if x.startswith(b'SQL_DSN=')), None)
    require(dsn is not None, 'legacy database configuration unavailable')
    url = urllib.parse.urlparse(dsn)
    require(url.scheme in ('postgres', 'postgresql'), 'unsupported database configuration')
    query = urllib.parse.parse_qs(url.query)
    env = {key: value for key, value in os.environ.items() if not key.startswith('PG')}
    env.update(
        PGHOST=query.get('host', [url.hostname or 'localhost'])[0],
        PGPORT=query.get('port', [str(url.port or 5432)])[0],
        PGUSER=urllib.parse.unquote(url.username or ''),
        PGPASSWORD=urllib.parse.unquote(url.password or ''),
        PGDATABASE=url.path.lstrip('/'),
        PGOPTIONS='-c default_transaction_read_only=on -c statement_timeout=10000',
        PGCONNECT_TIMEOUT='10', PGAPPNAME='lmm-disabled-ldc-migration')
    if 'sslmode' in query:
        env['PGSSLMODE'] = query['sslmode'][0]
    sql = ("SELECT json_object_agg(key,value) FROM lmm_prod_20260802.options "
           "WHERE key IN ('EpayId','EpayKey','PayAddress','PayMethods')")
    return json.loads(run(['psql', '-X', '-A', '-t', '-v', 'ON_ERROR_STOP=1', '-c', sql], env))


def history_clear():
    tables = (
        'core_payments.intents', 'core_payments.legacy_bases',
        'core_payments.accepted_proofs', 'core_payments.provider_events',
        'core_payments.provider_refunds', 'core_payments.refund_dispatch_bindings',
        'core_subscriptions.external_prices', 'core_subscriptions.external_orders')
    sql = 'SELECT ' + ' OR '.join(
        f'EXISTS(SELECT FROM {table} WHERE channel_id={CHANNEL})' for table in tables)
    found = run(['runuser', '-u', 'postgres', '--', 'env',
                 'PGOPTIONS=-c default_transaction_read_only=on -c statement_timeout=10000',
                 'psql', '-X', '-A', '-t', '-v', 'ON_ERROR_STOP=1',
                 '-d', 'lmm_core_native_20261115', '-c', sql]).strip()
    require(found == b'f', 'candidate channel already has a financial binding')


def channel_block(values):
    lines = ['[[channels]]']
    for key, value in values.items():
        lines.append(f'{key} = {json.dumps(value, ensure_ascii=True)}')
    return '\n'.join(lines) + '\n'


def build(core_raw, go_raw, main_raw, options):
    core = tomllib.loads(core_raw.decode())
    go = tomllib.loads(go_raw.decode())
    main = tomllib.loads(main_raw.decode())
    require(core.get('version') == 2 and go.get('version') == 2, 'unsupported payment document')
    require(main['payments']['config_file'] == str(CORE_SOURCE), 'active payment path changed')
    require(core.get('permissions', {}).get('read_user') is True, 'read-only configuration permission missing')
    for document in (core, go):
        require(0 < len(document['channels']) < 16, 'channel capacity exhausted')
        require(all(c['id'] != CHANNEL and c['provider'] != 'epay' for c in document['channels']),
                'EPay import already exists or candidate channel is in use')
    methods = json.loads(options['PayMethods'])
    matches = [method for method in methods if method.get('type') == 'epay']
    require(len(matches) == 1, 'ambiguous old EPay binding')
    policy = matches[0]
    require(str(policy.get('enabled')).lower() == 'false' and
            policy.get('settlement_unit') == 'LDC' and
            policy.get('audience_mode') == 'all', 'legacy disabled-LDC policy changed')
    require(decimal.Decimal(str(policy['unit_price'])) == 10 and
            decimal.Decimal(str(policy['min_topup'])) == 10, 'legacy base pricing changed')
    merchant, key, endpoint = (options[name] for name in ('EpayId', 'EpayKey', 'PayAddress'))
    require(re.fullmatch(r'[A-Za-z0-9_.-]{1,128}', merchant) is not None,
            'unsupported original Client ID')
    require(16 <= len(key) <= 4096 and all(33 <= ord(c) <= 126 for c in key),
            'unsupported original signing key')
    require(endpoint.rstrip('/') == LEGACY_ENDPOINT, 'legacy gateway address changed')
    # Old SDK appends /submit.php; native LDC appends /pay/submit.php.
    require(endpoint.rstrip('/') + '/submit.php' ==
            CANONICAL_ENDPOINT + '/pay/submit.php', 'gateway semantics differ')
    notify, back = ORIGIN + f'/callbacks/{CHANNEL}', ORIGIN + '/wallet'
    require(len(notify) <= 100 and len(back) <= 100, 'callback too long')
    identity = dict(id=CHANNEL, provider='epay', merchant=merchant,
                    environment='live', currency='LDC', enabled=False)
    native = dict(identity, credits_per_minor=CREDITS, min_amount_minor=MINIMUM,
                  max_amount_minor=MAXIMUM, webhook_secret_file=str(KEY_TARGET),
                  payment_type='epay', endpoint=CANONICAL_ENDPOINT,
                  notify_url=notify, return_url=back)
    adapter = dict(identity, subscription_enabled=False, min_amount_minor=MINIMUM,
                   payment_type='epay', endpoint=CANONICAL_ENDPOINT,
                   notify_url=notify, return_url=back)
    new_core = core_raw + b'\n' + channel_block(native).encode()
    new_go = go_raw + b'\n' + channel_block(adapter).encode()
    for before, after, expected in ((core, new_core, native), (go, new_go, adapter)):
        parsed = tomllib.loads(after.decode())
        require(parsed['channels'][:-1] == before['channels'] and
                parsed['channels'][-1] == expected, 'existing channel changed')
        require({k:v for k,v in parsed.items() if k != 'channels'} ==
                {k:v for k,v in before.items() if k != 'channels'}, 'existing policy changed')
    main_text = main_raw.decode()
    section = re.search(r'(?ms)^\[payments\]\s*\n(.*?)(?=^\[|\Z)', main_text)
    require(section is not None, 'payment section cannot be located')
    replacement, count = re.subn(
        r'(?m)^(\s*config_file\s*=\s*)(?:"[^"]*"|\'[^\']*\')(\s*(?:#.*)?)$',
        lambda m: m[1] + json.dumps(str(CORE_TARGET)) + m[2], section[1])
    require(count == 1, 'ambiguous main payment path')
    new_main = (main_text[:section.start(1)] + replacement + main_text[section.end(1):]).encode()
    parsed_main = tomllib.loads(new_main.decode())
    expected_main = {**main, 'payments': {**main['payments'], 'config_file': str(CORE_TARGET)}}
    require(parsed_main == expected_main, 'unrelated main configuration changed')
    report = {
        'contract': 'lmm.disabled-ldc-migration.v1', 'channel_id': CHANNEL,
        'core_enabled': False, 'wallet_enabled': False, 'subscription_enabled': False,
        'min_amount_minor': MINIMUM, 'max_amount_minor': MAXIMUM,
        'credits_per_minor': CREDITS, 'currency': 'LDC',
        'legacy_policy': {k:policy.get(k) for k in SAFE_POLICY_FIELDS},
        'pricing_review_required_before_activation': True,
        'maximum_kind': 'native protocol technical limit, not legacy business policy',
        'legacy_minimum_basis': '10 credited USD x 500000 raw CREDIT / 500 CREDIT per minor',
        'old_amount_discounts_migrated': False,
        'source_sha256': {'core':digest(core_raw), 'go':digest(go_raw), 'main':digest(main_raw)},
        'candidate_sha256': {'core':digest(new_core), 'go':digest(new_go), 'main':digest(new_main)},
        'existing_channels_preserved': True, 'gateway_destination_preserved': True,
        'targets': {'core':str(CORE_TARGET), 'key':str(KEY_TARGET), 'go':str(GO_SOURCE),
                    'main':str(CORE_MAIN)},
        'permissions': {'directory':'0700', 'core_and_key':'0600 / lmm-core',
                        'go':'0600 / existing owner', 'main':'existing mode and owner'},
    }
    return {'core-payments.toml':new_core, 'extensions-payments.toml':new_go,
            'core-final.toml':new_main, 'legacy-ldc-5004.secret':key.encode(),
            'plan.json':(json.dumps(report, indent=2) + '\n').encode()}, report


def write_private(path, value):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, 'wb') as handle:
        handle.write(value)
        handle.flush()
        os.fsync(handle.fileno())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage-to', type=pathlib.Path,
                        help='fresh directory under /srv/lmm-shadow; no active-file changes')
    args = parser.parse_args()
    os.umask(0o077)
    require(os.geteuid() == 0, 'server root is required for private local reads')
    require(not CORE_TARGET.exists() and
            not CORE_SOURCE.with_name(CORE_SOURCE.name + '.epay.json').exists(),
            'managed EPay configuration already exists; reconcile instead of overwriting')
    candidates, report = build(CORE_SOURCE.read_bytes(), GO_SOURCE.read_bytes(),
                               CORE_MAIN.read_bytes(), legacy_options())
    history_clear()
    if args.stage_to is not None:
        stage = args.stage_to
        require(stage.parent == pathlib.Path('/srv/lmm-shadow') and
                stage.name.startswith('.epay-stage-') and not stage.exists(),
                'stage must be a fresh private directory')
        stage.mkdir(mode=0o700)
        for name, contents in candidates.items():
            write_private(stage / name, contents)
        directory = os.open(stage, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        report['stage'] = str(stage)
    report['active_files_changed'] = False
    report['services_restarted'] = False
    print(json.dumps(report))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        # Do not emit exception text, subprocess stderr, options or connection URLs.
        print(json.dumps({'error':'migration preflight failed', 'kind':type(error).__name__}))
        raise SystemExit(1)

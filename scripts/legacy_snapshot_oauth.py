"""Owner-only continuation of a strict identity snapshot import; no DDL or funds."""
import hashlib
import json
import re
from pathlib import Path

import legacy_cutover_import as SOURCE

PROVIDER_COLUMNS = ('id', 'name', 'slug', 'enabled', 'client_id', 'authorization_endpoint',
                    'token_endpoint', 'user_info_endpoint', 'scopes', 'well_known',
                    'user_id_field', 'username_field', 'display_name_field', 'email_field',
                    'auth_style', 'access_policy')
BINDING_COLUMNS = ('id', 'user_id', 'provider_id', 'provider_user_id')
SOURCE_TABLES = ('custom_oauth_providers', 'user_oauth_bindings')


def reader_grants(schema, role):
    """Reviewable SQL for the restored offline instance, never auto-executed."""
    if not all(isinstance(s, str) and re.fullmatch(r'[a-z_][a-z_0-9]*', s) for s in (schema, role)):
        raise ValueError('oauth_reader_identifier_invalid')
    return '\n'.join(f'GRANT SELECT ({",".join(cols)}) ON "{schema}"."{table}" TO "{role}";'
                     for table, cols in ((SOURCE_TABLES[0], PROVIDER_COLUMNS),
                                         (SOURCE_TABLES[1], BINDING_COLUMNS))) + '\n'


def export_queries(schema):
    # schema is already checked by the parent entrypoint.
    sql = f"SELECT json_build_object('kind','oauth_reader','row',NOT has_column_privilege(current_user,'\"{schema}\".custom_oauth_providers','client_secret','SELECT'));\n"
    for table, kind, columns in ((SOURCE_TABLES[0], 'oauth_provider', PROVIDER_COLUMNS),
                                 (SOURCE_TABLES[1], 'oauth_binding', BINDING_COLUMNS)):
        sql += f"SELECT json_build_object('kind','{table}_count','row',count(*)) FROM \"{schema}\".{table};\n"
        fields = ','.join("'" + c + "'," + c for c in columns)
        sql += f"SELECT json_build_object('kind','{kind}','row',json_build_object({fields})) FROM \"{schema}\".{table} ORDER BY id;\n"
    return sql


def mapping(I, users, exported, identity, identity_manifest, source_sha):
    require = I.require
    records = identity.get('records')
    require(isinstance(records, list) and records, 'oauth_identity_mapping_invalid')
    require(identity_manifest.get('scope') == 'identity-only'
            and identity_manifest.get('source_archive_sha256') == source_sha
            and identity_manifest.get('mapping_sha256') == identity.get('mapping_sha256')
            and identity.get('mapping_sha256') == hashlib.sha256(I.canonical(
                {'source_sha256': source_sha, 'records': records}).encode()).hexdigest()
            and identity_manifest.get('mapping_file_sha256') == hashlib.sha256(
                (I.canonical(identity) + '\n').encode()).hexdigest()
            and identity_manifest.get('source_target_id_map_sha256') == hashlib.sha256(
                I.id_map_bytes(records).encode()).hexdigest(), 'oauth_identity_proof_mismatch')
    ids = [r.get('user_id') for r in records]
    require(all(type(i) is int and i > 0 for i in ids) and ids == sorted(set(ids)), 'oauth_identity_mapping_invalid')
    require(len(users) == len(records) and {u['id'] for u in users} == set(ids), 'oauth_source_cohort_mismatch')
    by_id = {u['id']: u for u in users}
    require(all(r['username'] == by_id[r['user_id']]['username'] for r in records), 'oauth_source_username_mismatch')
    data = {'users': users, 'oauth_bindings': exported['bindings'], 'oauth_providers': exported['providers']}
    try:
        native = SOURCE.identity_oauth_rows(data)
    except SystemExit as error:
        raise I.MappingError(str(error).removeprefix('error: ')) from None
    rows = [dict(family='oauth', issuer=issuer, subject=subject, user_id=user)
            for issuer, subject, user, _ in native]
    telegram_subjects = set()
    for user in users:
        subject = user.get('telegram_id')
        if subject is None or subject == '':
            continue
        require(isinstance(subject, str) and re.fullmatch(r'[1-9][0-9]{0,18}', subject)
                and int(subject) <= (1 << 63) - 1, 'telegram_source_subject_invalid')
        require(subject not in telegram_subjects, 'telegram_source_owner_collision')
        telegram_subjects.add(subject)
        rows.append(dict(family='telegram', issuer='', subject=subject, user_id=user['id']))
    occupied = set()
    for row in rows:
        owner = (row['family'], row['issuer'], row['user_id'])
        require(owner not in occupied, 'oauth_multiple_source_subjects_for_owner')
        occupied.add(owner)
    rows.sort(key=lambda r: (r['family'], r['issuer'], r['subject']))
    projection = {'users': [{k: u.get(k) for k in ('id', 'username', 'github_id', 'linux_do_id', 'discord_id', 'telegram_id')}
                            for u in sorted(users, key=lambda u: u['id'])], **exported}
    return {'format_version': 1, 'scope': 'identity-oauth-only', 'source_archive_sha256': source_sha,
            'source_projection_sha256': hashlib.sha256(I.canonical(projection).encode()).hexdigest(),
            'records': rows, 'users': [{k: r[k] for k in ('user_id', 'account_id', 'username')} for r in records],
            'counts': {family: sum(r['family'] == family for r in rows) for family in ('oauth', 'telegram')},
            'google_count': sum(r['issuer'] == 'https://accounts.google.com' for r in rows)}


def build_sql(I, plan, manifest, identity_manifest, *, operation, identity_operation, target, contract):
    require, literal = I.require, I.sql_literal
    require(I.MIGRATION_ID.fullmatch(operation) and I.MIGRATION_ID.fullmatch(identity_operation)
            and operation != identity_operation, 'oauth_operation_invalid')
    require(manifest.get('operation_id') == operation
            and plan['scope'] == manifest.get('scope') == 'identity-oauth-only'
            and manifest.get('mapping_sha256') == hashlib.sha256(I.canonical(plan).encode()).hexdigest()
            and manifest.get('identity_migration_id') == identity_operation
            and manifest.get('identity_manifest_sha256') == hashlib.sha256(I.canonical(identity_manifest).encode()).hexdigest(),
            'oauth_manifest_invalid')
    connection, instance = target['connection'], target['instance_identity']
    owner, login = I.connection_roles(connection)
    oid = instance.get('database_oid')
    oid_valid = type(oid) is int or (isinstance(oid, str) and re.fullmatch(r'[1-9][0-9]{0,9}', oid))
    require(type(connection.get('port')) is int and 1 <= connection['port'] <= 65535
            and isinstance(instance.get('server_instance_nonce'), str) and bool(instance['server_instance_nonce'])
            and all(isinstance(instance.get(k), str) and instance[k] for k in ('postmaster_started_at','server_version_num'))
            and oid_valid and 0 < int(oid) <= (1 << 32) - 1, 'oauth_instance_receipt_invalid')
    require(owner != login and instance.get('database_owner') == owner, 'oauth_owner_invalid')
    require(target.get('application_or_worker_started') is False, 'oauth_target_not_stopped')
    for key in ('component_fingerprints', 'catalog_query_sha256', 'source_file_hashes', 'import_profile'):
        require(target['contract'].get(key) == contract.get(key), 'oauth_target_contract_mismatch')
    catalog = target['contract']['catalog_fingerprint_sha256']
    require(I.HEX.fullmatch(catalog), 'oauth_catalog_invalid')
    expected = literal(I.canonical(plan['records'])) + '::jsonb'
    cohort = literal(I.canonical(plan['users'])) + '::jsonb'
    archive = literal(plan['source_archive_sha256'])
    mapping_sha = literal(manifest['mapping_sha256'])
    run_manifest = literal(I.canonical(manifest)) + '::jsonb'
    actual_rows = """SELECT coalesce(jsonb_agg(jsonb_build_object('family',family,'issuer',issuer,'subject',subject,'user_id',user_id)
 ORDER BY family COLLATE "C",issuer COLLATE "C",subject COLLATE "C"),'[]'::jsonb) FROM (
 SELECT 'oauth'::text AS family,issuer,subject,user_id FROM core_identity.oauth_identities
 UNION ALL SELECT 'telegram','',subject,user_id FROM core_telegram_auth.bindings) all_links"""
    # This continuation never calls the empty-target identity writer or ledger.
    sql = f"""BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL timezone='UTC';
SET LOCAL search_path=pg_catalog;
SET LOCAL synchronous_commit=on;
SET LOCAL standard_conforming_strings=on;
SET LOCAL statement_timeout='300s';
SET LOCAL lock_timeout='5s';
SELECT pg_advisory_xact_lock(741803121);
LOCK TABLE core_migration.import_runs,core_identity.users,core_identity.user_profiles,
 core_identity.oauth_identities,core_telegram_auth.bindings,core_account_links.oauth_states,
 core_meta.schema_contract,core_meta.component_contract,core_meta.runtime_contract IN SHARE ROW EXCLUSIVE MODE;
DO $oauth_stage$
DECLARE previous core_migration.import_runs%ROWTYPE; is_replay boolean; actual jsonb;
BEGIN
 IF current_user<>{literal(owner)} OR session_user<>{literal(login)}
 OR (SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database())<>current_user
 OR (SELECT rolcanlogin OR rolsuper OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls FROM pg_roles WHERE rolname=current_user)
 OR NOT EXISTS(SELECT FROM pg_roles WHERE rolname=session_user AND rolcanlogin AND NOT (rolsuper OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls))
 OR EXISTS(SELECT FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user AND has_database_privilege(oid,current_database(),'CONNECT'))
 OR current_database()<>{literal(connection['database'])}
 OR (SELECT oid::text FROM pg_database WHERE datname=current_database())<>{literal(str(instance['database_oid']))}
 OR pg_postmaster_start_time()<>{literal(instance['postmaster_started_at'])}::timestamptz
 OR current_setting('server_version_num')<>{literal(str(instance['server_version_num']))}
 OR current_setting('lmm.migration_instance_nonce',true) IS DISTINCT FROM {literal(instance['server_instance_nonce'])}
 OR current_setting('port')::int<>{connection['port']}
 OR {I.target_transport_guard_sql(instance)} THEN RAISE EXCEPTION 'oauth owner or instance mismatch'; END IF;
 PERFORM pg_stat_clear_snapshot();
 IF EXISTS(SELECT FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid())
 THEN RAISE EXCEPTION 'oauth target has other database clients'; END IF;
 IF ({I.catalog_hash_sql(contract)}) IS DISTINCT FROM {literal(catalog)}
 OR NOT EXISTS(SELECT FROM core_meta.runtime_contract WHERE singleton AND encode(catalog_fingerprint,'hex')={literal(catalog)})
 OR (SELECT count(*) FROM core_meta.runtime_contract)<>1
 OR (SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract) IS DISTINCT FROM {literal(I.canonical(contract['component_fingerprints']))}::jsonb
 OR (SELECT count(*) FROM core_meta.schema_contract)<>1
 OR NOT EXISTS(SELECT FROM core_meta.schema_contract WHERE singleton AND version={contract['identity_version']} AND encode(fingerprint,'hex')={literal(contract['identity_fingerprint_sha256'])})
 THEN RAISE EXCEPTION 'oauth actual catalog mismatch'; END IF;
 IF {I.profile_acl_guard_sql(contract)} THEN RAISE EXCEPTION 'oauth unsafe public or receipt ACL'; END IF;
 IF NOT EXISTS(SELECT FROM core_migration.import_runs WHERE migration_id={literal(identity_operation)}
  AND source_archive_sha256={archive} AND mapping_sha256={literal(identity_manifest['mapping_sha256'])}
  AND manifest={literal(I.canonical(identity_manifest))}::jsonb)
 THEN RAISE EXCEPTION 'oauth original identity receipt mismatch'; END IF;
 IF (SELECT count(*) FROM core_identity.users)<>jsonb_array_length({cohort})
 OR EXISTS(SELECT FROM jsonb_array_elements({cohort}) r LEFT JOIN core_identity.users u ON u.id=(r->>'user_id')::bigint
 LEFT JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id IS NULL
 OR u.personal_account_id IS DISTINCT FROM (r->>'account_id')::bigint OR p.username IS DISTINCT FROM r->>'username')
 THEN RAISE EXCEPTION 'oauth imported owner or username mismatch'; END IF;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id={literal(operation)};
 is_replay := FOUND;
 IF is_replay THEN
  IF previous.source_archive_sha256<>{archive} OR previous.mapping_sha256<>{mapping_sha} OR previous.manifest<>{run_manifest}
  THEN RAISE EXCEPTION 'oauth operation receipt mismatch'; END IF;
 ELSE
  IF EXISTS(SELECT FROM core_identity.oauth_identities) OR EXISTS(SELECT FROM core_telegram_auth.bindings)
  THEN RAISE EXCEPTION 'oauth target collision'; END IF;
  INSERT INTO core_identity.oauth_identities(issuer,subject,user_id)
   SELECT r->>'issuer',r->>'subject',(r->>'user_id')::bigint FROM jsonb_array_elements({expected}) r WHERE r->>'family'='oauth';
  INSERT INTO core_telegram_auth.bindings(subject,user_id)
   SELECT r->>'subject',(r->>'user_id')::bigint FROM jsonb_array_elements({expected}) r WHERE r->>'family'='telegram';
 END IF;
 SELECT ({actual_rows}) INTO actual;
 IF actual IS DISTINCT FROM {expected}
 OR EXISTS(SELECT FROM core_telegram_auth.bindings WHERE revoked_at IS NOT NULL)
 OR EXISTS(SELECT FROM core_account_links.oauth_states WHERE revoked_at IS NOT NULL)
 THEN RAISE EXCEPTION 'oauth exact binding readback mismatch'; END IF;
 IF NOT is_replay THEN
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
   VALUES({literal(operation)},{archive},{mapping_sha},{run_manifest});
 END IF;
END $oauth_stage$;
SELECT json_build_object('status','oauth-import-verified','operation_id',{literal(operation)},
 'oauth_count',{plan['counts']['oauth']},'telegram_count',{plan['counts']['telegram']},
 'google_count',{plan['google_count']},'mapping_sha256',{mapping_sha});
COMMIT;
"""
    delimiter = '$oauth_' + hashlib.sha256(sql.encode()).hexdigest()[:24] + '$'
    while delimiter in sql:
        delimiter = delimiter[:-1] + 'x$'
    sql = sql.replace('DO $oauth_stage$', 'DO ' + delimiter, 1)
    prefix, suffix = sql.rsplit('END $oauth_stage$;', 1)
    return prefix + 'END ' + delimiter + ';' + suffix


def run_cli(I, args, output, source, source_identity, users):
    attempted = False
    try:
        I.require(args.identity_mapping and args.identity_manifest and args.identity_migration_id,
                  'oauth_requires_original_identity_materials')
        identity = I.load_receipt(args.identity_mapping, private=True)
        original = I.load_receipt(args.identity_manifest, private=True)
        exported = I.load_receipt(output / 'source-oauth-export.private.json', private=True)
        source_sha = source['archive']['sha256']
        plan = mapping(I, users, exported, identity, original, source_sha)
        manifest = dict(scope='identity-oauth-only', format_version=1, operation_id=args.migration_id, source_archive_sha256=source_sha,
                        mapping_sha256=hashlib.sha256(I.canonical(plan).encode()).hexdigest(),
                        identity_migration_id=args.identity_migration_id,
                        identity_manifest_sha256=hashlib.sha256(I.canonical(original).encode()).hexdigest(),
                        validation_tool_sha256=I.sha256_file(Path(SOURCE.__file__).resolve()),
                        import_tool_sha256=I.sha256_file(Path(I.__file__).resolve()),
                        oauth_tool_sha256=I.sha256_file(Path(__file__).resolve()))
        I.write_private(output, 'oauth-mapping.private.json', I.canonical(plan) + '\n')
        I.write_private(output, 'oauth-manifest.private.json', I.canonical(manifest) + '\n')
        public = dict(status='oauth-dry-run-mapped', operation_id=args.migration_id,
                      source_archive_sha256=source_sha, mapping_sha256=manifest['mapping_sha256'],
                      **plan['counts'], google_count=plan['google_count'], apply=False)
        if args.target_receipt:
            target = I.load_receipt(args.target_receipt, private=True)
            I.require(target.get('status') == 'initialized-empty-identity-target'
                      and target.get('migration_id') == args.identity_migration_id
                      and target.get('source_archive_sha256') == source_sha
                      and target.get('registry_provisioned') is True, 'oauth_target_receipt_invalid')
            contract = I.current_contract()
            sql = build_sql(I, plan, manifest, original, operation=args.migration_id,
                            identity_operation=args.identity_migration_id, target=target, contract=contract)
            connection = target['connection']
            actual = json.loads(I.psql(connection, "BEGIN READ ONLY; SET LOCAL timezone='UTC';\n" + I.IDENTITY_SQL + '\nCOMMIT;',
                                      readonly=True, private_dir=output, label='oauth-target-identity'))
            I.verify_instance(actual, connection, target['instance_identity'], source=False)
            I.require((actual['database_oid'],actual['postmaster_started_at']) !=
                      (source_identity['database_oid'],source_identity['postmaster_started_at']), 'source_target_same_database')
            for key in ('datcollate','datctype','datcollversion'):
                I.require(actual.get(key) == source_identity.get(key), 'source_target_collation_mismatch')
            I.write_private(output, 'oauth-import.private.sql', sql)
            public['status'] = 'oauth-dry-run-sql-plan-ready'
            if args.apply:
                I.require(args.confirm_source_sha256 == source_sha, 'apply_source_confirmation_missing')
                I.require(I.sha256_file(Path(source['archive']['path'])) == source_sha, 'source_archive_changed_before_import')
                attempted = True
                raw = I.psql(connection, sql, readonly=False, private_dir=output, label='oauth-target-import')
                I.write_private(output, 'oauth-target-import.private.jsonl', raw)
                replies = [json.loads(line) for line in raw.splitlines() if line.startswith('{')]
                I.require(len(replies) == 1 and replies[0].get('status') == 'oauth-import-verified'
                          and replies[0].get('mapping_sha256') == manifest['mapping_sha256']
                          and replies[0].get('operation_id') == args.migration_id
                          and all(replies[0].get(k) == v for k,v in
                                  {'oauth_count':plan['counts']['oauth'],'telegram_count':plan['counts']['telegram'],
                                   'google_count':plan['google_count']}.items()), 'oauth_commit_ack_invalid')
                public.update(status='oauth-import-verified', apply=True)
        else:
            I.require(not args.apply, 'apply_requires_target_receipt')
        I.require(I.sha256_file(Path(source['archive']['path'])) == source_sha, 'source_archive_changed_during_import')
        I.write_private(output, 'oauth-report.private.json', I.canonical(public) + '\n')
        print(I.canonical(public))
        return 0
    except (I.MappingError, OSError, ValueError, KeyError, TypeError) as error:
        result = dict(status='apply-outcome-unconfirmed' if attempted else 'aborted',
                      error_code=str(error) if isinstance(error, I.MappingError) else type(error).__name__)
        I.write_private(output, 'failure.private.json', I.canonical(result) + '\n')
        import sys
        print(I.canonical(result), file=sys.stderr)
        return 1

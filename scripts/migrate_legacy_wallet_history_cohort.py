#!/usr/bin/env python3
"""Install complete top-up history for the committed identity cohort, retaining orphans.

Reuses the official snapshot format and native session-scoped reader. Complete
means all orders of mapped source users, not an assertion that orphan owners
were recovered. Only the import receipt is written; no payment is settled.
"""
import argparse
import os
from pathlib import Path
import sys
import time
import uuid
import migrate_legacy_available_wallet as W
import prepare_wallet_history_snapshot as H


def prepare(export,wallet):
    W.require(wallet['scope']==W.SCOPE and export['source_row_count']==len(export['rows']),
              'cohort_source_or_wallet_invalid')
    mapping=[{'source_user_id':x['source_user_id'],'target_user_id':x['target_user_id'],
              'target_personal_account_id':x['account_id']} for x in wallet['positions']]
    known={x['source_user_id'] for x in mapping}
    selected=[r for r in export['rows'] if r['user_id'] in known]
    orphan=[r for r in export['rows'] if r['user_id'] not in known]
    cohort={**export,'rows':selected,'source_row_count':len(selected),'source_user_ids':sorted(known)}
    manifest=H.prepare(cohort,mapping,expected_rows=len(selected),
                       archive_sha256=wallet['source_archive_sha256'],export_sha256=W.digest(export))
    manifest['source_cohort']={'policy':'all-orders-of-committed-source-identity-cohort',
        'full_namespace_row_count':len(export['rows']),'mapped_owner_row_count':len(selected),
        'unmapped_owner_row_count':len(orphan),'full_namespace_restore_complete':not orphan,
        'orphan_policy':'retain-without-owner-reassignment-or-credit','unmapped_source_rows':orphan}
    return {'manifest':manifest,'target':wallet['target'],'connection':wallet['connection'],
            'wallet_migration_id':wallet['migration_id'],'wallet_mapping_sha256':wallet['mapping_sha256']}


def build_sql(m):
    manifest=m['manifest'];t=m['target'];L=W.literal
    sql=H.import_sql(manifest)
    old='IF current_user<>session_user OR NOT EXISTS('
    W.require(sql.count(old)==1,'official_history_guard_shape_changed')
    # Independent LOGIN installer SET ROLE to the NOLOGIN installation owner;
    # never enable login on the schema owner just to satisfy the old CLI shape.
    replacement=f"""IF current_user<>{L(t['role'])} OR session_user<>{L(t['login'])}
 OR NOT EXISTS(SELECT FROM pg_roles WHERE rolname=current_user AND NOT rolcanlogin AND NOT rolsuper)
 OR NOT pg_has_role(session_user,current_user,'SET')
 OR current_database()<>{L(t['database'])}
 OR (SELECT oid FROM pg_database WHERE datname=current_database())<>{int(t['database_oid'])}
 OR (pg_control_system()).system_identifier::text<>{L(t['system_identifier'])}
 OR current_setting('lmm.migration_instance_nonce')<>{L(t['nonce'])}
 OR NOT EXISTS(SELECT FROM core_migration.import_runs WHERE migration_id={L(m['wallet_migration_id'])} AND mapping_sha256={L(m['wallet_mapping_sha256'])} AND source_archive_sha256={L(manifest['source_archive_sha256'])} AND manifest->>'scope'={L(W.SCOPE)})
 OR NOT EXISTS("""
    return sql.replace(old,replacement,1).replace('SET LOCAL standard_conforming_strings=on;', 'SET LOCAL standard_conforming_strings=on;\nSET LOCAL synchronous_commit=on;',1)


def main():
    os.umask(0o077);p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--apply',action='store_true')
    for key in ['export','wallet-material','material']:p.add_argument('--'+key,type=Path)
    p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(mode=0o700)
    out={'target_write_attempted':False,'automatic_retry':False,'actual_exit':1}
    try:
        if not a.apply:
            m=prepare(W.load(a.export),W.load(a.wallet_material));W.save(a.output,'material.private.json',m)
            W.save(a.output,'history.private.sql',build_sql(m));out.update(actual_exit=0,status='PREPARED_NOT_APPLIED',mapped_rows=len(m['manifest']['rows']),unmapped_rows=m['manifest']['source_cohort']['unmapped_owner_row_count'])
        else:
            m=W.load(a.material);app='lmm-topup-cohort-'+uuid.uuid4().hex
            W.save(a.output,'attempt.private.json',{'application_name':app,'started_epoch':time.time(),'migration_id':m['manifest']['migration_id'],'material_sha256':W.digest(m)})
            out['target_write_attempted']=True;r=W.psql(m['connection'],build_sql(m),a.output,'apply',appname=app)
            if r.returncode==0:out.update(actual_exit=0,status='COMMITTED_HISTORY_COHORT',mapped_rows=len(m['manifest']['rows']),unmapped_rows=m['manifest']['source_cohort']['unmapped_owner_row_count'])
            else:out.update(status='OUTCOME_UNCONFIRMED',psql_exit=r.returncode)
    except Exception as e:out.update(status='OUTCOME_UNCONFIRMED' if out['target_write_attempted'] else 'REFUSED_BEFORE_WRITE',error=str(e))
    finally:W.save(a.output,'result.private.json',out)
    print(W.canonical(out));return out['actual_exit']


if __name__=='__main__':sys.exit(main())

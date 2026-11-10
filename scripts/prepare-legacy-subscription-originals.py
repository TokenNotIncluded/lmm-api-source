#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Prepare private, separately approved offline SQL; never connect to a database."""
import argparse
import json
import os
from pathlib import Path
import legacy_pancake_subscriptions as W

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('financial-material', 'source', 'review', 'migration-id', 'output-dir'):
        parser.add_argument('--'+name, required=True)
    parser.add_argument('--approval')
    args = parser.parse_args()
    def load(path):
        # This reader accepts the pinned private export, not a live SQL source.
        return json.loads(Path(path).read_text())
    m = W.prepare_material(load(args.financial_material), load(args.source), load(args.review), migration_id=args.migration_id)
    sql = W.build_apply_sql(m)
    if args.approval:
        W.verify_approval(m, sql, load(args.approval))
    out = Path(args.output_dir)
    out.mkdir(mode=0o700, parents=True, exist_ok=True)
    files = {'material.json': json.dumps(m, ensure_ascii=False, indent=2)+'\n', 'apply.sql': sql,
             'approval-preview.json': json.dumps({'scope': W.SCOPE, 'decision': 'REVIEW_REQUIRED',
                'material_sha256': W.F.digest(m), 'sql_sha256': W.F.sha(sql.encode()),
                'review_sha256': W.F.digest(m['review'])}, indent=2)+'\n'}
    W.require(sum(len(v.encode()) for v in files.values()) < 16*1024*1024, 'original_output_too_large')
    # Preflight all names. Never replace existing approval/recovery material.
    W.require(not any((out/name).exists() for name in files), 'original_output_exists')
    for name, text in files.items():
        fd = os.open(out/name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as f:
            f.write(text)
    print(json.dumps({'status': 'prepared-not-applied', 'records': len(m['records']),
                      'approval_verified': bool(args.approval), 'sql_sha256': W.F.sha(sql.encode())}))

if __name__ == '__main__':
    main()

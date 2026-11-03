#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Offline legacy Hero SMS ciphertext audit; never output plaintext or its hashes.

Codec follows TokenNotIncluded/lmm-api 3948a0a1ee1cefdd016a7aa45a11851545bb4b85,
apps/api-go/common/persistent_secret.go and model/hero_sms_email.go (AGPL-3.0).
The key file contains the effective original HERO_SMS_ENCRYPTION_KEY, or the
original CRYPTO_SECRET only when the primary was unset. No other key is tried.
"""
import argparse
import base64
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import stat

from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

FIELDS = ('phone_ciphertext', 'code_ciphertext', 'message_ciphertext', 'provider_snapshot_ciphertext')
PURPOSE = 'hero_sms.payload'


class PayloadError(ValueError):
    pass


def private_bytes(path, maximum):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, 'rb') as f:
        info = os.fstat(f.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) not in (0o400, 0o600):
            raise PayloadError('private_file_permissions_invalid')
        data = f.read(maximum + 1)
        if len(data) > maximum:
            raise PayloadError('private_file_too_large')
        return data


def legacy_payload_key(secret):
    secret = secret.strip()
    if len(secret.encode('utf-8')) < 32 or len(set(secret)) < 4 or any(
        marker in secret.lower() for marker in ('replace_with', 'random_string', 'your_secret', 'example-secret', 'change-me', 'changeme')
    ):
        raise PayloadError('legacy_key_material_invalid')
    return hashlib.sha256((PURPOSE + ':' + secret).encode('utf-8')).digest()


def envelope(value):
    if not isinstance(value, str) or not value.strip().startswith('v1:'):
        raise PayloadError('envelope_invalid')
    encoded = value.strip()[3:]
    if re.fullmatch('[A-Za-z0-9_-]+', encoded) is None:
        raise PayloadError('envelope_invalid')
    try:
        raw = base64.b64decode(encoded + '=' * (-len(encoded) % 4), altchars=b'-_', validate=True)
    except ValueError:
        raise PayloadError('envelope_invalid') from None
    if len(raw) < 28:
        raise PayloadError('payload_truncated')
    return raw[:12], raw[12:]


def decrypt_payload(value, key):
    nonce, body = envelope(value)
    try:
        plaintext = AESGCM(key).decrypt(nonce, body, None)
    except InvalidTag:
        raise PayloadError('authentication_failed') from None
    try:
        return plaintext.decode('utf-8')
    except UnicodeDecodeError:
        raise PayloadError('plaintext_encoding_invalid') from None


def verify(manifest, key=None):
    if not isinstance(manifest, dict) or manifest.get('scope') != 'hero-sms-history.v1':
        raise PayloadError('manifest_scope_invalid')
    tables = manifest.get('tables')
    if not isinstance(tables, dict):
        raise PayloadError('manifest_tables_invalid')
    orders = tables.get('hero_sms_sms_orders')
    if not isinstance(orders, list):
        raise PayloadError('manifest_orders_invalid')
    counts = {field: {'encrypted': 0, 'authenticated': 0, 'empty': 0} for field in FIELDS}
    failures = Counter()
    for order in orders:
        if not isinstance(order, dict):
            raise PayloadError('manifest_order_invalid')
        for field in FIELDS:
            if field not in order:
                failures['ciphertext_field_missing'] += 1
                continue
            value = order[field]
            if value is None or (isinstance(value, str) and not value.strip()):
                counts[field]['empty'] += 1
                continue
            counts[field]['encrypted'] += 1
            try:
                envelope(value)
                if key is not None:
                    decrypt_payload(value, key)
                    counts[field]['authenticated'] += 1
            except PayloadError as error:
                failures[str(error)] += 1
    return {'status': 'FAILED' if failures else 'KEY_REQUIRED' if key is None else 'VERIFIED',
            'orders': len(orders), 'fields': counts, 'failures': dict(failures),
            'encrypted_cells': sum(v['encrypted'] for v in counts.values()),
            'authenticated_cells': sum(v['authenticated'] for v in counts.values()),
            'plaintexts_written': False, 'plaintext_hashes_written': False,
            'supplier_requests': 0, 'native_supplier_acceptance': False}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--manifest', required=True, type=Path)
    p.add_argument('--key-file', type=Path)
    p.add_argument('--report', required=True, type=Path)
    args = p.parse_args()
    raw = private_bytes(args.manifest, 16 * 1024 * 1024)
    key = legacy_payload_key(private_bytes(args.key_file, 4096).decode('utf-8')) if args.key_file else None
    result = verify(json.loads(raw), key)
    result['manifest_file_sha256'] = hashlib.sha256(raw).hexdigest()
    with os.fdopen(os.open(args.report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as f:
        f.write(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
    return 0 if result['status'] == 'VERIFIED' else 2


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (PayloadError, OSError, UnicodeError, json.JSONDecodeError) as error:
        # No paths, key material, ciphertexts or decrypted values in diagnostics.
        print(json.dumps({'status':'FAILED','code':str(error) if isinstance(error,PayloadError) else 'private_input_or_report_unavailable'}))
        raise SystemExit(2) from None

#!/usr/bin/env python3
"""Prepare private, disabled Core ProviderWrite requests from an explicit owner export.

No database, HTTP, discovery, credential search or production operation occurs.
Input contains source_revision/source_archive_sha256, oidc (the legacy settings
object), custom_oauth_providers (owner-exported rows), and optional oidc_metadata
plus expected_oidc_issuer. Metadata is an explicit owner capture, not a live probe.
The existing POST /api/user/federated-providers endpoint is the import path; it
enforces administrator sessions, MFA, encrypted secrets and operation receipts.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import stat
from urllib.parse import urlsplit

SOURCE_REVISION = "cc52aa195c6f2a01d703a1a407be73c31b89867d"
FIELDS = ("client_id", "authorization_endpoint", "token_endpoint", "user_info_endpoint",
          "scopes", "well_known", "user_id_field", "username_field", "display_name_field",
          "email_field", "auth_style", "access_policy", "access_denied_message")
ALGORITHMS = {"RS256": "RSA", "RS384": "RSA", "RS512": "RSA",
              "PS256": "RSA", "PS384": "RSA", "PS512": "RSA",
              "ES256": "EC", "ES384": "EC", "EdDSA": "OKP"}


def valid_jwk(key, algorithm):
    from cryptography.hazmat.primitives.asymmetric import ec, ed25519, rsa
    try:
        def raw(name):
            text = key[name]
            require(isinstance(text, str) and re.fullmatch(r"[A-Za-z0-9_-]+", text), "jwk_invalid")
            return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))
        if key["kty"] == "RSA":
            n, e = int.from_bytes(raw("n")), int.from_bytes(raw("e"))
            require(n.bit_length() >= 2048, "jwk_invalid")
            rsa.RSAPublicNumbers(e, n).public_key()
        elif key["kty"] == "EC":
            curve, crv = {"ES256": (ec.SECP256R1(), "P-256"),
                          "ES384": (ec.SECP384R1(), "P-384")}[algorithm]
            require(key["crv"] == crv, "jwk_invalid")
            ec.EllipticCurvePublicNumbers(int.from_bytes(raw("x")), int.from_bytes(raw("y")), curve).public_key()
        else:
            require(algorithm == "EdDSA" and key["crv"] == "Ed25519", "jwk_invalid")
            ed25519.Ed25519PublicKey.from_public_bytes(raw("x"))
        return True
    except (KeyError, TypeError, ValueError):
        return False


def require(ok, code):
    if not ok:
        raise ValueError(code)


def endpoint(value, *, issuer=False):
    require(isinstance(value, str), "provider_endpoint_invalid")
    parsed = urlsplit(value)
    require(parsed.scheme == "https" and parsed.hostname and not parsed.username
            and not parsed.password and not parsed.fragment and not (issuer and parsed.query),
            "provider_endpoint_invalid")
    return value


def oidc_configuration(source):
    metadata = source.get("oidc_metadata")
    if metadata is None:
        return None
    expected = source.get("expected_oidc_issuer")
    require(isinstance(metadata, dict) and expected and metadata.get("issuer") == expected,
            "oidc_issuer_mismatch")
    issuer = endpoint(expected, issuer=True)
    jwks_uri = endpoint(metadata.get("jwks_uri"))
    algorithms = metadata.get("id_token_signing_alg_values_supported")
    require(isinstance(algorithms, list) and algorithms and len(algorithms) <= 16
            and all(isinstance(a, str) and a in ALGORITHMS for a in algorithms),
            "oidc_algorithms_unverified")
    jwks = metadata.get("jwks")
    require(isinstance(jwks, dict) and isinstance(jwks.get("keys"), list) and jwks["keys"],
            "oidc_jwks_missing")
    for algorithm in algorithms:
        require(any(k.get("kty") == ALGORITHMS[algorithm]
                    and k.get("alg", algorithm) == algorithm
                    and k.get("use", "sig") == "sig"
                    and ("key_ops" not in k or "verify" in k["key_ops"])
                    and valid_jwk(k, algorithm)
                    for k in jwks["keys"] if isinstance(k, dict)), "oidc_jwks_algorithm_mismatch")
    return {"issuer": issuer, "jwks_uri": jwks_uri, "id_token_algorithms": algorithms,
            **{k: endpoint(metadata.get(k)) for k in
               ("authorization_endpoint", "token_endpoint", "userinfo_endpoint")}}


def plan(source):
    require(source.get("source_revision") == SOURCE_REVISION
            and re.fullmatch(r"[0-9a-f]{64}", source.get("source_archive_sha256", "")),
            "provider_source_binding_invalid")
    requests, blocked, seen = [], [], set()

    def append(kind, row, source_id, configuration=None):
        slug = row.get("slug", "oidc" if kind == "oidc" else "")
        require(re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", slug or "") and slug not in seen,
                "provider_slug_invalid_or_duplicate")
        require(kind == "oidc" or slug not in
                ("oidc", "wechat", "github", "discord", "linuxdo", "telegram", "email", "state", "passkey"),
                "provider_slug_reserved")
        seen.add(slug)
        secret, client = row.get("client_secret"), row.get("client_id")
        if not isinstance(secret, str) or not secret or not isinstance(client, str) or not client:
            blocked.append({"source_id": source_id, "reason": "owner_client_credentials_required"})
            return
        config = {k: row[k] for k in FIELDS if k in row}
        if configuration:
            config.update(configuration)
            config["user_info_endpoint"] = config.pop("userinfo_endpoint")
        for field in ("authorization_endpoint", "token_endpoint", "user_info_endpoint"):
            endpoint(config.get(field))
        if config.get("well_known"):
            endpoint(config["well_known"])
        binding = f"{source['source_archive_sha256']}:{kind}:{source_id}"
        body = {"operation_key": "legacy-provider-" + hashlib.sha256(binding.encode()).hexdigest(),
                "kind": kind, "slug": slug, "name": row.get("name") or slug,
                "icon": row.get("icon", ""), "enabled": False,
                "client_secret": secret, **config}
        requests.append({"source_id": source_id, "source_enabled": row.get("enabled", False),
                         "method": "POST", "path": "/api/user/federated-providers", "body": body})

    oidc = source.get("oidc")
    if oidc:
        configuration = oidc_configuration(source)
        if configuration is None:
            blocked.append({"source_id": "oidc", "reason": "owner_issuer_discovery_jwks_capture_required"})
        else:
            append("oidc", {**oidc, "name": oidc.get("display_name") or "OIDC"}, "oidc", configuration)
    for row in source.get("custom_oauth_providers", []):
        require(type(row.get("id")) is int and row["id"] > 0, "provider_source_id_invalid")
        append("custom_oauth", row, row["id"])
    return {"scope": "legacy-provider-config-owner-plan.v1",
            "source_revision": SOURCE_REVISION, "source_archive_sha256": source["source_archive_sha256"],
            "requests": requests, "blocked": blocked, "automatically_enabled": 0,
            "live_discovery_performed": False, "target_writes": 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        st = args.source.lstat()
        require(args.source.is_absolute() and stat.S_ISREG(st.st_mode)
                and stat.S_IMODE(st.st_mode) in (0o400, 0o600) and st.st_uid == os.getuid(),
                "provider_source_must_be_owner_private")
        parent = args.output.parent
        require(args.output.is_absolute() and not parent.is_symlink() and parent.is_dir()
                and stat.S_IMODE(parent.stat().st_mode) == 0o700 and parent.stat().st_uid == os.getuid(),
                "provider_output_directory_must_be_owner_private")
        raw = args.source.read_bytes()
        result = plan(json.loads(raw))
        result["owner_export_sha256"] = hashlib.sha256(raw).hexdigest()
        fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as out:
            json.dump(result, out, ensure_ascii=False, indent=2)
            out.write("\n")
        print(json.dumps({"planned": len(result["requests"]), "blocked": len(result["blocked"]),
                          "enabled": 0, "target_writes": 0}))
    except (ValueError, OSError, TypeError, KeyError):
        parser.exit(1, "error: provider_private_plan_invalid_or_output_exists\n")


if __name__ == "__main__":
    main()

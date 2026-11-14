"""Legacy COSE public keys -> webauthn-rs 0.5.5 serialised public-key types.

go-v0.2.118 model/passkey.go stores go-webauthn Credential.PublicKey as CBOR.
Only combinations supported by the pinned Rust verifier are accepted.
"""
import base64
import io


def legacy_cose_key(raw: bytes) -> dict:
    import cbor2

    def fail():
        raise SystemExit("error: passkey_cose_key_unsupported")

    if not 1 <= len(raw) <= 4096:
        fail()
    stream = io.BytesIO(raw)
    try:
        key = cbor2.CBORDecoder(stream, max_depth=4, allow_duplicate_keys=False).decode()
    except (ValueError, TypeError, cbor2.CBORDecodeError):
        fail()
    if stream.read() or type(key) is not dict or any(type(k) is not int for k in key):
        fail()
    if type(key.get(1)) is not int or type(key.get(3)) is not int:
        fail()

    def coordinate(label, size):
        value = key.get(label)
        if type(value) is not bytes or len(value) != size:
            fail()
        return base64.urlsafe_b64encode(value).rstrip(b"=").decode()

    curves = {(-7, 1): ("ES256", "SECP256R1", 32),
              (-35, 2): ("ES384", "SECP384R1", 48),
              (-36, 3): ("ES512", "SECP521R1", 66)}
    if key[1] == 2 and type(key.get(-1)) is int and (key[3], key[-1]) in curves:
        alg, curve, size = curves[key[3], key[-1]]
        return {"type_": alg, "key": {"EC_EC2": {
            "curve": curve, "x": coordinate(-2, size), "y": coordinate(-3, size)}}}
    if key[1] == 3 and key[3] == -257:
        modulus = coordinate(-1, 256)
        coordinate(-2, 3)
        return {"type_": "RS256", "key": {"RSA": {"n": modulus, "e": list(key[-2])}}}
    if key[1] == 1 and key[3] == -8 and type(key.get(-1)) is int and key[-1] == 6:
        return {"type_": "EDDSA", "key": {"EC_OKP": {
            "curve": "ED25519", "x": coordinate(-2, 32)}}}
    fail()

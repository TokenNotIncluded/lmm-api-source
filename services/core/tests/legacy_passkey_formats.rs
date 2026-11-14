//! Public synthetic keys converted by the Python migration must load in pinned Rust WebAuthn.
#[test]
fn migrated_cose_algorithms_load_as_passkeys_and_valid_public_keys() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/legacy-passkey-formats.json")).unwrap();
    assert_eq!(rows.len(), 5);
    for row in rows {
        let key = &row["passkey"]["cred"]["cred"];
        let _: webauthn_rs::prelude::Passkey =
            serde_json::from_value(row["passkey"].clone()).unwrap();
        let cose: webauthn_rs_core::proto::COSEKey = serde_json::from_value(key.clone()).unwrap();
        cose.get_openssl_pkey()
            .expect("migrated public key must be valid");
    }
}

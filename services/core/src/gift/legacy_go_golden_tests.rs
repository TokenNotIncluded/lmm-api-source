//! Independent synthetic reference output from the original cc52 Go functions.
//! The reference program keeps Go 1.25.1 Unicode 15 tables, with its host Go
//! version and original source hashes recorded in the fixture itself.
use super::*;
use sha2::Digest;

const GOLDENS: &str = include_str!("../../tests/fixtures/legacy_gift_go125_goldens.json");

fn hex(value: impl AsRef<[u8]>) -> String {
    crate::hex_codec::encode(value)
}

#[test]
fn original_go_redaction_identity_network_and_hmac_goldens() {
    assert_eq!(
        hex(Sha256::digest(GOLDENS.as_bytes())),
        "a17bb371cb050bb8bb9e5de4a7526e3634a468ec9a93a583d5eb3cd1e6754102"
    );
    let source: serde_json::Value = serde_json::from_str(GOLDENS).unwrap();
    assert_eq!(source["production_data_used"], false);
    assert_eq!(
        source["source_revision"],
        "cc52aa195c6f2a01d703a1a407be73c31b89867d"
    );
    let secret = source["synthetic_secret"].as_str().unwrap().as_bytes();
    let cases = source["redaction_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 60);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let input = case["input"].as_str().unwrap();
        let redacted = redact(input);
        assert_eq!(
            redacted,
            case["redacted"].as_str().unwrap(),
            "redaction {id}"
        );
        assert_eq!(
            hex(Sha256::digest(redacted.as_bytes())),
            case["redacted_sha256"].as_str().unwrap(),
            "redaction hash {id}"
        );
        let (digest, tokens) = fingerprints(secret, input).unwrap();
        assert_eq!(
            hex(digest),
            case["digest"].as_str().unwrap(),
            "message {id}"
        );
        let actual: Vec<_> = tokens.into_iter().map(hex).collect();
        let expected: Vec<_> = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        assert_eq!(actual, expected, "tokens {id}");
    }
    let cases = source["identity_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 28);
    let installation = super::super::Installation {
        namespace: "independent-go-reference-only".into(),
        secret: secret.to_vec(),
        oidc_issuer: None,
        enabled: true,
        cap: 1001,
    };
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let input = case["email_input"].as_str().unwrap();
        let normalized: String = input
            .trim_matches(unicode::space)
            .chars()
            .map(unicode::lower)
            .collect();
        assert_eq!(
            normalized,
            case["normalized_email"].as_str().unwrap(),
            "normalization {id}"
        );
        let email = canonical_email(input);
        assert_eq!(
            email.as_deref().unwrap_or(""),
            case["canonical_email"].as_str().unwrap(),
            "email {id}"
        );
        // The original helper trims its textual IP before net.ParseIP. Core receives IpAddr.
        let ip = case["ip_input"]
            .as_str()
            .unwrap()
            .trim_matches(unicode::space)
            .parse::<IpAddr>()
            .ok();
        let canonical = ip.map(|value| match value {
            IpAddr::V6(v) => v.to_ipv4_mapped().map_or(value, IpAddr::V4),
            value => value,
        });
        assert_eq!(
            canonical.map(|v| v.to_string()).unwrap_or_default(),
            case["canonical_ip"].as_str().unwrap(),
            "network {id}"
        );
        let accepted = email.is_some() && ip.is_some();
        assert_eq!(
            accepted,
            case["accepted"].as_bool().unwrap(),
            "identity acceptance {id}"
        );
        if let (Some(email), Some(ip)) = (email, ip) {
            assert_eq!(
                hex(hash(secret, "assistant-gift-identity-v1:", &email).unwrap()),
                case["identity_hmac"].as_str().unwrap(),
                "identity HMAC {id}"
            );
            assert_eq!(
                hex(super::super::network(&installation, ip).unwrap()),
                case["network_hmac"].as_str().unwrap(),
                "network HMAC {id}"
            );
        }
    }
}

#[test]
fn all_unicode_scalars_match_original_go15_letter_digit_space_and_lower() {
    let source: serde_json::Value = serde_json::from_str(GOLDENS).unwrap();
    assert_eq!(source["unicode"]["source_unicode_version"], "15.0.0");
    let mut letter_digit = Sha256::new();
    let mut whitespace = Sha256::new();
    let mut lowercase = Sha256::new();
    let (mut scalars, mut letters_digits, mut spaces, mut changed) = (0u64, 0u64, 0u64, 0u64);
    for codepoint in 0..=0x10ffffu32 {
        let Some(c) = char::from_u32(codepoint) else {
            continue;
        };
        let ld = unicode::letter_or_digit(c);
        let space = unicode::space(c);
        let lower = unicode::lower(c) as u32;
        letter_digit.update(codepoint.to_be_bytes());
        letter_digit.update([u8::from(ld)]);
        whitespace.update(codepoint.to_be_bytes());
        whitespace.update([u8::from(space)]);
        lowercase.update(codepoint.to_be_bytes());
        lowercase.update(lower.to_be_bytes());
        scalars += 1;
        letters_digits += u64::from(ld);
        spaces += u64::from(space);
        changed += u64::from(lower != codepoint);
    }
    assert_eq!(
        (scalars, letters_digits, spaces, changed),
        (1_112_064, 136_784, 25, 1433)
    );
    for (name, observed) in [
        ("letter_or_digit", letter_digit),
        ("space", whitespace),
        ("lower", lowercase),
    ] {
        assert_eq!(
            hex(observed.finalize()),
            source["unicode"]["sha256"][name].as_str().unwrap(),
            "full scalar reference {name}"
        );
    }
}

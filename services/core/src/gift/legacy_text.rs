//! Exact cc52 registration-v1 correlation. Ported from model/assistant_history.go
//! and internal/registrationguard/policy.go (AGPL-3.0-or-later, LIghtJUNction).
//! The source Go 1.25.1 Unicode tables are frozen separately: an upgraded Rust
//! toolchain must not change an existing user's historical comparison keys.
use super::Error;
use hmac::{Hmac, KeyInit as _, Mac};
use regex::{Captures, Regex};
use sha2::Sha256;
use std::{collections::BTreeSet, net::IpAddr, sync::LazyLock};

#[path = "legacy_unicode.rs"]
mod unicode;

pub(super) fn hash(secret: &[u8], domain: &str, value: &str) -> Result<Vec<u8>, Error> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| Error::Unavailable)?;
    mac.update(domain.as_bytes());
    mac.update(value.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

pub(super) fn canonical_email(value: &str) -> Option<String> {
    let value: String = value
        .trim_matches(unicode::space)
        .chars()
        .map(unicode::lower)
        .collect();
    let (local, domain) = value.split_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return None;
    }
    let domain = if domain == "googlemail.com" {
        "gmail.com"
    } else {
        domain
    };
    let local = match domain {
        "gmail.com" => local.replace('.', "").split('+').next()?.to_string(),
        "hotmail.com" | "live.com" | "outlook.com" => local.split('+').next()?.to_string(),
        _ => local.to_string(),
    };
    (!local.is_empty()).then(|| format!("{local}@{domain}"))
}

// Go's regexp Perl classes and word boundaries are ASCII. Rust's default ones
// are Unicode, so their exact ASCII spelling is intentional here.
static REPLACEMENTS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----", "[REDACTED_PRIVATE_KEY]"),
        (r"(?i)([?&](?:api[_-]?key|access[_-]?token|token|password|passwd|secret)=)[^&#\t\n\x0c\r ]+", "${1}[REDACTED]"),
        (r"(?i)(?-u:\b)(cookie|set-cookie|session(?:[_ -]?id)?|csrf(?:[_ -]?token)?)[\t\n\x0c\r ]*[:=：][\t\n\x0c\r ]*[^\t\n\x0c\r ;,]+", "${1}: [REDACTED]"),
        (r"(?i)(?-u:\b)bearer[\t\n\x0c\r ]+[a-z0-9._~+/-]{6,}=*", "Bearer [REDACTED_TOKEN]"),
        (r"(?i)(?-u:\b)(password|passwd|pwd|api[ _-]?key|access[ _-]?token|refresh[ _-]?token|bearer|authorization|密碼|密码|密钥|令牌)[\t\n\x0c\r ]*[:=：][\t\n\x0c\r ]*[^\t\n\x0c\r ,;]+", "${1}: [REDACTED]"),
        (r"(?i)\[(model_providers|providers)\.[^\]\r\n]*(?:sk|rk|pk|ak)[_-][a-z0-9._~+/-]{20,}[^\]\r\n]*\]", "[${1}.REDACTED_API_KEY]"),
        (r"(?i)(?-u:\b)(?:sk|rk|pk|ak|tok|token|key|secret)[_-][a-z0-9._~+/-]{8,}(?-u:\b)", "[REDACTED_API_KEY]"),
        (r"(?-u:\b)eyJ[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}(?-u:\b)", "[REDACTED_TOKEN]"),
        (r"(?i)(?-u:\b)[a-z0-9.!#$%&'*+/=?^_\x60{|}~-]+@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+(?-u:\b)", "[REDACTED_EMAIL]"),
        (r"(^|[^a-zA-Z0-9_])((?:\+?86[\t\n\x0c\r -]?)?1[3-9][0-9]{9}|\+[0-9]{1,3}(?:[\t\n\x0c\r .-]?[0-9]{2,4}){2,4})([^a-zA-Z0-9_]|$)", "${1}[REDACTED_PHONE]${3}"),
    ]
    .into_iter()
    .map(|(pattern, replacement)| (Regex::new(pattern).expect("fixed cc52 redaction pattern"), replacement))
    .collect()
});
static IPV4: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)(?:[0-9]{1,3}\.){3}[0-9]{1,3}(?-u:\b)").unwrap());
static IPV6: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[0-9A-Fa-f:]{2,39}").unwrap());
static CARD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|[^0-9])([0-9][0-9 -]{11,22}[0-9])([^0-9]|$)").unwrap());

fn luhn(digits: &str) -> bool {
    let mut sum = 0;
    for (index, digit) in digits.bytes().rev().enumerate() {
        let mut value = u32::from(digit - b'0');
        if index % 2 == 1 {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
    }
    sum % 10 == 0
}

pub(super) fn redact(text: &str) -> String {
    let mut value = text.trim_matches(unicode::space).to_string();
    for (pattern, replacement) in REPLACEMENTS.iter() {
        value = pattern.replace_all(&value, *replacement).into_owned();
    }
    value = IPV4
        .replace_all(&value, |m: &Captures<'_>| {
            if m[0].parse::<IpAddr>().is_ok() {
                "[REDACTED_IP]".to_string()
            } else {
                m[0].to_string()
            }
        })
        .into_owned();
    value = IPV6
        .replace_all(&value, |m: &Captures<'_>| {
            if m[0].contains(':') && m[0].parse::<IpAddr>().is_ok() {
                "[REDACTED_IP]".to_string()
            } else {
                m[0].to_string()
            }
        })
        .into_owned();
    CARD.replace_all(&value, |m: &Captures<'_>| {
        let digits: String = m[2].chars().filter(char::is_ascii_digit).collect();
        if (13..=19).contains(&digits.len()) && luhn(&digits) {
            format!("{}[REDACTED_CARD]{}", &m[1], &m[3])
        } else {
            m[0].to_string()
        }
    })
    .into_owned()
}

pub(super) fn fingerprints(secret: &[u8], text: &str) -> Result<(Vec<u8>, Vec<Vec<u8>>), Error> {
    if secret.is_empty() {
        return Err(Error::Invalid);
    }
    let chars: Vec<_> = redact(text)
        .chars()
        .map(unicode::lower)
        .filter(|c| unicode::letter_or_digit(*c))
        .take(2048)
        .collect();
    if chars.len() < 40 {
        return Ok((vec![], vec![]));
    }
    let digest = hash(
        secret,
        "registration-v1:message:",
        &chars.iter().collect::<String>(),
    )?;
    let mut tokens = BTreeSet::new();
    for index in (0..=chars.len() - 20).step_by(4) {
        tokens.insert(hash(
            secret,
            "registration-v1:shingle:",
            &chars[index..index + 20].iter().collect::<String>(),
        )?);
    }
    Ok((digest, tokens.into_iter().take(8).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_source_email_and_unicode_are_not_native_qualification() {
        assert_eq!(
            canonical_email(" A.B+c@Googlemail.Com ").as_deref(),
            Some("ab@gmail.com")
        );
        assert_eq!(
            canonical_email("a@mailinator.com").as_deref(),
            Some("a@mailinator.com")
        );
        assert_eq!(
            canonical_email("a b@example.test").as_deref(),
            Some("a b@example.test")
        );
        assert_eq!(canonical_email("a@b@c"), None);
        assert_eq!(unicode::lower('İ'), 'i');
        assert!(!unicode::letter_or_digit('Ⅳ'));
        assert!(!unicode::letter_or_digit('\u{1c89}')); // Added after source Unicode 15.
        assert!(unicode::letter_or_digit('中'));
        assert!(unicode::letter_or_digit('９'));
    }
    #[test]
    fn redaction_precedes_legacy_correlation() {
        let a = "Please help me configure the existing desktop application with an endpoint and a correct request. email=a@example.test password=one-private-value";
        let b = "Please help me configure the existing desktop application with an endpoint and a correct request. email=b@example.test password=another-private-value";
        assert_eq!(
            fingerprints(b"synthetic-key", a).unwrap(),
            fingerprints(b"synthetic-key", b).unwrap()
        );
        assert_eq!(
            redact("192.0.2.1 ::1 4111 1111 1111 1111"),
            "[REDACTED_IP] [REDACTED_IP] [REDACTED_CARD]"
        );
        assert!(
            fingerprints(b"synthetic-key", "short message")
                .unwrap()
                .1
                .is_empty()
        );
        assert_eq!(fingerprints(b"synthetic-key", a).unwrap().1.len(), 8);
    }
}

#[cfg(test)]
#[path = "legacy_go_golden_tests.rs"]
mod go_reference;

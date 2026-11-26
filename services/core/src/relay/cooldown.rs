//! Bounded recovery time from the configured upstream's 429 response.
use axum::http::{HeaderMap, header};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_COOLDOWN: Duration = Duration::from_secs(86_400);

fn decimal(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

pub(crate) fn deadline(headers: &HeaderMap, now: SystemTime) -> Option<SystemTime> {
    let (name, relative) = if headers.contains_key(header::RETRY_AFTER) {
        ("retry-after", true)
    } else {
        ("x-ratelimit-reset", false)
    };
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?.trim();
    if value.len() > 128 || values.next().is_some() {
        return None;
    }
    let until = if relative {
        match decimal(value) {
            Some(seconds) => now.checked_add(Duration::from_secs(seconds))?,
            None => httpdate::parse_http_date(value).ok()?,
        }
    } else {
        UNIX_EPOCH.checked_add(Duration::from_secs(decimal(value)?))?
    };
    let delay = until.duration_since(now).ok()?;
    (!delay.is_zero() && delay <= MAX_COOLDOWN).then_some(until)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn parses_only_unambiguous_future_upstream_deadlines_within_one_day() {
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut h = HeaderMap::new();
        h.insert("x-ratelimit-reset", HeaderValue::from_static("1700000020"));
        assert_eq!(deadline(&h, now), Some(now + Duration::from_secs(20)));
        h.insert(header::RETRY_AFTER, HeaderValue::from_static("10"));
        assert_eq!(deadline(&h, now), Some(now + Duration::from_secs(10)));
        for value in [
            "0",
            "-1",
            "+1",
            "1, 2",
            "NaN",
            "86401",
            "99999999999999999999999",
        ] {
            h.insert(header::RETRY_AFTER, HeaderValue::from_str(value).unwrap());
            assert_eq!(deadline(&h, now), None, "{value}");
        }
        for seconds in [1, 86400] {
            let expected = now + Duration::from_secs(seconds);
            h.insert(
                header::RETRY_AFTER,
                httpdate::fmt_http_date(expected).parse().unwrap(),
            );
            assert_eq!(deadline(&h, now), Some(expected));
        }
        h.append(header::RETRY_AFTER, HeaderValue::from_static("4"));
        assert_eq!(deadline(&h, now), None);
        h.remove(header::RETRY_AFTER);
        h.insert("x-ratelimit-reset", HeaderValue::from_static("1699999999"));
        assert_eq!(deadline(&h, now), None);
    }
}

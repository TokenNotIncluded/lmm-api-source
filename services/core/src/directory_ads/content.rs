use super::*;
use std::net::IpAddr;
pub fn content_digest(name: &str, url: &str, summary: &str, description: &str) -> String {
    let mut b = b"lmm.directory-ad.content.v1\n".to_vec();
    for s in [name, url, summary, description] {
        b.extend_from_slice(format!("{}:", s.len()).as_bytes());
        b.extend_from_slice(s.as_bytes());
        b.push(b'\n');
    }
    hash(&b)
}
fn text(s: &str, max: usize, required: bool) -> bool {
    s == s.trim_matches([' ', '\t', '\r', '\n'])
        && (!required || !s.is_empty())
        && s.chars().count() <= max
        && !s.chars().any(|c| c < ' ' || c == '\u{7f}')
}
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (b == 18 || b == 19))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            ip.to_ipv4_mapped()
                .map(|v| public_ip(IpAddr::V4(v)))
                .unwrap_or_else(|| s[0] & 0xe000 == 0x2000 && !(s[0] == 0x2001 && s[1] == 0xdb8))
        }
    }
}
pub(super) fn validate(v: &pb::PrepareDirectoryAdRequest) -> Result<()> {
    if v.protocol_major != 1
        || v.content_version != 1
        || !key(&v.request_id)
        || !(100..=1_000_000).contains(&v.bid_cents)
        || !text(&v.name, 80, true)
        || !text(&v.summary, 180, false)
        || !text(&v.description, 1200, false)
        || !text(&v.url, 2048, true)
        || v.url.len() > 2048
        || v.url.chars().any(char::is_whitespace)
        || v.content_sha256 != content_digest(&v.name, &v.url, &v.summary, &v.description)
    {
        return Err(Error::Invalid);
    }
    let u = reqwest::Url::parse(&v.url).map_err(|_| Error::Invalid)?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() {
        return Err(Error::Invalid);
    }
    let h = u.host_str().ok_or(Error::Invalid)?;
    if let Ok(ip) = h.trim_matches(['[', ']']).parse::<IpAddr>() {
        if !public_ip(ip) {
            return Err(Error::Invalid);
        }
    } else if !h.contains('.')
        || h.ends_with('.')
        || [".localhost", ".local", ".internal", ".test"]
            .iter()
            .any(|suffix| h.ends_with(suffix))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

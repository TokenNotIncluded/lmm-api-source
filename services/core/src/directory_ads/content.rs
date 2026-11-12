use super::*;
pub fn content_digest(name: &str, url: &str, summary: &str, description: &str) -> String {
    let mut b = b"lmm.directory-ad.content.v1\n".to_vec();
    for s in [name, url, summary, description] {
        b.extend_from_slice(format!("{}:", s.len()).as_bytes());
        b.extend_from_slice(s.as_bytes());
        b.push(b'\n');
    }
    hash(&b)
}
// Publication policy belongs to the community extension and its own database.
// Core only bounds the consent snapshot and binds it to the authorized charge.
pub(super) fn validate_consent(v: &pb::PrepareDirectoryAdRequest) -> Result<()> {
    let fields = [&v.name, &v.url, &v.summary, &v.description];
    if v.protocol_major != 1
        || v.content_version != 1
        || !key(&v.request_id)
        || !(100..=1_000_000).contains(&v.bid_cents)
        || fields.iter().map(|s| s.len()).sum::<usize>() > (8 << 10)
        || fields
            .iter()
            .any(|s| s.chars().any(|c| c < ' ' || c == '\u{7f}'))
        || v.content_sha256 != content_digest(&v.name, &v.url, &v.summary, &v.description)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> pb::PrepareDirectoryAdRequest {
        let mut request = pb::PrepareDirectoryAdRequest {
            protocol_major: 1,
            content_version: 1,
            request_id: "advertisement-0001".into(),
            bid_cents: 100,
            // Deliberately not a publishable advertisement. This is consent
            // data, not a Core endorsement of the extension's publication.
            name: "x".repeat(81),
            url: "http://localhost".into(),
            ..Default::default()
        };
        bind_content(&mut request);
        request
    }
    fn bind_content(request: &mut pb::PrepareDirectoryAdRequest) {
        request.content_sha256 = content_digest(
            &request.name,
            &request.url,
            &request.summary,
            &request.description,
        );
    }
    #[test]
    fn consent_validation_leaves_publication_policy_to_extension() {
        let mut request = request();
        assert_eq!(validate_consent(&request), Ok(()));
        request.name.push('x');
        assert_eq!(validate_consent(&request), Err(Error::Invalid));
        bind_content(&mut request);
        assert_eq!(validate_consent(&request), Ok(()));
    }
    #[test]
    fn consent_validation_retains_charge_binding_and_input_bounds() {
        for mutate in [
            |r: &mut pb::PrepareDirectoryAdRequest| r.protocol_major = 2,
            |r: &mut pb::PrepareDirectoryAdRequest| r.content_version = 2,
            |r: &mut pb::PrepareDirectoryAdRequest| r.request_id = "short".into(),
            |r: &mut pb::PrepareDirectoryAdRequest| r.bid_cents = 99,
            |r: &mut pb::PrepareDirectoryAdRequest| r.bid_cents = 1_000_001,
            |r: &mut pb::PrepareDirectoryAdRequest| r.description = "x".repeat((8 << 10) + 1),
            |r: &mut pb::PrepareDirectoryAdRequest| r.description = "hidden\ntext".into(),
        ] {
            let mut request = request();
            mutate(&mut request);
            bind_content(&mut request);
            assert_eq!(validate_consent(&request), Err(Error::Invalid));
        }
    }
}

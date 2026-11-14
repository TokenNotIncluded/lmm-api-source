//! Mozilla WebPKI roots, matching the reqwest 0.12 transport trust boundary.
//!
//! reqwest 0.13 otherwise selects platform roots. Applications retain their
//! existing DNS, peer, hostname, redirect, proxy, and timeout policies.
pub(crate) fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().tls_certs_only(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().map(
        |certificate| {
            reqwest::Certificate::from_der(certificate.as_ref())
                .expect("upstream Mozilla root certificate is valid DER")
        },
    ))
}

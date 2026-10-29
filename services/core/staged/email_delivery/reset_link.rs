//! Reset mail links use only the explicitly configured Web origin.
use super::{MailError, Result};
use crate::identity::email::EmailAddress;
use reqwest::Url;
use zeroize::Zeroizing;

/// Validated operator configuration, never a request Host or forwarded header.
/// No Debug or Display: rendered messages contain a one-time recovery secret.
#[derive(Clone)]
pub struct ResetSite(Url);

impl ResetSite {
    /// Requires an absolute HTTPS origin, without a path, credentials or parameters.
    ///
    /// # Errors
    /// Rejects malformed URLs and any non-HTTPS origin.
    pub fn https(origin: &str) -> Result<Self> {
        Self::parse(origin, false)
    }

    /// Matches the existing explicit local Web HTTP profile.
    ///
    /// # Errors
    /// Rejects HTTP sites outside the existing exact local host allowlist.
    pub fn local_http(origin: &str) -> Result<Self> {
        Self::parse(origin, true)
    }

    fn parse(origin: &str, local_http: bool) -> Result<Self> {
        let prefix = if local_http { "http://" } else { "https://" };
        if origin.is_empty()
            || !origin.is_ascii()
            || !origin
                .get(..prefix.len())
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case(prefix))
            || origin
                .bytes()
                .any(|b| b.is_ascii_control() || b.is_ascii_whitespace() || b == b'\\')
        {
            return Err(MailError::Invalid);
        }
        let url = Url::parse(origin).map_err(|_| MailError::Invalid)?;
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port() == Some(0)
            || if local_http {
                url.scheme() != "http"
                    || !matches!(
                        url.host_str(),
                        Some("localhost" | "127.0.0.1" | "[::1]" | "api.lmm.local")
                    )
            } else {
                url.scheme() != "https"
            }
        {
            return Err(MailError::Invalid);
        }
        Ok(Self(url))
    }

    /// Renders the fixed plaintext reset message from an existing business proof.
    /// The URL fragment stays out of the initial HTTP request and access log.
    /// This method neither creates a proof nor extends its database deadline.
    ///
    /// # Errors
    /// Rejects anything except the existing 32-character lowercase reset proof.
    pub fn render(&self, email: &EmailAddress, token: &str) -> Result<Zeroizing<String>> {
        if token.len() != 32
            || !token
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(MailError::Invalid);
        }
        let mut url = self.0.clone();
        url.set_path("/user/reset");
        url.query_pairs_mut()
            .append_pair("email", email.as_str())
            .append_pair("token", token);
        let encoded = Zeroizing::new(url.query().ok_or(MailError::Invalid)?.to_owned());
        url.set_query(None);
        url.set_fragment(Some(&encoded));
        Ok(Zeroizing::new(format!(
            "To choose a new LMM password, open this link:\r\n{url}\r\nThis link expires with the original request (within 10 minutes) and can be used once. Opening it does not change your password.\r\nIf you did not request this, ignore this email.\r\n"
        )))
    }
}

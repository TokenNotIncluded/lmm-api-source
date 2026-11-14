//! Reuse Core's maintained address classification and origin-pinned transport.
//! No redirects, proxy, second DNS lookup, unsafe TLS, or caller-chosen origin.
use crate::{
    identity::{IdentityError, Result},
    relay::{
        Limits,
        honeypot_network::{self, NetworkTarget},
    },
};
use reqwest::{
    Method, Url,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::Semaphore,
    time::{Instant, timeout_at},
};
use zeroize::Zeroizing;

enum Connection {
    Shared(honeypot_network::PreparedConnection),
    Bridge(super::bridge_network::BridgeConnection),
}
impl Connection {
    fn client(&self) -> &reqwest::Client {
        match self {
            Self::Shared(connection) => &connection.client,
            Self::Bridge(connection) => &connection.client,
        }
    }
    fn matches_peer(&self, peer: Option<std::net::SocketAddr>) -> bool {
        match self {
            Self::Shared(connection) => connection.matches_peer(peer),
            Self::Bridge(connection) => connection.matches_peer(peer),
        }
    }
}

pub(super) struct ProviderNetwork {
    slots: Arc<Semaphore>,
    pub(super) profile_slots: Arc<Semaphore>,
    #[cfg(test)]
    local: bool,
    #[cfg(test)]
    test_ca: Option<reqwest::Certificate>,
}

struct ResponseBudget {
    deadline: Instant,
    max: usize,
}
impl ProviderNetwork {
    pub(super) fn new() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(2)),
            profile_slots: Arc::new(Semaphore::new(1)),
            #[cfg(test)]
            local: false,
            #[cfg(test)]
            test_ca: None,
        }
    }

    #[cfg(test)]
    pub(super) fn trusted_ca_for_test(ca: reqwest::Certificate) -> Self {
        let mut network = Self::new();
        assert!(!network.local);
        network.test_ca = Some(ca);
        network
    }

    fn additional_root(&self) -> Option<reqwest::Certificate> {
        #[cfg(test)]
        {
            self.test_ca.clone()
        }
        #[cfg(not(test))]
        {
            None
        }
    }

    fn local(&self) -> bool {
        #[cfg(test)]
        {
            self.local
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    pub(super) fn endpoint(&self, raw: &str) -> Result<Url> {
        self.checked_endpoint(raw, false)
    }

    pub(super) fn bridge_endpoint(&self, raw: &str) -> Result<Url> {
        self.checked_endpoint(raw, true)
    }

    fn checked_endpoint(&self, raw: &str, public_http: bool) -> Result<Url> {
        if raw.len() > 2048 || raw.contains('\\') || raw.chars().any(char::is_control) {
            return Err(IdentityError::Invalid);
        }
        let url = Url::parse(raw).map_err(|_| IdentityError::Invalid)?;
        let mut base = url.clone();
        base.set_query(None);
        // Reuse the maintained field/IP classifier for both schemes; the
        // original HTTP URL is used only by the separately pinned bridge client.
        if public_http && base.scheme() == "http" && !self.local() {
            base.set_scheme("https")
                .map_err(|_| IdentityError::Invalid)?;
        }
        honeypot_network::validate_base_url(base.as_str(), self.local())
            .map_err(|_| IdentityError::Invalid)?;
        Ok(url)
    }

    pub(super) async fn get(
        &self,
        url: &Url,
        authorization: Option<&str>,
        deadline: Instant,
        max: usize,
    ) -> Result<Zeroizing<String>> {
        self.request(
            Method::GET,
            url,
            authorization,
            None,
            ResponseBudget { deadline, max },
            false,
        )
        .await
    }

    pub(super) async fn bridge_get(
        &self,
        url: &Url,
        authorization: &str,
        deadline: Instant,
    ) -> Result<Zeroizing<String>> {
        self.request(
            Method::GET,
            url,
            Some(authorization),
            None,
            ResponseBudget {
                deadline: deadline.min(Instant::now() + Duration::from_secs(5)),
                max: 65536,
            },
            true,
        )
        .await
    }

    pub(super) async fn form(
        &self,
        url: &Url,
        authorization: Option<&str>,
        pairs: &[(String, String)],
        deadline: Instant,
    ) -> Result<Zeroizing<String>> {
        let encoded = Zeroizing::new(
            form_urlencoded::Serializer::new(String::new())
                .extend_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .finish(),
        );
        self.request(
            Method::POST,
            url,
            authorization,
            Some(&encoded),
            ResponseBudget {
                deadline,
                max: 1_048_576,
            },
            false,
        )
        .await
    }

    async fn request(
        &self,
        method: Method,
        url: &Url,
        authorization: Option<&str>,
        body: Option<&str>,
        budget: ResponseBudget,
        public_http: bool,
    ) -> Result<Zeroizing<String>> {
        let ResponseBudget { deadline, max } = budget;
        self.checked_endpoint(url.as_str(), public_http)?;
        timeout_at(deadline, async {
            let target = NetworkTarget {
                additional_root: self.additional_root(),
                local_http: self.local(),
                allowed_origins: [honeypot_network::canonical_origin(url)].into(),
            };
            let limits = Limits {
                connect_timeout: Duration::from_secs(3),
                ..Limits::default()
            };
            let connection = if public_http && url.scheme() == "http" && !self.local() {
                Connection::Bridge(
                    super::bridge_network::prepare(url, &limits, self.slots.clone()).await?,
                )
            } else {
                Connection::Shared(
                    target
                        .prepare(url, &limits, self.slots.clone())
                        .await
                        .map_err(|_| IdentityError::Unauthorized)?,
                )
            };
            let mut headers = HeaderMap::new();
            headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
            if let Some(value) = authorization {
                let mut value = HeaderValue::from_str(value).map_err(|_| IdentityError::Invalid)?;
                value.set_sensitive(true);
                headers.insert(AUTHORIZATION, value);
            }
            let mut request = connection
                .client()
                .request(method, url.clone())
                .headers(headers);
            if let Some(body) = body {
                request = request
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(body.to_owned());
            }
            let mut response = request
                .send()
                .await
                .map_err(|_| IdentityError::Unauthorized)?;
            if !response.status().is_success()
                || !connection.matches_peer(response.remote_addr())
                || response
                    .content_length()
                    .is_some_and(|size| size > max as u64)
            {
                return Err(IdentityError::Unauthorized);
            }
            let mut bytes = Zeroizing::new(Vec::new());
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| IdentityError::Unauthorized)?
            {
                if chunk.len() > max.saturating_sub(bytes.len()) {
                    return Err(IdentityError::Unauthorized);
                }
                bytes.extend_from_slice(&chunk);
            }
            let text = std::str::from_utf8(&bytes).map_err(|_| IdentityError::Unauthorized)?;
            Ok(Zeroizing::new(text.to_owned()))
        })
        .await
        .map_err(|_| IdentityError::Unauthorized)?
    }
}

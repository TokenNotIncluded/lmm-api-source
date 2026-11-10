//! Paid model traffic reuses the origin-bound transport used by moderation.
//! No second resolver, redirect, system proxy or relaxed TLS is introduced here.
use super::{
    Limits, Phase, RelayError, Route, UpstreamProtocol,
    honeypot_network::{self, NetworkTarget, PreparedConnection},
    responses_ws::PreparedCreate,
};
use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, Url};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

#[derive(Clone)]
pub(crate) struct ProviderTransport {
    dns_slots: Arc<Semaphore>,
}

pub(crate) struct ProviderConnection(PreparedConnection);
impl ProviderConnection {
    pub(crate) fn client(&self) -> &Client {
        &self.0.client
    }
    pub(crate) fn matches_peer(&self, peer: Option<SocketAddr>) -> bool {
        self.0.matches_peer(peer)
    }
}

pub(crate) struct ResponsesConnection {
    socket: WebSocketStream<reqwest::Upgraded>,
    opened_for: Route,
    write_timeout: Duration,
    usable: bool,
}

impl ResponsesConnection {
    pub(crate) fn accepts(&self, route: &Route) -> bool {
        self.usable
            && self.opened_for.is_available()
            && route.is_available()
            && route.protocol == UpstreamProtocol::Responses
            && route.id == self.opened_for.id
            && route.model == self.opened_for.model
            && route.endpoint == self.opened_for.endpoint
            && route.credential == self.opened_for.credential
    }

    pub(crate) async fn receive(&mut self) -> Result<Option<Message>, RelayError> {
        self.socket
            .next()
            .await
            .transpose()
            .map_err(|_| RelayError::UpstreamConnection)
    }

    pub(crate) async fn send_prepared(
        &mut self,
        route: &Route,
        frame: &PreparedCreate,
    ) -> Result<(), RelayError> {
        let text = std::str::from_utf8(&frame.wire_bytes)
            .map_err(|_| RelayError::Invalid("invalid prepared WebSocket frame"))?;
        self.send_text(route, text).await
    }

    pub(crate) async fn send_cancel(
        &mut self,
        route: &Route,
        text: &str,
    ) -> Result<(), RelayError> {
        self.send_text(route, text).await
    }

    async fn send_text(&mut self, route: &Route, text: &str) -> Result<(), RelayError> {
        if !self.accepts(route) {
            return Err(RelayError::RouteUnavailable);
        }
        // A failed or cancelled write may already have reached the provider.
        // Retire the socket so no later turn can reuse an ambiguous connection.
        self.usable = false;
        tokio::time::timeout(
            self.write_timeout,
            self.socket.send(Message::Text(text.into())),
        )
        .await
        .map_err(|_| RelayError::Timeout(Phase::Write))?
        .map_err(|_| RelayError::UpstreamConnection)?;
        self.usable = true;
        Ok(())
    }
}

impl ProviderTransport {
    pub(crate) fn new(max_in_flight: usize) -> Self {
        Self {
            dns_slots: honeypot_network::dns_slots(max_in_flight),
        }
    }

    /// The URL comes only from an authorized immutable route. DNS resolution
    /// and connection preparation must be inside the caller's total deadline.
    pub(crate) async fn prepare(
        &self,
        endpoint: &Url,
        limits: &Limits,
    ) -> Result<ProviderConnection, RelayError> {
        let mut base = endpoint.clone();
        // Gemini streaming has a server-generated alt=sse query. Reject all
        // other queries even for trusted route constructors.
        if base.query().is_some_and(|query| query != "alt=sse") {
            return Err(RelayError::RouteUnavailable);
        }
        base.set_query(None);
        honeypot_network::validate_base_url(base.as_str(), true)?;
        let target = NetworkTarget {
            additional_root: None,
            local_http: true,
            allowed_origins: [honeypot_network::canonical_origin(&base)].into(),
        };
        target
            .prepare(endpoint, limits, self.dns_slots.clone())
            .await
            .map(ProviderConnection)
    }

    /// Only the selected owner-enabled Responses WS host calls this. This is
    /// an HTTP Upgrade on the SAME origin-pinned TLS client, not a second DNS
    /// resolver or a new WebSocket TLS stack. No model create is sent here.
    pub(crate) async fn upgrade_responses(
        &self,
        route: &Route,
        limits: &Limits,
    ) -> Result<ResponsesConnection, RelayError> {
        use reqwest::header;
        use tokio_tungstenite::tungstenite::{
            handshake::{client::generate_key, derive_accept_key},
            protocol::{Role, WebSocketConfig},
        };
        if route.protocol != super::UpstreamProtocol::Responses
            || route.endpoint.query().is_some()
            || !route.is_available()
        {
            return Err(RelayError::RouteUnavailable);
        }
        let connection = tokio::time::timeout(
            limits.connect_timeout,
            self.prepare(&route.endpoint, limits),
        )
        .await
        .map_err(|_| RelayError::Timeout(super::Phase::Connect))??;
        if !route.is_available() {
            return Err(RelayError::RouteUnavailable);
        }
        let key = generate_key();
        let request = connection
            .client()
            .get(route.endpoint.clone())
            .version(reqwest::Version::HTTP_11)
            .header(header::CONNECTION, "Upgrade")
            .header(header::UPGRADE, "websocket")
            .header(header::SEC_WEBSOCKET_VERSION, "13")
            .header(header::SEC_WEBSOCKET_KEY, &key)
            .header(header::AUTHORIZATION, route.credential.clone())
            .header(header::ACCEPT_ENCODING, "identity");
        let response = tokio::time::timeout(limits.header_timeout, request.send())
            .await
            .map_err(|_| RelayError::Timeout(super::Phase::Headers))?
            .map_err(|_| RelayError::UpstreamConnection)?;
        // Check the actual connected peer before accepting headers or upgraded IO.
        if !connection.matches_peer(response.remote_addr()) || !route.is_available() {
            return Err(RelayError::UpstreamConnection);
        }
        validate_ws_upgrade(
            response.version(),
            response.status(),
            response.headers(),
            &derive_accept_key(key.as_bytes()),
        )?;
        let upgraded = tokio::time::timeout(limits.header_timeout, response.upgrade())
            .await
            .map_err(|_| RelayError::Timeout(super::Phase::Headers))?
            .map_err(|_| RelayError::UpstreamConnection)?;
        if !route.is_available() {
            return Err(RelayError::RouteUnavailable);
        }
        let config = WebSocketConfig::default()
            .read_buffer_size(limits.chunk_bytes)
            .write_buffer_size(0)
            .max_write_buffer_size(limits.request_bytes.saturating_add(1024))
            .max_message_size(Some(limits.event_bytes))
            .max_frame_size(Some(limits.event_bytes));
        Ok(ResponsesConnection {
            socket: WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(config)).await,
            opened_for: route.clone(),
            write_timeout: limits.write_timeout,
            usable: true,
        })
    }
}

fn validate_ws_upgrade(
    version: reqwest::Version,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    expected_accept: &str,
) -> Result<(), RelayError> {
    use reqwest::header;
    if status != reqwest::StatusCode::SWITCHING_PROTOCOLS {
        return Err(RelayError::UpstreamStatus(status.as_u16()));
    }
    let single = |name: &'static str| -> Option<&str> {
        if headers.get_all(name).iter().count() != 1 {
            return None;
        }
        headers.get(name)?.to_str().ok()
    };
    if version != reqwest::Version::HTTP_11
        || single("upgrade").is_none_or(|value| !value.eq_ignore_ascii_case("websocket"))
        || single("connection").is_none_or(|value| {
            !value.split(',').any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        })
        || single("sec-websocket-accept") != Some(expected_accept)
        // No extensions or subprotocol were requested. Never accept unsolicited
        // compression or echo a provider credential-bearing subprotocol.
        || headers.contains_key(header::SEC_WEBSOCKET_EXTENSIONS)
        || headers.contains_key(header::SEC_WEBSOCKET_PROTOCOL)
    {
        return Err(RelayError::Protocol("invalid upstream WebSocket handshake"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::post};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;

    #[test]
    fn websocket_handshake_requires_exact_accept_and_no_extensions() {
        use reqwest::{
            StatusCode, Version,
            header::{HeaderMap, HeaderValue},
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            "connection",
            HeaderValue::from_static("keep-alive, Upgrade"),
        );
        headers.insert("upgrade", HeaderValue::from_static("websocket"));
        headers.insert("sec-websocket-accept", HeaderValue::from_static("expected"));
        assert!(
            validate_ws_upgrade(
                Version::HTTP_11,
                StatusCode::SWITCHING_PROTOCOLS,
                &headers,
                "expected"
            )
            .is_ok()
        );
        assert!(
            validate_ws_upgrade(
                Version::HTTP_11,
                StatusCode::SWITCHING_PROTOCOLS,
                &headers,
                "different"
            )
            .is_err()
        );
        assert!(
            validate_ws_upgrade(
                Version::HTTP_2,
                StatusCode::SWITCHING_PROTOCOLS,
                &headers,
                "expected"
            )
            .is_err()
        );
        assert!(
            validate_ws_upgrade(
                Version::HTTP_11,
                StatusCode::TEMPORARY_REDIRECT,
                &headers,
                "expected"
            )
            .is_err()
        );
        headers.append("sec-websocket-accept", HeaderValue::from_static("expected"));
        assert!(
            validate_ws_upgrade(
                Version::HTTP_11,
                StatusCode::SWITCHING_PROTOCOLS,
                &headers,
                "expected"
            )
            .is_err()
        );
        headers.remove("sec-websocket-accept");
        headers.insert("sec-websocket-accept", HeaderValue::from_static("expected"));
        for field in ["sec-websocket-extensions", "sec-websocket-protocol"] {
            headers.insert(field, HeaderValue::from_static("unrequested"));
            assert!(
                validate_ws_upgrade(
                    Version::HTTP_11,
                    StatusCode::SWITCHING_PROTOCOLS,
                    &headers,
                    "expected"
                )
                .is_err()
            );
            headers.remove(field);
        }
    }

    #[tokio::test]
    async fn websocket_upgrade_uses_bound_get_and_keeps_one_real_connection() {
        use crate::relay::{Request, responses_ws::Create};
        use axum::{
            extract::ws::{Message, WebSocketUpgrade},
            http::HeaderMap,
            routing::get,
        };
        use futures_util::StreamExt;
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let app = Router::new().route(
            "/v1/responses",
            get(move |headers: HeaderMap, ws: WebSocketUpgrade| {
                assert_eq!(headers["authorization"], "Bearer synthetic-ws-only");
                assert!(!headers.contains_key("x-api-key"));
                assert!(!headers.contains_key("sec-websocket-protocol"));
                count.fetch_add(1, Ordering::SeqCst);
                async move {
                    ws.on_upgrade(|mut socket| async move {
                        // Echo actual independent WS messages on a retained connection;
                        // never translate them into synthetic HTTP POST requests.
                        for _ in 0..2 {
                            let Some(Ok(Message::Text(text))) = socket.recv().await else {
                                panic!("expected create frame")
                            };
                            socket.send(Message::Text(text)).await.unwrap();
                        }
                    })
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let route = super::super::Route::new(
            "r1".into(),
            super::super::UpstreamProtocol::Responses,
            &format!("http://{peer}/v1/responses"),
            "model".into(),
            "synthetic-ws-only",
            1,
        )
        .unwrap();
        let limits = Limits::default();
        let mut socket = ProviderTransport::new(1)
            .upgrade_responses(&route, &limits)
            .await
            .unwrap();
        let mut different_credential = route.clone();
        different_credential.credential = "Bearer other-synthetic-key".parse().unwrap();
        for raw in [
            r#"{"type":"response.create","model":"requested","input":[],"generate":false}"#,
            r#"{"type":"response.create","model":"requested","input":"continue","previous_response_id":"resp1"}"#,
        ] {
            let create = Create::parse(raw.as_bytes(), limits.request_bytes).unwrap();
            let request = Request::parse_responses_ws(
                &create.request_json,
                &limits,
                create.previous_response_id.is_some(),
                create.generate,
            )
            .unwrap();
            let prepared = create.prepare_selected(&request, &route, &limits).unwrap();
            assert_eq!(
                socket.send_prepared(&different_credential, &prepared).await,
                Err(RelayError::RouteUnavailable)
            );
            socket.send_prepared(&route, &prepared).await.unwrap();
            assert_eq!(
                socket
                    .socket
                    .next()
                    .await
                    .unwrap()
                    .unwrap()
                    .into_text()
                    .unwrap(),
                std::str::from_utf8(&prepared.wire_bytes).unwrap()
            );
        }
        assert_eq!(connections.load(Ordering::SeqCst), 1);
        drop(socket);
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn private_and_query_targets_are_rejected_before_connecting() {
        let transport = ProviderTransport::new(1);
        for raw in [
            "https://10.0.0.1/v1",
            "https://169.254.169.254/v1",
            "http://localhost/v1",
            "https://localhost/v1",
            "https://api.example/v1?key=secret",
        ] {
            assert!(
                transport
                    .prepare(&Url::parse(raw).unwrap(), &Limits::default())
                    .await
                    .is_err(),
                "{raw}"
            );
        }
    }

    #[tokio::test]
    async fn numeric_loopback_peer_is_pinned_and_redirect_never_replayed() {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        let app = Router::new().route(
            "/v1/responses",
            post(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [("location", "http://127.0.0.1:1/credential-leak")],
                    )
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let transport = ProviderTransport::new(1);
        let endpoint = Url::parse(&format!("http://{peer}/v1/responses")).unwrap();
        let connection = transport
            .prepare(&endpoint, &Limits::default())
            .await
            .unwrap();
        let response = connection
            .client()
            .post(endpoint)
            .bearer_auth("synthetic-only")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 307);
        assert!(connection.matches_peer(response.remote_addr()));
        assert!(!connection.matches_peer(Some("127.0.0.1:1".parse().unwrap())));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
    }
}

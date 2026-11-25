//! Native public ingress policy. Forwarding headers are never identity.
use axum::{
    Json, Router,
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use maxminddb::{Reader, path};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedConfig {
    pub geoip_database: Option<PathBuf>,
}

#[derive(Clone)]
pub struct NetworkAccess {
    policy_pool: Option<PgPool>,
    geoip: Option<Arc<Reader<Vec<u8>>>>,
}

impl NetworkAccess {
    /// Identity-only installations still establish the authenticated request IP.
    pub fn new(
        policy_pool: Option<PgPool>,
        config: TypedConfig,
    ) -> Result<Self, crate::edge_access::SetupError> {
        let geoip = config
            .geoip_database
            .map(|file| {
                if !file.is_absolute() {
                    return Err(crate::edge_access::SetupError);
                }
                let reader =
                    Reader::open_readfile(file).map_err(|_| crate::edge_access::SetupError)?;
                if !matches!(
                    reader.metadata().database_type.as_str(),
                    "GeoIP2-Country" | "GeoLite2-Country" | "GeoIP2-City" | "GeoLite2-City"
                ) {
                    return Err(crate::edge_access::SetupError);
                }
                Ok(Arc::new(reader))
            })
            .transpose()?;
        Ok(Self { policy_pool, geoip })
    }

    /// Attach before EdgeAccess::protect so edge proof verification runs first.
    pub fn protect(self, routes: Router) -> Router {
        routes.layer(middleware::from_fn_with_state(self, enforce))
    }

    fn country(
        &self,
        trusted_country: Option<&str>,
        peer: IpAddr,
    ) -> Result<Option<String>, crate::edge_access::SetupError> {
        if let Some(reader) = &self.geoip {
            let result = reader
                .lookup(peer)
                .map_err(|_| crate::edge_access::SetupError)?;
            let value: Option<&str> = result
                .decode_path(&path!["country", "iso_code"])
                .map_err(|_| crate::edge_access::SetupError)?;
            return value
                .map(|code| {
                    if code.len() != 2 || !code.bytes().all(|b| b.is_ascii_alphabetic()) {
                        return Err(crate::edge_access::SetupError);
                    }
                    Ok(code.to_ascii_uppercase())
                })
                .transpose();
        }
        Ok(trusted_country.map(str::to_owned))
    }

    async fn decision(
        &self,
        peer: Option<IpAddr>,
        trusted_country: Option<&str>,
    ) -> Result<bool, crate::edge_access::SetupError> {
        let Some(pool) = &self.policy_pool else {
            return Ok(true);
        };
        let source = tokio::time::timeout(
            Duration::from_secs(2),
            crate::console_compat::effective_option(pool, "IPAccessRoutingRules"),
        )
        .await
        .map_err(|_| crate::edge_access::SetupError)?
        .map_err(|_| crate::edge_access::SetupError)?;
        // An unset rule is the existing standalone default; never install a restriction implicitly.
        let Some(source) = source.filter(|source| !source.trim().is_empty()) else {
            return Ok(true);
        };
        let peer = peer.ok_or(crate::edge_access::SetupError)?;
        let country = self.country(trusted_country, peer)?;
        crate::edge_access::evaluate_policy(&source, peer, country.as_deref())
    }
}

async fn enforce(State(state): State<NetworkAccess>, request: Request, next: Next) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|peer| match peer.0.ip() {
            IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4),
            ip => ip,
        });
    // Availability probes must remain usable when an operator intentionally rejects a region.
    if !matches!(
        request.uri().path(),
        "/health/live" | "/health/ready" | "/health/model-ready"
    ) {
        let trusted_country = request
            .extensions()
            .get::<crate::edge_access::TrustedCountry>()
            .and_then(|country| country.0.clone());
        match state.decision(peer, trusted_country.as_deref()).await {
            Ok(true) => (),
            Ok(false) => {
                return rejected(
                    StatusCode::FORBIDDEN,
                    "IP_ACCESS_ROUTE_REJECTED",
                    "Access from this network or region is not allowed.",
                );
            }
            Err(_) => {
                return rejected(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "POLICY_UNAVAILABLE",
                    "Network access policy could not be evaluated.",
                );
            }
        }
    }
    crate::identity::scope_request_ip(peer, next.run(request)).await
}

fn rejected(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        [
            ("cache-control", "no-store"),
            ("x-lmm-access-policy", "denied"),
            ("x-content-type-options", "nosniff"),
        ],
        Json(json!({"success":false,"code":code,"message":message,
            "error":{"type":"access_policy_error","code":code,"message":message}})),
    )
        .into_response()
}

#[cfg(test)]
mod tests;

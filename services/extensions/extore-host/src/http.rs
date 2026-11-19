use extore_plugin_api::{MAX_RESPONSE_BYTES, RESPONSE_SCHEMA, Response as PluginResponse};
use http_body_util::Full;
use hyper::{
    Request, Response, StatusCode,
    body::{Body, Bytes, Incoming},
    header::{
        AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, HeaderName, HeaderValue, WWW_AUTHENTICATE,
    },
};

pub type Reply = Response<Full<Bytes>>;
const CATALOG: &str = "/api/integrations/commerce/products";

fn json(status: StatusCode, body: &'static str) -> Reply {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .header(CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::from_static(body.as_bytes())))
        .expect("static response")
}

pub fn handle(
    request: Request<Incoming>,
    origin: &str,
    catalog: &impl Fn(&str) -> Result<PluginResponse, ()>,
) -> Reply {
    if request
        .headers()
        .get("X-Extore-Origin")
        .and_then(|h| h.to_str().ok())
        != Some(origin)
    {
        return json(StatusCode::FORBIDDEN, r#"{"error":"source_mismatch"}"#);
    }
    if request.method() != hyper::Method::GET {
        return json(StatusCode::METHOD_NOT_ALLOWED, r#"{"error":"read_only"}"#);
    }
    if request.uri().query().is_some() || request.body().size_hint().upper() != Some(0) {
        return json(StatusCode::BAD_REQUEST, r#"{"error":"invalid_request"}"#);
    }
    if request.uri().path() == "/health/ready" {
        return json(StatusCode::OK, r#"{"status":"ready"}"#);
    }
    if request.uri().path() != CATALOG {
        return json(StatusCode::NOT_FOUND, r#"{"error":"not_found"}"#);
    }
    let token = match request.headers().get(AUTHORIZATION) {
        None => "",
        Some(header) => match header.to_str().ok().and_then(|v| v.strip_prefix("Bearer ")) {
            Some(token) if token.len() <= 8192 => token,
            _ => return json(StatusCode::UNAUTHORIZED, r#"{"error":"invalid_token"}"#),
        },
    };
    let translated = catalog(token).and_then(translate);
    translated.unwrap_or_else(|()| {
        json(
            StatusCode::BAD_GATEWAY,
            r#"{"error":"extore_plugin_unavailable"}"#,
        )
    })
}

fn translate(response: PluginResponse) -> Result<Reply, ()> {
    if response.schema != RESPONSE_SCHEMA {
        return Err(());
    }
    let status = StatusCode::from_u16(response.status).map_err(|_| ())?;
    if status.is_informational() {
        return Err(());
    }
    let body = serde_json::to_vec(&response.body).map_err(|_| ())?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(());
    }
    let mut result = Response::builder()
        .status(status)
        .header(CACHE_CONTROL, "no-store");
    for (name, value) in response.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| ())?;
        if name == CACHE_CONTROL {
            if value != "no-store" {
                return Err(());
            }
            continue;
        }
        if name != CONTENT_TYPE && name != WWW_AUTHENTICATE {
            return Err(());
        }
        result = result.header(name, HeaderValue::from_str(&value).map_err(|_| ())?);
    }
    result.body(Full::new(Bytes::from(body))).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_real_plugin_no_store_and_auth_headers() {
        let response = PluginResponse {
            schema: RESPONSE_SCHEMA.to_owned(),
            status: 401,
            headers: [
                (
                    "content-type".into(),
                    "application/json; charset=utf-8".into(),
                ),
                ("cache-control".into(), "no-store".into()),
                ("www-authenticate".into(), "Bearer".into()),
            ]
            .into(),
            body: serde_json::json!({"error": "invalid_token"}),
        };
        let reply = translate(response).expect("real plugin headers");
        assert_eq!(reply.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(reply.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(reply.headers()[WWW_AUTHENTICATE], "Bearer");
        assert_eq!(
            reply.headers()[CONTENT_TYPE],
            "application/json; charset=utf-8"
        );
    }
}

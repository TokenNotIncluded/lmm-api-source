//! Standard OpenAI transport for Codewhale's group-bound OAuth model IDs.
use super::*;
use crate::identity::grants::{LmmIssuer, ReadError};
use axum::extract::{FromRequest, RawQuery, Request};
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};
use serde_json::value::{RawValue, to_raw_value};

const CLIENT: &str = "lmm-codewhale";

pub(crate) async fn models(
    State(issuer): State<LmmIssuer>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    if query.is_some() || headers.contains_key("x-lmm-group") {
        return public_error(RelayError::Invalid("unexpected model catalog options"));
    }
    let access = match crate::oauth2_http::bearer(&headers) {
        Ok(access) => access,
        Err(error) => return crate::oauth2_http::read_error(ReadError::Identity(error)),
    };
    match issuer.codewhale_catalog(access).await {
        Ok(catalog) => {
            let data: Vec<_> = catalog
                .models
                .into_iter()
                .filter(|model| model.apis.contains(&"openai-completions"))
                .map(|model| {
                    let meta = &model.metadata;
                    let mut limit = serde_json::Map::new();
                    if let Some(maximum) =
                        minimum_tokens(meta.context_window, meta.max_input_tokens)
                    {
                        limit.insert("context".to_owned(), json!((maximum / 2).max(1)));
                    }
                    if let Some(maximum) =
                        minimum_tokens(meta.max_output_tokens, meta.gateway_max_output_tokens)
                    {
                        limit.insert("output".to_owned(), json!(maximum));
                    }
                    let mut row = json!({
                        "id":model.id,
                        "object":"model",
                        "created":0,
                        "owned_by":"lmm",
                        "name":format!("{} / {}",model.group,model.name)
                    });
                    if !limit.is_empty() {
                        row["limit"] = serde_json::Value::Object(limit);
                    }
                    row
                })
                .collect();
            Json(json!({"object":"list","data":data})).into_response()
        }
        Err(error) => crate::oauth2_http::read_error(error),
    }
}

fn minimum_tokens(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    [a, b]
        .into_iter()
        .flatten()
        .filter(|value| *value > 0)
        .min()
}

pub(super) async fn chat(
    State(gateway): State<ModelGateway>,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    request: Request,
) -> Response {
    match bind_chat(gateway, peer.ok().map(|peer| peer.0.ip()), request).await {
        Ok(response) => response,
        Err(error) => public_error(error),
    }
}

async fn bind_chat(
    gateway: ModelGateway,
    peer: Option<IpAddr>,
    request: Request,
) -> Result<Response, RelayError> {
    let headers = request.headers();
    if !oauth_header(headers) {
        return Err(RelayError::Unauthorized);
    }
    if request.uri().query().is_some() || headers.contains_key("x-lmm-group") {
        return Err(RelayError::Invalid("unexpected group or query selector"));
    }
    if !one_header(headers, header::CONTENT_TYPE.as_str())?.is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    }) || headers.contains_key(header::CONTENT_ENCODING)
    {
        return Err(RelayError::Invalid("expected an uncompressed JSON request"));
    }
    let (principal, secret, groups) = gateway.authenticate(headers).await?;
    if !principal
        .oauth_source()
        .is_some_and(|source| source.client_id() == CLIENT)
    {
        return Err(RelayError::Forbidden);
    }
    let (mut parts, body) = request.into_parts();
    let headers = std::mem::take(&mut parts.headers);
    let bytes = Bytes::from_request(Request::from_parts(parts, body), &gateway)
        .await
        .map_err(|rejection| {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                RelayError::Limit("request body")
            } else {
                RelayError::Invalid("could not read request body")
            }
        })?;
    let mut request = serde_json::from_slice::<BoundChat>(&bytes)
        .map_err(|_| RelayError::Invalid("invalid or ambiguous JSON body"))?;
    let id = request.model_id()?;
    let (group, model, version) = gateway
        .catalog_surface
        .exact_chat_model(&id, &groups)
        .map_err(|_| RelayError::RouteUnavailable)?
        .ok_or(RelayError::Forbidden)?;
    let bytes = request.replace_model(&model)?;
    Ok(dispatch_authorized(
        gateway,
        headers,
        bytes,
        (principal, secret, vec![group]),
        DispatchRequest {
            client: ClientProtocol::Chat,
            peer,
            native_operation: None,
            required_release: Some(version),
        },
    )
    .await)
}

/// Keep each nested value in its original representation, including schema
/// key order and duplicate keys, for the normal Chat parser to validate.
struct BoundChat(BTreeMap<String, Box<RawValue>>);

impl<'de> Deserialize<'de> for BoundChat {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BoundChatVisitor;
        impl<'de> Visitor<'de> for BoundChatVisitor {
            type Value = BoundChat;

            fn expecting(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                fmt.write_str("an unambiguous Chat Completion request")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut seen = std::collections::BTreeSet::new();
                let mut fields = BTreeMap::new();
                while let Some((name, value)) = map.next_entry::<String, Box<RawValue>>()? {
                    if !seen.insert(name.to_ascii_lowercase()) {
                        return Err(serde::de::Error::custom("duplicate request key"));
                    }
                    fields.insert(name, value);
                }
                Ok(BoundChat(fields))
            }
        }

        deserializer.deserialize_map(BoundChatVisitor)
    }
}

impl BoundChat {
    fn model_id(&self) -> Result<String, RelayError> {
        let raw = self
            .0
            .get("model")
            .ok_or(RelayError::Invalid("expected a group-bound model ID"))?;
        let id = serde_json::from_str::<String>(raw.get())
            .map_err(|_| RelayError::Invalid("expected a group-bound model ID"))?;
        if !id.starts_with("lmm:") {
            return Err(RelayError::Invalid("expected a group-bound model ID"));
        }
        Ok(id)
    }

    fn replace_model(&mut self, model: &str) -> Result<Bytes, RelayError> {
        self.0.insert(
            "model".to_owned(),
            to_raw_value(model).map_err(|_| RelayError::Invalid("invalid model"))?,
        );
        serde_json::to_vec(&self.0)
            .map(Bytes::from)
            .map_err(|_| RelayError::Invalid("invalid request"))
    }
}

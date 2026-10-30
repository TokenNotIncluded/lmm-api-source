//! Native model discovery is an authenticated view of executable gateway routes.
//! Wire contracts: https://platform.claude.com/docs/en/api/models/list and
//! https://ai.google.dev/api/models (reviewed 2026-10-30).
use super::*;
use axum::extract::{Path, RawQuery};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
#[path = "native_models_tests.rs"]
mod tests;

#[derive(Serialize)]
struct Model {
    id: String,
    input: i64,
    output: u32,
}
struct View {
    models: Vec<Model>,
    digest: [u8; 32],
}

fn json_response(value: Value) -> Response {
    let mut response = Json(value).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn query_pairs(query: Option<&str>) -> Result<Vec<(String, String)>, RelayError> {
    let query = query.unwrap_or("");
    if query.len() > 4096 {
        return Err(RelayError::Invalid("model list query too long"));
    }
    let pairs: Vec<_> = form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    let mut names = std::collections::BTreeSet::new();
    if pairs.iter().any(|(key, _)| !names.insert(key)) {
        return Err(RelayError::Invalid("duplicate model list query"));
    }
    Ok(pairs)
}

/// All authorization happens before pagination, including canonical model
/// restrictions on aliases. Route construction performs no supplier request.
async fn view(
    gateway: &ModelGateway,
    headers: HeaderMap,
    peer: Option<IpAddr>,
    client: ClientProtocol,
) -> Result<View, RelayError> {
    let headers = native::key_headers(headers, client)?;
    let caller = native_http::authenticate(gateway, &headers, peer).await?;
    let version = gateway
        .bridge
        .routing
        .version()
        .map_err(|_| RelayError::RouteUnavailable)?;
    let request = match client {
        ClientProtocol::Anthropic => relay::Request::parse(client,
            br#"{"model":"catalog-probe","max_tokens":1,"messages":[{"role":"user","content":"catalog"}]}"#,
            &gateway.limits)?,
        ClientProtocol::Gemini => relay::native_gemini::parse("catalog-probe", false,
            br#"{"contents":[{"role":"user","parts":[{"text":"catalog"}]}],"generationConfig":{"maxOutputTokens":1}}"#,
            &gateway.limits)?,
        _ => return Err(RelayError::Invalid("native model catalog protocol")),
    };
    let mut models = BTreeMap::new();
    for group in &caller.groups {
        let catalog = gateway
            .bridge
            .routing
            .catalog(group)
            .map_err(|_| RelayError::RouteUnavailable)?;
        for row in catalog {
            if models.contains_key(&row.name) {
                continue;
            }
            match gateway
                .authorize_request(
                    &caller.principal,
                    &caller.secret,
                    &row.canonical_model,
                    group,
                    caller.peer,
                )
                .await
            {
                Ok(()) => (),
                Err(IdentityError::Forbidden) => continue,
                Err(error) => return Err(identity_error(error)),
            }
            let Ok(mut plan) = gateway
                .bridge
                .routing
                .route_adapted(crate::routing::RouteRequest {
                    model: &row.name,
                    group,
                    speed: None,
                    protocol: super::routing::client_protocol(client),
                    replay: crate::routing::ReplayPermission::Denied,
                })
            else {
                continue;
            };
            let Ok(attempt) = plan.start() else {
                continue;
            };
            let Ok((selected, _)) = gateway
                .bridge
                .route_for_attempt(Arc::new(attempt), &request)
            else {
                continue;
            };
            let output = gateway
                .bridge
                .provider_policy
                .output_bound(&selected.attempt, gateway.limits.max_output_tokens)?;
            if output == 0 {
                continue;
            }
            models.insert(
                row.name.clone(),
                Model {
                    id: row.name,
                    input: selected.input_token_ceiling,
                    output,
                },
            );
        }
    }
    if gateway
        .bridge
        .routing
        .version()
        .map_err(|_| RelayError::RouteUnavailable)?
        != version
    {
        return Err(RelayError::RouteUnavailable);
    }
    let models: Vec<_> = models.into_values().collect();
    let mut digest = Sha256::new();
    digest.update(b"lmm-native-model-catalog-v1\0");
    digest.update(
        caller
            .principal
            .key_id()
            .ok_or(RelayError::Forbidden)?
            .to_be_bytes(),
    );
    digest.update(version.routing.to_be_bytes());
    digest.update(version.pricing.to_be_bytes());
    digest.update(serde_json::to_vec(&caller.groups).map_err(|_| RelayError::BillingUnavailable)?);
    digest.update(serde_json::to_vec(&models).map_err(|_| RelayError::BillingUnavailable)?);
    Ok(View {
        models,
        digest: digest.finalize().into(),
    })
}

#[derive(Default)]
struct AnthropicPage {
    limit: usize,
    after: Option<String>,
    before: Option<String>,
}
impl AnthropicPage {
    fn parse(query: Option<&str>) -> Result<Self, RelayError> {
        let mut page = Self {
            limit: 20,
            ..Self::default()
        };
        for (key, value) in query_pairs(query)? {
            match key.as_str() {
                "limit" => {
                    page.limit = value
                        .parse()
                        .ok()
                        .filter(|n| (1..=1000).contains(n))
                        .ok_or(RelayError::Invalid("model list limit"))?
                }
                "after_id" if !value.is_empty() => page.after = Some(value),
                "before_id" if !value.is_empty() => page.before = Some(value),
                _ => return Err(RelayError::Invalid("unsupported model list query")),
            }
        }
        if page.after.is_some() && page.before.is_some() {
            return Err(RelayError::Invalid("model list cursor direction"));
        }
        Ok(page)
    }
    fn render(&self, models: &[Model]) -> Result<Value, RelayError> {
        let index = |id: &str| {
            models
                .iter()
                .position(|m| m.id == id)
                .ok_or(RelayError::Invalid("model list cursor"))
        };
        let (start, end, has_more) = if let Some(before) = &self.before {
            let end = index(before)?;
            let start = end.saturating_sub(self.limit);
            (start, end, start > 0)
        } else {
            let start = self
                .after
                .as_deref()
                .map(index)
                .transpose()?
                .map_or(0, |n| n + 1);
            let end = start.saturating_add(self.limit).min(models.len());
            (start, end, end < models.len())
        };
        let page = &models[start..end];
        let data: Vec<_> = page
            .iter()
            .map(|model| {
                json!({
                    "type":"model","id":model.id,"display_name":model.id,
                    "created_at":"1970-01-01T00:00:00Z","capabilities":null,
                    "max_input_tokens":model.input,"max_tokens":model.output
                })
            })
            .collect();
        Ok(json!({"data":data,"first_id":page.first().map(|m|&m.id),
            "last_id":page.last().map(|m|&m.id),"has_more":has_more}))
    }
}

pub(super) async fn anthropic(
    gateway: ModelGateway,
    headers: HeaderMap,
    query: Option<String>,
    peer: Option<IpAddr>,
) -> Response {
    let result = async {
        if one_header(&headers, "anthropic-version")?
            .is_some_and(|v| !matches!(v, "2023-06-01" | "2026-10-21"))
            || headers.contains_key("anthropic-beta")
            || headers.contains_key("anthropic-workspace-id")
        {
            return Err(RelayError::Unsupported(
                "unsupported Anthropic model catalog version or workspace",
            ));
        }
        let page = AnthropicPage::parse(query.as_deref())?;
        let view = view(&gateway, headers, peer, ClientProtocol::Anthropic).await?;
        page.render(&view.models)
    }
    .await;
    match result {
        Ok(value) => json_response(value),
        Err(error) => native::error_response(ClientProtocol::Anthropic, error),
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    page_size: u32,
    offset: usize,
    view: [u8; 32],
}
struct GeminiPage {
    requested_size: u32,
    token: Option<String>,
}
impl GeminiPage {
    fn parse(query: Option<&str>) -> Result<Self, RelayError> {
        let mut page = Self {
            requested_size: 50,
            token: None,
        };
        for (key, value) in query_pairs(query)? {
            match key.as_str() {
                "pageSize" => {
                    page.requested_size = value
                        .parse()
                        .ok()
                        .filter(|n| *n > 0)
                        .ok_or(RelayError::Invalid("model list page size"))?
                }
                "pageToken" if !value.is_empty() && value.len() <= 2048 => page.token = Some(value),
                _ => return Err(RelayError::Invalid("unsupported model list query")),
            }
        }
        Ok(page)
    }
    fn render(&self, view: &View) -> Result<Value, RelayError> {
        let start = if let Some(token) = &self.token {
            let cursor: Cursor = URL_SAFE_NO_PAD
                .decode(token)
                .ok()
                .and_then(|raw| serde_json::from_slice(&raw).ok())
                .ok_or(RelayError::Invalid("model list page token"))?;
            if cursor.version != 1
                || cursor.page_size != self.requested_size
                || cursor.view != view.digest
                || cursor.offset == 0
                || cursor.offset >= view.models.len()
            {
                return Err(RelayError::Invalid(
                    "model list page token is stale or invalid",
                ));
            }
            cursor.offset
        } else {
            0
        };
        let end = start
            .saturating_add(self.requested_size.min(1000) as usize)
            .min(view.models.len());
        let mut result =
            json!({"models":view.models[start..end].iter().map(gemini_model).collect::<Vec<_>>()});
        if end < view.models.len() {
            let cursor = Cursor {
                version: 1,
                page_size: self.requested_size,
                offset: end,
                view: view.digest,
            };
            result["nextPageToken"] =
                json!(URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&cursor).map_err(|_| RelayError::BillingUnavailable)?
                ));
        }
        Ok(result)
    }
}
fn gemini_model(model: &Model) -> Value {
    json!({"name":format!("models/{}",model.id),"baseModelId":model.id,"version":"unknown",
        "displayName":model.id.chars().take(128).collect::<String>(),
        "description":"Gateway route. Provider version and release date are not published.",
        "inputTokenLimit":model.input,"outputTokenLimit":model.output,
        "supportedGenerationMethods":["generateContent"]})
}

pub(super) async fn gemini_list(
    State(gateway): State<ModelGateway>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    body: Bytes,
) -> Response {
    let result = async {
        if !body.is_empty() {
            return Err(RelayError::Invalid("model list body must be empty"));
        }
        let page = GeminiPage::parse(query.as_deref())?;
        let view = view(
            &gateway,
            headers,
            peer.ok().map(|p| p.0.ip()),
            ClientProtocol::Gemini,
        )
        .await?;
        page.render(&view)
    }
    .await;
    match result {
        Ok(value) => json_response(value),
        Err(error) => native::error_response(ClientProtocol::Gemini, error),
    }
}
pub(super) async fn gemini_get(
    State(gateway): State<ModelGateway>,
    Path(model): Path<String>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    body: Bytes,
) -> Response {
    let result = async {
        if !body.is_empty() || query.is_some() || model.is_empty() || model.len() > 256 {
            return Err(RelayError::Invalid("model get request"));
        }
        let view = view(
            &gateway,
            headers,
            peer.ok().map(|p| p.0.ip()),
            ClientProtocol::Gemini,
        )
        .await?;
        Ok(view.models.iter().find(|m| m.id == model).map(gemini_model))
    }
    .await;
    match result {
        Ok(Some(value)) => json_response(value),
        Ok(None) => {
            let mut response = json_response(
                json!({"error":{"code":404,"status":"NOT_FOUND","message":"Model not found"}}),
            );
            *response.status_mut() = StatusCode::NOT_FOUND;
            response
        }
        Err(error) => native::error_response(ClientProtocol::Gemini, error),
    }
}

//! Read-only metadata from the exact immutable release installed in ModelGateway.
//! API labels describe the gateway's bounded text/function adapter surface. They
//! do not grant invocation, promise current health, enable SSE, or lock a price.
use crate::billing::{
    text_request_price,
    tool_price::{PersistedToolPrice, ToolKind},
};
use crate::relay::Limits;
use crate::routing::{
    BillingMode, DecimalRate, ModelRoute, PayloadPolicy, PriceQuote, PriceSnapshot, Protocol,
    ReleaseVersion, Router, Speed, TieredTariff, UpstreamConfig,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::Number;
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
    time::Duration,
};

const MAX_MODELS: usize = 4096;
const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogError {
    Unavailable,
}

/// Opaque server-injected proof. Only ModelGateway can create a serving surface.
/// No URL, credential material, raw release, or NativePrincipal is serialized.
pub struct CatalogSurface {
    inventory_router: Arc<Router>,
    state: RwLock<SurfaceState>,
}

enum SurfaceState {
    Disabled,
    Ready {
        router: Arc<Router>,
        version: ReleaseVersion,
        groups: BTreeMap<String, CatalogGroup>,
        models: Vec<CatalogModel>,
    },
}

#[derive(Clone, Serialize)]
pub struct CatalogV1 {
    pub schema_version: u32,
    pub resource: String,
    pub updated_at: u64,
    pub routing_version: u64,
    pub pricing_version: u64,
    pub groups: Vec<CatalogGroup>,
    pub models: Vec<CatalogModel>,
}

#[derive(Clone, Serialize)]
pub struct CatalogGroup {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub multiplier: Option<Number>,
}

#[derive(Clone, Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub group_id: String,
    pub group: String,
    pub upstream_model: String,
    pub name: String,
    pub metadata: ModelMetadata,
    pub apis: Vec<&'static str>,
    pub pricing: CatalogPricing,
    pub native_cost: Option<NativeCost>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCost {
    pub input: Number,
    pub output: Number,
    pub cache_read: Number,
    pub cache_write: Number,
}
impl NativeCost {
    fn from_pricing(price: &CatalogPricing) -> Option<Self> {
        if price.currency != "USD" || price.unit != "million_tokens" {
            return None;
        }
        Some(Self {
            input: price.input.clone()?,
            output: price.output.clone()?,
            cache_read: price.cache_read.clone()?,
            cache_write: price.cache_write.clone()?,
        })
    }
}

#[derive(Clone, Default, Serialize)]
pub struct ModelMetadata {
    pub context_window: Option<u32>,
    pub max_input_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
    /// Independent of the upstream model's capability, enforced on gateway input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_max_output_tokens: Option<u32>,
    pub input_modalities: Option<Vec<String>>,
    pub output_modalities: Option<Vec<String>>,
    pub reasoning: Option<bool>,
    pub tool_call: Option<bool>,
    pub source: &'static str,
}
impl ModelMetadata {
    pub(super) fn unknown() -> Self {
        Self {
            source: "unknown",
            ..Self::default()
        }
    }
}

/// A fallback route cannot advertise a capability absent from any candidate.
/// Read the exact release's contracts, never a newer global model-name table.
pub(super) fn model_metadata(
    route: &ModelRoute,
    upstreams: &BTreeMap<u64, &UpstreamConfig>,
    policy: Option<&super::ProviderPolicy>,
    version: u64,
) -> Result<ModelMetadata, CatalogError> {
    let Some(policy) = policy else {
        return Ok(ModelMetadata::unknown());
    };
    let mut contracts = Vec::new();
    for target in route
        .targets
        .iter()
        .filter(|t| t.enabled && t.weight > 0 && t.speeds.contains(&Speed::Standard))
    {
        let Some(upstream) = upstreams.get(&target.upstream_id).filter(|u| u.enabled) else {
            continue;
        };
        let Some(contract) = policy
            .context(Some(version), upstream.id, &target.upstream_model)
            .map_err(|_| CatalogError::Unavailable)?
        else {
            return Ok(ModelMetadata::unknown());
        };
        if contract.protocol != upstream.protocol
            || contract.endpoint.trim_end_matches('/')
                != upstream.endpoint.as_str().trim_end_matches('/')
        {
            return Ok(ModelMetadata::unknown());
        }
        contracts.push(contract);
    }
    if contracts.is_empty() {
        return Ok(ModelMetadata::unknown());
    }
    let minimum = |values: Vec<Option<u32>>| -> Option<u32> {
        values
            .into_iter()
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .min()
    };
    let boolean = |values: Vec<Option<bool>>| -> Option<bool> {
        Some(
            values
                .into_iter()
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .all(|v| v),
        )
    };
    let modalities = |values: Vec<Option<Vec<String>>>| -> Option<Vec<String>> {
        let mut values = values.into_iter().collect::<Option<Vec<_>>>()?.into_iter();
        let mut common = values.next()?;
        for value in values {
            common.retain(|m| value.contains(m));
        }
        Some(common)
    };
    Ok(ModelMetadata {
        context_window: minimum(
            contracts
                .iter()
                .map(|c| c.metadata.as_ref().and_then(|m| m.context_window))
                .collect(),
        ),
        max_input_tokens: contracts.iter().map(|c| c.max_input_tokens).min(),
        max_output_tokens: contracts.iter().map(|c| c.max_output_tokens).min(),
        input_modalities: modalities(
            contracts
                .iter()
                .map(|c| c.metadata.as_ref().and_then(|m| m.input_modalities.clone()))
                .collect(),
        ),
        output_modalities: modalities(
            contracts
                .iter()
                .map(|c| {
                    c.metadata
                        .as_ref()
                        .and_then(|m| m.output_modalities.clone())
                })
                .collect(),
        ),
        reasoning: boolean(
            contracts
                .iter()
                .map(|c| c.metadata.as_ref().and_then(|m| m.reasoning))
                .collect(),
        ),
        tool_call: boolean(
            contracts
                .iter()
                .map(|c| c.metadata.as_ref().and_then(|m| m.tool_call))
                .collect(),
        ),
        source: "reviewed_provider_contract",
        gateway_max_output_tokens: None,
    })
}

#[derive(Clone, Serialize)]
pub struct CatalogPricing {
    pub currency: &'static str,
    pub unit: &'static str,
    pub price_basis: &'static str,
    pub group_multiplier: Option<Number>,
    pub input: Option<Number>,
    pub output: Option<Number>,
    pub cache_read: Option<Number>,
    pub cache_write: Option<Number>,
    pub request: Option<Number>,
    pub final_cost_depends_on_usage: bool,
    /// Observation time of this projection, not a fabricated activation timestamp.
    pub updated_at: u64,
}

impl CatalogSurface {
    /// Resolve only an exact ID from the active Chat-capable catalog and a
    /// currently authorized group. Metadata is not spending authority.
    pub(super) fn exact_chat_model(
        &self,
        id: &str,
        authorized_groups: &[String],
    ) -> Result<Option<(String, String, ReleaseVersion)>, CatalogError> {
        let state = self.state.read().map_err(|_| CatalogError::Unavailable)?;
        let SurfaceState::Ready {
            router,
            version,
            models,
            ..
        } = &*state
        else {
            return Err(CatalogError::Unavailable);
        };
        if router.version().map_err(|_| CatalogError::Unavailable)? != *version {
            return Err(CatalogError::Unavailable);
        }
        Ok(models
            .iter()
            .find(|model| {
                model.id == id
                    && model.apis.contains(&"openai-completions")
                    && authorized_groups.contains(&model.group)
            })
            .map(|model| (model.group.clone(), model.upstream_model.clone(), *version)))
    }

    #[cfg_attr(not(test), allow(dead_code))] // Tests build a surface without provider policy.
    pub(super) fn from_gateway(
        router: Arc<Router>,
        credentials: &BTreeMap<(u64, u64), String>,
        total_timeout: Duration,
    ) -> Self {
        let limits = Limits {
            total_timeout,
            ..Limits::default()
        };
        Self::load(router.clone(), credentials, &limits, None).unwrap_or(Self {
            inventory_router: router,
            state: RwLock::new(SurfaceState::Disabled),
        })
    }
    pub(super) fn from_gateway_with_provider_policy(
        router: Arc<Router>,
        credentials: &BTreeMap<(u64, u64), String>,
        limits: &Limits,
        policy: &super::ProviderPolicy,
        reserved_input_tokens: i64,
    ) -> Self {
        Self::load(
            router.clone(),
            credentials,
            limits,
            Some((policy, reserved_input_tokens)),
        )
        .unwrap_or(Self {
            inventory_router: router,
            state: RwLock::new(SurfaceState::Disabled),
        })
    }

    fn load(
        router: Arc<Router>,
        credentials: &BTreeMap<(u64, u64), String>,
        limits: &Limits,
        provider_policy: Option<(&super::ProviderPolicy, i64)>,
    ) -> Result<Self, CatalogError> {
        // catalog_document takes one Arc snapshot, so config and prices cannot
        // come from different releases. It never consults the database pointer.
        let document = router
            .catalog_document()
            .map_err(|_| CatalogError::Unavailable)?;
        let version = ReleaseVersion {
            routing: document.routing.version,
            pricing: document.pricing.version,
        };
        let prices =
            PriceSnapshot::compile(document.pricing).map_err(|_| CatalogError::Unavailable)?;
        let upstreams: BTreeMap<_, _> = document
            .routing
            .upstreams
            .iter()
            .map(|u| (u.id, u))
            .collect();
        // Index once instead of scanning every route/alias for every group.
        let mut routes_by_group: BTreeMap<&str, Vec<&ModelRoute>> = BTreeMap::new();
        for route in &document.routing.routes {
            routes_by_group.entry(&route.group).or_default().push(route);
        }
        let mut aliases_by_route: BTreeMap<(&str, &str), Vec<&str>> = BTreeMap::new();
        for alias in &document.routing.aliases {
            aliases_by_route
                .entry((&alias.group, &alias.model))
                .or_default()
                .push(&alias.alias);
        }
        let mut groups = BTreeMap::new();
        let mut models = Vec::new();
        for group in document.routing.groups.iter().filter(|g| g.enabled) {
            let group_id = URL_SAFE_NO_PAD.encode(group.name.as_bytes());
            let mut group_ppm = Vec::new();
            for route in routes_by_group
                .get(group.name.as_str())
                .into_iter()
                .flatten()
                .copied()
                .filter(|r| r.enabled && r.speeds.contains(&Speed::Standard))
            {
                // Catalog metadata is not admission-time charging. quote() hides
                // calendar tariffs until a request timestamp exists; template()
                // keeps those publishable routes visible in the OAuth catalog.
                let quote = prices
                    .template(&route.model, &group.name, Speed::Standard)
                    .ok_or(CatalogError::Unavailable)?;
                // quote() omits calendar schedules, whose base template cannot
                // describe a single rate for all admission times.
                let fixed_tariff = prices
                    .quote(&route.model, &group.name, Speed::Standard)
                    .and_then(|price| price.tiered())
                    .map(|compiled| compiled.tariff());
                let names = std::iter::once(route.model.as_str()).chain(
                    aliases_by_route
                        .get(&(group.name.as_str(), route.model.as_str()))
                        .into_iter()
                        .flatten()
                        .copied(),
                );
                for name in names {
                    let apis: Vec<_> = [
                        (Protocol::OpenAiChat, "openai-completions"),
                        (Protocol::OpenAiResponses, "openai-responses"),
                        (Protocol::SystemOne, "system-one"),
                        (Protocol::OpenAiEmbeddings, "openai-embeddings"),
                        (Protocol::JinaRerank, "jina-rerank"),
                        (Protocol::DeepSeekSearch, "deepseek-search"),
                        (Protocol::DeepSeekFetch, "deepseek-fetch"),
                    ]
                    .into_iter()
                    .filter_map(|(client, label)| {
                        api_supported(
                            route,
                            name,
                            client,
                            &upstreams,
                            credentials,
                            limits.total_timeout,
                            quote,
                            provider_policy,
                            &document.routing,
                        )
                        .then_some(label)
                    })
                    .collect();
                    if apis.is_empty() {
                        continue;
                    }
                    let tool = apis
                        .iter()
                        .any(|api| matches!(*api, "deepseek-search" | "deepseek-fetch"));
                    // The fixed client wire contract sends upstream_model as
                    // JSON model. api_supported proves it equals this request
                    // alias for every possible target; do not rewrite aliases.
                    let mut metadata = model_metadata(
                        route,
                        &upstreams,
                        provider_policy.map(|(policy, _)| policy),
                        document.routing.version,
                    )?;
                    if apis.iter().any(|api| {
                        matches!(
                            *api,
                            "openai-completions" | "openai-responses" | "anthropic-messages"
                        )
                    }) {
                        metadata.gateway_max_output_tokens = Some(limits.max_output_tokens);
                    }
                    models.push(CatalogModel {
                        id: format!("lmm:{group_id}:{}", URL_SAFE_NO_PAD.encode(name.as_bytes())),
                        group_id: group_id.clone(),
                        group: group.name.clone(),
                        upstream_model: name.to_owned(),
                        name: name.to_owned(),
                        metadata,
                        apis,
                        pricing: pricing(
                            quote,
                            fixed_tariff.as_ref(),
                            tool || text_request_price::TextRequestPrice::from_quote(quote)
                                .is_some(),
                        )?,
                        native_cost: None,
                    });
                    group_ppm.push(quote.multipliers_ppm().0);
                    if models.len() > MAX_MODELS {
                        return Err(CatalogError::Unavailable);
                    }
                }
            }
            let multiplier = match group_ppm.first().copied() {
                Some(first) if group_ppm.iter().all(|ppm| *ppm == first) => {
                    Some(decimal(u128::from(first), 6)?)
                }
                _ => None,
            };
            groups.insert(
                group.name.clone(),
                CatalogGroup {
                    id: group_id.clone(),
                    name: group.name.clone(),
                    scope: format!("group:{group_id}"),
                    multiplier,
                },
            );
        }
        models.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
        if router.version().map_err(|_| CatalogError::Unavailable)? != version {
            return Err(CatalogError::Unavailable);
        }
        Ok(Self {
            inventory_router: router.clone(),
            state: RwLock::new(SurfaceState::Ready {
                router,
                version,
                groups,
                models,
            }),
        })
    }

    /// Authorized Router configuration inventory, not invocation or price authority.
    /// Independent image-service models are explicitly outside this version's scope.
    pub(crate) fn project_inventory(
        &self,
        authorized_groups: &[String],
        resource: &str,
        observed_at: u64,
    ) -> Result<serde_json::Value, CatalogError> {
        if resource.is_empty()
            || resource.len() > 2048
            || observed_at == 0
            || authorized_groups.is_empty()
        {
            return Err(CatalogError::Unavailable);
        }
        let document = self
            .inventory_router
            .catalog_document()
            .map_err(|_| CatalogError::Unavailable)?;
        let version = ReleaseVersion {
            routing: document.routing.version,
            pricing: document.pricing.version,
        };
        let allowed: std::collections::BTreeSet<_> = authorized_groups.iter().collect();
        let enabled: std::collections::BTreeSet<_> = document
            .routing
            .groups
            .iter()
            .filter(|g| g.enabled && allowed.contains(&g.name))
            .map(|g| g.name.as_str())
            .collect();
        let mut groups = Vec::new();
        for group in &enabled {
            let id = URL_SAFE_NO_PAD.encode(group.as_bytes());
            groups.push(serde_json::json!({"id":id,"name":group,"scope":format!("group:{id}")}));
        }
        let mut aliases: BTreeMap<(&str, &str), Vec<&str>> = BTreeMap::new();
        for alias in &document.routing.aliases {
            aliases
                .entry((&alias.group, &alias.model))
                .or_default()
                .push(&alias.alias);
        }
        let mut rows = Vec::new();
        for route in document
            .routing
            .routes
            .iter()
            .filter(|r| r.enabled && enabled.contains(r.group.as_str()))
        {
            let group_id = URL_SAFE_NO_PAD.encode(route.group.as_bytes());
            for name in std::iter::once(route.model.as_str()).chain(
                aliases
                    .get(&(route.group.as_str(), route.model.as_str()))
                    .into_iter()
                    .flatten()
                    .copied(),
            ) {
                // Bound temporary allocation as well as the final response. Never truncate.
                if rows.len() >= 20_000 {
                    return Err(CatalogError::Unavailable);
                }
                rows.push((
                    route.group.clone(),
                    name.to_owned(),
                    serde_json::json!({
                        "id":format!("lmm:{group_id}:{}", URL_SAFE_NO_PAD.encode(name.as_bytes())),
                        "group_id":group_id,"group":route.group,"public_model":name,
                        "canonical_model":route.model,"speeds":route.speeds,
                        "metadata_status":"unknown",
                        "context_window":null,"max_input_tokens":null,"max_output_tokens":null,
                        "capabilities":null
                    }),
                ));
            }
        }
        rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        let models: Vec<_> = rows.into_iter().map(|(_, _, row)| row).collect();
        let result = serde_json::json!({
            "inventory_schema_version":1,"scope":"router_routes_and_aliases",
            "includes_independent_image_service_models":false,
            "resource":resource,"observed_at":observed_at,
            "routing_version":version.routing,"pricing_version":version.pricing,
            "groups":groups,"models":models
        });
        if serde_json::to_vec(&result)
            .map_err(|_| CatalogError::Unavailable)?
            .len()
            > MAX_CATALOG_BYTES
            || self
                .inventory_router
                .version()
                .map_err(|_| CatalogError::Unavailable)?
                != version
        {
            return Err(CatalogError::Unavailable);
        }
        Ok(result)
    }

    /// groups must come from issuer ReadAuthority: token/family consent/current
    /// user policy/installation intersection, never a query or client nonce.
    /// Metadata alone grants no NativeKey or spending authority.
    pub(super) fn refresh(
        &self,
        credentials: &BTreeMap<(u64, u64), String>,
        limits: &Limits,
        policy: &super::ProviderPolicy,
        reserved_input_tokens: i64,
    ) -> Result<(), CatalogError> {
        let current = self
            .inventory_router
            .version()
            .map_err(|_| CatalogError::Unavailable)?;
        {
            let state = self.state.read().map_err(|_| CatalogError::Unavailable)?;
            if matches!(&*state, SurfaceState::Ready { version, .. } if *version == current) {
                return Ok(());
            }
        }
        let next = Self::load(
            self.inventory_router.clone(),
            credentials,
            limits,
            Some((policy, reserved_input_tokens)),
        )?;
        *self.state.write().map_err(|_| CatalogError::Unavailable)? = next
            .state
            .into_inner()
            .map_err(|_| CatalogError::Unavailable)?;
        Ok(())
    }

    pub fn project(
        &self,
        authorized_groups: &[String],
        resource: &str,
        updated_at: u64,
    ) -> Result<CatalogV1, CatalogError> {
        let state = self.state.read().map_err(|_| CatalogError::Unavailable)?;
        let SurfaceState::Ready {
            router,
            version,
            groups,
            models,
        } = &*state
        else {
            return Err(CatalogError::Unavailable);
        };
        if resource.is_empty()
            || resource.len() > 2048
            || updated_at == 0
            || authorized_groups.is_empty()
            || router.version().map_err(|_| CatalogError::Unavailable)? != *version
        {
            return Err(CatalogError::Unavailable);
        }
        let allowed: std::collections::BTreeSet<_> = authorized_groups.iter().collect();
        let mut result = CatalogV1 {
            schema_version: 1,
            resource: resource.to_owned(),
            updated_at,
            routing_version: version.routing,
            pricing_version: version.pricing,
            groups: groups
                .iter()
                .filter(|(name, _)| allowed.contains(name))
                .map(|(_, g)| g.clone())
                .collect(),
            models: models
                .iter()
                .filter(|m| allowed.contains(&m.group))
                .cloned()
                .collect(),
        };
        for model in &mut result.models {
            model.pricing.updated_at = updated_at;
            model.native_cost = NativeCost::from_pricing(&model.pricing);
        }
        let used_groups: std::collections::BTreeSet<_> =
            result.models.iter().map(|m| m.group.as_str()).collect();
        result
            .groups
            .retain(|g| used_groups.contains(g.name.as_str()));
        if serde_json::to_vec(&result)
            .map_err(|_| CatalogError::Unavailable)?
            .len()
            > MAX_CATALOG_BYTES
            || router.version().map_err(|_| CatalogError::Unavailable)? != *version
        {
            return Err(CatalogError::Unavailable);
        }
        Ok(result)
    }
}

// Route, protocol, credentials and provider policy are independent inputs.
#[allow(clippy::too_many_arguments)]
fn api_supported(
    route: &ModelRoute,
    name: &str,
    client: Protocol,
    upstreams: &BTreeMap<u64, &UpstreamConfig>,
    credentials: &BTreeMap<(u64, u64), String>,
    total_timeout: Duration,
    quote: PriceQuote,
    provider_policy: Option<(&super::ProviderPolicy, i64)>,
    routing: &crate::routing::RoutingConfig,
) -> bool {
    let routing_version = routing.version;
    let tool = ToolKind::from_protocol(client);
    if let Some(kind) = tool {
        if name != kind.model() || !PersistedToolPrice::supports_quote(kind, quote) {
            return false;
        }
    } else if super::routing::credit_price(quote, &route.model, &route.group).is_err() {
        return false;
    }
    // Health/operational stops can recover without publishing new config. Check
    // every potentially selectable target, including lower priority targets.
    let targets: Vec<_> = route
        .targets
        .iter()
        .filter_map(|t| upstreams.get(&t.upstream_id).map(|u| (t, *u)))
        .filter(|(t, u)| {
            t.enabled
                && t.weight > 0
                && u.enabled
                && t.speeds.contains(&Speed::Standard)
                && t.allows_client(client)
                && (t.payload.adapts(client, u.protocol)
                    || t.raw_http_model(
                        client,
                        u.protocol,
                        routing.global_model_policy.as_ref(),
                        routing.channel_overrides.get(&u.id),
                    ) == Some(name))
        })
        .collect();
    // Catalog v1 has no way to require an explicit output cap or streamed Chat
    // usage receipts. Its fixed clients must work with every selectable target.
    !targets.is_empty()
        && targets.into_iter().all(|(t, u)| {
            t.payload.adapts(client, u.protocol)
                && u128::from(t.timeout_ms) >= total_timeout.as_millis()
                && (t.upstream_model == name || client.input_only() || tool.is_some())
                && (quote.billing_mode() != BillingMode::UpstreamCost
                    || t.upstream_model == route.model)
                && credentials
                    .get(&(u.credential.id, u.credential.revision))
                    .is_some_and(|secret| {
                        if let Some(kind) = tool {
                            u.protocol == client
                                && t.payload == PayloadPolicy::Adapt
                                && provider_policy.is_some_and(|(policy, _)| {
                                    policy
                                        .validate_binding_at(
                                            routing_version,
                                            u.id,
                                            &t.upstream_model,
                                            u.protocol,
                                            u.endpoint.as_str(),
                                            0,
                                            0,
                                        )
                                        .is_ok()
                                })
                                && reqwest::header::HeaderValue::from_str(&format!(
                                    "Bearer {secret}"
                                ))
                                .is_ok()
                                && super::tool_api::endpoint(u.endpoint.as_str(), kind).is_ok()
                        } else if client == Protocol::SystemOne {
                            u.protocol == Protocol::SystemOne
                                && t.payload == PayloadPolicy::Adapt
                                && provider_policy.is_some_and(|(policy, bound)| {
                                    bound >= 65_536
                                        && policy
                                            .validate_binding_at(
                                                routing_version,
                                                u.id,
                                                &t.upstream_model,
                                                u.protocol,
                                                u.endpoint.as_str(),
                                                bound,
                                                0,
                                            )
                                            .is_ok()
                                })
                                && quote.is_input_only_scalar()
                                && reqwest::header::HeaderValue::from_str(&format!(
                                    "Bearer {secret}"
                                ))
                                .is_ok()
                                && super::systemone::endpoint(u.endpoint.as_str()).is_ok()
                        } else if client.input_only() {
                            u.protocol == client
                                && t.payload == PayloadPolicy::Adapt
                                && provider_policy.is_some_and(|(policy, bound)| {
                                    policy
                                        .validate_binding_at(
                                            routing_version,
                                            u.id,
                                            &t.upstream_model,
                                            u.protocol,
                                            u.endpoint.as_str(),
                                            bound,
                                            0,
                                        )
                                        .is_ok()
                                })
                                && quote.is_input_only_scalar()
                                && reqwest::header::HeaderValue::from_str(&format!(
                                    "Bearer {secret}"
                                ))
                                .is_ok()
                                && super::input_api::endpoint(u.endpoint.as_str(), u.protocol)
                                    .is_ok()
                        } else {
                            route_is_constructible(route, t, u, secret, quote)
                        }
                    })
        })
}

fn route_is_constructible(
    route: &ModelRoute,
    target: &crate::routing::RouteTarget,
    upstream: &UpstreamConfig,
    secret: &str,
    quote: PriceQuote,
) -> bool {
    let protocol = match upstream.protocol {
        Protocol::OpenAiChat => crate::relay::UpstreamProtocol::Chat,
        Protocol::OpenAiResponses => crate::relay::UpstreamProtocol::Responses,
        _ => return false,
    };
    // Match operation_endpoint's no-I/O URL boundary before the real relay
    // constructor, including public-literal validation and explicit loopback.
    let Ok(mut endpoint) =
        crate::relay::honeypot_network::validate_base_url(upstream.endpoint.as_str(), true)
    else {
        return false;
    };
    {
        let Ok(mut segments) = endpoint.path_segments_mut() else {
            return false;
        };
        segments.pop_if_empty();
        match protocol {
            crate::relay::UpstreamProtocol::Chat => {
                segments.push("chat").push("completions");
            }
            crate::relay::UpstreamProtocol::Responses => {
                segments.push("responses");
            }
            _ => return false,
        }
    }
    // Use the real no-I/O route constructor, including sensitive HeaderValue
    // validation. A credential ref being loaded does not prove a usable header.
    crate::relay::Route::new(
        crate::routing::route_key(route.id, target.id, upstream.id),
        protocol,
        endpoint.as_str(),
        target.upstream_model.clone(),
        secret,
        quote.version(),
    )
    .is_ok()
}

fn pricing(
    quote: PriceQuote,
    fixed_tariff: Option<&TieredTariff>,
    tool: bool,
) -> Result<CatalogPricing, CatalogError> {
    let (group_ppm, speed_ppm) = quote.multipliers_ppm();
    let known = quote.billing_mode() == BillingMode::Tokens;
    let rate = |value: u64| {
        if !known {
            return Ok(None);
        }
        let numerator = u128::from(value)
            .checked_mul(u128::from(group_ppm))
            .and_then(|v| v.checked_mul(u128::from(speed_ppm)))
            .ok_or(CatalogError::Unavailable)?;
        // micro-USD per million tokens, with both ppm multipliers applied.
        // arbitrary_precision is already enabled in the product dependency.
        decimal(numerator, 18).map(Some)
    };
    let rates = quote.rates();
    let mut pricing = CatalogPricing {
        currency: "USD",
        unit: if tool {
            "successful_requests"
        } else if known {
            "million_tokens"
        } else {
            "unknown"
        },
        price_basis: if known {
            "configured_base_rates"
        } else {
            "unknown"
        },
        group_multiplier: Some(decimal(u128::from(group_ppm), 6)?),
        input: if tool { None } else { rate(rates.input)? },
        output: if tool { None } else { rate(rates.output)? },
        cache_read: if tool { None } else { rate(rates.cache_read)? },
        cache_write: if tool { None } else { rate(rates.cache_write)? },
        request: if tool { rate(rates.request)? } else { None },
        final_cost_depends_on_usage: true,
        updated_at: 0,
    };
    if !tool
        && let Some(tariff) = fixed_tariff
        && let [tier] = tariff.tiers.as_slice()
        && tier.up_to_input_tokens.is_none()
    {
        let rates = tier.rates;
        let effective = |rate| exact_tier_rate(rate, group_ppm, speed_ppm);
        pricing.input = effective(rates.input)?;
        pricing.output = effective(rates.output)?;
        // An omitted cache dimension can be part of inclusive input.
        if tariff.uses_cache_read {
            pricing.cache_read = effective(rates.cache_read)?;
        }
        // One generic cache-write field can only carry a common TTL price.
        // Rev 1 forbids creation. In rev 2 an absent one-hour rate is zero.
        if tariff.uses_cache_write {
            let write = effective(rates.cache_write)?;
            let alternate = match tariff.revision {
                2 => Some(rates.cache_write_1h.unwrap_or(DecimalRate {
                    numerator: 0,
                    denominator: 1,
                })),
                3 => rates.cache_write_1h,
                4 => rates.cache_write_12h,
                _ => None,
            };
            if let Some(alternate) = alternate
                && write.is_some()
                && effective(alternate)? == write
            {
                pricing.cache_write = write;
            }
        }
        if pricing.input.is_some()
            || pricing.output.is_some()
            || pricing.cache_read.is_some()
            || pricing.cache_write.is_some()
        {
            pricing.unit = "million_tokens";
            pricing.price_basis = "configured_base_rates";
        }
    }
    Ok(pricing)
}

// The catalog wire carries JSON decimals. Return only a terminating expansion,
// including cases where a multiplier cancels a recurring source denominator.
fn exact_tier_rate(
    rate: DecimalRate,
    group_ppm: u32,
    speed_ppm: u32,
) -> Result<Option<Number>, CatalogError> {
    let numerator = u128::from(rate.numerator) * u128::from(group_ppm) * u128::from(speed_ppm);
    if rate.denominator == 0 {
        return Err(CatalogError::Unavailable);
    }
    // The ppm denominator adds only factors of 2 and 5. Every other
    // prime factor of the source denominator must cancel in the numerator.
    let mut other = rate.denominator >> rate.denominator.trailing_zeros();
    while other.is_multiple_of(5) {
        other /= 5;
    }
    if !numerator.is_multiple_of(u128::from(other)) {
        return Ok(None);
    }
    let denominator = u128::from(rate.denominator) * 1_000_000_000_000;
    let mut remainder = numerator % denominator;
    let mut text = (numerator / denominator).to_string();
    if remainder != 0 {
        text.push('.');
        while remainder != 0 {
            // Even ten times the largest u64 denominator scaled by 10^12 fits u128.
            remainder *= 10;
            text.push(char::from(b'0' + (remainder / denominator) as u8));
            remainder %= denominator;
        }
    }
    text.parse()
        .map(Some)
        .map_err(|_| CatalogError::Unavailable)
}

fn decimal(numerator: u128, scale: u32) -> Result<Number, CatalogError> {
    let denominator = 10_u128.pow(scale);
    let whole = numerator / denominator;
    let remainder = numerator % denominator;
    let text = if remainder == 0 {
        whole.to_string()
    } else {
        let fraction = format!("{remainder:0width$}", width = scale as usize);
        format!("{whole}.{}", fraction.trim_end_matches('0'))
    };
    text.parse().map_err(|_| CatalogError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{
        CredentialRef, Endpoint, FailoverPolicy, ModelAlias, ModelGroup, PriceConfig, PriceRule,
        RouteTarget, RoutingConfig, TokenRates,
    };

    fn fixture(model: &str) -> (RoutingConfig, PriceConfig) {
        (
            RoutingConfig {
                channel_overrides: Default::default(),
                global_model_policy: None,
                channel_affinity: None,
                routing_reliability: None,
                version: 1,
                groups: vec![ModelGroup {
                    name: "synthetic".into(),
                    enabled: true,
                }],
                upstreams: vec![UpstreamConfig {
                    id: 1,
                    endpoint: Endpoint::local_loopback("http://127.0.0.1:4444/v1".into()).unwrap(),
                    protocol: Protocol::OpenAiChat,
                    credential: CredentialRef { id: 1, revision: 1 },
                    enabled: true,
                }],
                routes: vec![ModelRoute {
                    id: 1,
                    model: model.into(),
                    group: "synthetic".into(),
                    enabled: true,
                    speeds: vec![Speed::Standard],
                    targets: vec![RouteTarget {
                        id: 1,
                        upstream_id: 1,
                        upstream_model: model.into(),
                        enabled: true,
                        priority: 0,
                        weight: 1,
                        speeds: vec![Speed::Standard],
                        client_protocols: None,
                        payload: PayloadPolicy::Adapt,
                        timeout_ms: 5000,
                    }],
                    failover: FailoverPolicy::default(),
                }],
                aliases: vec![],
            },
            PriceConfig {
                version: 1,
                rules: vec![PriceRule {
                    rounding: None,
                    billing_mode: BillingMode::Tokens,
                    tiered: None,
                    id: 1,
                    model: model.into(),
                    group: "synthetic".into(),
                    speed: Speed::Standard,
                    rates: TokenRates {
                        input: 2_000_000,
                        output: 4_000_000,
                        cache_read: 1_000_000,
                        cache_write: 0,
                        request: 0,
                    },
                    group_multiplier_ppm: 1_250_000,
                    speed_multiplier_ppm: 1_000_000,
                }],
            },
        )
    }

    fn surface(config: RoutingConfig, prices: PriceConfig) -> (Arc<Router>, CatalogSurface) {
        let router = Arc::new(
            Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap())).unwrap(),
        );
        let credentials = BTreeMap::from([((1, 1), "synthetic-only-not-a-provider-key".into())]);
        let surface =
            CatalogSurface::from_gateway(router.clone(), &credentials, Duration::from_secs(1));
        (router, surface)
    }

    fn view(surface: &CatalogSurface) -> CatalogV1 {
        surface
            .project(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123,
            )
            .unwrap()
    }

    #[test]
    fn router_inventory_keeps_aliases_without_callable_v1_apis() {
        let (mut r, p) = fixture("model");
        r.routes[0].targets[0].upstream_model = "mapped-upstream".into();
        r.aliases.push(ModelAlias {
            group: "synthetic".into(),
            alias: "alias".into(),
            model: "model".into(),
        });
        let (_, s) = surface(r, p);
        assert!(view(&s).models.is_empty());
        let inventory = s
            .project_inventory(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123,
            )
            .unwrap();
        let rows = inventory["models"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["public_model"], "alias");
        assert_eq!(rows[0]["canonical_model"], "model");
        assert!(rows[0]["context_window"].is_null());
        assert_eq!(
            inventory["includes_independent_image_service_models"],
            false
        );
        let wire = inventory.to_string();
        assert!(!wire.contains("mapped-upstream"));
        assert!(!wire.contains("127.0.0.1"));
        assert!(!wire.contains("synthetic-only-not-a-provider-key"));
        let denied = s
            .project_inventory(
                &["other".into()],
                "https://synthetic.invalid/api/oauth2",
                123,
            )
            .unwrap();
        assert!(denied["models"].as_array().unwrap().is_empty());
        assert!(denied["groups"].as_array().unwrap().is_empty());
        let disabled = CatalogSurface {
            inventory_router: s.inventory_router.clone(),
            state: RwLock::new(SurfaceState::Disabled),
        };
        assert_eq!(
            disabled
                .project_inventory(
                    &["synthetic".into()],
                    "https://synthetic.invalid/api/oauth2",
                    123
                )
                .unwrap(),
            inventory
        );
    }

    #[test]
    fn router_inventory_keeps_fast_only_and_filters_disabled_routes() {
        let (mut r, mut p) = fixture("model");
        r.routes[0].speeds = vec![Speed::Fast];
        r.routes[0].targets[0].speeds = vec![Speed::Fast];
        p.rules[0].speed = Speed::Fast;
        let (_, s) = surface(r.clone(), p.clone());
        assert!(view(&s).models.is_empty());
        let inventory = s
            .project_inventory(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123,
            )
            .unwrap();
        assert_eq!(
            inventory["models"][0]["speeds"],
            serde_json::json!(["fast"])
        );
        r.routes[0].enabled = false;
        let (_, s) = surface(r, p);
        assert!(
            s.project_inventory(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123
            )
            .unwrap()["models"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn client_protocols_public_metadata_matches_target_restrictions() {
        for (restriction, apis) in [
            (None, vec!["openai-completions", "openai-responses"]),
            (
                Some(vec![Protocol::OpenAiResponses]),
                vec!["openai-responses"],
            ),
        ] {
            let (mut config, prices) = fixture("model");
            config.routes[0].targets[0].client_protocols = restriction;
            let (router, surface) = surface(config, prices);
            let value = serde_json::to_value(view(&surface)).unwrap();
            assert_eq!(value["models"][0]["apis"], serde_json::json!(apis));
            for (protocol, api) in [
                (Protocol::OpenAiChat, "openai-completions"),
                (Protocol::OpenAiResponses, "openai-responses"),
            ] {
                let callable = router
                    .route_adapted(crate::routing::RouteRequest {
                        model: "model",
                        group: "synthetic",
                        protocol,
                        speed: None,
                        replay: crate::routing::ReplayPermission::Denied,
                    })
                    .is_ok();
                assert_eq!(callable, apis.contains(&api));
            }
        }
    }

    #[test]
    fn catalog_reports_the_configured_relay_output_limit() {
        let (routing, prices) = fixture("model");
        let (router, _) = surface(routing, prices);
        let credentials = BTreeMap::from([((1, 1), "synthetic-catalog-key".into())]);
        let limits = Limits {
            max_output_tokens: 12_345,
            ..Limits::default()
        };
        let catalog = CatalogSurface::from_gateway_with_provider_policy(
            router,
            &credentials,
            &limits,
            &super::super::ProviderPolicy::default(),
            65536,
        );
        let result = view(&catalog);
        assert_eq!(result.models.len(), 1);
        let metadata = serde_json::to_value(&result.models[0].metadata).unwrap();
        assert_eq!(metadata["gateway_max_output_tokens"], 12_345);
        assert!(metadata["max_output_tokens"].is_null());
    }

    #[test]
    fn metadata_uses_reviewed_release_and_conservative_fallback_bounds() {
        let (mut routing, _) = fixture("new-model");
        routing.upstreams[0].endpoint =
            serde_json::from_value(serde_json::json!("https://api.example/v1")).unwrap();
        let mut second = routing.routes[0].targets[0].clone();
        second.id = 2;
        second.upstream_model = "fallback-model".into();
        routing.routes[0].targets.push(second);
        let contract = |model: &str, input: u32, output: u32, metadata: serde_json::Value| {
            serde_json::from_value::<super::super::ExternalModelContext>(serde_json::json!({
                "upstream_id":1,"upstream_model":model,"protocol":"openai_chat",
                "endpoint":"https://api.example/v1","max_input_tokens":input,
                "max_output_tokens":output,"contract_revision":"test-reviewed-v1","metadata":metadata
            })).unwrap()
        };
        let first = contract(
            "new-model",
            64000,
            8000,
            serde_json::json!({
                "context_window":128000,"input_modalities":["text","image"],
                "output_modalities":["text"],"reasoning":true,"tool_call":true
            }),
        );
        let second = contract(
            "fallback-model",
            32000,
            4000,
            serde_json::json!({
                "context_window":64000,"input_modalities":["text"],
                "output_modalities":["text"],"reasoning":false,"tool_call":true
            }),
        );
        let policy =
            super::super::ProviderPolicy::new(vec![first.clone(), second.clone()], 128000).unwrap();
        let upstreams = routing.upstreams.iter().map(|u| (u.id, u)).collect();
        let value = serde_json::to_value(
            model_metadata(&routing.routes[0], &upstreams, Some(&policy), 1).unwrap(),
        )
        .unwrap();
        assert_eq!(value["context_window"], 64000);
        assert_eq!(value["max_input_tokens"], 32000);
        assert_eq!(value["max_output_tokens"], 4000);
        assert_eq!(value["input_modalities"], serde_json::json!(["text"]));
        assert_eq!(value["reasoning"], false);
        assert_eq!(value["tool_call"], true);
        let mut unknown = second.clone();
        unknown.metadata = None;
        policy
            .publish_contexts(2, vec![first.clone(), unknown], 128000)
            .unwrap();
        let value = serde_json::to_value(
            model_metadata(&routing.routes[0], &upstreams, Some(&policy), 2).unwrap(),
        )
        .unwrap();
        assert!(value["context_window"].is_null());
        assert!(value["input_modalities"].is_null());
        assert_eq!(value["max_input_tokens"], 32000);
        assert_eq!(
            model_metadata(&routing.routes[0], &upstreams, Some(&policy), 1)
                .unwrap()
                .context_window,
            Some(64000)
        );
        let incomplete = super::super::ProviderPolicy::new(vec![first.clone()], 128000).unwrap();
        assert_eq!(
            model_metadata(&routing.routes[0], &upstreams, Some(&incomplete), 1)
                .unwrap()
                .source,
            "unknown"
        );
        let mut invalid = first;
        invalid.metadata.as_mut().unwrap().context_window = Some(1);
        assert!(super::super::ProviderPolicy::new(vec![invalid], 128000).is_err());
    }

    #[test]
    fn configured_rates_are_exact_and_group_price_is_not_multiplied_twice() {
        let (r, p) = fixture("model");
        let (_, s) = surface(r, p);
        let v = serde_json::to_value(view(&s)).unwrap();
        assert_eq!(
            v["models"][0]["apis"],
            serde_json::json!(["openai-completions", "openai-responses"])
        );
        assert_eq!(v["models"][0]["pricing"]["input"].to_string(), "2.5");
        assert_eq!(v["models"][0]["pricing"]["output"].to_string(), "5");
        assert_eq!(v["models"][0]["pricing"]["cache_read"].to_string(), "1.25");
        assert!(v["models"][0]["pricing"].get("trust_multiplier").is_none());
        assert_eq!(
            v["models"][0]["native_cost"],
            serde_json::json!({"input":2.5,"output":5,"cacheRead":1.25,"cacheWrite":0})
        );
        assert_eq!(v["groups"][0]["scope"], "group:c3ludGhldGlj");
    }

    #[test]
    fn legacy_openai_catalog_advertises_only_the_selected_wire_protocol() {
        for (protocol, api) in [
            (Protocol::OpenAiChat, "openai-completions"),
            (Protocol::OpenAiResponses, "openai-responses"),
        ] {
            let (mut config, prices) = fixture("model");
            config.upstreams[0].protocol = protocol;
            config.routes[0].targets[0].payload =
                serde_json::from_value(serde_json::json!({"legacy_openai":{}})).unwrap();
            let (_, surface) = surface(config, prices);
            let value = serde_json::to_value(view(&surface)).unwrap();
            assert_eq!(value["models"][0]["apis"], serde_json::json!([api]));
            assert_eq!(value["models"][0]["pricing"]["input"].to_string(), "2.5");
        }
    }

    #[test]
    fn one_supported_target_does_not_hide_an_unsupported_recoverable_target() {
        let (mut r, p) = fixture("model");
        let mut other = r.upstreams[0].clone();
        other.id = 2;
        other.protocol = Protocol::Gemini;
        r.upstreams.push(other);
        let mut target = r.routes[0].targets[0].clone();
        target.id = 2;
        target.upstream_id = 2;
        target.priority = 99;
        r.routes[0].targets.push(target);
        let router =
            Arc::new(Router::new(r, Arc::new(PriceSnapshot::compile(p).unwrap())).unwrap());
        router
            .set_health(
                router.version().unwrap(),
                2,
                crate::routing::HealthStatus::Unhealthy,
            )
            .unwrap();
        let credentials = BTreeMap::from([((1, 1), "synthetic-only-not-a-provider-key".into())]);
        let s = CatalogSurface::from_gateway(router, &credentials, Duration::from_secs(1));
        assert!(view(&s).models.is_empty());
    }

    #[test]
    fn public_alias_is_preserved_only_when_wire_model_is_unambiguous() {
        let (mut r, p) = fixture("canonical");
        r.routes[0].targets[0].upstream_model = "actual-alias".into();
        r.aliases.push(ModelAlias {
            group: "synthetic".into(),
            alias: "actual-alias".into(),
            model: "canonical".into(),
        });
        r.aliases.push(ModelAlias {
            group: "synthetic".into(),
            alias: "other-alias".into(),
            model: "canonical".into(),
        });
        let (_, s) = surface(r, p);
        let v = view(&s);
        assert_eq!(v.models.len(), 1);
        assert_eq!(v.models[0].upstream_model, "actual-alias");
        assert_eq!(v.models[0].name, "actual-alias");
        assert!(v.models[0].id.ends_with("YWN0dWFsLWFsaWFz"));
    }

    #[test]
    fn input_only_catalog_advertises_its_native_api_and_validates_price() {
        for (protocol, api) in [
            (Protocol::OpenAiEmbeddings, "openai-embeddings"),
            (Protocol::JinaRerank, "jina-rerank"),
        ] {
            let (mut config, mut prices) = fixture("public-input");
            config.upstreams[0].protocol = protocol;
            config.routes[0].targets[0].upstream_model = "provider-input".into();
            prices.rules[0].rates = TokenRates {
                input: 2000000,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                request: 0,
            };
            for valid in [true, false] {
                prices.rules[0].rates.output = if valid { 0 } else { 1000000 };
                let router = Arc::new(
                    Router::new(
                        config.clone(),
                        Arc::new(PriceSnapshot::compile(prices.clone()).unwrap()),
                    )
                    .unwrap(),
                );
                let credentials = BTreeMap::from([((1, 1), "synthetic-input-only-key".into())]);
                let surface = CatalogSurface::from_gateway_with_provider_policy(
                    router,
                    &credentials,
                    &Limits {
                        total_timeout: Duration::from_secs(1),
                        ..Limits::default()
                    },
                    &super::super::ProviderPolicy::default(),
                    2000,
                );
                let result = view(&surface);
                if valid {
                    assert_eq!(result.models.len(), 1);
                    assert_eq!(result.models[0].apis, [api]);
                } else {
                    assert!(result.models.is_empty());
                }
            }
        }
    }

    #[test]
    fn systemone_catalog_admits_only_safe_scalar_decimal_tariff() {
        let (mut config, mut prices) = fixture("jev-native");
        config.upstreams[0].protocol = Protocol::SystemOne;
        prices.rules[0].billing_mode = BillingMode::TieredDecimal;
        prices.rules[0].rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        let scalar: crate::routing::TieredTariff = serde_json::from_value(serde_json::json!({
            "revision":1,"source_digest":"af0d4b7d188d58f20f9eb13cef18353659ca208151824014795189b70dc0afd7",
            "uses_cache_read":false,"uses_cache_write":false,
            "tiers":[{"rates":{"input":{"numerator":21,"denominator":500},"output":{"numerator":0,"denominator":1},"cache_read":{"numerator":0,"denominator":1},"cache_write":{"numerator":0,"denominator":1}}}]
        })).unwrap();
        for accepted in [true, false] {
            let mut tariff = scalar.clone();
            if !accepted {
                tariff.tiers[0].rates.output.numerator = 1;
            }
            prices.rules[0].tiered = Some(tariff);
            let router = Arc::new(
                Router::new(
                    config.clone(),
                    Arc::new(PriceSnapshot::compile(prices.clone()).unwrap()),
                )
                .unwrap(),
            );
            let credentials = BTreeMap::from([((1, 1), "synthetic-systemone-only".into())]);
            let surface = CatalogSurface::from_gateway_with_provider_policy(
                router,
                &credentials,
                &Limits {
                    total_timeout: Duration::from_secs(1),
                    ..Limits::default()
                },
                &super::super::ProviderPolicy::default(),
                65536,
            );
            let result = view(&surface);
            if accepted {
                assert_eq!(result.models.len(), 1);
                assert_eq!(result.models[0].apis, ["system-one"]);
                let price = serde_json::to_value(&result.models[0].pricing).unwrap();
                assert_eq!(price["input"].to_string(), "0.0525");
                assert_eq!(price["output"], 0);
                assert!(result.models[0].native_cost.is_none());
                for field in ["cache_read", "cache_write", "request"] {
                    assert!(price[field].is_null(), "{field}");
                }
            } else {
                assert!(result.models.is_empty());
            }
        }
    }

    #[test]
    fn native_tool_catalog_requires_explicit_protocol_and_request_only_price() {
        for (protocol, kind) in [
            (Protocol::DeepSeekSearch, ToolKind::DeepSeekSearch),
            (Protocol::DeepSeekFetch, ToolKind::DeepSeekFetch),
        ] {
            let (mut config, mut prices) = fixture(kind.model());
            config.upstreams[0].protocol = protocol;
            config.routes[0].targets[0].upstream_model = "provider-tool-contract".into();
            config.aliases.push(ModelAlias {
                group: "synthetic".into(),
                alias: "uncallable-tool-alias".into(),
                model: kind.model().into(),
            });
            prices.rules[0].rates = TokenRates {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                request: 750,
            };
            let project = |config: RoutingConfig, prices: PriceConfig| {
                let router = Arc::new(
                    Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap())).unwrap(),
                );
                let credentials = BTreeMap::from([((1, 1), "synthetic-native-tool-key".into())]);
                view(&CatalogSurface::from_gateway_with_provider_policy(
                    router,
                    &credentials,
                    &Limits {
                        total_timeout: Duration::from_secs(1),
                        ..Limits::default()
                    },
                    &super::super::ProviderPolicy::default(),
                    2000,
                ))
            };
            let result = project(config.clone(), prices.clone());
            assert_eq!(result.models.len(), 1);
            let model = &result.models[0];
            assert_eq!(model.name, kind.model());
            assert_eq!(model.apis, [kind.model()]);
            assert_eq!(model.pricing.unit, "successful_requests");
            assert!(model.pricing.input.is_none() && model.pricing.output.is_none());
            assert_eq!(
                model.pricing.request.as_ref().unwrap().to_string(),
                "0.0009375"
            );
            for dimension in 0..4 {
                let mut invalid = prices.clone();
                match dimension {
                    0 => invalid.rules[0].rates.input = 1,
                    1 => invalid.rules[0].rates.output = 1,
                    2 => invalid.rules[0].rates.cache_read = 1,
                    _ => invalid.rules[0].rates.cache_write = 1,
                };
                assert!(project(config.clone(), invalid).models.is_empty());
            }
            config.upstreams[0].protocol = Protocol::OpenAiChat;
            config.routes[0].targets[0].upstream_model = kind.model().into();
            let text = project(config, prices);
            assert_eq!(text.models.len(), 1);
            assert_eq!(
                text.models[0].apis,
                ["openai-completions", "openai-responses"]
            );
            assert_eq!(text.models[0].pricing.unit, "successful_requests");
            assert!(text.models[0].pricing.input.is_none());
            assert!(text.models[0].pricing.request.is_some());
        }
    }
    #[test]
    fn tiered_catalog_prices_exact_flat_dimensions_and_preserves_variable_prices() {
        use serde_json::json;
        use sha2::{Digest, Sha256};

        let model = |tariff| {
            let (routing, mut config) = fixture("model");
            let rule = &mut config.rules[0];
            rule.billing_mode = BillingMode::TieredDecimal;
            rule.tiered = Some(serde_json::from_value(tariff).unwrap());
            rule.rates = TokenRates {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                request: 0,
            };
            let (_, catalog) = surface(routing, config);
            serde_json::to_value(view(&catalog).models.remove(0)).unwrap()
        };
        let rate = |n: u64, d: u64| json!({"numerator": n, "denominator": d});
        let tier = json!({"rates": {
            "input":rate(2, 5), "output":rate(1, 3), "cache_read":rate(1, 10),
            "cache_write":rate(1, 2), "cache_write_1h":rate(3, 2)
        }});
        let source = "catalog-test";
        let mut tariff = json!({
            "revision":3, "source_digest":crate::hex_codec::encode(Sha256::digest(source.as_bytes())),
            "uses_cache_read":true, "uses_cache_write":true, "uses_cache_write_1h":true,
            "tiers":[tier]
        });
        let row = model(tariff.clone());
        let price = &row["pricing"];
        assert_eq!(price["input"].to_string(), "0.5");
        assert_eq!(price["cache_read"].to_string(), "0.125");
        assert!(price["output"].is_null()); // 5/12 has no finite JSON decimal.
        assert!(price["cache_write"].is_null());
        assert!(row["native_cost"].is_null());
        tariff["tiers"][0]["rates"]["output"] = rate(3, 1);
        let row = model(tariff.clone());
        assert!(row["native_cost"].is_null()); // The two write TTLs alone differ.
        tariff["tiers"][0]["rates"]["cache_write_1h"] = rate(1, 2);
        let row = model(tariff.clone());
        assert_eq!(
            row["native_cost"],
            json!({
                "input":0.5, "output":3.75, "cacheRead":0.125, "cacheWrite":0.625
            })
        );

        // Revision 4 has a separate 12h rate and no 1h category.
        let mut twelve = tariff.clone();
        twelve["revision"] = json!(4);
        twelve["uses_cache_write_1h"] = json!(false);
        twelve["uses_cache_write_12h"] = json!(true);
        let rates = twelve["tiers"][0]["rates"].as_object_mut().unwrap();
        rates.remove("cache_write_1h");
        rates.insert("cache_write_12h".into(), rate(1, 1));
        let row = model(twelve.clone());
        assert!(row["pricing"]["cache_write"].is_null());
        assert!(row["native_cost"].is_null());
        twelve["tiers"][0]["rates"]["cache_write_12h"] = rate(2, 4);
        assert_eq!(
            model(twelve)["native_cost"]["cacheWrite"].to_string(),
            "0.625"
        );

        // Revision 2 bills a missing one-hour term at zero.
        let mut short = tariff.clone();
        short["revision"] = json!(2);
        short["uses_cache_write_1h"] = json!(false);
        short["tiers"][0]["rates"]
            .as_object_mut()
            .unwrap()
            .remove("cache_write_1h");
        assert!(model(short.clone())["pricing"]["cache_write"].is_null());
        short["tiers"][0]["rates"]["cache_write"] = rate(0, 1);
        assert_eq!(model(short)["native_cost"]["cacheWrite"], 0);

        // Independent fixtures for time-varying and length-dependent tariffs.
        tariff["revision"] = json!(2);
        let mut peak = tariff["tiers"][0].clone();
        peak["rates"]["input"] = rate(2, 1);
        let mut calendar = tariff.clone();
        calendar["calendar"] = json!({
            "revision":1, "source_expression":source, "weekday_timezone":"UTC", "weekdays":[1],
            "hour_timezone":"UTC", "hour_ranges":[{"start":1,"end":4}],
            "excluded_date_timezone":"Asia/Shanghai", "excluded_dates":[], "match_tiers":[peak]
        });
        let mut bounded = tariff["tiers"][0].clone();
        bounded["up_to_input_tokens"] = json!(100);
        bounded["rates"]["input"] = rate(2, 1);
        tariff["tiers"].as_array_mut().unwrap().insert(0, bounded);
        for variable in [calendar, tariff] {
            let row = model(variable);
            assert_eq!(row["pricing"]["price_basis"], "unknown");
            assert!(row["pricing"]["input"].is_null());
            assert!(row["native_cost"].is_null());
        }
    }

    #[test]
    fn tiered_catalog_decimal_is_exact_after_both_multipliers() {
        let price = |n, d, group, speed| {
            exact_tier_rate(
                DecimalRate {
                    numerator: n,
                    denominator: d,
                },
                group,
                speed,
            )
        };
        assert!(price(1, 0, 1_000_000, 1_000_000).is_err());
        assert_eq!(
            price(1, 3, 1_500_000, 1_000_000)
                .unwrap()
                .unwrap()
                .to_string(),
            "0.5"
        );
        assert_eq!(price(1, 3, 1_000_000, 1_000_000).unwrap(), None);
        assert_eq!(
            price(1, 1_000_000, 1_000_000, 500_000)
                .unwrap()
                .unwrap()
                .to_string(),
            "0.0000005"
        );
        assert_eq!(
            price(9_007_199_254_740_993, 10, 1_000_000, 1_000_000)
                .unwrap()
                .unwrap()
                .to_string(),
            "900719925474099.3"
        );
        assert_eq!(price(1, 3, 0, 1_000_000).unwrap().unwrap().to_string(), "0");
    }

    #[test]
    fn upstream_cost_sentinel_is_unknown_and_never_a_free_price() {
        let (r, mut p) = fixture("openrouter/fusion");
        p.rules[0].billing_mode = BillingMode::UpstreamCost;
        p.rules[0].rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        let (_, s) = surface(r, p);
        let v = serde_json::to_value(view(&s)).unwrap();
        assert_eq!(v["models"].as_array().unwrap().len(), 1);
        let price = &v["models"][0]["pricing"];
        assert_eq!(price["unit"], "unknown");
        for field in ["input", "output", "cache_read", "cache_write", "request"] {
            assert!(price[field].is_null(), "{field}");
        }
    }

    #[test]
    fn default_speed_uses_standard_price_and_fast_only_has_no_default_row() {
        let (mut r, mut p) = fixture("model");
        r.routes[0].speeds.push(Speed::Fast);
        r.routes[0].targets[0].speeds.push(Speed::Fast);
        let mut fast = p.rules[0].clone();
        fast.id = 2;
        fast.speed = Speed::Fast;
        fast.speed_multiplier_ppm = 3_000_000;
        p.rules.push(fast);
        let (_, s) = surface(r.clone(), p.clone());
        assert_eq!(
            view(&s).models[0]
                .pricing
                .input
                .as_ref()
                .unwrap()
                .to_string(),
            "2.5"
        );
        r.routes[0].speeds = vec![Speed::Fast];
        r.routes[0].targets[0].speeds = vec![Speed::Fast];
        let (_, s) = surface(r, p);
        assert!(view(&s).models.is_empty());
    }

    #[test]
    fn changed_live_release_invalidates_old_surface_without_reading_db_active() {
        let (r, p) = fixture("model");
        let (router, s) = surface(r.clone(), p.clone());
        let old = router.version().unwrap();
        let mut next = r;
        next.version = 2;
        router
            .publish(old, next, Arc::new(PriceSnapshot::compile(p).unwrap()))
            .unwrap();
        assert!(matches!(
            s.project(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123
            ),
            Err(CatalogError::Unavailable)
        ));
    }

    #[test]
    fn unknown_groups_cannot_select_another_groups_models_and_disabled_is_503() {
        let (r, p) = fixture("model");
        let (_, s) = surface(r, p);
        let v = s
            .project(
                &["client-forged-group".into()],
                "https://synthetic.invalid/api/oauth2",
                123,
            )
            .unwrap();
        assert!(v.models.is_empty());
        assert!(v.groups.is_empty());
        let disabled = CatalogSurface {
            inventory_router: s.inventory_router.clone(),
            state: RwLock::new(SurfaceState::Disabled),
        };
        assert!(matches!(
            disabled.project(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123
            ),
            Err(CatalogError::Unavailable)
        ));
    }

    #[test]
    fn missing_credentials_timeout_passthrough_and_inexact_charge_are_not_apis() {
        let (base, prices) = fixture("model");
        for mode in 1..5 {
            let mut r = base.clone();
            let mut p = prices.clone();
            r.upstreams[0].endpoint = "https://synthetic.invalid/v1"
                .to_owned()
                .try_into()
                .unwrap();
            match mode {
                1 => r.upstreams[0].credential.revision = 2,
                2 => r.routes[0].targets[0].timeout_ms = 999,
                3 => r.routes[0].targets[0].payload = PayloadPolicy::Passthrough,
                4 => p.rules[0].rates.input = 1,
                _ => unreachable!(),
            }
            let (_, s) = surface(r, p);
            let catalog = view(&s);
            assert!(catalog.models.is_empty(), "mode {mode}");
            assert!(
                catalog.groups.is_empty(),
                "group without live model, mode {mode}"
            );
        }
    }
    #[test]
    fn configured_https_chat_and_responses_are_catalogued_without_provider_io() {
        // The former mode=0 exclusion was a fixture restriction, not the
        // gateway's production endpoint contract. Route::new is authoritative.
        for protocol in [Protocol::OpenAiChat, Protocol::OpenAiResponses] {
            for endpoint in [
                "https://synthetic.invalid/v1",
                "https://[2606:4700:4700::1111]:8443/v1",
            ] {
                let (mut r, p) = fixture("model");
                r.upstreams[0].endpoint = endpoint.to_owned().try_into().unwrap();
                r.upstreams[0].protocol = protocol;
                let (_, s) = surface(r, p);
                let catalog = view(&s);
                assert_eq!(catalog.models.len(), 1);
                assert_eq!(catalog.groups.len(), 1);
                assert_eq!(
                    catalog.models[0].apis,
                    ["openai-completions", "openai-responses"]
                );
                assert_eq!(
                    catalog.models[0]
                        .pricing
                        .input
                        .as_ref()
                        .unwrap()
                        .to_string(),
                    "2.5"
                );
                let wire = serde_json::to_string(&catalog).unwrap();
                assert!(!wire.contains("synthetic.invalid/v1"));
                assert!(!wire.contains("synthetic-only-not-a-provider-key"));
            }
        }
    }
    #[test]
    fn https_recoverable_targets_must_all_pass_the_same_constructor_and_contract() {
        for mode in 0..5 {
            let (mut r, p) = fixture("model");
            r.upstreams[0].endpoint = "https://synthetic.invalid/v1"
                .to_owned()
                .try_into()
                .unwrap();
            let mut other = r.upstreams[0].clone();
            other.id = 2;
            other.credential.id = 2;
            r.upstreams.push(other);
            let mut target = r.routes[0].targets[0].clone();
            target.id = 2;
            target.upstream_id = 2;
            target.priority = 99;
            match mode {
                1 => target.timeout_ms = 999,
                2 | 4 => {
                    target.payload = PayloadPolicy::Passthrough;
                    r.global_model_policy = Some(crate::routing::model_policy::GlobalModelPolicy {
                        pass_through_request_enabled: mode == 2,
                        thinking_model_blacklist: vec![],
                        chat_completions_to_responses_policy: Default::default(),
                        ping_interval_enabled: false,
                        ping_interval_seconds: 10,
                    });
                }
                3 => target.upstream_model = "different-model".into(),
                _ => {}
            }
            r.routes[0].targets.push(target);
            let router =
                Arc::new(Router::new(r, Arc::new(PriceSnapshot::compile(p).unwrap())).unwrap());
            router
                .set_health(
                    router.version().unwrap(),
                    2,
                    crate::routing::HealthStatus::Unhealthy,
                )
                .unwrap();
            let credentials = BTreeMap::from([
                ((1, 1), "synthetic-only-first".into()),
                (
                    (2, 1),
                    if mode == 0 {
                        "synthetic\ninvalid-header".into()
                    } else {
                        "synthetic-only-second".into()
                    },
                ),
            ]);
            let surface =
                CatalogSurface::from_gateway(router, &credentials, Duration::from_secs(1));
            let catalog = view(&surface);
            if mode == 2 {
                // Chat's passthrough target is not selectable for Responses;
                // retain the first Adapt target's actual Responses surface.
                assert_eq!(catalog.models.len(), 1);
                assert_eq!(catalog.models[0].apis, ["openai-responses"]);
            } else if mode == 4 {
                // A disabled raw target cannot be selected; the adapted route serves both APIs.
                assert_eq!(catalog.models.len(), 1);
                assert_eq!(
                    catalog.models[0].apis,
                    ["openai-completions", "openai-responses"]
                );
            } else {
                assert!(catalog.models.is_empty(), "mode {mode}");
            }
        }
    }
    #[test]
    fn https_catalog_fix_does_not_admit_insecure_urls_or_extra_protocols() {
        for endpoint in [
            "http://192.0.2.1/v1",
            "http://synthetic.invalid/v1",
            "ftp://synthetic.invalid/v1",
            "https://user:pass@synthetic.invalid/v1",
            "https://synthetic.invalid/v1?token=secret",
            "https://synthetic.invalid/v1#fragment",
        ] {
            assert!(
                crate::relay::Route::new(
                    "synthetic".into(),
                    crate::relay::UpstreamProtocol::Chat,
                    endpoint,
                    "model".into(),
                    "synthetic-only",
                    1
                )
                .is_err(),
                "{endpoint}"
            );
        }
        for endpoint in [
            "https://10.0.0.1/v1",
            "https://169.254.169.254/v1",
            "https://[2001:db8::1]:8443/v1",
            "https://[fe80::1]/v1",
        ] {
            let (mut r, p) = fixture("model");
            r.upstreams[0].endpoint = endpoint.to_owned().try_into().unwrap();
            let (_, surface) = surface(r, p);
            assert!(view(&surface).models.is_empty(), "{endpoint}");
        }
        for protocol in [Protocol::AnthropicMessages, Protocol::Gemini] {
            let (mut r, p) = fixture("model");
            r.upstreams[0].endpoint = "https://synthetic.invalid/v1"
                .to_owned()
                .try_into()
                .unwrap();
            r.upstreams[0].protocol = protocol;
            let (_, s) = surface(r, p);
            assert!(view(&s).models.is_empty());
        }
    }
    #[test]
    fn loaded_credential_reference_with_invalid_bearer_header_is_not_an_api() {
        let (r, p) = fixture("model");
        let router =
            Arc::new(Router::new(r, Arc::new(PriceSnapshot::compile(p).unwrap())).unwrap());
        let credentials = BTreeMap::from([((1, 1), "synthetic\ninvalid-header".into())]);
        let s = CatalogSurface::from_gateway(router, &credentials, Duration::from_secs(1));
        assert!(view(&s).models.is_empty());
    }
    #[test]
    fn valid_large_catalog_rejects_oversized_dto_instead_of_returning_partial_rows() {
        let (mut r, mut p) = fixture("model");
        for id in 2..=MAX_MODELS as u64 {
            let name = format!("model-{id:04}-{}", "x".repeat(117));
            let mut route = r.routes[0].clone();
            route.id = id;
            route.model = name.clone();
            route.targets[0].id = id;
            route.targets[0].upstream_model = name.clone();
            r.routes.push(route);
            let mut price = p.rules[0].clone();
            price.id = id;
            price.model = name;
            p.rules.push(price);
        }
        let (_, s) = surface(r, p);
        let state = s.state.read().unwrap();
        let SurfaceState::Ready { models, .. } = &*state else {
            panic!("valid catalog within factory row budget must load");
        };
        assert_eq!(models.len(), MAX_MODELS);
        assert!(serde_json::to_vec(models).unwrap().len() > MAX_CATALOG_BYTES);
        assert!(matches!(
            s.project(
                &["synthetic".into()],
                "https://synthetic.invalid/api/oauth2",
                123
            ),
            Err(CatalogError::Unavailable)
        ));
    }
}

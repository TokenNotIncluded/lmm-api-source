//! Fixed administrative catalogue reads from the serving router and price
//! projection. Tool arguments never select an action, actor, SQL, URL or secret.
use super::{Error, context::VerifiedToolContext};
use crate::{
    identity::permissions::{self, Action, PlatformResource, PlatformRole, Scope},
    model_http::{
        ModelGateway,
        pricing::{PricingModel, PricingProjection, PricingSurface},
    },
    routing::{Router, RoutingConfig},
};
use serde::Deserialize;
use serde_json::{Number, Value, json};
use sqlx::PgConnection;
use std::collections::{BTreeMap, BTreeSet};

const MAX_RESULT_BYTES: usize = 60 * 1024;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricingAuditArgs {
    #[serde(default)]
    pub model_ids: Vec<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}
impl PricingAuditArgs {
    fn validate(&self) -> Result<(usize, usize), Error> {
        let offset = self.offset.unwrap_or(0);
        let limit = self.limit.unwrap_or(100);
        if offset > 2_147_483_647
            || !(1..=100).contains(&limit)
            || self.model_ids.len() > 100
            || self.model_ids.iter().any(|id| {
                id.trim().is_empty() || id.chars().count() > 200 || id.chars().any(char::is_control)
            })
        {
            return Err(Error::Invalid);
        }
        Ok((offset, limit))
    }
}

/// The same current ChannelRead policy used by the channel console. A native
/// development administrator may use its default policy only when no imported
/// privileged facts exist; any stored complete policy, including an explicit
/// deny, is evaluated even for native administrators and superadministrators.
pub(super) async fn authorize(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
) -> Result<(), Error> {
    let facts = context.revalidate(c).await?;
    let role = match facts.role.as_str() {
        "admin" => PlatformRole::Admin,
        "superadmin" => PlatformRole::Root,
        _ => return Err(Error::Forbidden),
    };
    let imported = permissions::has_privileged_import_facts(c, facts.user)
        .await
        .map_err(|_| Error::Unavailable)?;
    let policy = match permissions::load_complete_policy(c, facts.user).await {
        Ok(policy) => Some(policy),
        Err(permissions::PolicyLoadError::Missing) if !imported => None,
        Err(permissions::PolicyLoadError::Storage) => return Err(Error::Unavailable),
        Err(_) => return Err(Error::Forbidden),
    };
    let allowed = if let Some(policy) = policy {
        permissions::decide(
            &permissions::Actor {
                user_id: facts.user,
                personal_account_id: facts.account,
                role,
                credential: permissions::Credential::Session,
                active: true,
                account_active: true,
                auth_version: facts.auth_version,
                credential_auth_version: facts.auth_version,
            },
            &policy.as_policy(),
            Action::ChannelRead,
            Scope::Platform(PlatformResource::Channel),
            None,
        )
        .allowed()
    } else {
        permissions::catalog_decision(role, &[], Action::ChannelRead).allowed()
    };
    if allowed {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DisplayBindings {
    names: BTreeMap<u64, String>,
    tags: BTreeMap<u64, String>,
    model_route_ids: BTreeSet<u64>,
}

async fn display_bindings(
    c: &mut PgConnection,
    routing_version: u64,
) -> Result<Option<DisplayBindings>, Error> {
    // Project only labels and metadata IDs inside PostgreSQL. Provider contexts,
    // key pools, endpoints and the private release document are never returned.
    let raw: Option<Value> = sqlx::query_scalar(
        "WITH published AS (SELECT public.routing_control_bindings($1) AS body)
         SELECT CASE WHEN body IS NULL THEN NULL ELSE jsonb_build_object(
           'names',COALESCE(body->'names','{}'::jsonb),
           'tags',COALESCE(body->'tags','{}'::jsonb),
           'model_route_ids',COALESCE((SELECT jsonb_agg(key::bigint)
             FROM jsonb_object_keys(COALESCE(body->'models','{}'::jsonb)) AS key),'[]'::jsonb))
         END FROM published",
    )
    .bind(i64::try_from(routing_version).map_err(|_| Error::Unavailable)?)
    .fetch_one(c)
    .await?;
    raw.map(|value| serde_json::from_value(value).map_err(|_| Error::Unavailable))
        .transpose()
}

fn check_version(router: &Router, routing: u64, pricing: u64) -> Result<(), Error> {
    let current = router.version().map_err(|_| Error::Unavailable)?;
    if current.routing == routing && current.pricing == pricing {
        Ok(())
    } else {
        Err(Error::Unavailable)
    }
}

fn channel_rows(config: &RoutingConfig, display: Option<&DisplayBindings>) -> Vec<Value> {
    let mut upstreams: Vec<_> = config.upstreams.iter().collect();
    upstreams.sort_by_key(|upstream| upstream.id);
    upstreams
        .into_iter()
        .take(1000)
        .map(|upstream| {
            let mut models = BTreeSet::new();
            let mut groups = BTreeSet::new();
            let mut routes = Vec::new();
            for route in &config.routes {
                for target in route
                    .targets
                    .iter()
                    .filter(|target| target.upstream_id == upstream.id)
                {
                    models.insert(route.model.clone());
                    groups.insert(route.group.clone());
                    // The native route stores priority and weight per target. Do
                    // not invent one legacy channel-wide value for several routes.
                    routes.push(json!({"model":route.model,"group":route.group,
                    "route_enabled":route.enabled,"target_id":target.id,
                    "enabled":target.enabled,"priority":target.priority,
                    "weight":target.weight,"speeds":target.speeds,
                    "client_protocols":target.client_protocols}));
                }
            }
            json!({"id":upstream.id,"protocol":upstream.protocol,
            "enabled":upstream.enabled,"status":if upstream.enabled {1}else{2},
            "name":display.and_then(|d|d.names.get(&upstream.id)),
            "tag":display.and_then(|d|d.tags.get(&upstream.id)),
            "models":models,"groups":groups,"routing_targets":routes,
            "metadata_available":display.is_some()})
        })
        .collect()
}

fn bounded_channels(
    mut rows: Vec<Value>,
    total: usize,
    routing: u64,
    pricing: u64,
) -> Result<Value, Error> {
    loop {
        let value = json!({"ok":true,"read_only":true,"channels":rows,
            "total_channels":total,"truncated":rows.len()<total,
            "routing_version":routing,"pricing_version":pricing,
            "sensitive_settings_omitted":true,"upstream_availability_checked":false,
            "priority_weight_scope":"per routing target",
            "write_rule":"Review and confirm changes in the authenticated channel console."});
        if serde_json::to_vec(&value)
            .map_err(|_| Error::Unavailable)?
            .len()
            <= MAX_RESULT_BYTES
        {
            return Ok(value);
        }
        if rows.len() <= 1 {
            return Err(Error::Unavailable);
        }
        rows.truncate(rows.len() / 2);
    }
}

pub async fn channels_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    gateway: &ModelGateway,
) -> Result<Value, Error> {
    authorize(c, context).await?;
    let router = gateway.dashboard_routing();
    let document = router.catalog_document().map_err(|_| Error::Unavailable)?;
    let display = display_bindings(c, document.routing.version).await?;
    let value = bounded_channels(
        channel_rows(&document.routing, display.as_ref()),
        document.routing.upstreams.len(),
        document.routing.version,
        document.pricing.version,
    )?;
    check_version(&router, document.routing.version, document.pricing.version)?;
    context.revalidate(c).await?;
    Ok(value)
}

pub async fn inventory_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    gateway: &ModelGateway,
) -> Result<Value, Error> {
    authorize(c, context).await?;
    let router = gateway.dashboard_routing();
    let document = router.catalog_document().map_err(|_| Error::Unavailable)?;
    let surface = PricingSurface::from_gateway(gateway).map_err(|_| Error::Unavailable)?;
    let prices = surface
        .project(None, None)
        .map_err(|_| Error::Unavailable)?;
    if prices.routing_version != document.routing.version
        || prices.pricing_version != document.pricing.version
    {
        return Err(Error::Unavailable);
    }
    let display = display_bindings(c, document.routing.version).await?;
    let ids: BTreeSet<_> = prices
        .models
        .iter()
        .map(|model| model.model.clone())
        .collect();
    let enabled: BTreeSet<_> = document
        .routing
        .groups
        .iter()
        .filter(|group| group.enabled)
        .map(|group| group.name.as_str())
        .collect();
    let missing = display.as_ref().map(|display| {
        document
            .routing
            .routes
            .iter()
            .filter(|route| {
                route.enabled
                    && enabled.contains(route.group.as_str())
                    && ids.contains(&route.model)
                    && !display.model_route_ids.contains(&route.id)
            })
            .map(|route| route.model.clone())
            .collect::<BTreeSet<_>>()
    });
    let value = json!({"ok":!ids.is_empty(),
        "status":if ids.is_empty(){"catalog_unavailable"}else{"available"},
        "model_ids":ids,"groups":prices.groups,
        "missing_model_ids":missing,"missing_model_inventory_available":display.is_some(),
        "missing_scope":"published local route metadata; no provider discovery is performed",
        "routing_version":prices.routing_version,"pricing_version":prices.pricing_version,
        "upstream_availability_checked":false,"read_only":true});
    if serde_json::to_vec(&value)
        .map_err(|_| Error::Unavailable)?
        .len()
        > MAX_RESULT_BYTES
    {
        return Err(Error::Unavailable);
    }
    surface.check_version().map_err(|_| Error::Unavailable)?;
    check_version(&router, document.routing.version, document.pricing.version)?;
    context.revalidate(c).await?;
    Ok(value)
}

fn zero(number: &Number) -> bool {
    let text = number.to_string();
    text.split(['e', 'E'])
        .next()
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii_digit)
        .all(|digit| digit == '0')
}

fn audit_group(model: &PricingModel) -> Value {
    let mut issues = Vec::new();
    let mut unavailable_scalars = Vec::new();
    if let Some(price) = &model.token_price {
        if zero(&price.group_multiplier) {
            issues.push("zero_group_price_review_intent");
        }
        for (name, amount) in [
            ("input", &price.input),
            ("output", &price.output),
            ("cache_read", &price.cache_read),
            ("cache_write", &price.cache_write),
            ("request", &price.request),
        ] {
            match amount {
                Some(amount) if zero(amount) => issues.push(match name {
                    "input" => "zero_input_price_review_intent",
                    "output" => "zero_output_price_review_intent",
                    "cache_read" => "zero_cache_read_price_review_intent",
                    "cache_write" => "zero_cache_write_price_review_intent",
                    _ => "zero_request_price_review_intent",
                }),
                None => unavailable_scalars.push(name),
                _ => {}
            }
        }
    }
    if model.token_price.is_none()
        && model.upstream_cost_price.is_none()
        && model.image_variants.is_empty()
    {
        issues.push("no_executable_price_projection");
    }
    json!({"group":model.group,"billing_status":model.billing_status,
        "unavailable_reason":model.unavailable_reason,"apis":model.apis,
        "token_price":model.token_price,"upstream_cost_price":model.upstream_cost_price,
        "image_variants":model.image_variants,"unavailable_scalar_price_dimensions":unavailable_scalars,
        "issues":issues,"group_multiplier_already_applied":true})
}

fn pricing_audit(projection: &PricingProjection, input: &PricingAuditArgs) -> Result<Value, Error> {
    let (offset, limit) = input.validate()?;
    let selected: BTreeSet<_> = input.model_ids.iter().map(String::as_str).collect();
    let mut models: BTreeMap<&str, Vec<&PricingModel>> = BTreeMap::new();
    for model in &projection.models {
        if selected.is_empty() || selected.contains(model.model.as_str()) {
            models.entry(&model.model).or_default().push(model);
        }
    }
    let missing: Vec<_> = selected
        .into_iter()
        .filter(|id| !models.contains_key(id))
        .collect();
    let entries: Vec<_> = models.into_iter().collect();
    let start = offset.min(entries.len());
    let mut end = start.saturating_add(limit).min(entries.len());
    loop {
        let rows: Vec<_> = entries[start..end].iter().map(|(name, groups)| {
            json!({"model_id":name,"groups":groups.iter().map(|model|audit_group(model)).collect::<Vec<_>>()})
        }).collect();
        let mut result = json!({"ok":true,"read_only":true,"models":rows,
            "total_models":entries.len(),"offset":offset,"returned_models":end-start,
            "missing_requested_models":missing,"truncated":end<entries.len(),
            "routing_version":projection.routing_version,"pricing_version":projection.pricing_version,
            "image_pricing_version":projection.image_pricing_version,
            "rate_scope":"Actual published group rates; unknown dimensions remain null. A request still needs its own frozen quote, budget and usage evidence.",
            "upstream_availability_checked":false,"profitability_established":false});
        if end < entries.len() {
            result["next_offset"] = json!(end);
        }
        if serde_json::to_vec(&result)
            .map_err(|_| Error::Unavailable)?
            .len()
            <= MAX_RESULT_BYTES
        {
            return Ok(result);
        }
        if end <= start + 1 {
            return Err(Error::Unavailable);
        }
        end = start + (end - start) / 2;
    }
}

pub async fn pricing_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    gateway: &ModelGateway,
    input: &PricingAuditArgs,
) -> Result<Value, Error> {
    input.validate()?;
    authorize(c, context).await?;
    let surface = PricingSurface::from_gateway(gateway).map_err(|_| Error::Unavailable)?;
    let projection = surface
        .project(None, None)
        .map_err(|_| Error::Unavailable)?;
    let value = pricing_audit(&projection, input)?;
    surface.check_version().map_err(|_| Error::Unavailable)?;
    context.revalidate(c).await?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn admin_price_projection_keeps_exact_units_and_paginates_distinct_models() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:9/unused")
            .unwrap();
        let gateway = crate::model_http::pricing::tests::gateway(pool);
        let projection = PricingSurface::from_gateway(&gateway)
            .unwrap()
            .project(None, None)
            .unwrap();
        let result = pricing_audit(
            &projection,
            &PricingAuditArgs {
                model_ids: vec!["model".into(), "absent".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result["total_models"], 1);
        assert_eq!(result["missing_requested_models"], json!(["absent"]));
        let groups = result["models"][0]["groups"].as_array().unwrap();
        assert_eq!(
            groups.iter().find(|g| g["group"] == "allowed").unwrap()["token_price"]["input"]
                .to_string(),
            "2.5"
        );
        assert_eq!(
            groups.iter().find(|g| g["group"] == "private").unwrap()["token_price"]["input"]
                .to_string(),
            "4"
        );
        let first = pricing_audit(
            &projection,
            &PricingAuditArgs {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(first["next_offset"], 1);
        let second = pricing_audit(
            &projection,
            &PricingAuditArgs {
                offset: Some(1),
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(
            first["models"][0]["model_id"],
            second["models"][0]["model_id"]
        );
        for secret in [
            "synthetic-pricing-private-provider-credential",
            "127.0.0.1",
            "credential",
            "provider-model",
        ] {
            assert!(!result.to_string().contains(secret));
        }
        assert!(
            pricing_audit(
                &projection,
                &PricingAuditArgs {
                    limit: Some(101),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(
            pricing_audit(
                &projection,
                &PricingAuditArgs {
                    offset: Some(2_147_483_648),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn admin_channels_exclude_provider_secrets_and_missing_names_stay_null() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:9/unused")
            .unwrap();
        let gateway = crate::model_http::pricing::tests::gateway(pool);
        let router = gateway.dashboard_routing();
        let document = router.catalog_document().unwrap();
        let rows = channel_rows(&document.routing, None);
        assert_eq!(rows.len(), 5);
        assert!(
            rows.iter()
                .all(|row| row["name"].is_null() && row["metadata_available"] == false)
        );
        let mut display = DisplayBindings::default();
        display.names.insert(1, "Actual display name".into());
        let rows = channel_rows(&document.routing, Some(&display));
        assert_eq!(rows[0]["name"], "Actual display name");
        let value = bounded_channels(rows, 5, 9, 11).unwrap();
        assert_eq!(value["truncated"], false);
        let raw = value.to_string();
        for secret in [
            "synthetic-pricing-private-provider-credential",
            "127.0.0.1",
            "credential",
            "provider-model",
            "endpoint",
        ] {
            assert!(!raw.contains(secret));
        }
    }

    #[test]
    fn decimal_zero_detection_does_not_round_tiny_nonzero_prices() {
        for text in ["0", "0.0", "0.000000", "0e-100", "-0.0"] {
            assert!(zero(&text.parse().unwrap()));
        }
        for text in ["0.000000000000000000000000000001", "1e-100", "2.5", "4"] {
            assert!(!zero(&text.parse().unwrap()));
        }
    }
    #[tokio::test]
    async fn admin_revision4_tiered_projection_preserves_exact_rates_without_scalar_claim() {
        use crate::routing::{
            BillingMode, CredentialRef, DecimalRate, PriceSnapshot, TieredRates, TieredTariff,
            TokenRates, TokenTier,
        };
        use std::{sync::Arc, time::Duration};
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:9/unused")
            .unwrap();
        let initial = crate::model_http::pricing::tests::gateway(pool.clone());
        let mut document = initial.dashboard_routing().catalog_document().unwrap();
        let rate = |numerator, denominator| DecimalRate {
            numerator,
            denominator,
        };
        let rule = document
            .pricing
            .rules
            .iter_mut()
            .find(|rule| rule.id == 1)
            .unwrap();
        rule.billing_mode = BillingMode::TieredDecimal;
        rule.group_multiplier_ppm = 1_250_000;
        rule.speed_multiplier_ppm = 2_000_000;
        rule.rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        rule.tiered = Some(TieredTariff {
            calendar: None,
            revision: 4,
            source_digest: "ab".repeat(32),
            uses_cache_read: true,
            uses_cache_write: true,
            uses_cache_write_1h: false,
            uses_cache_write_12h: true,
            tiers: [Some(272_000), None]
                .into_iter()
                .enumerate()
                .map(|(index, bound)| {
                    let scale = index as u64 + 1;
                    TokenTier {
                        up_to_input_tokens: bound,
                        rates: TieredRates {
                            input: rate(scale, 1),
                            output: rate(5 * scale, 1),
                            cache_read: rate(0, 1),
                            cache_write: rate(5 * scale, 4),
                            cache_write_1h: None,
                            cache_write_12h: Some(rate(5 * scale, 2)),
                        },
                    }
                })
                .collect(),
        });
        let router = Arc::new(
            Router::new(
                document.routing,
                Arc::new(PriceSnapshot::compile(document.pricing).unwrap()),
            )
            .unwrap(),
        );
        let gateway = ModelGateway::new(
            pool,
            router,
            vec![crate::model_http::UpstreamCredential {
                reference: CredentialRef { id: 1, revision: 1 },
                secret: "synthetic-revision4-admin-credential".into(),
            }],
            BTreeMap::new(),
            crate::relay::Limits {
                total_timeout: Duration::from_secs(1),
                hook_timeout: Duration::from_millis(100),
                ..Default::default()
            },
            crate::model_http::ChargePolicy {
                input_token_ceiling: 65536,
                lease_seconds: 10,
            },
        )
        .unwrap();
        let projection = PricingSurface::from_gateway(&gateway)
            .unwrap()
            .project(None, None)
            .unwrap();
        let value = pricing_audit(
            &projection,
            &PricingAuditArgs {
                model_ids: vec!["model".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let allowed = value["models"][0]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["group"] == "allowed")
            .unwrap();
        assert_eq!(allowed["billing_status"], "usage_dependent");
        assert_eq!(allowed["token_price"]["currency"], "USD");
        assert_eq!(allowed["token_price"]["unit"], "million_tokens");
        assert!(allowed["token_price"]["input"].is_null());
        assert!(allowed.get("unknown_price_dimensions").is_none());
        assert_eq!(
            allowed["unavailable_scalar_price_dimensions"],
            json!(["input", "output", "cache_read", "cache_write", "request"])
        );
        assert_eq!(allowed["group_multiplier_already_applied"], true);
        assert_eq!(
            allowed["token_price"]["group_multiplier"].to_string(),
            "1.25"
        );
        let tiers = allowed["token_price"]["cache_write_12h_tiers"]
            .as_array()
            .unwrap();
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0]["up_to_input_tokens"], "272000");
        assert!(tiers[1]["up_to_input_tokens"].is_null());
        assert_eq!(
            tiers[0]["input"],
            json!({"numerator":"5","denominator":"2"})
        );
        assert_eq!(
            tiers[0]["cache_write"],
            json!({"numerator":"25","denominator":"8"})
        );
        assert_eq!(
            tiers[0]["cache_write_12h"],
            json!({"numerator":"25","denominator":"4"})
        );
        assert_eq!(
            tiers[1]["cache_write_12h"],
            json!({"numerator":"25","denominator":"2"})
        );
        assert_eq!(
            tiers[0]["cache_read"],
            json!({"numerator":"0","denominator":"1"})
        );
        assert_eq!(allowed["issues"], json!([]));
        assert!(
            !value
                .to_string()
                .contains("synthetic-revision4-admin-credential")
        );
    }
}

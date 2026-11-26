use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use rand::{RngExt, SeedableRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::config::{
    MAX_ATTEMPTS, MAX_ROUTES, MAX_TARGETS_PER_ROUTE, requested_model, valid_group, valid_id,
    valid_model,
};
use super::{
    CredentialRef, ModelRoute, PayloadPolicy, PriceQuote, PriceSnapshot, Protocol, RetryCause,
    RouteTarget, RoutingConfig, RoutingError, Speed, UpstreamConfig,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseVersion {
    pub routing: u64,
    pub pricing: u64,
}

type ReliabilityProbes = (
    ReleaseVersion,
    Arc<super::RoutingReliability>,
    Vec<UpstreamConfig>,
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HealthStatus {
    Unknown = 0,
    Healthy = 1,
    Unhealthy = 2,
}

struct RuntimeHealth {
    status: AtomicU8,
    enabled: AtomicBool,
}

impl RuntimeHealth {
    fn available(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
            && self.status.load(Ordering::Acquire) == HealthStatus::Healthy as u8
    }
}

struct CredentialPool {
    references: Vec<CredentialRef>,
    next: AtomicUsize,
}

impl CredentialPool {
    fn select(&self) -> CredentialRef {
        if self.references.len() == 1 {
            return self.references[0];
        }
        self.references[self.next.fetch_add(1, Ordering::Relaxed) % self.references.len()]
    }
}

struct Upstream {
    reliability: Option<Arc<super::RoutingReliability>>,
    // Quarantined credentials; true also closes the channel.
    disabled_keys: Arc<RwLock<HashMap<CredentialRef, bool>>>,
    config: UpstreamConfig,
    health: Arc<RuntimeHealth>,
    credentials: Arc<CredentialPool>,
}

impl Upstream {
    fn key_available(&self, key: CredentialRef) -> bool {
        self.disabled_keys.read().is_ok_and(|entries| {
            !entries.contains_key(&key) && !entries.values().any(|channel| *channel)
        })
    }
    fn select_key(&self) -> Option<CredentialRef> {
        if self.disabled_keys.read().is_ok_and(|keys| keys.is_empty()) {
            return Some(self.credentials.select());
        }
        let keys = &self.credentials.references;
        let first = self.credentials.next.fetch_add(1, Ordering::Relaxed) % keys.len();
        (0..keys.len())
            .map(|i| keys[(first + i) % keys.len()])
            .find(|&key| self.key_available(key))
    }
}

struct Target {
    config: RouteTarget,
    upstream: Arc<Upstream>,
}

impl Target {
    fn available(&self) -> bool {
        self.config.enabled
            && self.config.weight > 0
            && self.upstream.config.enabled
            && self.upstream.health.available()
            && self
                .upstream
                .credentials
                .references
                .iter()
                .any(|&key| self.upstream.key_available(key))
    }
}

struct Route {
    config: ModelRoute,
    targets: Vec<Arc<Target>>,
}

struct Group {
    enabled: bool,
    routes: HashMap<String, Arc<Route>>,
    aliases: HashMap<String, String>,
}

impl Group {
    fn canonical_name<'a>(
        &'a self,
        name: &'a str,
        policy: Option<&super::model_policy::Compiled>,
    ) -> &'a str {
        let name = if policy.is_some_and(|p| !p.preserve_suffix(name))
            && !self.routes.contains_key(name)
            && !self.aliases.contains_key(name)
        {
            name.strip_suffix("-nothinking")
                .or_else(|| name.strip_suffix("-thinking"))
                .unwrap_or(name)
        } else {
            name
        };
        self.aliases.get(name).map_or(name, String::as_str)
    }
}

pub(crate) struct ScopedModelName<'a> {
    pub(crate) group: &'a str,
    pub(crate) requested: &'a str,
    pub(crate) canonical: Option<String>,
}

struct Snapshot {
    channel_overrides: BTreeMap<u64, super::model_policy::ChannelOverrides>,
    global_model_policy: Option<Arc<super::model_policy::Compiled>>,
    affinity: Arc<super::affinity::Runtime>,
    reliability: Option<Arc<super::RoutingReliability>>,
    version: ReleaseVersion,
    prices: Arc<PriceSnapshot>,
    upstreams: HashMap<u64, Arc<Upstream>>,
    groups: HashMap<String, Group>,
}

impl Snapshot {
    fn compile(
        config: RoutingConfig,
        prices: Arc<PriceSnapshot>,
        previous: Option<&Self>,
        credential_pools: Option<&BTreeMap<u64, Vec<CredentialRef>>>,
    ) -> Result<Self, RoutingError> {
        if !valid_id(config.version)
            || config.routes.len() > MAX_ROUTES
            || config.upstreams.len() > MAX_ROUTES
            || config.groups.len() > MAX_ROUTES
            || config.aliases.len() > MAX_ROUTES * 4
        {
            return Err(RoutingError::InvalidConfig("routing version or size"));
        }
        let reliability = config.routing_reliability.map(Arc::new);
        if let Some(policy) = &reliability {
            policy.validate()?;
        }
        let affinity = super::affinity::Runtime::compile(
            config.channel_affinity,
            previous.map(|s| &s.affinity),
        )?;
        super::model_policy::validate_channel_overrides(
            &config.channel_overrides,
            &config.upstreams,
        )?;
        let global_model_policy = config
            .global_model_policy
            .map(|p| p.compile(&config.upstreams).map(Arc::new))
            .transpose()?;
        let mut groups = HashMap::new();
        for group in config.groups {
            if !valid_group(&group.name)
                || groups
                    .insert(
                        group.name,
                        Group {
                            enabled: group.enabled,
                            routes: HashMap::new(),
                            aliases: HashMap::new(),
                        },
                    )
                    .is_some()
            {
                return Err(RoutingError::InvalidConfig("group"));
            }
        }
        let mut upstreams = HashMap::new();
        for upstream in config.upstreams {
            if !valid_id(upstream.id)
                || !valid_id(upstream.credential.id)
                || !valid_id(upstream.credential.revision)
                || upstreams.contains_key(&upstream.id)
            {
                return Err(RoutingError::InvalidConfig(
                    "upstream or credential reference",
                ));
            }
            let health = previous
                .and_then(|old| old.upstreams.get(&upstream.id))
                .filter(|old| old.config.same_destination(&upstream))
                .map(|old| Arc::clone(&old.health))
                .unwrap_or_else(|| {
                    Arc::new(RuntimeHealth {
                        // Optimistic bootstrap. Rust probes can explicitly set Unknown.
                        status: AtomicU8::new(HealthStatus::Healthy as u8),
                        enabled: AtomicBool::new(true),
                    })
                });
            let previous_pool = previous
                .and_then(|old| old.upstreams.get(&upstream.id))
                .filter(|old| old.config.same_destination(&upstream))
                .map(|old| &old.credentials);
            let references = match credential_pools {
                Some(pools) => pools
                    .get(&upstream.id)
                    .cloned()
                    .unwrap_or_else(|| vec![upstream.credential]),
                None => previous_pool
                    .map(|pool| pool.references.clone())
                    .unwrap_or_else(|| vec![upstream.credential]),
            };
            let unique: HashSet<_> = references.iter().copied().collect();
            if references.is_empty()
                || references.len() > MAX_ROUTES
                || unique.len() != references.len()
                || !references.contains(&upstream.credential)
                || references
                    .iter()
                    .any(|reference| !valid_id(reference.id) || !valid_id(reference.revision))
            {
                return Err(RoutingError::InvalidConfig("credential pool"));
            }
            let credentials = previous_pool
                .filter(|pool| pool.references == references)
                .cloned()
                .unwrap_or_else(|| {
                    Arc::new(CredentialPool {
                        references,
                        next: AtomicUsize::new(0),
                    })
                });
            let disabled_keys = previous
                .and_then(|old| old.upstreams.get(&upstream.id))
                .filter(|old| {
                    old.config.same_destination(&upstream)
                        && previous.is_some_and(|p| p.version.routing == config.version)
                })
                .map(|old| old.disabled_keys.clone())
                .unwrap_or_default();
            upstreams.insert(
                upstream.id,
                Arc::new(Upstream {
                    config: upstream,
                    reliability: reliability.clone(),
                    disabled_keys,
                    health,
                    credentials,
                }),
            );
        }
        if credential_pools.is_some_and(|pools| pools.keys().any(|id| !upstreams.contains_key(id)))
        {
            return Err(RoutingError::InvalidConfig("credential pool upstream"));
        }
        let mut route_ids = HashSet::new();
        let mut target_ids = HashSet::new();
        for route in config.routes {
            let unique_speeds: HashSet<_> = route.speeds.iter().copied().collect();
            let retry_causes: HashSet<_> = route.failover.retry_on.iter().copied().collect();
            if !valid_id(route.id)
                || !route_ids.insert(route.id)
                || !valid_model(&route.model)
                || !valid_group(&route.group)
                || unique_speeds.is_empty()
                || unique_speeds.len() != route.speeds.len()
                || route.targets.len() > MAX_TARGETS_PER_ROUTE
                || !(1..=MAX_ATTEMPTS).contains(&route.failover.max_attempts)
                || retry_causes.len() != route.failover.retry_on.len()
            {
                return Err(RoutingError::InvalidConfig("route"));
            }
            let group = groups
                .get_mut(&route.group)
                .ok_or(RoutingError::InvalidConfig("route group"))?;
            if group.routes.contains_key(&route.model) {
                return Err(RoutingError::InvalidConfig("duplicate route"));
            }
            if route.enabled
                && route
                    .speeds
                    .iter()
                    .any(|&speed| prices.template(&route.model, &route.group, speed).is_none())
            {
                return Err(RoutingError::MissingPrice);
            }
            let mut targets = Vec::with_capacity(route.targets.len());
            let mut destinations = HashSet::new();
            for target in &route.targets {
                let target_speeds: HashSet<_> = target.speeds.iter().copied().collect();
                if !valid_id(target.id)
                    || !target_ids.insert(target.id)
                    || !destinations.insert(target.upstream_id)
                    || !valid_model(&target.upstream_model)
                    || target_speeds.is_empty()
                    || target_speeds.len() != target.speeds.len()
                    || target.client_protocols.as_ref().is_some_and(|protocols| {
                        protocols.is_empty()
                            || protocols
                                .iter()
                                .enumerate()
                                .any(|(index, protocol)| protocols[..index].contains(protocol))
                    })
                    || !target_speeds.is_subset(&unique_speeds)
                    || !(1..=3_600_000).contains(&target.timeout_ms)
                {
                    return Err(RoutingError::InvalidConfig("route target"));
                }
                let upstream = upstreams
                    .get(&target.upstream_id)
                    .ok_or(RoutingError::InvalidConfig("target upstream"))?;
                if matches!(target.payload, PayloadPolicy::LegacyOpenai { .. })
                    && !matches!(
                        upstream.config.protocol,
                        Protocol::OpenAiChat | Protocol::OpenAiResponses
                    )
                {
                    return Err(RoutingError::InvalidConfig(
                        "legacy OpenAI payload protocol",
                    ));
                }
                if target.payload == PayloadPolicy::LegacyAnthropicHaikuSystemCacheV1
                    && upstream.config.protocol != Protocol::AnthropicMessages
                {
                    return Err(RoutingError::InvalidConfig(
                        "legacy Anthropic cache payload protocol",
                    ));
                }
                if target.payload.openrouter_chat()
                    && upstream.config.protocol != Protocol::OpenAiChat
                {
                    return Err(RoutingError::InvalidConfig("OpenRouter payload protocol"));
                }
                targets.push(Arc::new(Target {
                    config: target.clone(),
                    upstream: Arc::clone(upstream),
                }));
            }
            targets.sort_by_key(|target| target.config.priority);
            group.routes.insert(
                route.model.clone(),
                Arc::new(Route {
                    config: route,
                    targets,
                }),
            );
        }
        for alias in config.aliases {
            if !valid_model(&alias.alias) || !valid_model(&alias.model) {
                return Err(RoutingError::InvalidConfig("alias"));
            }
            let group = groups
                .get_mut(&alias.group)
                .ok_or(RoutingError::InvalidConfig("alias group"))?;
            if group.routes.contains_key(&alias.alias)
                || !group.routes.contains_key(&alias.model)
                || group.aliases.insert(alias.alias, alias.model).is_some()
            {
                return Err(RoutingError::InvalidConfig("alias collision or chain"));
            }
        }
        Ok(Self {
            channel_overrides: config.channel_overrides,
            global_model_policy,
            affinity,
            reliability,
            version: ReleaseVersion {
                routing: config.version,
                pricing: prices.version(),
            },
            prices,
            upstreams,
            groups,
        })
    }
}

/// One short read lock per new request. No locks are held while selecting or sending.
/// Price and routing state ALWAYS switch together, never through two pointers.
pub struct Router {
    active: RwLock<Arc<Snapshot>>,
}

#[derive(Clone, Copy)]
enum RouteTransport {
    Http {
        allow_passthrough: bool,
    },
    ResponsesWebSocket {
        pinned_target: Option<(u64, CredentialRef)>,
    },
    ResponsesHttp {
        pinned_target: (u64, CredentialRef),
    },
}

impl Router {
    pub fn new(config: RoutingConfig, prices: Arc<PriceSnapshot>) -> Result<Self, RoutingError> {
        Ok(Self {
            active: RwLock::new(Arc::new(Snapshot::compile(config, prices, None, None)?)),
        })
    }

    fn snapshot(&self) -> Result<Arc<Snapshot>, RoutingError> {
        self.active
            .read()
            .map(|active| Arc::clone(&active))
            .map_err(|_| RoutingError::LockPoisoned)
    }

    /// Internal metadata input captured from the same immutable serving snapshot.
    /// This document contains internal destinations/references and is never an
    /// HTTP DTO. CatalogSurface alone projects safe metadata from this input.
    pub(crate) fn catalog_document(&self) -> Result<super::store::ReleaseDocument, RoutingError> {
        let active = self.snapshot()?;
        let mut groups = Vec::new();
        let mut routes = Vec::new();
        let mut aliases = Vec::new();
        for (name, group) in &active.groups {
            groups.push(super::ModelGroup {
                name: name.clone(),
                enabled: group.enabled,
            });
            routes.extend(group.routes.values().map(|route| route.config.clone()));
            aliases.extend(
                group
                    .aliases
                    .iter()
                    .map(|(alias, model)| super::ModelAlias {
                        group: name.clone(),
                        alias: alias.clone(),
                        model: model.clone(),
                    }),
            );
        }
        let mut upstreams: Vec<_> = active
            .upstreams
            .values()
            .map(|upstream| upstream.config.clone())
            .collect();
        groups.sort_by(|a, b| a.name.cmp(&b.name));
        routes.sort_by_key(|route| route.id);
        aliases.sort_by(|a, b| (&a.group, &a.alias).cmp(&(&b.group, &b.alias)));
        upstreams.sort_by_key(|upstream| upstream.id);
        Ok(super::store::ReleaseDocument {
            contract_version: super::CONTRACT_VERSION,
            routing: RoutingConfig {
                channel_overrides: active.channel_overrides.clone(),
                global_model_policy: active
                    .global_model_policy
                    .as_ref()
                    .map(|p| p.config.clone()),
                channel_affinity: active.affinity.config.clone(),
                routing_reliability: active.reliability.as_deref().cloned(),
                version: active.version.routing,
                groups,
                upstreams,
                routes,
                aliases,
            },
            pricing: active.prices.config().clone(),
        })
    }

    pub(crate) fn affinity_config(&self) -> Result<Option<super::affinity::Config>, RoutingError> {
        Ok(self.snapshot()?.affinity.config.clone())
    }

    pub fn version(&self) -> Result<ReleaseVersion, RoutingError> {
        Ok(self.snapshot()?.version)
    }

    /// Validate outside the write lock, then compare-and-swap the complete release.
    /// To roll back content, publish it with NEW version numbers.
    pub fn publish(
        &self,
        expected: ReleaseVersion,
        config: RoutingConfig,
        prices: Arc<PriceSnapshot>,
    ) -> Result<ReleaseVersion, RoutingError> {
        self.publish_with_pools(expected, config, prices, None)
    }

    pub(crate) fn publish_with_credential_pools(
        &self,
        expected: ReleaseVersion,
        config: RoutingConfig,
        prices: Arc<PriceSnapshot>,
        pools: &BTreeMap<u64, Vec<CredentialRef>>,
    ) -> Result<ReleaseVersion, RoutingError> {
        self.publish_with_pools(expected, config, prices, Some(pools))
    }

    fn publish_with_pools(
        &self,
        expected: ReleaseVersion,
        config: RoutingConfig,
        prices: Arc<PriceSnapshot>,
        pools: Option<&BTreeMap<u64, Vec<CredentialRef>>>,
    ) -> Result<ReleaseVersion, RoutingError> {
        let previous = self.snapshot()?;
        if previous.version != expected {
            return Err(RoutingError::VersionConflict);
        }
        if config.version <= expected.routing {
            return Err(RoutingError::VersionNotIncreasing);
        }
        if prices.version() < expected.pricing
            || (prices.version() == expected.pricing && !prices.same_content(&previous.prices))
        {
            return Err(RoutingError::PriceVersionConflict);
        }
        let next = Arc::new(Snapshot::compile(config, prices, Some(&previous), pools)?);
        let mut active = self
            .active
            .write()
            .map_err(|_| RoutingError::LockPoisoned)?;
        if active.version != expected {
            return Err(RoutingError::VersionConflict);
        }
        let version = next.version;
        // Drop retired allocations outside the publication lock.
        let retired = std::mem::replace(&mut *active, next);
        drop(active);
        drop(retired);
        Ok(version)
    }

    /// Recover the credential pools for an already-installed durable release.
    /// Existing plans retain their selected immutable credential revisions.
    pub(crate) fn install_credential_pools(
        &self,
        expected: ReleaseVersion,
        pools: &BTreeMap<u64, Vec<CredentialRef>>,
    ) -> Result<(), RoutingError> {
        let previous = self.snapshot()?;
        if previous.version != expected {
            return Err(RoutingError::VersionConflict);
        }
        if pools.keys().all(|id| previous.upstreams.contains_key(id))
            && previous
                .upstreams
                .iter()
                .all(|(id, upstream)| match pools.get(id) {
                    Some(references) => upstream.credentials.references == *references,
                    None => {
                        upstream.credentials.references.as_slice() == [upstream.config.credential]
                    }
                })
        {
            return Ok(());
        }
        let config = self.catalog_document()?.routing;
        let next = Arc::new(Snapshot::compile(
            config,
            previous.prices.clone(),
            Some(&previous),
            Some(pools),
        )?);
        let mut active = self
            .active
            .write()
            .map_err(|_| RoutingError::LockPoisoned)?;
        if !Arc::ptr_eq(&active, &previous) {
            return Err(RoutingError::VersionConflict);
        }
        *active = next;
        Ok(())
    }

    /// Call with an authenticated group from task 01/03, not a raw client group header.
    pub fn route(&self, request: RouteRequest<'_>) -> Result<RoutePlan, RoutingError> {
        self.route_with_rng(request, &mut rand::rng())
    }

    /// Restrict candidates to payloads supported by the model gateway's encoder.
    pub(crate) fn route_adapted(
        &self,
        request: RouteRequest<'_>,
    ) -> Result<RoutePlan, RoutingError> {
        self.route_with_payload_support(request, &mut rand::rng(), false)
    }

    /// Pin every already-authorized group to one immutable release. The caller
    /// decides whether cross-group retry is authorized; this method grants none.
    pub(crate) fn route_adapted_groups(
        &self,
        model: &str,
        groups: &[String],
        protocol: Protocol,
        affinity: Option<[u8; 32]>,
    ) -> Result<Vec<RoutePlan>, RoutingError> {
        self.route_groups(
            model,
            groups,
            protocol,
            affinity,
            RouteTransport::Http {
                allow_passthrough: false,
            },
        )
    }

    pub(crate) fn route_responses_http_bound(
        &self,
        request: RouteRequest<'_>,
        pinned_target: (u64, CredentialRef),
    ) -> Result<RoutePlan, RoutingError> {
        Self::route_in_snapshot(
            self.snapshot()?,
            request,
            &mut rand::rng(),
            RouteTransport::ResponsesHttp { pinned_target },
            &mut HashMap::new(),
        )
    }

    pub(crate) fn route_responses_ws_groups(
        &self,
        model: &str,
        groups: &[String],
        affinity: Option<[u8; 32]>,
        pinned_target: Option<(u64, CredentialRef)>,
    ) -> Result<Vec<RoutePlan>, RoutingError> {
        self.route_groups(
            model,
            groups,
            Protocol::OpenAiResponses,
            affinity,
            RouteTransport::ResponsesWebSocket { pinned_target },
        )
    }

    fn route_groups(
        &self,
        model: &str,
        groups: &[String],
        protocol: Protocol,
        affinity: Option<[u8; 32]>,
        transport: RouteTransport,
    ) -> Result<Vec<RoutePlan>, RoutingError> {
        let snapshot = self.snapshot()?;
        let mut plans = Vec::new();
        // All group alternatives belong to one request. Advance each channel's
        // pool once so unused group plans cannot starve a key from actual traffic.
        let mut credentials = HashMap::new();
        for group in groups {
            let request = RouteRequest {
                model,
                group,
                speed: None,
                protocol,
                replay: ReplayPermission::Denied,
            };
            let plan = if let Some(scope) = affinity {
                let mut digest = Sha256::new();
                digest.update(b"lmm-route-affinity-v1\0");
                digest.update(scope);
                digest.update(model.as_bytes());
                digest.update([0]);
                digest.update(group.as_bytes());
                let mut rng = rand::rngs::StdRng::from_seed(digest.finalize().into());
                Self::route_in_snapshot(
                    snapshot.clone(),
                    request,
                    &mut rng,
                    transport,
                    &mut credentials,
                )
            } else {
                Self::route_in_snapshot(
                    snapshot.clone(),
                    request,
                    &mut rand::rng(),
                    transport,
                    &mut credentials,
                )
            };
            match plan {
                Ok(plan) => plans.push(plan),
                Err(error @ (RoutingError::InvalidRequest | RoutingError::ConflictingSpeed)) => {
                    return Err(error);
                }
                Err(_) => (),
            }
        }
        Ok(plans)
    }

    pub(super) fn route_with_rng<R: RngExt + ?Sized>(
        &self,
        request: RouteRequest<'_>,
        rng: &mut R,
    ) -> Result<RoutePlan, RoutingError> {
        self.route_with_payload_support(request, rng, true)
    }

    fn route_with_payload_support<R: RngExt + ?Sized>(
        &self,
        request: RouteRequest<'_>,
        rng: &mut R,
        allow_passthrough: bool,
    ) -> Result<RoutePlan, RoutingError> {
        Self::route_in_snapshot(
            self.snapshot()?,
            request,
            rng,
            RouteTransport::Http { allow_passthrough },
            &mut HashMap::new(),
        )
    }

    fn route_in_snapshot<R: RngExt + ?Sized>(
        snapshot: Arc<Snapshot>,
        request: RouteRequest<'_>,
        rng: &mut R,
        transport: RouteTransport,
        credentials: &mut HashMap<u64, CredentialRef>,
    ) -> Result<RoutePlan, RoutingError> {
        if !valid_group(request.group) {
            return Err(RoutingError::InvalidRequest);
        }
        let (name, speed) = requested_model(request.model, request.speed)?;
        let group = snapshot
            .groups
            .get(request.group)
            .ok_or(RoutingError::UnknownGroup)?;
        if !group.enabled {
            return Err(RoutingError::GroupDisabled);
        }
        let canonical = group.canonical_name(name, snapshot.global_model_policy.as_deref());
        let route = group
            .routes
            .get(canonical)
            .ok_or(RoutingError::UnknownModel)?;
        if !route.config.enabled {
            return Err(RoutingError::RouteDisabled);
        }
        if !route.config.speeds.contains(&speed) {
            return Err(RoutingError::SpeedDisabled);
        }
        let (quote, calendar) = snapshot
            .prices
            .quote_at(
                canonical,
                request.group,
                speed,
                chrono::Utc::now().timestamp(),
            )?
            .ok_or(RoutingError::MissingPrice)?;
        let calendar = calendar.map(Arc::new);
        // Apply encoder support before priority and weight selection.
        // @context-fold begin route-payload-candidates
        let mut pool: Vec<_> = route
            .targets
            .iter()
            .filter(|target| {
                target.available()
                    && match transport {
                        RouteTransport::Http { .. } => true,
                        RouteTransport::ResponsesHttp { pinned_target } => {
                            target.upstream.config.protocol == Protocol::OpenAiResponses
                                && target.config.id == pinned_target.0
                                && target
                                    .upstream
                                    .credentials
                                    .references
                                    .contains(&pinned_target.1)
                        }
                        RouteTransport::ResponsesWebSocket { pinned_target } => {
                            target.upstream.config.protocol == Protocol::OpenAiResponses
                                && pinned_target.is_none_or(|(id, reference)| {
                                    target.config.id == id
                                        && target
                                            .upstream
                                            .credentials
                                            .references
                                            .contains(&reference)
                                })
                        }
                    }
                    && target.config.allows_client(request.protocol)
                    && target.config.speeds.contains(&speed)
                    && (target
                        .config
                        .payload
                        .adapts(request.protocol, target.upstream.config.protocol)
                        || (matches!(transport, RouteTransport::Http { .. })
                            && target.config.raw_http_model(
                                request.protocol,
                                target.upstream.config.protocol,
                                snapshot.global_model_policy.as_ref().map(|p| &p.config),
                                snapshot.channel_overrides.get(&target.upstream.config.id),
                            ) == Some(request.model))
                        || (matches!(
                            transport,
                            RouteTransport::Http {
                                allow_passthrough: true
                            }
                        ) && target.upstream.config.protocol == request.protocol
                            && target.config.upstream_model == request.model))
            })
            .cloned()
            .collect();
        if pool.is_empty() {
            return Err(RoutingError::NoEligibleUpstream);
        }
        // @context-fold end route-payload-candidates
        let capacity = pool.len();
        let mut candidates = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            let priority = pool[0].config.priority;
            let total: u64 = pool
                .iter()
                .take_while(|target| target.config.priority == priority)
                .map(|target| u64::from(target.config.weight))
                .sum();
            let mut ticket = rng.random_range(0..total);
            let mut selected = 0;
            for (index, target) in pool.iter().enumerate() {
                let weight = u64::from(target.config.weight);
                if ticket < weight {
                    selected = index;
                    break;
                }
                ticket -= weight;
            }
            let target = pool.remove(selected);
            let credential = match transport {
                RouteTransport::ResponsesHttp { pinned_target }
                | RouteTransport::ResponsesWebSocket {
                    pinned_target: Some(pinned_target),
                } => pinned_target.1,
                _ => match credentials.get(&target.upstream.config.id) {
                    Some(&key) => key,
                    None => {
                        let Some(key) = target.upstream.select_key() else {
                            continue;
                        };
                        credentials.insert(target.upstream.config.id, key);
                        key
                    }
                },
            };
            if target.upstream.key_available(credential) {
                candidates.push(Candidate {
                    target,
                    credential,
                    alternate_key: 0,
                });
            }
        }
        // Keep the normal weighted order and prefer other eligible destinations
        // first. A short route may use spare keys from the same target, provided
        // each key is independently frozen into the original reservation.
        if let Some(policy) = &snapshot.reliability
            && matches!(transport, RouteTransport::Http { .. })
        {
            let mut extras = Vec::new();
            let budget = usize::from(policy.retry_times) + 1;
            for candidate in &candidates {
                if candidates.len() + extras.len() >= budget {
                    break;
                }
                for &key in &candidate.target.upstream.credentials.references {
                    if key != candidate.credential && candidate.target.upstream.key_available(key) {
                        extras.push(Candidate {
                            target: candidate.target.clone(),
                            credential: key,
                            alternate_key: (extras.len() + 1) as u8,
                        });
                        if candidates.len() + extras.len() >= budget {
                            break;
                        }
                    }
                }
            }
            candidates.extend(extras);
        }
        let route = Arc::clone(route);
        Ok(RoutePlan {
            affinity_binding: None,
            snapshot,
            route,
            quote,
            calendar,
            candidates,
            requested_model: request.model.into(),
            speed,
            replay: request.replay,
            next_index: 0,
            attempts: 0,
            current: None,
            stopped: false,
        })
    }

    pub(crate) fn channel_health_keys(
        &self,
        version: ReleaseVersion,
        channel: u64,
    ) -> Result<Vec<CredentialRef>, RoutingError> {
        let snapshot = self.snapshot()?;
        if snapshot.version != version {
            return Err(RoutingError::VersionConflict);
        }
        Ok(snapshot
            .upstreams
            .get(&channel)
            .map(|u| u.credentials.references.clone())
            .unwrap_or_default())
    }

    pub(crate) fn health_key_refs(
        &self,
        version: u64,
    ) -> Result<HashMap<u64, Vec<CredentialRef>>, RoutingError> {
        let active = self.snapshot()?;
        if version != active.version.routing {
            return Err(RoutingError::VersionConflict);
        }
        Ok(active
            .upstreams
            .iter()
            .map(|(id, u)| (*id, u.credentials.references.clone()))
            .collect())
    }
    pub(crate) fn reliability_version(&self) -> Result<Option<ReleaseVersion>, RoutingError> {
        let active = self.snapshot()?;
        Ok(active.reliability.as_ref().map(|_| active.version))
    }

    pub(crate) fn reliability_probes(&self) -> Result<Option<ReliabilityProbes>, RoutingError> {
        let active = self.snapshot()?;
        let Some(policy) = active.reliability.as_ref().filter(|p| p.probe_enabled) else {
            return Ok(None);
        };
        let mut probes = Vec::new();
        for upstream in active.upstreams.values() {
            if !upstream.config.enabled || !upstream.health.enabled.load(Ordering::Acquire) {
                continue;
            }
            for key in &upstream.credentials.references {
                let mut config = upstream.config.clone();
                config.credential = *key;
                probes.push(config);
            }
        }
        probes.sort_by_key(|u| (u.id, u.credential.id, u.credential.revision));
        Ok(Some((active.version, policy.clone(), probes)))
    }

    pub(crate) fn install_health(
        &self,
        version: ReleaseVersion,
        disabled: &HashMap<u64, HashMap<CredentialRef, bool>>,
    ) -> Result<(), RoutingError> {
        let active = self.snapshot()?;
        if version != active.version {
            return Err(RoutingError::VersionConflict);
        }
        for upstream in active.upstreams.values() {
            let mut entries = upstream
                .disabled_keys
                .write()
                .map_err(|_| RoutingError::LockPoisoned)?;
            entries.clear();
            if let Some(keys) = disabled.get(&upstream.config.id) {
                entries.extend(
                    keys.iter()
                        .filter(|(k, _)| upstream.credentials.references.contains(k))
                        .map(|(k, v)| (*k, *v)),
                );
            }
        }
        Ok(())
    }

    /// Rust health probes use a release lease; stale probes cannot revive new endpoints.
    pub fn set_health(
        &self,
        expected: ReleaseVersion,
        upstream_id: u64,
        health: HealthStatus,
    ) -> Result<(), RoutingError> {
        let active = self.active.read().map_err(|_| RoutingError::LockPoisoned)?;
        if active.version != expected {
            return Err(RoutingError::VersionConflict);
        }
        let upstream = active
            .upstreams
            .get(&upstream_id)
            .ok_or(RoutingError::NoEligibleUpstream)?;
        upstream
            .health
            .status
            .store(health as u8, Ordering::Release);
        Ok(())
    }

    /// Emergency operational stop, including not-yet-sent attempts in existing plans.
    /// Health reports never clear this flag. It survives route/price-only publication.
    pub fn set_operational_enabled(
        &self,
        expected: ReleaseVersion,
        upstream_id: u64,
        enabled: bool,
    ) -> Result<(), RoutingError> {
        let active = self.active.read().map_err(|_| RoutingError::LockPoisoned)?;
        if active.version != expected {
            return Err(RoutingError::VersionConflict);
        }
        let upstream = active
            .upstreams
            .get(&upstream_id)
            .ok_or(RoutingError::NoEligibleUpstream)?;
        upstream.health.enabled.store(enabled, Ordering::Release);
        Ok(())
    }

    /// Resolve only published names in the caller's groups, independently of upstream health.
    pub(crate) fn scoped_model_names<'a>(
        &self,
        model: &'a str,
        groups: &'a [String],
    ) -> Result<Vec<ScopedModelName<'a>>, RoutingError> {
        let (requested, _) = requested_model(model, None)?;
        let active = self.snapshot()?;
        Ok(groups
            .iter()
            .filter_map(|group_name| {
                let group = active.groups.get(group_name).filter(|g| g.enabled)?;
                let name = group.canonical_name(requested, active.global_model_policy.as_deref());
                let canonical = group
                    .routes
                    .get(name)
                    .filter(|r| r.config.enabled)
                    .map(|_| name.to_owned());
                Some(ScopedModelName {
                    group: group_name,
                    requested,
                    canonical,
                })
            })
            .collect())
    }

    /// Return a group multiplier only when every enabled model and speed agrees.
    pub(crate) fn group_price_multiplier(&self, name: &str) -> Result<Option<u32>, RoutingError> {
        let active = self.snapshot()?;
        let group = active.groups.get(name).ok_or(RoutingError::UnknownGroup)?;
        if !group.enabled {
            return Err(RoutingError::GroupDisabled);
        }
        let mut multiplier = None;
        for route in group.routes.values().filter(|route| route.config.enabled) {
            for speed in &route.config.speeds {
                let Some(price) = active.prices.template(&route.config.model, name, *speed) else {
                    return Ok(None);
                };
                let (current, _) = price.multipliers_ppm();
                if multiplier.is_some_and(|previous| previous != current) {
                    return Ok(None);
                }
                multiplier = Some(current);
            }
        }
        Ok(multiplier)
    }

    /// The only public configuration projection. Contains no endpoints or credentials.
    pub fn catalog(&self, authorized_group: &str) -> Result<Vec<PublicModel>, RoutingError> {
        let active = self.snapshot()?;
        let group = active
            .groups
            .get(authorized_group)
            .ok_or(RoutingError::UnknownGroup)?;
        if !group.enabled {
            return Err(RoutingError::GroupDisabled);
        }
        let mut models = Vec::new();
        for route in group.routes.values().filter(|route| route.config.enabled) {
            models.push(PublicModel {
                name: route.config.model.clone(),
                canonical_model: route.config.model.clone(),
                speeds: route.config.speeds.clone(),
                routing_version: active.version.routing,
                price_version: active.version.pricing,
            });
        }
        for (alias, canonical) in &group.aliases {
            if let Some(route) = group
                .routes
                .get(canonical)
                .filter(|route| route.config.enabled)
            {
                models.push(PublicModel {
                    name: alias.clone(),
                    canonical_model: canonical.clone(),
                    speeds: route.config.speeds.clone(),
                    routing_version: active.version.routing,
                    price_version: active.version.pricing,
                });
            }
        }
        models.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(models)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReplayPermission {
    #[default]
    Denied,
    /// Granted by the caller's explicit replay/idempotency contract, never inferred.
    ExplicitlyAllowed,
}

#[derive(Clone, Copy, Debug)]
pub struct RouteRequest<'a> {
    pub model: &'a str,
    pub group: &'a str,
    /// Prefer a path/header speed with an unchanged model body for passthrough.
    pub speed: Option<Speed>,
    pub protocol: Protocol,
    pub replay: ReplayPermission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    NotSent,
    /// Provider definitively rejected the request without accepting work.
    Rejected,
    PossiblyAccepted,
    /// Any response body has been exposed downstream, including streaming output.
    ResponseStarted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Connect,
    Timeout,
    RateLimited,
    ServerError,
    Authentication,
    ClientError,
    InvalidResponse,
    Cancelled,
}

impl FailureKind {
    fn retry_cause(self) -> Option<RetryCause> {
        match self {
            Self::Connect => Some(RetryCause::Connect),
            Self::Timeout => Some(RetryCause::Timeout),
            Self::RateLimited => Some(RetryCause::RateLimited),
            Self::ServerError => Some(RetryCause::ServerError),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub kind: FailureKind,
    pub delivery: DeliveryState,
}

/// A request owns this plan until completion. Do not call Router::route on retry.
/// Routing version, price, model, group, speed and all allowed candidates are pinned.
struct Candidate {
    alternate_key: u8,
    target: Arc<Target>,
    credential: CredentialRef,
}

/// A request owns this plan until completion. Do not call Router::route on retry.
pub struct RoutePlan {
    pub(crate) affinity_binding: Option<super::affinity::Binding>,
    snapshot: Arc<Snapshot>,
    route: Arc<Route>,
    quote: PriceQuote,
    calendar: Option<Arc<super::FrozenCalendar>>,
    candidates: Vec<Candidate>,
    requested_model: Box<str>,
    speed: Speed,
    replay: ReplayPermission,
    next_index: usize,
    attempts: u8,
    current: Option<usize>,
    stopped: bool,
}

/// Exact candidate identity within one pinned routing release.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AttemptKey {
    pub route_id: u64,
    pub target_id: u64,
    pub upstream_id: u64,
    pub native_model: String,
}
/// Persisted route-key format. Never change historical byte representation.
pub fn route_key(route: u64, target: u64, upstream: u64) -> String {
    format!("r{route}-t{target}-u{upstream}")
}
impl RoutePlan {
    pub(crate) fn affinity_policy(&self) -> &super::affinity::Runtime {
        &self.snapshot.affinity
    }
    pub(crate) fn prefer_channel(&mut self, upstream: u64) {
        if let Some(index) = self
            .candidates
            .iter()
            .position(|c| c.target.upstream.config.id == upstream)
        {
            self.candidates[..=index].rotate_right(1);
        }
    }

    /// Consume the original plan; preserve its frozen order and random choices.
    pub(crate) fn responses_capability(
        mut self,
        eligible: &HashSet<AttemptKey>,
    ) -> Result<Self, RoutingError> {
        let targets = self
            .candidate_attempts()
            .into_iter()
            .filter(|a| eligible.contains(&a.key()))
            .map(|a| a.target_id())
            .collect();
        self.retain_targets(&targets)?;
        if self.candidates.is_empty() {
            return Err(RoutingError::NoEligibleUpstream);
        }
        Ok(self)
    }
    pub(crate) fn retain_dispatches(&mut self, ids: &HashSet<String>) -> Result<(), RoutingError> {
        if self.current.is_some() || self.attempts != 0 {
            return Err(RoutingError::InvalidTransition);
        }
        let keep: Vec<_> = self
            .candidate_attempts()
            .into_iter()
            .map(|a| ids.contains(&a.route_key()))
            .collect();
        let mut keep = keep.into_iter();
        self.candidates.retain(|_| keep.next() == Some(true));
        Ok(())
    }

    pub(crate) fn retain_targets(&mut self, targets: &HashSet<u64>) -> Result<(), RoutingError> {
        if self.current.is_some() || self.attempts != 0 {
            return Err(RoutingError::InvalidTransition);
        }
        self.candidates
            .retain(|candidate| targets.contains(&candidate.target.config.id));
        Ok(())
    }
    /// These are the bounded original candidates, before any dispatch. Live
    /// health may remove them but a retry cannot introduce an unpinned target.
    pub(crate) fn global_attempt_limit(&self) -> Option<u8> {
        self.snapshot
            .reliability
            .as_ref()
            .map(|p| 1 + p.retry_times)
    }

    fn attempt_limit(&self) -> u8 {
        self.global_attempt_limit()
            .unwrap_or(self.route.config.failover.max_attempts)
    }

    pub(crate) fn planned_attempts(&self) -> Vec<Attempt> {
        self.attempts_iter()
            .take(usize::from(self.attempt_limit()))
            .collect()
    }
    /// Full immutable ordered pool, used only for pre-dispatch capability filtering.
    pub(crate) fn candidate_attempts(&self) -> Vec<Attempt> {
        self.attempts_iter().collect()
    }

    fn attempts_iter(&self) -> impl Iterator<Item = Attempt> + '_ {
        self.candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| Attempt {
                alternate_key: candidate.alternate_key,
                overrides: self
                    .snapshot
                    .channel_overrides
                    .get(&candidate.target.upstream.config.id)
                    .cloned(),
                model_policy: self.snapshot.global_model_policy.clone(),
                target: candidate.target.clone(),
                credential: candidate.credential,
                route: self.route.clone(),
                version: self.snapshot.version,
                quote: self.quote,
                calendar: self.calendar.clone(),
                number: index as u8 + 1,
                speed: self.speed,
            })
    }

    pub(crate) fn allows_failure(&self, failure: Failure, http_status: Option<u16>) -> bool {
        self.validate_retry(failure, http_status).is_ok()
    }

    fn validate_retry(
        &self,
        failure: Failure,
        http_status: Option<u16>,
    ) -> Result<(), RoutingError> {
        if failure.delivery == DeliveryState::ResponseStarted {
            return Err(RoutingError::ResponseAlreadyStarted);
        }
        let permitted = match (&self.snapshot.reliability, http_status) {
            (Some(policy), Some(status)) => policy.retry_status_codes.includes(status),
            _ => failure
                .kind
                .retry_cause()
                .is_some_and(|cause| self.route.config.failover.retry_on.contains(&cause)),
        };
        if !permitted {
            return Err(RoutingError::RetryDisallowed);
        }
        if failure.delivery == DeliveryState::PossiblyAccepted
            && !(self.route.config.failover.allow_replay_after_send
                && self.replay == ReplayPermission::ExplicitlyAllowed)
        {
            return Err(RoutingError::UnsafeReplay);
        }
        Ok(())
    }
    pub fn version(&self) -> ReleaseVersion {
        self.snapshot.version
    }

    pub fn quote(&self) -> PriceQuote {
        self.quote
    }

    pub fn canonical_model(&self) -> &str {
        &self.route.config.model
    }

    pub(crate) fn global_policy(&self) -> Option<&super::model_policy::Compiled> {
        self.snapshot.global_model_policy.as_deref()
    }

    pub fn requested_model(&self) -> &str {
        &self.requested_model
    }

    pub fn group(&self) -> &str {
        &self.route.config.group
    }

    pub fn speed(&self) -> Speed {
        self.speed
    }

    pub fn start(&mut self) -> Result<Attempt, RoutingError> {
        if self.stopped || self.current.is_some() {
            return Err(RoutingError::InvalidTransition);
        }
        self.dispatch_next()
    }

    pub fn retry(&mut self, failure: Failure) -> Result<Attempt, RoutingError> {
        self.retry_http(failure, None)
    }

    pub(crate) fn retry_http(
        &mut self,
        failure: Failure,
        status: Option<u16>,
    ) -> Result<Attempt, RoutingError> {
        if self.stopped || self.current.take().is_none() {
            return Err(RoutingError::InvalidTransition);
        }
        // Make every rejection terminal. A second call cannot bypass a denied retry.
        self.stopped = true;
        self.validate_retry(failure, status)?;
        self.stopped = false;
        self.dispatch_next()
    }

    fn dispatch_next(&mut self) -> Result<Attempt, RoutingError> {
        while self.next_index < self.candidates.len() && self.attempts < self.attempt_limit() {
            let index = self.next_index;
            self.next_index += 1;
            let candidate = &self.candidates[index];
            // This can REMOVE a candidate. It can never add or re-resolve a route.
            if !candidate.target.available()
                || !candidate
                    .target
                    .upstream
                    .key_available(candidate.credential)
            {
                continue;
            }
            self.attempts += 1;
            self.current = Some(index);
            return Ok(Attempt {
                alternate_key: candidate.alternate_key,
                overrides: self
                    .snapshot
                    .channel_overrides
                    .get(&candidate.target.upstream.config.id)
                    .cloned(),
                model_policy: self.snapshot.global_model_policy.clone(),
                target: Arc::clone(&candidate.target),
                credential: candidate.credential,
                route: Arc::clone(&self.route),
                version: self.snapshot.version,
                quote: self.quote,
                calendar: self.calendar.clone(),
                number: self.attempts,
                speed: self.speed,
            });
        }
        self.stopped = true;
        Err(RoutingError::AttemptsExhausted)
    }
}

impl fmt::Debug for RoutePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoutePlan")
            .field("version", &self.version())
            .field("route_id", &self.route.config.id)
            .field("speed", &self.speed)
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}

/// Private fields prevent a relay from accidentally rebuilding an unapproved target.
/// No Serialize implementation: never send this object to a public response or log.
pub struct Attempt {
    pub(crate) overrides: Option<super::model_policy::ChannelOverrides>,
    alternate_key: u8,
    pub(crate) model_policy: Option<Arc<super::model_policy::Compiled>>,
    target: Arc<Target>,
    credential: CredentialRef,
    route: Arc<Route>,
    version: ReleaseVersion,
    quote: PriceQuote,
    calendar: Option<Arc<super::FrozenCalendar>>,
    number: u8,
    speed: Speed,
}

impl Attempt {
    pub(crate) fn raw_http_model(&self, client: Protocol) -> Option<&str> {
        self.target.config.raw_http_model(
            client,
            self.protocol(),
            self.model_policy.as_ref().map(|p| &p.config),
            self.overrides.as_ref(),
        )
    }

    pub fn key(&self) -> AttemptKey {
        AttemptKey {
            route_id: self.route_id(),
            target_id: self.target_id(),
            upstream_id: self.upstream_id(),
            native_model: self.upstream_model().into(),
        }
    }
    pub fn route_key(&self) -> String {
        let base = route_key(self.route_id(), self.target_id(), self.upstream_id());
        if self.alternate_key != 0 {
            // Receipts distinguish alternatives by plan ordinal, without placing
            // a credential ID or revision in an externally displayed route label.
            format!("{base}-a{}", self.alternate_key)
        } else {
            base
        }
    }

    pub(crate) fn calendar(&self) -> Option<&super::FrozenCalendar> {
        self.calendar.as_deref()
    }
    pub fn version(&self) -> ReleaseVersion {
        self.version
    }
    pub fn quote(&self) -> PriceQuote {
        self.quote
    }
    pub fn number(&self) -> u8 {
        self.number
    }
    pub fn speed(&self) -> Speed {
        self.speed
    }
    pub fn route_id(&self) -> u64 {
        self.route.config.id
    }
    pub fn target_id(&self) -> u64 {
        self.target.config.id
    }
    pub(crate) fn affinity_revision(&self) -> [u8; 32] {
        let upstream = &self.target.upstream;
        let bytes = serde_json::to_vec(&(
            &upstream.config,
            &upstream.credentials.references,
            &self.target.config.upstream_model,
            &self.target.config.payload,
        ))
        .expect("validated routing configuration serializes");
        Sha256::digest(bytes).into()
    }
    pub fn upstream_id(&self) -> u64 {
        self.target.upstream.config.id
    }
    pub fn canonical_model(&self) -> &str {
        &self.route.config.model
    }
    pub fn group(&self) -> &str {
        &self.route.config.group
    }
    pub fn endpoint(&self) -> &str {
        self.target.upstream.config.endpoint.as_str()
    }
    pub fn upstream_model(&self) -> &str {
        &self.target.config.upstream_model
    }
    pub fn protocol(&self) -> Protocol {
        self.target.upstream.config.protocol
    }
    pub fn payload_policy(&self) -> PayloadPolicy {
        self.target.config.payload
    }
    pub fn credential_ref(&self) -> CredentialRef {
        self.credential
    }
    /// Zero-based ordinal in this attempt's original pool, never the live release.
    pub(crate) fn credential_selection(&self) -> (bool, u32) {
        let references = &self.target.upstream.credentials.references;
        let index = references
            .iter()
            .position(|reference| *reference == self.credential)
            .expect("selected credential belongs to the frozen pool");
        (references.len() > 1, index as u32)
    }
    pub fn timeout_ms(&self) -> u32 {
        self.target.config.timeout_ms
    }

    /// Check again immediately before send; no API can revoke bytes already sent.
    pub fn is_available(&self) -> bool {
        self.target.available() && self.target.upstream.key_available(self.credential)
    }

    pub(crate) fn reliability(&self) -> Option<&super::RoutingReliability> {
        self.target.upstream.reliability.as_deref()
    }

    /// Infrastructure failures may close this destination. Reopening requires a Rust probe.
    /// A retired credential/endpoint has separate state and cannot poison its replacement.
    pub fn mark_unhealthy(&self) {
        self.target
            .upstream
            .health
            .status
            .store(HealthStatus::Unhealthy as u8, Ordering::Release);
    }
}

impl fmt::Debug for Attempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attempt")
            .field("version", &self.version)
            .field("route_id", &self.route_id())
            .field("target_id", &self.target_id())
            .field("upstream_id", &self.upstream_id())
            .field("number", &self.number)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PublicModel {
    pub name: String,
    pub canonical_model: String,
    pub speeds: Vec<Speed>,
    pub routing_version: u64,
    pub price_version: u64,
}

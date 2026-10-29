//! Small development harness. The transaction tests include store.rs directly
//! because its new entry is crate-private; routing types come from the frozen
//! Core library. The separate binary test calls the library's public API.

pub use lmm_core::routing::{
    BillingMode, CONTRACT_VERSION, CredentialRef, FailoverPolicy, MULTIPLIER_ONE, ModelAlias,
    ModelGroup, ModelRoute, PayloadPolicy, PriceConfig, PriceRule, PriceSnapshot, Protocol,
    ReleaseVersion, RetryCause, RouteTarget, Router, RoutingConfig, RoutingError, Speed,
    TokenRates, UpstreamConfig,
};

// The included module imports these two private items from its parent. Match
// and check the real config.rs source before using this adapter in any test.
mod config {
    pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

    pub(super) fn valid_id(value: u64) -> bool {
        value > 0 && value <= i64::MAX as u64
    }
}

#[path = "../../services/core/src/routing/store.rs"]
pub mod store;

use sqlx::PgPool;
use store::{PostgresRoutingStore, StoreError};

async fn database(pool: &PgPool) -> PostgresRoutingStore {
    let config_source = include_str!("../../services/core/src/routing/config.rs");
    assert!(config_source.contains("pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;"));
    assert!(config_source.contains(
        "pub(super) fn valid_id(value: u64) -> bool {\n    value > 0 && value <= i64::MAX as u64\n}"
    ));
    sqlx::raw_sql(include_str!("../../services/core/schema/routing.sql"))
        .execute(pool)
        .await
        .unwrap();
    let store = PostgresRoutingStore::new(pool.clone());
    for id in [100, 200] {
        store
            .register_credential_reference(CredentialRef { id, revision: 1 })
            .await
            .unwrap();
    }
    store
}

fn fixture(version: u64) -> (RoutingConfig, PriceConfig) {
    let speeds = vec![Speed::Standard, Speed::Fast, Speed::Ultrafast];
    let config = RoutingConfig {
        version,
        groups: vec![ModelGroup {
            name: "default".into(),
            enabled: true,
        }],
        upstreams: (1..=2)
            .map(|id| UpstreamConfig {
                id,
                endpoint: format!("https://upstream-{id}.invalid/v1")
                    .try_into()
                    .unwrap(),
                protocol: Protocol::OpenAiChat,
                credential: CredentialRef {
                    id: id * 100,
                    revision: 1,
                },
                enabled: true,
            })
            .collect(),
        routes: vec![ModelRoute {
            id: 1,
            model: "model-a".into(),
            group: "default".into(),
            enabled: true,
            speeds: speeds.clone(),
            targets: (1..=2)
                .map(|id| RouteTarget {
                    id,
                    upstream_id: id,
                    upstream_model: "model-a".into(),
                    enabled: true,
                    priority: 0,
                    weight: if id == 1 { 1 } else { 3 },
                    speeds: speeds.clone(),
                    payload: PayloadPolicy::Adapt,
                    timeout_ms: 30_000,
                })
                .collect(),
            failover: FailoverPolicy {
                max_attempts: 2,
                retry_on: vec![RetryCause::Connect, RetryCause::Timeout],
                allow_replay_after_send: false,
            },
        }],
        aliases: vec![ModelAlias {
            group: "default".into(),
            alias: "preferred".into(),
            model: "model-a".into(),
        }],
    };
    let prices = PriceConfig {
        version,
        rules: speeds
            .into_iter()
            .enumerate()
            .map(|(index, speed)| PriceRule {
                billing_mode: BillingMode::Tokens,
                tiered: None,
                id: index as u64 + 1,
                model: "model-a".into(),
                group: "default".into(),
                speed,
                rates: TokenRates {
                    input: 3_000_000,
                    output: 12_000_000,
                    cache_read: 300_000,
                    cache_write: 3_750_000,
                    request: 10,
                },
                group_multiplier_ppm: MULTIPLIER_ONE,
                speed_multiplier_ppm: MULTIPLIER_ONE * (index as u32 + 1),
            })
            .collect(),
    };
    (config, prices)
}

#[path = "publish_tests.rs"]
mod publish_tests;

#[sqlx::test]
async fn postgres_frozen_library_public_publish(pool: PgPool) {
    let _ = database(&pool).await;
    let store = lmm_core::routing::store::PostgresRoutingStore::new(pool.clone());
    let (first, prices) = fixture(1);
    store.stage_prices(prices).await.unwrap();
    let published = store.publish(first, 1, None).await.unwrap();
    assert_eq!(store.load_active().await.unwrap(), published);

    let (second, _) = fixture(2);
    assert_eq!(
        store.publish(second.clone(), 1, None).await.unwrap_err(),
        lmm_core::routing::store::StoreError::VersionConflict
    );
    assert_eq!(store.load_active().await.unwrap(), published);
    let next = store
        .publish(
            second,
            1,
            Some(ReleaseVersion {
                routing: 1,
                pricing: 1,
            }),
        )
        .await
        .unwrap();
    assert_eq!(next.routing.version, 2);
    assert_eq!(store.load_active().await.unwrap(), next);
}

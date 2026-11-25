use super::*;
use std::collections::BTreeMap;

#[test]
fn key_pool_round_robin_freezes_credentials_and_retains_in_flight_plans() {
    let (mut config, prices) = fixture(1);
    config.routes[0].targets.truncate(1);
    let primary = config.upstreams[0].credential;
    let secondary = CredentialRef {
        id: 101,
        revision: 2,
    };
    let router = build(config.clone(), prices.clone());
    let pools = BTreeMap::from([(1, vec![primary, secondary])]);
    router
        .install_credential_pools(router.version().unwrap(), &pools)
        .unwrap();

    let mut first = router.route(request()).unwrap();
    let mut second = router.route(request()).unwrap();
    let frozen = second.planned_attempts()[0].credential_ref();
    assert_eq!(first.start().unwrap().credential_ref(), primary);
    assert_eq!(frozen, secondary);

    // Refreshing the same durable release does not restart the cursor.
    router
        .install_credential_pools(router.version().unwrap(), &pools)
        .unwrap();
    assert_eq!(
        router
            .route(request())
            .unwrap()
            .start()
            .unwrap()
            .credential_ref(),
        primary
    );
    assert_eq!(
        router
            .route(request())
            .unwrap()
            .start()
            .unwrap()
            .credential_ref(),
        secondary
    );

    // A disabled key leaves new plans; an admitted plan keeps its exact revision.
    config.version += 1;
    router
        .publish_with_credential_pools(
            router.version().unwrap(),
            config,
            Arc::new(PriceSnapshot::compile(prices).unwrap()),
            &BTreeMap::new(),
        )
        .unwrap();
    let attempt = second.start().unwrap();
    assert_eq!(attempt.credential_ref(), frozen);
    let binding = crate::billing::charging::contract::ReservationBinding::for_attempt(
        &attempt,
        "chat".into(),
        attempt.upstream_model().into(),
        attempt.endpoint().into(),
        "synthetic-v1".into(),
        "context-max-v1".into(),
        attempt.quote().version(),
    )
    .unwrap();
    assert_eq!(binding.credential_ref, secondary);
    assert_eq!(
        router
            .route(request())
            .unwrap()
            .start()
            .unwrap()
            .credential_ref(),
        primary
    );
}

#[test]
fn key_pool_failover_uses_the_prepared_revision_for_every_candidate() {
    let (mut config, prices) = fixture(1);
    config.routes[0].targets[1].priority = 1;
    let primary = config.upstreams[0].credential;
    let backup = config.upstreams[1].credential;
    let secondary = CredentialRef {
        id: 101,
        revision: 1,
    };
    let backup_secondary = CredentialRef {
        id: 201,
        revision: 1,
    };
    let router = build(config, prices);
    router
        .install_credential_pools(
            router.version().unwrap(),
            &BTreeMap::from([
                (1, vec![primary, secondary]),
                (2, vec![backup, backup_secondary]),
            ]),
        )
        .unwrap();
    let _first = router.route(request()).unwrap();
    let mut second = router.route(request()).unwrap();
    let planned = second.planned_attempts();
    assert_eq!(planned[0].credential_ref(), secondary);
    assert_eq!(planned[1].credential_ref(), backup_secondary);
    let _later = router.route(request()).unwrap();
    assert_eq!(second.start().unwrap().credential_ref(), secondary);
    assert_eq!(
        second.retry(failure()).unwrap().credential_ref(),
        backup_secondary
    );
}

#[test]
fn key_pool_responses_continuations_keep_the_original_enabled_key() {
    let (mut config, prices) = fixture(1);
    config.routes[0].targets.truncate(1);
    config.upstreams[0].protocol = Protocol::OpenAiResponses;
    let primary = config.upstreams[0].credential;
    let secondary = CredentialRef {
        id: 101,
        revision: 1,
    };
    let router = build(config, prices);
    router
        .install_credential_pools(
            router.version().unwrap(),
            &BTreeMap::from([(1, vec![primary, secondary])]),
        )
        .unwrap();
    let req = || RouteRequest {
        protocol: Protocol::OpenAiResponses,
        ..request()
    };
    assert_eq!(
        router
            .route(req())
            .unwrap()
            .start()
            .unwrap()
            .credential_ref(),
        primary
    );
    assert_eq!(
        router
            .route(req())
            .unwrap()
            .start()
            .unwrap()
            .credential_ref(),
        secondary
    );
    assert_eq!(
        router
            .route_responses_http_bound(req(), (1, secondary))
            .unwrap()
            .start()
            .unwrap()
            .credential_ref(),
        secondary,
    );
    let mut plans = router
        .route_responses_ws_groups("model-a", &["default".into()], None, Some((1, secondary)))
        .unwrap();
    assert_eq!(plans[0].start().unwrap().credential_ref(), secondary);

    router
        .install_credential_pools(router.version().unwrap(), &BTreeMap::new())
        .unwrap();
    assert!(
        router
            .route_responses_http_bound(req(), (1, secondary))
            .is_err()
    );
    assert!(
        router
            .route_responses_ws_groups("model-a", &["default".into()], None, Some((1, secondary)),)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn key_pool_group_alternatives_advance_each_channel_once_per_request() {
    let (mut config, mut prices) = fixture(1);
    config.routes[0].targets.truncate(1);
    config.groups.push(ModelGroup {
        name: "fallback".into(),
        enabled: true,
    });
    let mut fallback = config.routes[0].clone();
    fallback.id = 2;
    fallback.group = "fallback".into();
    fallback.targets[0].id = 3;
    config.routes.push(fallback);
    let additional_prices: Vec<_> = prices
        .rules
        .iter()
        .map(|price| {
            let mut price = price.clone();
            price.id += 3;
            price.group = "fallback".into();
            price
        })
        .collect();
    prices.rules.extend(additional_prices);
    let primary = config.upstreams[0].credential;
    let secondary = CredentialRef {
        id: 101,
        revision: 2,
    };
    let router = build(config, prices);
    router
        .install_credential_pools(
            router.version().unwrap(),
            &BTreeMap::from([(1, vec![primary, secondary])]),
        )
        .unwrap();

    for expected in [primary, secondary, primary, secondary] {
        let mut plans = router
            .route_adapted_groups(
                "model-a",
                &["default".into(), "fallback".into()],
                Protocol::OpenAiChat,
                None,
            )
            .unwrap();
        assert_eq!(plans.len(), 2);
        for plan in &mut plans {
            assert_eq!(plan.planned_attempts()[0].credential_ref(), expected);
            assert_eq!(plan.start().unwrap().credential_ref(), expected);
        }
    }
}

#[test]
fn key_snapshot_keeps_admission_ordinal_after_reorder_and_removal() {
    use crate::billing::charging::contract::{CredentialSelection, ReservationBinding};

    fn binding(attempt: &Attempt) -> ReservationBinding {
        ReservationBinding::for_attempt(
            attempt,
            "chat".into(),
            attempt.upstream_model().into(),
            attempt.endpoint().into(),
            "synthetic-key-snapshot-v1".into(),
            "context-max-v1".into(),
            attempt.quote().version(),
        )
        .unwrap()
    }

    let (mut config, prices) = fixture(1);
    config.routes[0].targets.truncate(1);
    let primary = config.upstreams[0].credential;
    let secondary = CredentialRef {
        id: 101,
        revision: 2,
    };
    let router = build(config.clone(), prices.clone());
    router
        .install_credential_pools(
            router.version().unwrap(),
            &BTreeMap::from([(1, vec![primary, secondary])]),
        )
        .unwrap();
    let _first = router.route(request()).unwrap();
    let mut accepted = router.route(request()).unwrap();
    let original = binding(&accepted.planned_attempts()[0]);
    assert_eq!(original.credential_ref, secondary);
    assert_eq!(
        original.credential_selection,
        Some(CredentialSelection {
            is_multi_key: true,
            multi_key_index: 1,
        })
    );

    config.version += 1;
    config.upstreams[0].credential = secondary;
    router
        .publish_with_credential_pools(
            router.version().unwrap(),
            config.clone(),
            Arc::new(PriceSnapshot::compile(prices.clone()).unwrap()),
            &BTreeMap::from([(1, vec![secondary, primary])]),
        )
        .unwrap();
    let reordered = binding(&router.route(request()).unwrap().start().unwrap());
    assert_eq!(reordered.credential_ref, secondary);
    assert_eq!(reordered.credential_selection.unwrap().multi_key_index, 0);

    config.version += 1;
    config.upstreams[0].credential = primary;
    router
        .publish_with_credential_pools(
            router.version().unwrap(),
            config,
            Arc::new(PriceSnapshot::compile(prices).unwrap()),
            &BTreeMap::new(),
        )
        .unwrap();
    let single = binding(&router.route(request()).unwrap().start().unwrap());
    assert_eq!(
        single.credential_selection,
        Some(CredentialSelection {
            is_multi_key: false,
            multi_key_index: 0,
        })
    );
    // Dispatch can happen after both releases; its source is the accepted plan.
    assert_eq!(binding(&accepted.start().unwrap()), original);
    let persisted = serde_json::to_value(&original).unwrap();
    assert_eq!(
        serde_json::from_value::<ReservationBinding>(persisted).unwrap(),
        original,
    );
}

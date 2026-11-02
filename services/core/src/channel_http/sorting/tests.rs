use crate::channel_http::control::observations::{Balance, Statistics};
use crate::channel_http::{Params, page_with_statistics};
use num_bigint::BigInt;
use num_rational::BigRational;
use std::collections::BTreeMap;

fn page(
    sort: &str,
    order: &str,
    unit: Option<&str>,
    page: usize,
    size: usize,
) -> serde_json::Value {
    let (mut config, _) = crate::channel_http::tests::fixture();
    config.upstreams.truncate(4);
    let mut stats = BTreeMap::new();
    for id in 1..=4 {
        let mut s = Statistics::default();
        if id <= 3 {
            // Same 18-place display rounding; comparison must retain rational precision.
            s.amount = Some(BigRational::new(
                BigInt::from(if id == 2 { 2 } else { 1 }),
                BigInt::from(10_u64).pow(30),
            ));
            s.observation = Some(Balance {
                balance: Some("5".into()),
                unit_label: Some(if id == 2 { "CNY" } else { "USD" }.into()),
                balance_updated_time: Some(1),
                recorded_at: Some(1),
                source_channel_id: None,
                kind: "provider".into(),
                number: Some(5.0),
            });
            s.response_time = (id != 2).then_some(10);
            s.test_time = Some(20);
        }
        stats.insert(id, s);
    }
    page_with_statistics(
        &config,
        Params {
            sort_by: Some(sort.into()),
            sort_order: Some(order.into()),
            sort_unit: unit.map(str::to_owned),
            p: Some(page),
            page_size: Some(size),
            ..Default::default()
        },
        &BTreeMap::new(),
        stats,
    )
    .unwrap()
}
fn ids(value: serde_json::Value) -> Vec<u64> {
    value["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_u64().unwrap())
        .collect()
}
#[test]
fn precise_amount_unknown_last_stable_ids_and_sort_before_pagination() {
    assert_eq!(ids(page("lifetime", "asc", None, 1, 10)), [1, 3, 2, 4]);
    assert_eq!(ids(page("lifetime", "desc", None, 1, 10)), [2, 1, 3, 4]);
    assert_eq!(ids(page("lifetime", "desc", None, 2, 1)), [1]);
}
#[test]
fn balances_never_compare_different_or_unknown_units() {
    for order in ["asc", "desc"] {
        assert_eq!(
            ids(page("balance", order, Some("USD"), 1, 10)),
            [1, 3, 2, 4]
        );
        assert_eq!(
            ids(page("balance", order, Some("CNY"), 1, 10)),
            [2, 1, 3, 4]
        );
        assert_eq!(
            ids(page("balance", order, Some("points"), 1, 10)),
            [1, 2, 3, 4]
        );
    }
}
#[test]
fn failed_or_absent_probes_are_not_zero_duration() {
    for order in ["asc", "desc"] {
        assert_eq!(ids(page("response_time", order, None, 1, 10)), [1, 3, 2, 4]);
        assert_eq!(ids(page("test_time", order, None, 1, 10)), [1, 2, 3, 4]);
    }
}

use super::*;
use serde_json::json;

fn ratio(n: u64, d: u64) -> DiscountRatio {
    DiscountRatio::new(n, d).unwrap()
}
fn historical_receipt() -> serde_json::Value {
    json!({
        "user_id": 7,
        "policy_revision": 1,
        "basis_revision": 1,
        "effective_level": 2,
        "level_source": "automatic",
        "discount": {"numerator": 97, "denominator": 100},
        "evaluated_at": 1_700_000_000_i64,
        "next_decay_at": 1_707_776_000_i64,
        "evidence_digest": vec![1_u8; 32],
    })
}

#[test]
fn exact_ratios_reject_invalid_and_normalize() {
    assert_eq!(ratio(90, 100), ratio(9, 10));
    for (n, d) in [(0, 1), (1, 0), (2, 1), (1, 1_000_000_000_000_000_001)] {
        assert!(DiscountRatio::new(n, d).is_err());
    }
    assert!(
        serde_json::from_value::<DiscountRatio>(json!({"numerator":1,"denominator":0})).is_err()
    );
}

#[test]
fn historical_frozen_receipt_roundtrip_preserves_exact_evidence() {
    let wire = historical_receipt();
    let receipt: FrozenTrustPrice = serde_json::from_value(wire.clone()).unwrap();
    receipt.validate().unwrap();
    assert_eq!(receipt.user_id(), 7);
    assert_eq!(receipt.level(), 2);
    assert_eq!(receipt.discount(), ratio(97, 100));
    assert_eq!(receipt.next_decay_at(), Some(1_707_776_000));
    assert_eq!(serde_json::to_value(&receipt).unwrap(), wire);
    let encoded = serde_json::to_vec(&receipt).unwrap();
    let restarted: FrozenTrustPrice = serde_json::from_slice(&encoded).unwrap();
    restarted.validate().unwrap();
    assert_eq!(restarted, receipt);
}

#[test]
fn historical_frozen_receipt_rejects_corrupt_evidence() {
    for (field, value) in [
        ("user_id", json!(0)),
        ("policy_revision", json!(0)),
        ("basis_revision", json!(-1)),
        ("effective_level", json!(7)),
        ("level_source", json!("unknown")),
        ("evaluated_at", json!(-1)),
        ("next_decay_at", json!(1_700_000_000)),
        ("evidence_digest", json!(vec![0_u8; 32])),
    ] {
        let mut wire = historical_receipt();
        wire[field] = value;
        let receipt: FrozenTrustPrice = serde_json::from_value(wire).unwrap();
        assert_eq!(receipt.validate(), Err(Error::Invalid), "{field}");
    }
    let mut wire = historical_receipt();
    wire["discount"] = json!({"numerator":2,"denominator":1});
    assert!(serde_json::from_value::<FrozenTrustPrice>(wire).is_err());
    let mut wire = historical_receipt();
    wire["new_authority"] = json!(true);
    assert!(serde_json::from_value::<FrozenTrustPrice>(wire).is_err());
}

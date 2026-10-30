use super::*;

fn request(delta: &str, key: &str) -> AdjustmentRequest {
    AdjustmentRequest {
        operation_key: key.into(),
        delta_units: delta.into(),
        reason: "Synthetic correction".into(),
    }
}
const KEY: &str = "synthetic-adjustment-operation-0001";

#[test]
fn adjustments_reject_ambiguous_or_non_integer_money() {
    assert_eq!(request("9223372036854775807", KEY).validate(), Ok(i64::MAX));
    for amount in [
        "0",
        "-0",
        "+1",
        "01",
        "-01",
        "1.0",
        "1e3",
        " 1",
        "9223372036854775808",
        "-9223372036854775808",
    ] {
        assert_eq!(
            request(amount, KEY).validate(),
            Err(ProfileError::Invalid),
            "{amount}"
        );
    }
    assert_eq!(request("1", "short").validate(), Err(ProfileError::Invalid));
}

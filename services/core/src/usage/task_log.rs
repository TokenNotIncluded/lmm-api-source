//! Public `other` objects for task settlement and refund usage logs.
//! The tiered log UI selects the task branch only when `is_task` is true.
//! These maps do not reserve, settle, or refund credits.

use serde_json::{Map, Value};

/// Settlement difference log. `pre_consumed_quota` and `actual_quota` are the
/// already decided quota figures; this function does not recompute them.
pub fn task_settlement_other(
    task_id: &str,
    pre_consumed_quota: i64,
    actual_quota: i64,
    billing: &Map<String, Value>,
) -> Value {
    let mut other = task_billing_other(task_id, billing);
    other.insert(
        "pre_consumed_quota".to_owned(),
        Value::Number(pre_consumed_quota.into()),
    );
    other.insert(
        "actual_quota".to_owned(),
        Value::Number(actual_quota.into()),
    );
    Value::Object(other)
}

/// Failure refund log. `reason` is display text, not a second pricing decision.
pub fn task_refund_other(task_id: &str, reason: &str, billing: &Map<String, Value>) -> Value {
    let mut other = task_billing_other(task_id, billing);
    other.insert("reason".to_owned(), Value::String(reason.to_owned()));
    Value::Object(other)
}

fn task_billing_other(task_id: &str, billing: &Map<String, Value>) -> Map<String, Value> {
    let mut other = Map::new();
    for (key, value) in billing {
        other.insert(key.clone(), value.clone());
    }
    other.insert("task_id".to_owned(), Value::String(task_id.to_owned()));
    // Forced last so a copied billing snapshot cannot drop the task flag.
    other.insert("is_task".to_owned(), Value::Bool(true));
    other
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn billing() -> Map<String, Value> {
        json!({
            "billing_mode": "tiered_expr",
            "matched_tier": "recorded",
            "expr_b64": "dGllcg",
            "is_task": false
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn settlement_and_refund_logs_are_task_logs() {
        let billing = billing();
        let settlement = task_settlement_other("task-1", 10, 4, &billing);
        assert_eq!(settlement["is_task"], json!(true));
        assert_eq!(settlement["task_id"], json!("task-1"));
        assert_eq!(settlement["pre_consumed_quota"], json!(10));
        assert_eq!(settlement["actual_quota"], json!(4));
        assert_eq!(settlement["billing_mode"], json!("tiered_expr"));
        assert_eq!(settlement["matched_tier"], json!("recorded"));
        assert_eq!(settlement["expr_b64"], json!("dGllcg"));
        assert!(settlement.get("reason").is_none());

        let refund = task_refund_other("task-1", "upstream failure", &billing);
        assert_eq!(refund["is_task"], json!(true));
        assert_eq!(refund["task_id"], json!("task-1"));
        assert_eq!(refund["reason"], json!("upstream failure"));
        assert_eq!(refund["billing_mode"], json!("tiered_expr"));
        assert!(refund.get("pre_consumed_quota").is_none());
        assert!(refund.get("actual_quota").is_none());
    }

    #[test]
    fn empty_billing_context_still_marks_the_task() {
        let empty = Map::new();
        let settlement = task_settlement_other("task-2", 0, 0, &empty);
        let refund = task_refund_other("task-2", "", &empty);
        assert_eq!(
            settlement,
            json!({
                "task_id": "task-2",
                "is_task": true,
                "pre_consumed_quota": 0,
                "actual_quota": 0
            })
        );
        assert_eq!(
            refund,
            json!({
                "task_id": "task-2",
                "is_task": true,
                "reason": ""
            })
        );
    }
}

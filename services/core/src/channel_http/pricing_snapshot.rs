//! Read-only original pricing evidence. This document never supplies relay prices.
use super::{ChannelState, auth_error, authorized_read_snapshot, error_response};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

const OPTION_KEY: &str = "legacy_pricing_snapshot.v1";
const CONTRACT: &str = "core.legacy-pricing-snapshot.v1";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const OPTION_KEYS: &[&str] = &[
    "ModelRatio",
    "CompletionRatio",
    "CacheRatio",
    "CreateCacheRatio",
    "AudioRatio",
    "AudioCompletionRatio",
    "ImageRatio",
    "ModelPrice",
    "ModelPriceLock",
    "GroupRatio",
    "GroupGroupRatio",
    "group_ratio_setting.group_ratio",
    "group_ratio_setting.group_special_usable_group",
    "group_ratio_setting.group_warnings",
    "UserUsableGroups",
    "DefaultUseAutoGroup",
    "billing_setting.billing_mode",
    "billing_setting.billing_expr",
    "ServiceTierPricingCatalog",
    "ServiceTierPricingPolicy",
    "dynamic_pricing_setting.base_price_usd_per_million",
    "USDExchangeRate",
];

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    contract: String,
    purpose: String,
    source_sha256: String,
    observed_at: i64,
    options: Vec<RawOption>,
    enabled_channels: Vec<ChannelMapping>,
    unsupported: Vec<UnsupportedMapping>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawOption {
    key: String,
    // Keep original text, including decimal/exponent lexemes and whitespace.
    value: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChannelMapping {
    id: u64,
    name: String,
    models: String,
    groups: String,
    model_mapping: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UnsupportedMapping {
    model: String,
    group: String,
    reason: String,
    source_channel_ids: Vec<u64>,
}

fn text(value: &str, max: usize) -> bool {
    value.len() <= max && !value.contains('\0')
}
fn parse(raw: &str) -> Result<Snapshot, ()> {
    if raw.len() > MAX_BYTES {
        return Err(());
    }
    let snapshot: Snapshot = serde_json::from_str(raw).map_err(|_| ())?;
    if snapshot.contract != CONTRACT
        || snapshot.purpose != "reference_only"
        || snapshot.source_sha256.len() != 64
        || !snapshot
            .source_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !(1..=253_402_300_799).contains(&snapshot.observed_at)
        || snapshot.options.len() > OPTION_KEYS.len()
        || snapshot.enabled_channels.len() > 4096
        || snapshot.unsupported.len() > 10_000
    {
        return Err(());
    }
    let mut keys = BTreeSet::new();
    for option in &snapshot.options {
        if !OPTION_KEYS.contains(&option.key.as_str())
            || !keys.insert(option.key.as_str())
            || !text(&option.value, 512 * 1024)
        {
            return Err(());
        }
    }
    let mut channels = BTreeSet::new();
    for channel in &snapshot.enabled_channels {
        if channel.id == 0
            || channel.id > 9_007_199_254_740_991
            || !channels.insert(channel.id)
            || !text(&channel.name, 256)
            || !text(&channel.models, 128 * 1024)
            || !text(&channel.groups, 8192)
            || !text(&channel.model_mapping, 128 * 1024)
        {
            return Err(());
        }
    }
    let mut pairs = BTreeSet::new();
    for item in &snapshot.unsupported {
        if item.model.is_empty()
            || item.group.is_empty()
            || !text(&item.model, 1024)
            || !text(&item.group, 1024)
            || item.reason.is_empty()
            || item.reason.len() > 160
            || !item
                .reason
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || !pairs.insert((&item.model, &item.group))
            || item.source_channel_ids.len() > 4096
            || item
                .source_channel_ids
                .iter()
                .any(|id| !channels.contains(id))
        {
            return Err(());
        }
    }
    Ok(snapshot)
}

pub(super) async fn read(State(state): State<ChannelState>, headers: HeaderMap) -> Response {
    let mut tx = match authorized_read_snapshot(&state, &headers).await {
        Ok(tx) => tx,
        Err(error) => return auth_error(error),
    };
    let raw: Result<Option<Option<String>>, sqlx::Error> = sqlx::query_scalar(
        "SELECT CASE WHEN octet_length(value)<=$2 THEN value ELSE NULL END
         FROM core_console.options WHERE key=$1",
    )
    .bind(OPTION_KEY)
    .bind(MAX_BYTES as i64)
    .fetch_optional(&mut *tx)
    .await;
    let snapshot = match raw {
        Ok(None) => None,
        Ok(Some(Some(raw))) => match parse(&raw) {
            Ok(snapshot) => Some(snapshot),
            Err(()) => {
                return error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "PRICING_SOURCE_SNAPSHOT_INVALID",
                    "The stored original pricing snapshot is invalid",
                );
            }
        },
        Ok(Some(None)) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "PRICING_SOURCE_SNAPSHOT_INVALID",
                "The stored original pricing snapshot exceeds the size limit",
            );
        }
        Err(_) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "PRICING_SOURCE_SNAPSHOT_UNAVAILABLE",
                "The original pricing snapshot could not be read",
            );
        }
    };
    if tx.commit().await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PRICING_SOURCE_SNAPSHOT_UNAVAILABLE",
            "The original pricing snapshot could not be read",
        );
    }
    Json(json!({"success":true,"data":snapshot})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use std::sync::Arc;
    use tower::ServiceExt;

    fn fixture() -> serde_json::Value {
        json!({
            "contract":CONTRACT,"purpose":"reference_only",
            "source_sha256":"a".repeat(64),"observed_at":1_794_852_236_i64,
            "options":[{"key":"ModelRatio","value":"{\"model\":1.2300e-7}"}],
            "enabled_channels":[{"id":1,"name":"Channel","models":"model","groups":"default","model_mapping":""}],
            "unsupported":[{"model":"model","group":"default","reason":"media_contract_required","source_channel_ids":[1]}]
        })
    }

    #[test]
    fn preserves_original_numeric_text_and_rejects_authority_fields() {
        let value = fixture();
        let parsed = parse(&value.to_string()).unwrap();
        assert_eq!(parsed.options[0].value, "{\"model\":1.2300e-7}");
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
        for change in [
            ("contract", json!("other")),
            ("purpose", json!("active_billing")),
            ("source_sha256", json!("not-a-source-hash")),
            ("observed_at", json!(0)),
            ("secret", json!("must-not-be-returned")),
        ] {
            let mut candidate = fixture();
            candidate[change.0] = change.1;
            assert!(parse(&candidate.to_string()).is_err());
        }
        let mut candidate = fixture();
        candidate["options"][0]["key"] = json!("StripeSecretKey");
        assert!(parse(&candidate.to_string()).is_err());
        let mut candidate = fixture();
        candidate["enabled_channels"][0]["key"] = json!("must-not-be-returned");
        assert!(parse(&candidate.to_string()).is_err());
    }

    #[test]
    fn rejects_oversized_duplicate_and_unbound_entries() {
        assert!(parse(&" ".repeat(MAX_BYTES + 1)).is_err());
        let mut candidate = fixture();
        candidate["options"][0]["value"] = json!("1".repeat(512 * 1024 + 1));
        assert!(parse(&candidate.to_string()).is_err());
        for name in ["options", "enabled_channels", "unsupported"] {
            let mut candidate = fixture();
            let duplicate = candidate[name][0].clone();
            candidate[name].as_array_mut().unwrap().push(duplicate);
            assert!(parse(&candidate.to_string()).is_err());
        }
        let mut candidate = fixture();
        candidate["unsupported"][0]["source_channel_ids"] = json!([2]);
        assert!(parse(&candidate.to_string()).is_err());
    }

    #[tokio::test]
    async fn endpoint_requires_session_and_has_no_write_method() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let (routing, prices) = crate::channel_http::tests::fixture();
        let routing = crate::routing::Router::new(
            routing,
            Arc::new(crate::routing::PriceSnapshot::compile(prices).unwrap()),
        )
        .unwrap();
        let app = crate::channel_http::router(pool, Arc::new(routing));
        for (method, status) in [
            ("GET", StatusCode::UNAUTHORIZED),
            ("POST", StatusCode::METHOD_NOT_ALLOWED),
            ("PUT", StatusCode::METHOD_NOT_ALLOWED),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/core/v1/admin/routing/pricing-source-snapshot")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
    }
}

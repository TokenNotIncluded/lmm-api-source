use super::*;
use crate::billing::charging::CacheCreationUsage;

fn request(state: &str) -> ValidatedRequest {
    ValidatedRequest {
        id: "request-1".into(),
        user: 7,
        key: Some(9),
        oauth_source: None,
        credential_source: "api_key",
        payer_is_owner: true,
        sponsored: false,
        at: 1700000000,
        token_name: " =SUM(A1)".into(),
        state: state.into(),
        outcome: (state == "settled").then(|| "completed".into()),
        payer_kind: "personal".into(),
        source: Source::Wallet,
        reserved: 100,
        price: Price {
            tool: None,
            grounding: None,

            user_trust: None,
            image: None,
            version: "frozen-1".into(),
            model: "model,one".into(),
            group: "default".into(),
            input_per_million: 0,
            cached_per_million: 0,
            output_per_million: 0,
            upstream_cost_group_ppm: None,
            tiered: None,
        },
        usage: Usage {
            input: 100,
            cached: 20,
            output: 10,
            cache_creation: Some(CacheCreationUsage {
                total: 30,
                five_minute: Some(5),
                one_hour: None,
                twelve_hour: Some(25),
                cache_creation_mode: None,
                reported_total: None,
            }),
            ..Default::default()
        },
        amounts: Amounts {
            gross: 80,
            refunded: 20,
        },
    }
}

#[test]
fn csv_preserves_receipts_unknown_values_and_text_safety() {
    let mut export = export::NativeCsv::new().unwrap();
    export.include(&request("settled")).unwrap();
    export.include(&request("reconcile")).unwrap();
    let value = String::from_utf8(export.finish()).unwrap();
    let lines: Vec<_> = value.split("\r\n").collect();
    assert!(lines[0].contains("\"price_version\""));
    assert!(lines[0].contains("\"cache_write_12h_tokens\""));
    assert!(lines[1].contains("\"' =SUM(A1)\""));
    assert!(lines[1].contains("\"model,one\""));
    assert!(lines[1].contains("\"80\",\"20\",\"60\""));
    assert!(lines[1].contains("\"30\",\"5\",\"\",\"25\""));
    assert!(lines[2].contains("\"reconcile\""));
    assert!(!lines[2].contains("\"80\""));
    assert!(!lines[2].contains("\"25\""));
    assert!(!value.contains("oauth_source"));
}

#[test]
fn cache_summary_excludes_unsettled_and_retains_unreported_lifetimes() {
    let mut summary = Summary::default();
    summary.include(&request("settled"), 1700000001).unwrap();
    summary.include(&request("reconcile"), 1700000001).unwrap();
    let value = summary.project();
    assert_eq!(value["cache_read_tokens"], 20);
    assert_eq!(value["cache_write_tokens"], 30);
    assert_eq!(value["cache_write_5m_tokens"], 5);
    assert!(value["cache_write_1h_tokens"].is_null());
    assert_eq!(value["cache_write_12h_tokens"], 25);
    let mut absent = request("settled");
    absent.usage.cache_creation = None;
    summary.include(&absent, 1700000001).unwrap();
    assert_eq!(summary.project()["cache_read_tokens"], 40);
    assert!(summary.project()["cache_write_tokens"].is_null());
}

#[test]
fn export_rejects_pagination_and_invalid_self_filters() {
    let query = UsageQuery {
        p: Some(2),
        ..Default::default()
    };
    assert!(query.validate_export().is_err());
    let query = UsageQuery {
        page_size: Some(10),
        ..Default::default()
    };
    assert!(query.validate_export().is_err());
    let query = UsageQuery {
        username: Some("other".into()),
        ..Default::default()
    };
    assert!(query.validate(1700000000, false).is_err());
    assert!(query.validate(1700000000, true).is_ok());
}

#[test]
fn tool_receipts_export_success_counts_without_synthetic_tokens() {
    use crate::model_http::tool_price::{PersistedToolPrice, ToolKind, ToolUsage};
    let mut tool = request("settled");
    tool.price.model = "deepseek-search".into();
    tool.price.tool = Some(PersistedToolPrice {
        kind: ToolKind::DeepSeekSearch,
        usd_micros_per_request: 750,
        group_multiplier_ppm: 1_000_000,
        speed_multiplier_ppm: 1_000_000,
        credits_per_usd: 500_000,
        provider_contract_revision: "synthetic-tools-v1".into(),
    });
    tool.usage = Usage {
        tool: ToolUsage::from_response(
            ToolKind::DeepSeekSearch,
            200,
            br#"{"results":[{"title":"first"},{"title":"second"}]}"#,
        ),
        ..Default::default()
    };
    tool.amounts = Amounts {
        gross: 375,
        refunded: 0,
    };
    let log = tool.log();
    assert_eq!(log["pricing_dimension"], "tool");
    assert_eq!(log["tool_kind"], "deepseek_search");
    assert_eq!(log["provider_http_status"], 200);
    assert_eq!(log["outcome"], "completed");
    assert_eq!(log["tool_requests"], 1);
    assert!(log.get("prompt_tokens").is_none());
    assert!(log.get("completion_tokens").is_none());
    let mut export = export::NativeCsv::new().unwrap();
    export.include(&tool).unwrap();
    let csv = String::from_utf8(export.finish()).unwrap();
    assert!(csv.ends_with("\"per_successful_request\",\"deepseek_search\",\"1\",\"\",\"\"\r\n"));
    // The eight token/cache/image columns remain unknown/not applicable.
    assert!(csv.contains(
        "\"375\",\"0\",\"375\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"per_successful_request\""
    ));

    tool.usage.tool = ToolUsage::from_response(ToolKind::DeepSeekSearch, 200, br#"{"results":[]}"#);
    tool.amounts = Amounts::default();
    assert_eq!(tool.log()["tool_requests"], 0);
    let mut summary = Summary::default();
    summary.include(&tool, 1700000001).unwrap();
    assert_eq!(summary.project()["tool_requests"], 0);
    assert_eq!(summary.project()["net_consumption_credits"], 0);
}

#[test]
fn grounding_receipt_keeps_reported_tool_input_separate_from_token_usage() {
    use crate::model_http::grounding_price::{
        GroundingContract, GroundingMode, GroundingUsage, PersistedGroundingPrice,
        ToolInputSemantics,
    };
    let mut grounded = request("settled");
    grounded.price.grounding = Some(PersistedGroundingPrice {
        contract: GroundingContract {
            mode: GroundingMode::GroundedRequestV1,
            tool_input: ToolInputSemantics::ExcludeSearchRetrievalV1,
            usd_micros_per_grounded_request: 1,
        },
        group_multiplier_ppm: 1_000_000,
        speed_multiplier_ppm: 1_000_000,
        provider_contract_revision: "synthetic-grounding-v1".into(),
    });
    grounded.usage.cache_creation = None;
    grounded.usage.grounding = Some(GroundingUsage {
        grounded_requests: 1,
        tool_input_tokens: Some(7),
    });
    let log = grounded.log();
    assert_eq!(log["grounded_requests"], 1);
    assert_eq!(log["prompt_tokens"], 100);
    let details: Value = serde_json::from_str(log["other"].as_str().unwrap()).unwrap();
    assert_eq!(details["tool_input_tokens"], 7);
    let mut export = export::NativeCsv::new().unwrap();
    export.include(&grounded).unwrap();
    assert!(
        String::from_utf8(export.finish())
            .unwrap()
            .ends_with("\"measured_usage\",\"\",\"\",\"1\",\"7\"\r\n")
    );
}

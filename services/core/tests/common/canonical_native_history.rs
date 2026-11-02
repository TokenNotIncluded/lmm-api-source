//! Valid frozen Native prices for44 canonical backfill, separate from fixed43
//! receipt-free identity-continuity examples. No validator or source DDL bypass.
use lmm_core::{
    billing::charging::Price,
    model_http::image_price::{ImagePriceConfig, ImageQuoteRequest},
};
use serde_json::json;
struct NativeCase {
    text: Price,
    image: Price,
}
impl NativeCase {
    fn free() -> Self {
        let text = Price {
            version: "synthetic-bootstrap-v1".into(),
            model: "synthetic-text".into(),
            group: "default".into(),
            input_per_million: 0,
            cached_per_million: 0,
            output_per_million: 0,
            text_request: None,
            tool: None,
            audio: None,
            grounding: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
        };
        let binding:ImageQuoteRequest=serde_json::from_value(json!({"upstream_id":203,"vendor":"openai","billing_group":"default","upstream_model":"synthetic-image","semantic_model":"synthetic-image","endpoint":"generations","size":"1024x1024","quality":"standard","n":1})).unwrap();
        let config:ImagePriceConfig=serde_json::from_value(json!({"revision":1,"rules":[{"upstream_id":203,"vendor":"openai","billing_group":"default","upstream_model":"synthetic-image","semantic_model":"synthetic-image","endpoint":"generations","size":"1024x1024","quality":"standard","usd_micros_per_image":{"numerator":0,"denominator":1},"group_multiplier":{"numerator":1,"denominator":1},"max_n":1,"count_basis":"requested","max_budget":0,"source_revision":"synthetic-frozen42"}]})).unwrap();
        let quote = config.quote(&binding).unwrap();
        let image = Price {
            version: format!("image-{}", quote.revision()),
            model: binding.semantic_model.clone(),
            group: binding.billing_group.clone(),
            image: Some(quote.persisted()),
            ..text.clone()
        };
        text.validate().unwrap();
        image.validate().unwrap();
        assert_eq!(i64::from(quote.reservation_credits()), 0);
        Self { text, image }
    }
}
pub async fn seed_history(pool: &sqlx::PgPool) {
    super::history::seed_published_history(pool).await;
    let case = NativeCase::free();
    // Held zero-value requests have no settlement/refund receipts to invent.
    // The existing partial attribution proof stays unknown; image proof remains
    // the exact quote restored and validated by the normal canonical validator.
    super::history::seed_native_history_prices(
        pool,
        &serde_json::to_value(case.text).unwrap(),
        &serde_json::to_value(case.image).unwrap(),
    )
    .await;
}

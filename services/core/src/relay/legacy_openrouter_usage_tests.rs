use super::*;
use crate::{billing::trust_pricing::DiscountRatio, routing::{InputSemantics, PersistedTieredTariff, TieredUsage}};
use serde_json::json;

fn frozen_tariff() -> PersistedTieredTariff {
    serde_json::from_value(json!({"tariff":{"revision":3,"source_digest":"a".repeat(64),
        "uses_cache_read":true,"uses_cache_write":true,"uses_cache_write_1h":true,
        "tiers":[{"up_to_input_tokens":null,"rates":{
            "input":{"numerator":10,"denominator":1},"output":{"numerator":50,"denominator":1},
            "cache_read":{"numerator":1,"denominator":4},"cache_write":{"numerator":25,"denominator":2},
            "cache_write_1h":{"numerator":20,"denominator":1}}}]},
        "input_semantics":"anthropic_text","group_multiplier_ppm":1200000,
        "speed_multiplier_ppm":1000000,"credits_per_usd":500000,
        "missing_cache_creation_is_zero":true,"cache_creation_mode":"generic_remainder_v1"})).unwrap()
}
fn amount(u: &Usage, tariff: &PersistedTieredTariff, trust: Option<DiscountRatio>) -> u64 {
    let d=u.cache_creation_details.unwrap();
    tariff.cost_with_discount(TieredUsage {input:u.input_tokens.unwrap(),cached:u.cached_input_tokens.unwrap(),
        output:u.output_tokens.unwrap(),created:u.cache_creation_input_tokens.unwrap(),
        created_5m:d.ephemeral_5m_input_tokens,created_1h:d.ephemeral_1h_input_tokens,..Default::default()},trust).unwrap()
}
fn cost(u: &Usage) -> u64 { amount(u,&frozen_tariff(),Some(DiscountRatio::new(9,10).unwrap())) }
fn body() -> Value { json!({"prompt_tokens":100,"completion_tokens":19,"prompt_tokens_details":{"cached_tokens":20,"cached_creation_tokens":10}}) }

#[test]
fn fixed_old_three_counterexamples_and_ordinary_roundtrip() {
    // Exact cc52 source formulas: 1251.45 / 1035.45 / 947.7, round once.
    let mut ttl=body();ttl["claude_cache_creation_5_m_tokens"]=json!(5);ttl["claude_cache_creation_1_h_tokens"]=json!(15);
    let mut anthropic=ttl.clone();anthropic["usage_semantic"]=json!("anthropic");
    let canonical=json!({"prompt_tokens":100,"completion_tokens":19,"prompt_tokens_details":{"cached_tokens":20},"cache_creation_input_tokens":10,"cache_creation":7});
    for (raw,expected) in [(ttl,1251),(anthropic,1035),(canonical,948),(body(),961)] {
        let u=project(&raw).unwrap();assert_eq!(cost(&u),expected);
        assert_eq!(u.legacy_openrouter_raw.as_deref().map(|v| &v.0),Some(&raw));
        let private:Usage=serde_json::from_value(serde_json::to_value(&u).unwrap()).unwrap();
        assert_eq!(private,u);assert_eq!(cost(&private),expected);
        // Frozen disjoint counters alone suffice after restart, independently
        // of any live payload policy. The raw witness is separately audited.
        let checkpoint=u.checkpoint_usage();assert!(checkpoint.legacy_openrouter_raw.is_none());
        let restored:Usage=serde_json::from_value(serde_json::to_value(checkpoint).unwrap()).unwrap();
        assert_eq!(cost(&restored),expected);
        let frozen:PersistedTieredTariff=serde_json::from_value(serde_json::to_value(frozen_tariff()).unwrap()).unwrap();
        assert_eq!(amount(&restored,&frozen,Some(DiscountRatio::new(9,10).unwrap())),expected);
    }
}
#[test]
fn dto_alias_max_ttl_max_and_semantic_source_priority() {
    let mut b=body();b["prompt_tokens_details"]["cache_write_tokens"]=json!(12);
    assert_eq!(project(&b).unwrap().cache_creation_input_tokens,Some(12));
    b["claude_cache_creation_5_m_tokens"]=json!(5);b["claude_cache_creation_1_h_tokens"]=json!(15);
    let u=project(&b).unwrap();assert_eq!(u.cache_creation_input_tokens,Some(20));assert_eq!(u.uncached_input_tokens,Some(100));
    for (semantic,source,uncached,hour) in [("anthropic","anything",60,15),("openai","",60,0),("","oai_chat",60,0),("Anthropic","",60,0),("","",100,15)] {
        b["usage_semantic"]=json!(semantic);b["usage_source"]=json!(source);
        let u=project(&b).unwrap();assert_eq!(u.uncached_input_tokens,Some(uncached));
        assert_eq!(u.cache_creation_details.unwrap().ephemeral_1h_input_tokens,Some(hour));
    }
}
#[test]
fn billing_envelope_priority_openai_fallback_and_anthropic_nested_ttl() {
    let mut b=body();b["billing_usage"]=json!({"source":" OAI_RESPONSES ","semantic":"anthropic",
        "openai_usage":{"input_tokens":100,"output_tokens":19,"input_tokens_details":{"cached_tokens":20,"cache_write_tokens":10}},
        "claude_usage":{"input_tokens":999,"output_tokens":999}});
    assert_eq!(cost(&project(&b).unwrap()),961);
    b["billing_usage"]=json!({"source":"claude_messages","claude_usage":{"input_tokens":100,"output_tokens":19,
        "cache_read_input_tokens":20,"cache_creation_input_tokens":10,"claude_cache_creation_1_h_tokens":1,
        "cache_creation":{"ephemeral_5m_input_tokens":5,"ephemeral_1h_input_tokens":15}}});
    assert_eq!(cost(&project(&b).unwrap()),1035);
    // A non-zero negative field is not a zero fallback in the fixed DTO.
    b["billing_usage"]=json!({"source":"oai_chat","openai_usage":{"prompt_tokens":-1,"input_tokens":100,"completion_tokens":19}});
    assert_eq!(project(&b).unwrap().input_tokens,Some(0));
}
#[test]
fn raw_is_private_no_public_metadata_and_default_none_bytes_unchanged() {
    let mut raw=body();raw["private_cost"]=json!({"internal":"evidence-only"});raw["unknown_metadata"]=json!("private-marker");
    let u=project(&raw).unwrap();let private=serde_json::to_string(&u).unwrap();assert!(private.contains("private-marker"));
    assert!(!format!("{u:?}").contains("private-marker"));
    for client in [super::super::ClientProtocol::Chat,super::super::ClientProtocol::Responses] {
        let public=serde_json::to_string(&u.wire(client)).unwrap();
        for forbidden in ["legacy_openrouter_raw","private_cost","unknown_metadata","private-marker"] {assert!(!public.contains(forbidden));}
    }
    assert!(!serde_json::to_string(&u.checkpoint_usage()).unwrap().contains("private-marker"));
    assert!(serde_json::to_vec(&u.checkpoint_usage()).unwrap().len()<1024);
    assert_eq!(serde_json::to_string(&Usage::default()).unwrap(),
        r#"{"input_tokens":null,"output_tokens":null,"cached_input_tokens":null,"cache_creation_input_tokens":null,"reasoning_output_tokens":null}"#);
    let native=super::super::decode::usage_with_cache_creation_mode(super::super::UpstreamProtocol::Chat,&body(),Some(CacheCreationMode::GenericRemainderV1)).unwrap();
    assert!(native.legacy_openrouter_raw.is_none());
    assert!(!serde_json::to_string(&native).unwrap().contains("legacy_openrouter_raw"));
}
#[test]
fn last_usage_sse_snapshot_reprojects_without_relaxing_native_merge() {
    let mut u=Usage::default();let first=project(&json!({"prompt_tokens":100,"completion_tokens":19})).unwrap();u.merge(first).unwrap();
    u.merge(project(&body()).unwrap()).unwrap();assert_eq!(u.uncached_input_tokens,Some(70));assert_eq!(cost(&u),961);
    let mut native=Usage{input_tokens:Some(100),..Default::default()};
    assert!(native.merge(Usage{input_tokens:Some(99),..Default::default()}).is_err());
    assert!(u.merge(native).is_err());
}
#[test]
fn positive_domain_minimum_zero_group_trust_and_reservation_bound() {
    let mut tariff=frozen_tariff();
    let zero=project(&json!({"prompt_tokens":0,"completion_tokens":0})).unwrap();assert_eq!(cost(&zero),0);
    let one=project(&json!({"prompt_tokens":1,"completion_tokens":0})).unwrap();
    assert_eq!(cost(&one),5);
    let small=project(&json!({"prompt_tokens":1,"completion_tokens":0,"prompt_tokens_details":{"cached_tokens":1}})).unwrap();assert_eq!(cost(&small),1);
    tariff.group_multiplier_ppm=0;assert_eq!(amount(&one,&tariff,None),0);
    tariff.group_multiplier_ppm=1200000;assert!(DiscountRatio::new(0,1).is_err());
    assert_eq!(amount(&one,&tariff,Some(DiscountRatio::new(1,1000).unwrap())),1);
    let max=tariff.tariff.compile().unwrap().reservation_ceiling(1000,100,1200000,1000000,500000).unwrap();
    assert!(max>=cost(&project(&body()).unwrap()));assert_eq!(tariff.input_semantics,InputSemantics::AnthropicText);
}
#[test]
fn malformed_overflow_audio_and_unreviewed_envelope_reject() {
    for raw in [json!({"private_cost":1}),json!({"prompt_tokens":"100"}),json!({"prompt_tokens_details":7}),
        json!({"prompt_tokens":9223372036854775807u64,"claude_cache_creation_5_m_tokens":1}),
        json!({"prompt_tokens":100,"prompt_tokens_details":{"audio_tokens":1}}),
        json!({"billing_usage":{"source":"gemini_chat","gemini_usage_metadata":{}}})] {
        assert!(project(&raw).is_err(),"{raw}");
    }
}

#[test]
fn explicit_policy_is_chat_only_distinct_and_no_implicit_model_activation() {
    use crate::routing::{PayloadPolicy,Protocol};
    let old=PayloadPolicy::OpenRouter;let legacy=PayloadPolicy::LegacyOpenRouterChatUsageV1;
    assert_eq!(serde_json::to_string(&legacy).unwrap(),"\"legacy_openrouter_chat_usage_v1\"");
    assert_ne!(serde_json::to_value(old).unwrap(),serde_json::to_value(legacy).unwrap());
    assert!(legacy.adapts(Protocol::OpenAiChat,Protocol::OpenAiChat));
    for other in [Protocol::AnthropicMessages,Protocol::Gemini,Protocol::OpenAiResponses] {
        assert!(!legacy.adapts(other,Protocol::OpenAiChat));assert!(!legacy.adapts(Protocol::OpenAiChat,other));
    }
}

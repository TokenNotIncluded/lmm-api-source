//! Client tier controls require both global opt-in and a priced provider contract.
use super::{ClientProtocol, Limits, Request, Route, UpstreamProtocol};
use crate::routing::PayloadPolicy;
use serde_json::{Value, json};

#[test]
fn tier_fields_are_ignored_by_default_and_require_a_priced_attempt_when_enabled() {
    for (client, upstream) in [
        (ClientProtocol::Chat, UpstreamProtocol::Chat),
        (ClientProtocol::Responses, UpstreamProtocol::Responses),
    ] {
        for stream in [false, true] {
            let base = if client == ClientProtocol::Chat {
                json!({"model":"alias","messages":[{"role":"user","content":"hi"}],"stream":stream})
            } else {
                json!({"model":"alias","input":"hi","stream":stream})
            };
            let policy = PayloadPolicy::LegacyOpenai {
                allow_service_tier: true,
                allow_inference_geo: false,
                allow_speed: true,
                allow_safety_identifier: false,
                allow_include_obfuscation: false,
            };
            let route = Route::new(
                "selected".into(),
                upstream,
                "https://provider.invalid/v1",
                "real".into(),
                "synthetic",
                1,
            )
            .unwrap()
            .with_payload_policy(policy);
            for field in ["service_tier", "speed"] {
                for tier in ["fast", "ultrafast", "priority", "auto", "flex", "scale"] {
                    let mut body = base.clone();
                    body[field] = json!(tier);
                    let mut request = Request::parse_deferred_openai(
                        client,
                        &serde_json::to_vec(&body).unwrap(),
                        &Limits::default(),
                    )
                    .unwrap();
                    assert!(!request.can_forward_original_tier());
                    assert_eq!(request.apply_service_tier_policy(false).unwrap(), None);
                    let payload = request.upstream(&route).unwrap();
                    assert!(payload.get(field).is_none());
                    if client == ClientProtocol::Chat {
                        let router = route.clone().with_payload_policy(PayloadPolicy::OpenRouter);
                        assert!(request.upstream(&router).unwrap().get(field).is_none());
                    }
                    if (field == "speed" && matches!(tier, "fast" | "ultrafast"))
                        || (field == "service_tier"
                            && (matches!(tier, "fast" | "priority")
                                || (client == ClientProtocol::Responses && tier == "ultrafast")))
                    {
                        let speed = if tier != "ultrafast" {
                            crate::routing::Speed::Fast
                        } else {
                            crate::routing::Speed::Ultrafast
                        };
                        assert_eq!(
                            request.apply_service_tier_policy(true).unwrap(),
                            Some(speed)
                        );
                        assert!(request.upstream(&route).is_err());
                        assert!(request.upstream_body_to(upstream, "real").is_err());
                        assert!(
                            request
                                .upstream(&route.clone().with_payload_policy(PayloadPolicy::Adapt))
                                .is_err()
                        );
                    } else {
                        assert!(request.apply_service_tier_policy(true).is_err());
                    }
                }
            }
            for tier in [Value::Null, json!("default")] {
                let mut body = base.clone();
                body["service_tier"] = tier.clone();
                body["speed"] = json!("standard");
                let mut request = Request::parse_deferred_openai(
                    client,
                    &serde_json::to_vec(&body).unwrap(),
                    &Limits::default(),
                )
                .unwrap();
                assert_eq!(
                    request.apply_service_tier_policy(true).unwrap(),
                    Some(crate::routing::Speed::Standard)
                );
                let payload = request.upstream(&route).unwrap();
                assert_eq!(payload["service_tier"], tier);
                assert_eq!(payload["speed"], "standard");
            }
        }
    }
}

#[test]
fn native_anthropic_drops_speed_from_cached_and_uncached_requests() {
    for system in [
        json!("system"),
        json!([{"type":"text","text":"system","cache_control":{"type":"ephemeral"}}]),
    ] {
        let mut body = json!({
            "model":"claude",
            "system":system,
            "messages":[{"role":"user","content":"hello"}],
            "max_tokens":16
        });
        let encode = |body: &Value| {
            Request::parse(
                ClientProtocol::Anthropic,
                &serde_json::to_vec(body).unwrap(),
                &Limits::default(),
            )
            .unwrap()
            .upstream_to(UpstreamProtocol::Anthropic, "claude")
            .unwrap()
        };
        let baseline = encode(&body);
        for speed in ["fast", "ultrafast"] {
            body["speed"] = speed.into();
            assert_eq!(encode(&body), baseline);
        }
    }
}

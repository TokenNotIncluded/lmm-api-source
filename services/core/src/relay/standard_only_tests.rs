//! First-release OpenAI compatibility may not select an unpriced service tier.
use super::{ClientProtocol, Limits, Request, Route, UpstreamProtocol};
use crate::routing::PayloadPolicy;
use serde_json::{Value, json};

#[test]
fn standard_only_openai_rejects_unpriced_tiers_and_speed_before_encoding() {
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
                    let request = Request::parse_deferred_openai(
                        client,
                        &serde_json::to_vec(&body).unwrap(),
                        &Limits::default(),
                    )
                    .unwrap();
                    assert!(
                        request.upstream(&route).is_err(),
                        "{client:?} {stream} {field} {tier}"
                    );
                    assert!(request.upstream_body(&route).is_err());
                    // OpenRouter never grants legacy OpenAI tier controls.
                    if client == ClientProtocol::Chat {
                        assert!(
                            request
                                .upstream(
                                    &route.clone().with_payload_policy(PayloadPolicy::OpenRouter)
                                )
                                .is_err()
                        );
                    }
                }
            }
            for tier in [Value::Null, json!("default")] {
                let mut body = base.clone();
                body["service_tier"] = tier.clone();
                body["speed"] = json!("standard");
                let request = Request::parse_deferred_openai(
                    client,
                    &serde_json::to_vec(&body).unwrap(),
                    &Limits::default(),
                )
                .unwrap();
                let payload = request.upstream(&route).unwrap();
                assert_eq!(payload["service_tier"], tier);
                assert_eq!(payload["speed"], "standard");
            }
        }
    }
}

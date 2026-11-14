use super::*;
use serde_json::json;

fn create(id: &str) -> Create {
    Create::parse(
        &serde_json::to_vec(&json!({"type":"response.create", "event_id":id,
        "model":"model", "input":"hello"}))
        .unwrap(),
        4096,
    )
    .unwrap()
}

#[test]
fn stateful_create_keeps_generate_previous_id_and_schema_order_in_actual_frame() {
    let request = br#"{"type":"response.create","event_id":"local-id","stream_id":"s1","model":"model","input":[],"generate":false,"previous_response_id":"resp_prior","tools":[{"type":"function","name":"f","parameters":{"type":"object","properties":{"z":{"type":"string"},"a":{"type":"string"}}}}]}"#;
    let create = Create::parse(request, 4096).unwrap();
    assert_eq!(create.generate, Some(false));
    let canonical = String::from_utf8(create.request_json.clone()).unwrap();
    assert!(canonical.contains(r#""z":{"type":"string"},"a":{"type":"string"}"#));
    assert!(!canonical.contains("previous_response_id"));
    let limits = Limits::default();
    let request =
        Request::parse_responses_ws(&create.request_json, &limits, true, create.generate).unwrap();
    let route = Route::new(
        "synthetic".into(),
        UpstreamProtocol::Responses,
        "https://example.invalid/v1/responses",
        "selected-model".into(),
        "synthetic-credential",
        1,
    )
    .unwrap();
    let prepared = create.prepare_selected(&request, &route, &limits).unwrap();
    let wire: Value = serde_json::from_slice(&prepared.wire_bytes).unwrap();
    assert_eq!(wire["model"], "selected-model");
    assert_eq!(wire["previous_response_id"], "resp_prior");
    assert_eq!(wire["generate"], false);
    assert_eq!(wire["stream_id"], "s1");
    assert!(wire.get("event_id").is_none());
    assert_eq!(
        prepared.wire_sha256,
        <[u8; 32]>::from(Sha256::digest(&prepared.wire_bytes))
    );
    assert_ne!(prepared.wire_sha256, prepared.frame_commitment);
    assert_ne!(
        prepared.wire_sha256,
        <[u8; 32]>::from(Sha256::digest(&create.request_json))
    );
    assert!(
        String::from_utf8(prepared.wire_bytes)
            .unwrap()
            .contains(r#""z":{"type":"string"},"a":{"type":"string"}"#)
    );
}

#[test]
fn wrapped_flat_and_escaped_type_preserve_same_semantics() {
    let flat = Create::parse(
        br#"{"type":"response.create","model":"m","input":"x","generate":false}"#,
        4096,
    )
    .unwrap();
    let wrapped = Create::parse(
        br#"{"type":"response.\u0063reate","request":{"model":"m","input":"x","generate":false}}"#,
        4096,
    )
    .unwrap();
    assert_eq!(
        flat.prepare(&flat.request_json, 4096).unwrap().wire_bytes,
        wrapped
            .prepare(&wrapped.request_json, 4096)
            .unwrap()
            .wire_bytes
    );
    for invalid in [
        r#"{"type":"response.create","TYPE":"response.create"}"#,
        r#"{"type":"response.create","request":{"model":"a","model":"b"}}"#,
        r#"{"type":"response.create","request":{},"model":"m"}"#,
        r#"{"type":"response.create","generate":false,"request":{"generate":true}}"#,
        r#"{"type":"response.create","generate":"false"}"#,
        r#"{"type":"response.create","previous_response_id":1}"#,
        r#"{"type":"response.create","background":true}"#,
    ] {
        assert!(
            Create::parse(invalid.as_bytes(), 4096).is_err(),
            "{invalid}"
        );
    }
    assert!(Create::parse(br#"{"type":"response.create"}"#, 4).is_err());
    let deep = format!(
        "{{\"type\":\"response.create\",\"input\":{}0{}}}",
        "[".repeat(140),
        "]".repeat(140)
    );
    assert!(Create::parse(deep.as_bytes(), 4096).is_err());
}

#[test]
fn prepare_refuses_envelope_smuggling_and_binds_each_real_change() {
    let a = create("a");
    for field in [
        "type",
        "generate",
        "event_id",
        "stream_id",
        "previous_response_id",
    ] {
        let body = serde_json::to_vec(&json!({"model":"m",field:"injected"})).unwrap();
        assert!(a.prepare(&body, 4096).is_err(), "{field}");
    }
    let first = a.prepare(&a.request_json, 4096).unwrap();
    let mut b = create("a");
    b.generate = Some(false);
    assert_ne!(
        first.frame_commitment,
        b.prepare(&b.request_json, 4096).unwrap().frame_commitment
    );
    assert!(a.prepare(&a.request_json, 10).is_err());
    assert_ne!(
        turn_id(&[1; 32], &[2; 32], 1, Some("a")),
        turn_id(&[1; 32], &[3; 32], 1, Some("a"))
    );
    assert_ne!(
        turn_id(&[1; 32], &[2; 32], 1, None),
        turn_id(&[1; 32], &[2; 32], 2, None)
    );
    assert_ne!(
        turn_id(&[1; 32], &[2; 32], 1, None),
        turn_id(&[1; 32], &[2; 32], 1, Some("1"))
    );
    assert_eq!(
        turn_id(&[1; 32], &[2; 32], 1, Some("a")),
        turn_id(&[1; 32], &[2; 32], 2, Some("a"))
    );
}

#[test]
fn finalizer_fence_duplicate_create_and_late_terminal_are_distinct() {
    let mut state = Correlation::default();
    let first = state.begin(&create("first")).unwrap();
    assert!(state.begin(&create("second")).is_err());
    assert_eq!(
        state
            .attribute(&json!({"type":"response.created","response":{"id":"resp1"}}))
            .unwrap(),
        Attribution::Active
    );
    assert_eq!(state.attribute(&json!({"type":"response.completed","response":{"id":"resp1","usage":{"input_tokens":0,"output_tokens":0}}})).unwrap(), Attribution::Active);
    // Observing a terminal is NOT settlement confirmation.
    assert!(state.begin(&create("second")).is_err());
    assert!(state.finish(first + 1).is_err());
    state.finish(first).unwrap();
    assert!(state.begin(&create("first")).is_err());
    let second = state.begin(&create("second")).unwrap();
    assert_eq!(state.attribute(&json!({"type":"response.completed","response":{"id":"resp1","usage":{"input_tokens":900}}})).unwrap(), Attribution::Foreign);
    assert_eq!(
        state
            .attribute(&json!({"type":"response.created","response":{"id":"resp2"}}))
            .unwrap(),
        Attribution::Active
    );
    assert_eq!(
        state
            .attribute(&json!({"type":"response.cancelled","response":{"id":"resp1"}}))
            .unwrap(),
        Attribution::Foreign
    );
    state.finish(second).unwrap();
    assert!(state.finish(second).is_err());
}

#[test]
fn cancel_errors_and_ambiguous_errors_cannot_borrow_turn_usage() {
    let mut state = Correlation::default();
    let mut c = create("active");
    c.stream_id = Some("stream".into());
    state.begin(&c).unwrap();
    state.register_control("cancel1").unwrap();
    assert!(state.register_control("cancel1").is_err());
    assert!(state.register_control("active").is_err());
    assert_eq!(
        state
            .attribute(&json!({"type":"error","error":{"event_id":"cancel1"}}))
            .unwrap(),
        Attribution::Control
    );
    assert_eq!(
        state
            .attribute(&json!({"type":"error","error":{"message":"ambiguous"}}))
            .unwrap(),
        Attribution::Control
    );
    assert_eq!(
        state
            .attribute(&json!({"type":"error","error":{"event_id":"other"}}))
            .unwrap(),
        Attribution::Foreign
    );
    assert_eq!(
        state
            .attribute(&json!({"type":"error","error":{"event_id":"active"}}))
            .unwrap(),
        Attribution::Active
    );
    assert_eq!(
        state
            .attribute(
                &json!({"type":"response.created","stream_id":"other","response":{"id":"resp"}})
            )
            .unwrap(),
        Attribution::Foreign
    );
    assert_eq!(
        state
            .attribute(&json!({"type":"response.created","response_id":"a","response":{"id":"b"}})),
        Err(ProtocolError("conflicting upstream response identity"))
    );
}

#[test]
fn cancel_control_binds_to_the_active_response_and_cannot_claim_its_usage() {
    let mut state = Correlation::default();
    let mut active = create("create");
    active.stream_id = Some("s1".into());
    state.begin(&active).unwrap();
    state
        .attribute(&json!({"type":"response.created","response":{"id":"active"}}))
        .unwrap();
    for raw in [
        br#"{"type":"response.cancel","event_id":"c1","stream_id":"s2"}"#.as_slice(),
        br#"{"type":"response.cancel","event_id":"c1","response_id":"foreign"}"#,
        br#"{"type":"response.cancel","event_id":"c1","credential":"unexpected"}"#,
    ] {
        assert!(state.authorize_cancel(raw, 4096).is_err());
    }
    let cancel =
        br#"{"type":"response.cancel","event_id":"c1","stream_id":"s1","response_id":"active"}"#;
    state.authorize_cancel(cancel, 4096).unwrap();
    assert!(state.authorize_cancel(cancel, 4096).is_err());
    assert_eq!(
        state
            .attribute(&json!({"type":"error","error":{"event_id":"c1"}}))
            .unwrap(),
        Attribution::Control
    );
}

#[test]
fn session_retirement_is_bounded_without_id_eviction() {
    let mut state = Correlation::default();
    for n in 0..MAX_SESSION_TURNS {
        let sequence = state.begin(&create(&format!("e{n}"))).unwrap();
        state.finish(sequence).unwrap();
    }
    assert!(state.begin(&create("more")).is_err());
    assert!(state.begin(&create("e0")).is_err());
    let mut state = Correlation::default();
    state.begin(&create("active")).unwrap();
    for n in 0..MAX_SESSION_CONTROLS {
        state.register_control(&format!("c{n}")).unwrap();
    }
    assert!(state.register_control("more").is_err());
}

#[test]
fn failed_cancelled_and_warmup_terminal_keep_zero_distinct_from_missing() {
    for (kind, outcome) in [
        ("response.failed", TerminalOutcome::Failed),
        ("response.cancelled", TerminalOutcome::Cancelled),
        ("response.canceled", TerminalOutcome::Cancelled),
        ("response.incomplete", TerminalOutcome::Incomplete),
    ] {
        let missing = json!({"type":kind,"response":{"id":"resp1"}});
        let receipt = terminal(&missing).unwrap().unwrap();
        assert_eq!(receipt.outcome, outcome);
        assert!(receipt.usage.is_none());
        let zero = json!({"type":kind,"response":{"id":"resp1","usage":{"input_tokens":0,"output_tokens":0}}});
        let receipt = terminal(&zero).unwrap().unwrap();
        assert_eq!(receipt.usage.unwrap()["input_tokens"], 0);
        assert_eq!(receipt.outcome, outcome);
    }
    let warmup = json!({"type":"response.completed","response":{"id":"warm","status":"completed","output":[],"usage":{"input_tokens":20,"output_tokens":0}}});
    let receipt = terminal(&warmup).unwrap().unwrap();
    assert_eq!(receipt.outcome, TerminalOutcome::Completed);
    assert_eq!(receipt.usage.unwrap()["input_tokens"], 20);
    assert!(
        terminal(&json!({"type":"response.completed","response":{"status":"pending"}})).is_err()
    );
    assert!(terminal(&json!({"type":"response.failed","response":{"usage":0}})).is_err());
}

#[test]
fn anonymous_controls_never_occupy_unresolvable_slots_and_cancel_stays_owned() {
    let mut state = Correlation::default();
    let mut c = create("active");
    c.stream_id = Some("lane".into());
    let sequence = state.begin(&c).unwrap();
    state
        .attribute(&json!({"type":"response.created","response":{"id":"response"}}))
        .unwrap();
    for _ in 0..32 {
        for kind in ["input_audio_buffer.clear", "response.cancel"] {
            let raw = serde_json::to_vec(
                &json!({"type":kind,"stream_id":"lane","response_id":"response"}),
            )
            .unwrap();
            state.authorize_control(&raw, 4096).unwrap();
        }
    }
    assert!(state.control_records.is_empty());
    assert!(
        state
            .authorize_control(
                br#"{"type":"response.cancel","response_id":"foreign"}"#,
                4096
            )
            .is_err()
    );
    assert!(state.control_records.is_empty());
    state
        .authorize_control(
            br#"{"type":"input_audio_buffer.clear","event_id":"named"}"#,
            4096,
        )
        .unwrap();
    assert_eq!(
        state.control_records.values().filter(|r| r.pending).count(),
        1
    );
    state.finish(sequence).unwrap();
    assert!(
        state
            .authorize_control(br#"{"type":"response.cancel"}"#, 4096)
            .is_err()
    );
    assert_eq!(
        state.control_records.values().filter(|r| r.pending).count(),
        0
    );
}

#[test]
fn named_pending_limit_duplicate_error_and_finalized_turn_release_are_exact() {
    let mut state = Correlation::default();
    let sequence = state.begin(&create("active")).unwrap();
    for n in 0..31 {
        state
            .authorize_control(
                &serde_json::to_vec(
                    &json!({"type":"input_audio_buffer.clear","event_id":format!("p{n}")}),
                )
                .unwrap(),
                4096,
            )
            .unwrap();
    }
    assert!(
        state
            .authorize_control(
                br#"{"type":"input_audio_buffer.clear","event_id":"p0"}"#,
                4096
            )
            .is_err()
    );
    assert_eq!(
        state.control_records.values().filter(|r| r.pending).count(),
        31
    );
    state
        .authorize_control(
            br#"{"type":"input_audio_buffer.clear","event_id":"p31"}"#,
            4096,
        )
        .unwrap();
    assert!(
        state
            .authorize_control(br#"{"type":"response.cancel","event_id":"overflow"}"#, 4096)
            .is_err()
    );
    state
        .authorize_control(br#"{"type":"response.cancel"}"#, 4096)
        .unwrap();
    assert_eq!(
        state.control_records.values().filter(|r| r.pending).count(),
        32
    );
    let mut error = json!({"type":"error","error":{"event_id":"p0"}});
    assert!(state.control_error(&mut error).unwrap());
    assert!(state.control_error(&mut error).unwrap());
    assert_eq!(
        state.control_records.values().filter(|r| r.pending).count(),
        31
    );
    state
        .authorize_control(
            br#"{"type":"response.cancel","event_id":"replacement"}"#,
            4096,
        )
        .unwrap();
    assert_eq!(
        state.control_records.values().filter(|r| r.pending).count(),
        32
    );
    assert!(state.finish(sequence + 1).is_err());
    assert_eq!(
        state.control_records.values().filter(|r| r.pending).count(),
        32
    );
    state.finish(sequence).unwrap();
    assert_eq!(
        state.control_records.values().filter(|r| r.pending).count(),
        0
    );
    assert!(
        state
            .authorize_control(
                br#"{"type":"input_audio_buffer.clear","event_id":"p1"}"#,
                4096
            )
            .is_err()
    );
    let next = state.begin(&create("next")).unwrap();
    for n in 0..32 {
        state
            .authorize_control(
                &serde_json::to_vec(
                    &json!({"type":"input_audio_buffer.clear","event_id":format!("n{n}")}),
                )
                .unwrap(),
                4096,
            )
            .unwrap();
    }
    assert_eq!(
        state.control_records.values().filter(|r| r.pending).count(),
        32
    );
    state.finish(next).unwrap();
    assert_eq!(
        state.control_records.values().filter(|r| r.pending).count(),
        0
    );
    state
        .authorize_control(
            br#"{"type":"input_audio_buffer.clear","event_id":"idle-pending"}"#,
            4096,
        )
        .unwrap();
    let rejected = state.begin(&create("rejected")).unwrap();
    state.reject_unreserved(rejected);
    assert!(state.control_records["idle-pending"].pending);
    assert_eq!(state.control_records["idle-pending"].turn, None);
    let admitted = state.begin(&create("admitted")).unwrap();
    assert_eq!(state.control_records["idle-pending"].turn, Some(admitted));
    state.finish(admitted).unwrap();
    assert!(!state.control_records["idle-pending"].pending);
}

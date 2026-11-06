use super::*;
use serde_json::json;
fn packet(choices: Value, usage: Value) -> Vec<u8> {
    format!("data: {}\n\n",json!({"id":"s1","object":"chat.completion.chunk","model":"m","choices":choices,"usage":usage})).into_bytes()
}
fn finish() -> Vec<u8> {
    packet(
        json!([{"index":0,"delta":{},"finish_reason":"stop"}]),
        Value::Null,
    )
}
fn usage() -> Vec<u8> {
    packet(
        json!([]),
        json!({"prompt_tokens":4,"completion_tokens":6,"total_tokens":10,
        "prompt_tokens_details":{"text_tokens":1,"audio_tokens":3},
        "completion_tokens_details":{"text_tokens":2,"audio_tokens":4}}),
    )
}
#[test]
fn audio_stream_requires_finish_usage_and_done_in_order() {
    let mut state = TerminalState::default();
    assert!(
        state
            .observe(b"data: [DONE]\n\n", "m", AudioUsageFormat::OpenAiChatV1)
            .is_err()
    );
    assert!(
        !state
            .observe(&finish(), "m", AudioUsageFormat::OpenAiChatV1)
            .unwrap()
    );
    assert!(
        state
            .observe(b"data: [DONE]\n\n", "m", AudioUsageFormat::OpenAiChatV1)
            .is_err()
    );
    assert!(
        !state
            .observe(&usage(), "m", AudioUsageFormat::OpenAiChatV1)
            .unwrap()
    );
    assert!(
        state
            .observe(&usage(), "m", AudioUsageFormat::OpenAiChatV1)
            .is_err()
    );
    assert!(
        state
            .observe(b"data: [DONE]\n\n", "m", AudioUsageFormat::OpenAiChatV1)
            .unwrap()
    );
    assert!(
        state
            .observe(b": late\n\n", "m", AudioUsageFormat::OpenAiChatV1)
            .is_err()
    );
}
#[test]
fn audio_stream_rejects_missing_counters_and_changed_identity() {
    let mut state = TerminalState::default();
    state
        .observe(&finish(), "m", AudioUsageFormat::OpenAiChatV1)
        .unwrap();
    let missing = String::from_utf8(usage())
        .unwrap()
        .replace("\"audio_tokens\":4,", "");
    assert!(
        state
            .observe(missing.as_bytes(), "m", AudioUsageFormat::OpenAiChatV1)
            .is_err()
    );
    let changed = String::from_utf8(usage())
        .unwrap()
        .replace("\"id\":\"s1\"", "\"id\":\"s2\"");
    assert!(
        state
            .observe(changed.as_bytes(), "m", AudioUsageFormat::OpenAiChatV1)
            .is_err()
    );
}
#[test]
fn audio_stream_frame_boundaries_wait_for_split_crlf_and_utf8() {
    let bytes = "data: {\"text\":\"音\"}\r\n\r\n".as_bytes();
    for end in 0..bytes.len() {
        assert_eq!(frame_end(&bytes[..end]), None);
    }
    assert_eq!(frame_end(bytes), Some(bytes.len()));
}

#[test]
fn audio_stream_replay_requires_usage_packet_and_rejects_tail() {
    let finish = finish();
    let usage = usage();
    let done = b"data: [DONE]\n\n";
    let valid = [finish.as_slice(), usage.as_slice(), done.as_slice()];
    let raw = replay_packets(valid, "m", AudioUsageFormat::OpenAiChatV1).unwrap();
    assert_eq!(
        AudioUsageFormat::OpenAiChatV1
            .parse(raw.as_bytes())
            .unwrap()
            .audio_output,
        4
    );
    assert!(replay_packets([done.as_slice()], "m", AudioUsageFormat::OpenAiChatV1).is_err());
    assert!(
        replay_packets(
            [finish.as_slice(), done.as_slice()],
            "m",
            AudioUsageFormat::OpenAiChatV1
        )
        .is_err()
    );
    assert!(
        replay_packets(
            valid.into_iter().chain([usage.as_slice()]),
            "m",
            AudioUsageFormat::OpenAiChatV1
        )
        .is_err()
    );
    assert!(
        replay_packets(
            valid.into_iter().chain([done.as_slice()]),
            "m",
            AudioUsageFormat::OpenAiChatV1
        )
        .is_err()
    );
    assert!(
        replay_packets(
            [finish.as_slice(), usage.as_slice(), b"data: [DONE]\n"],
            "m",
            AudioUsageFormat::OpenAiChatV1
        )
        .is_err()
    );
}

#[test]
fn audio_stream_journal_retains_only_strict_usage_packet() {
    let body = packet(
        json!([{"index":0,"delta":{"content":"PRIVATE_TEXT","audio":{"data":"PRIVATE_AUDIO"}},"finish_reason":null}]),
        Value::Null,
    );
    let (state, retained) = journal_step(
        &JournalState::default(),
        &body,
        "m",
        AudioUsageFormat::OpenAiChatV1,
    )
    .unwrap();
    assert!(retained.is_none());
    let encoded = serde_json::to_string(&state).unwrap();
    assert!(!encoded.contains("PRIVATE"));
    assert!(!encoded.contains("s1"));
    let (state, retained) =
        journal_step(&state, &finish(), "m", AudioUsageFormat::OpenAiChatV1).unwrap();
    assert!(retained.is_none());
    let clean = usage();
    let (next, retained) =
        journal_step(&state, &clean, "m", AudioUsageFormat::OpenAiChatV1).unwrap();
    assert_eq!(
        retained.as_deref(),
        Some(
            journal_usage(
                std::str::from_utf8(&clean).unwrap(),
                "m",
                AudioUsageFormat::OpenAiChatV1
            )
            .unwrap()
            .as_str()
        )
    );
    assert!(next.has_usage);
    let clean = String::from_utf8(clean).unwrap();
    for bad in [
        clean.replacen("data: {", "data: {\"delta\":\"PRIVATE_BODY\",", 1),
        clean.replacen(
            "\"prompt_tokens\":4",
            "\"prompt_tokens\":\"PRIVATE_HIDDEN_DUPLICATE\",\"prompt_tokens\":4",
            1,
        ),
        clean.replacen("\"usage\":{", "\"usage\":{\"body\":\"PRIVATE_BODY\",", 1),
    ] {
        assert!(
            journal_step(&state, bad.as_bytes(), "m", AudioUsageFormat::OpenAiChatV2).is_err(),
            "accepted body-bearing usage packet"
        );
    }
}

#[test]
fn audio_stream_v2_accepts_default_official_metadata_without_retention() {
    let (state, _) = journal_step(
        &JournalState::default(),
        &finish(),
        "m",
        AudioUsageFormat::OpenAiChatV2,
    )
    .unwrap();
    let packet = String::from_utf8(usage())
        .unwrap()
        .replace("\"text_tokens\":1,", "")
        .replace(",\"text_tokens\":1", "")
        .replace("\"text_tokens\":2,", "")
        .replace(",\"text_tokens\":2", "")
        .replacen(
            "data: {",
            "data: {\"service_tier\":\"default\",\"obfuscation\":\"EPHEMERAL_PADDING\",",
            1,
        );
    let (_, raw) = journal_step(
        &state,
        packet.as_bytes(),
        "m",
        AudioUsageFormat::OpenAiChatV2,
    )
    .unwrap();
    let raw = raw.unwrap();
    assert!(!raw.contains("EPHEMERAL"));
    assert!(!raw.contains("service_tier"));
    assert!(!raw.contains("choices"));
    assert_eq!(
        AudioUsageFormat::OpenAiChatV2
            .parse(raw.as_bytes())
            .unwrap()
            .text_input,
        1
    );
    assert!(
        journal_step(
            &state,
            packet.as_bytes(),
            "m",
            AudioUsageFormat::OpenAiChatV1
        )
        .is_err()
    );
}

#[test]
fn audio_stream_live_and_replay_preserve_frozen_journal_states() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/chat-journal-v3.json")).unwrap();
    let packets = fixture["frames"].as_array().unwrap();
    let model = "synthetic-audio-upstream";
    for format in [
        AudioUsageFormat::OpenAiChatV1,
        AudioUsageFormat::OpenAiChatV2,
    ] {
        let mut live = TerminalState::default();
        let mut replay = JournalState::default();
        let mut retained = None;
        for (index, packet) in packets.iter().enumerate() {
            let packet = packet.as_str().unwrap().as_bytes();
            let done = live.observe(packet, model, format).unwrap();
            let (next, usage) = journal_step(&replay, packet, model, format).unwrap();
            assert_eq!(
                serde_json::to_value(&next).unwrap(),
                fixture["rows"][index]["state"]
            );
            assert_eq!(done, next.done);
            assert_eq!(live.has_usage(), next.has_usage);
            replay = serde_json::from_slice(&serde_json::to_vec(&next).unwrap()).unwrap();
            if let Some(usage) = usage {
                assert!(retained.replace(usage).is_none());
            }
        }
        assert!(live.is_done());
        assert_eq!(retained.as_deref(), fixture["raw_usage"].as_str());
        let usage_packet = packets[1].as_str().unwrap();
        assert_eq!(
            live.into_usage_packet().as_deref(),
            Some(usage_packet.as_bytes())
        );
        assert_eq!(
            journal_usage(usage_packet, model, format).unwrap(),
            fixture["raw_usage"].as_str().unwrap()
        );
    }
}

//! Bounded synthetic tool response used only when a test explicitly enables it.
use axum::{
    body::{Body, Bytes},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::convert::Infallible;

pub(super) fn stream(request: &Value, replacement: &str) -> Response {
    let tool = request["tools"].as_array().and_then(|tools| {
        tools.iter().find(|tool| {
            let function = &tool["function"];
            tool["type"] == "function"
                && function["name"] == "rewrite_section"
                && function["parameters"]["type"] == "object"
                && function["parameters"]["properties"]["replacement_text"]["type"] == "string"
        })
    });
    let Some(tool) = tool else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "A declared rewrite tool is required",
        )
            .into_response();
    };
    if request["stream"] != true {
        return (
            StatusCode::BAD_REQUEST,
            "A streamed completion is required",
        )
            .into_response();
    }
    let arguments = json!({"replacement_text":replacement}).to_string();
    let split = arguments.find(':').unwrap() + 1;
    let tool_name = &tool["function"]["name"];
    let first = json!({"role":"assistant","tool_calls":[{
        "index":0,"id":"fixture-call","type":"function",
        "function":{"name":tool_name,"arguments":&arguments[..split]}
    }]});
    let last = json!({"tool_calls":[{
        "index":0,"function":{"arguments":&arguments[split..]}
    }]});
    let chunks = [
        json!({"choices":[{"index":0,"delta":first,"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":last,"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30,
            "prompt_tokens_details":{"cached_tokens":2}}}),
    ];
    let frames = chunks
        .into_iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .chain(std::iter::once("data: [DONE]\n\n".to_owned()))
        .map(|frame| Ok::<_, Infallible>(Bytes::from(frame)));
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(tokio_stream::iter(frames)),
    )
        .into_response()
}

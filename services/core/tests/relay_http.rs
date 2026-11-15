//! Real TCP upstream and downstream tests; no external API keys or paid requests.
use axum::{
    Router,
    body::{Body, Bytes},
    http::Response,
    routing::post,
};
use lmm_core::relay::*;
use serde_json::{Value, json};
use std::{
    io,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Semaphore, mpsc},
    task::JoinHandle,
    time::{sleep, timeout},
};

const WAIT: Duration = Duration::from_secs(8);
static IDS: AtomicUsize = AtomicUsize::new(1);
fn context() -> RequestContext {
    RequestContext {
        request_id: format!("test_{}", IDS.fetch_add(1, Ordering::Relaxed)),
        actor_user_id: 7,
        key_id: Some(9),
        oauth_source: None,
    }
}

#[derive(Clone)]
enum Step {
    Data(Vec<u8>),
    Sleep(Duration),
    Wait(Arc<Semaphore>),
}
#[derive(Clone)]
struct Script {
    status: u16,
    content_type: &'static str,
    header_delay: Duration,
    steps: Vec<Step>,
    complete: bool,
}
impl Script {
    fn sse(data: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
            header_delay: Duration::ZERO,
            steps: vec![Step::Data(data)],
            complete: true,
        }
    }
    fn json(v: Value) -> Self {
        Self {
            content_type: "application/json",
            ..Self::sse(serde_json::to_vec(&v).unwrap())
        }
    }
}
#[derive(Default)]
struct Observed {
    hits: AtomicUsize,
    chunks: AtomicUsize,
    disconnected: AtomicBool,
    requests: Mutex<Vec<(String, Value)>>,
}
struct Upstream {
    address: String,
    observed: Arc<Observed>,
    task: JoinHandle<()>,
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn hold(socket: &mut TcpStream, step: &Step) -> bool {
    let mut byte = [0u8; 1];
    match step {
        Step::Sleep(d) => {
            tokio::select! { _ = sleep(*d) => true, _ = socket.read(&mut byte) => false }
        }
        Step::Wait(s) => {
            tokio::select! { permit = s.acquire() => { permit.unwrap().forget(); true }, _ = socket.read(&mut byte) => false }
        }
        Step::Data(_) => true,
    }
}
async fn read_request(socket: &mut TcpStream) -> io::Result<(String, Value)> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    let end = loop {
        if let Some(i) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break i + 4;
        }
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&buf[..n]);
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(io::ErrorKind::InvalidData.into());
        }
    };
    let headers =
        String::from_utf8(bytes[..end].to_vec()).map_err(|_| io::ErrorKind::InvalidData)?;
    let len = headers
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    if len > 2 * 1024 * 1024 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    while bytes.len() < end + len {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    let v =
        serde_json::from_slice(&bytes[end..end + len]).map_err(|_| io::ErrorKind::InvalidData)?;
    Ok((headers, v))
}
impl Upstream {
    async fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let observed = Arc::new(Observed::default());
        let state = observed.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let Ok(Ok(request)) = timeout(WAIT, read_request(&mut socket)).await else {
                    continue;
                };
                state.hits.fetch_add(1, Ordering::SeqCst);
                state.requests.lock().unwrap().push(request);
                if !hold(&mut socket, &Step::Sleep(script.header_delay)).await {
                    state.disconnected.store(true, Ordering::SeqCst);
                    continue;
                }
                let headers = format!(
                    "HTTP/1.1 {} Test\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                    script.status, script.content_type
                );
                if socket.write_all(headers.as_bytes()).await.is_err() {
                    state.disconnected.store(true, Ordering::SeqCst);
                    continue;
                }
                for step in &script.steps {
                    if let Step::Data(data) = step {
                        if data.is_empty() {
                            continue;
                        }
                        let mut chunk = format!("{:x}\r\n", data.len()).into_bytes();
                        chunk.extend(data);
                        chunk.extend(b"\r\n");
                        if socket.write_all(&chunk).await.is_err() {
                            state.disconnected.store(true, Ordering::SeqCst);
                            break;
                        }
                        state.chunks.fetch_add(1, Ordering::SeqCst);
                    } else if !hold(&mut socket, step).await {
                        state.disconnected.store(true, Ordering::SeqCst);
                        break;
                    }
                }
                if script.complete {
                    let _ = socket.write_all(b"0\r\n\r\n").await;
                }
                let _ = socket.shutdown().await;
            }
        });
        Self {
            address,
            observed,
            task,
        }
    }
}
struct Routes {
    address: String,
    protocol: UpstreamProtocol,
}
impl RouteProvider for Routes {
    fn select<'a>(&'a self, _: &'a RequestContext, r: &'a Request) -> RelayFuture<'a, Route> {
        Box::pin(async move {
            let path = match self.protocol {
                UpstreamProtocol::Chat => "/v1/chat/completions",
                UpstreamProtocol::Responses => "/v1/responses",
                UpstreamProtocol::Anthropic => "/v1/messages",
                UpstreamProtocol::Gemini => {
                    if r.stream {
                        "/v1beta/models/upstream-real:streamGenerateContent?alt=sse"
                    } else {
                        "/v1beta/models/upstream-real:generateContent"
                    }
                }
            };
            Route::new(
                "route_test".into(),
                self.protocol,
                &format!("{}{path}", self.address),
                "upstream-real".into(),
                "upstream-test-secret",
                42,
            )
        })
    }
}
#[derive(Default)]
struct Money {
    reserves: AtomicUsize,
    finals: Mutex<Vec<Report>>,
    pinned: Mutex<Vec<(String, u64)>>,
    reject: bool,
    fail_finalize: bool,
    reserve_wait: Option<Arc<Semaphore>>,
    finalize_wait: Option<Arc<Semaphore>>,
    checkpoints: Mutex<Vec<OutputCheckpoint>>,
    fail_checkpoint: bool,
}
impl Billing for Money {
    fn reserve<'a>(
        &'a self,
        c: &'a RequestContext,
        r: &'a Route,
        _: &'a Request,
    ) -> RelayFuture<'a, Reservation> {
        Box::pin(async move {
            self.reserves.fetch_add(1, Ordering::SeqCst);
            assert_eq!((c.actor_user_id, c.key_id), (7, Some(9)));
            self.pinned
                .lock()
                .unwrap()
                .push((c.request_id.clone(), r.price_version));
            if let Some(gate) = &self.reserve_wait {
                gate.acquire().await.unwrap().forget();
            }
            if self.reject {
                return Err(RelayError::BillingUnavailable);
            }
            Ok(Reservation {
                id: format!("reservation_{}", c.request_id),
            })
        })
    }
    // Explicit protocol-test adapter only. model_http tests below the real
    // gateway separately require PostgreSQL checkpoints and ledger settlement.
    fn checkpoint<'a>(
        &'a self,
        _: &'a Reservation,
        checkpoint: &'a OutputCheckpoint,
    ) -> RelayFuture<'a, ()> {
        Box::pin(async move {
            self.checkpoints.lock().unwrap().push(checkpoint.clone());
            if self.fail_checkpoint {
                Err(RelayError::SettlementPending)
            } else {
                Ok(())
            }
        })
    }
    fn finalize<'a>(&'a self, r: &'a Reservation, report: &'a Report) -> RelayFuture<'a, ()> {
        Box::pin(async move {
            assert_eq!(r.id, format!("reservation_{}", report.request_id));
            self.finals.lock().unwrap().push(report.clone());
            if let Some(gate) = &self.finalize_wait {
                gate.acquire().await.unwrap().forget();
            }
            if self.fail_finalize {
                Err(RelayError::SettlementPending)
            } else {
                Ok(())
            }
        })
    }
}
fn relay(u: &Upstream, p: UpstreamProtocol, l: Limits, m: Arc<Money>) -> Relay {
    Relay::new(
        Arc::new(Routes {
            address: u.address.clone(),
            protocol: p,
        }),
        m,
        l,
    )
    .unwrap()
}
fn input(client: ClientProtocol, stream: bool) -> Value {
    let function = json!({"name":"weather","parameters":{"type":"object","properties":{"city":{"type":"string"}}}});
    match client {
        ClientProtocol::Chat => {
            json!({"model":"public-model","messages":[{"role":"user","content":"hello"}],"stream":stream,
            "stream_options":{"include_usage":true},"tools":[{"type":"function","function":function}]})
        }
        ClientProtocol::Responses => {
            let mut tool = function;
            tool["type"] = json!("function");
            json!({"model":"public-model","input":"hello","stream":stream,"tools":[tool]})
        }

        ClientProtocol::Anthropic => json!({
            "model":"public-model", "max_tokens":128,
            "messages":[{"role":"user","content":"hello"}], "stream":stream,
            "tools":[{"name":"weather","input_schema":function["parameters"]}]
        }),
        ClientProtocol::Gemini => unreachable!("Gemini requires its path model and operation"),
    }
}
fn request(client: ClientProtocol, stream: bool, l: &Limits) -> Request {
    Request::parse(
        client,
        &serde_json::to_vec(&input(client, stream)).unwrap(),
        l,
    )
    .unwrap()
}
fn data(v: Value) -> Vec<u8> {
    format!("data: {v}\r\n\r\n").into_bytes()
}
fn event(v: Value) -> Vec<u8> {
    format!(
        "event: {}\r\ndata: {v}\r\n\r\n",
        v["type"].as_str().unwrap()
    )
    .into_bytes()
}
fn cat(parts: Vec<Vec<u8>>) -> Vec<u8> {
    parts.into_iter().flatten().collect()
}
fn openai_usage(chat: bool) -> Value {
    if chat {
        json!({"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_tokens_details":{"cached_tokens":3},"completion_tokens_details":{"reasoning_tokens":2}})
    } else {
        json!({"input_tokens":10,"output_tokens":5,"total_tokens":15,"input_tokens_details":{"cached_tokens":3},"output_tokens_details":{"reasoning_tokens":2}})
    }
}
fn full(p: UpstreamProtocol) -> Value {
    let args = json!({"city":"上海"});
    match p {
        UpstreamProtocol::Chat => {
            json!({"id":"up_1","choices":[{"index":0,"message":{"role":"assistant","content":"鲸鱼","reasoning_content":"think",
            "tool_calls":[{"id":"call_1","type":"function","function":{"name":"weather","arguments":args.to_string()}}]},"finish_reason":"tool_calls"}],"usage":openai_usage(true)})
        }
        UpstreamProtocol::Responses => json!({"id":"up_1","status":"completed","output":[
            {"id":"rs_1","type":"reasoning","summary":[{"type":"summary_text","text":"think"}]},
            {"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"鲸鱼","annotations":[]}]},
            {"id":"fc_1","type":"function_call","call_id":"call_1","name":"weather","arguments":args.to_string()}],"usage":openai_usage(false)}),
        UpstreamProtocol::Anthropic => {
            json!({"id":"up_1","type":"message","role":"assistant","model":"upstream-real","content":[{"type":"thinking","thinking":"think","signature":"signed_thinking"},
            {"type":"text","text":"鲸鱼"},{"type":"tool_use","id":"call_1","name":"weather","input":args}],"stop_reason":"tool_use",
            "usage":{"input_tokens":7,"cache_read_input_tokens":3,"output_tokens":5}})
        }
        UpstreamProtocol::Gemini => {
            json!({"responseId":"up_1","candidates":[{"index":0,"content":{"role":"model","parts":[
            {"text":"think","thought":true,"thoughtSignature":"gemini_think"},{"text":"鲸鱼"},
            {"functionCall":{"id":"call_1","name":"weather","args":args},"thoughtSignature":"gemini_call"}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":3,"thoughtsTokenCount":2,"cachedContentTokenCount":3}})
        }
    }
}
fn streaming(p: UpstreamProtocol) -> Vec<u8> {
    match p {
        UpstreamProtocol::Chat => cat(vec![
            data(
                json!({"id":"up_1","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}),
            ),
            data(
                json!({"id":"up_1","choices":[{"index":0,"delta":{"reasoning_content":"think"}}]}),
            ),
            data(json!({"id":"up_1","choices":[{"index":0,"delta":{"content":"鲸"}}]})),
            data(json!({"id":"up_1","choices":[{"index":0,"delta":{"content":"鱼"}}]})),
            data(
                json!({"id":"up_1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"weather","arguments":""}}]}}]}),
            ),
            data(
                json!({"id":"up_1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":\""}}]}}]}),
            ),
            data(
                json!({"id":"up_1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"上海\"}"}}]}}]}),
            ),
            data(
                json!({"id":"up_1","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
            ),
            data(json!({"id":"up_1","choices":[],"usage":openai_usage(true)})),
            b"data: [DONE]\n\n".to_vec(),
        ]),
        UpstreamProtocol::Responses => cat(vec![
            event(
                json!({"type":"response.created","response":{"id":"up_1","status":"in_progress"}}),
            ),
            event(
                json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1","summary":[]}}),
            ),
            event(
                json!({"type":"response.reasoning_summary_part.added","item_id":"rs_1","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":""}}),
            ),
            event(
                json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","output_index":0,"summary_index":0,"delta":"think"}),
            ),
            event(
                json!({"type":"response.reasoning_summary_part.done","item_id":"rs_1","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":"think"}}),
            ),
            event(
                json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"rs_1","status":"completed","summary":[{"type":"summary_text","text":"think"}]}}),
            ),
            event(
                json!({"type":"response.output_item.added","output_index":1,"item":{"type":"message","id":"msg_1","role":"assistant","content":[]}}),
            ),
            event(
                json!({"type":"response.content_part.added","item_id":"msg_1","output_index":1,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
            ),
            event(
                json!({"type":"response.output_text.delta","item_id":"msg_1","output_index":1,"content_index":0,"delta":"鲸鱼"}),
            ),
            event(
                json!({"type":"response.content_part.done","item_id":"msg_1","output_index":1,"content_index":0,"part":{"type":"output_text","text":"鲸鱼","annotations":[]}}),
            ),
            event(
                json!({"type":"response.output_item.done","output_index":1,"item":{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":"鲸鱼","annotations":[]}]}}),
            ),
            event(
                json!({"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"weather","arguments":""}}),
            ),
            event(
                json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":2,"delta":"{\"city\":\""}),
            ),
            event(
                json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":2,"delta":"上海\"}"}),
            ),
            event(
                json!({"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":2,"arguments":"{\"city\":\"上海\"}"}),
            ),
            event(
                json!({"type":"response.output_item.done","output_index":2,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"weather","arguments":"{\"city\":\"上海\"}","status":"completed"}}),
            ),
            event(
                json!({"type":"response.completed","response":full(UpstreamProtocol::Responses)}),
            ),
        ]),
        UpstreamProtocol::Anthropic => cat(vec![
            event(
                json!({"type":"message_start","message":{"id":"up_1","usage":{"input_tokens":7,"cache_read_input_tokens":3,"output_tokens":0}}}),
            ),
            event(
                json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            ),
            event(
                json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"think"}}),
            ),
            event(
                json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed_"}}),
            ),
            event(
                json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"thinking"}}),
            ),
            event(json!({"type":"content_block_stop","index":0})),
            event(
                json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            ),
            event(
                json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"鲸鱼"}}),
            ),
            event(json!({"type":"content_block_stop","index":1})),
            event(
                json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"call_1","name":"weather","input":{}}}),
            ),
            event(
                json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"city\":\""}}),
            ),
            event(
                json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"上海\"}"}}),
            ),
            event(json!({"type":"content_block_stop","index":2})),
            event(
                json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}),
            ),
            event(json!({"type":"message_stop"})),
        ]),
        UpstreamProtocol::Gemini => cat(vec![
            data(
                json!({"responseId":"up_1","candidates":[{"index":0,"content":{"parts":[{"text":"think","thought":true,"thoughtSignature":"gemini_think"}]}}]}),
            ),
            data(
                json!({"responseId":"up_1","candidates":[{"index":0,"content":{"parts":[{"text":"鲸鱼"}]}}]}),
            ),
            data(
                json!({"responseId":"up_1","candidates":[{"index":0,"content":{"parts":[{"functionCall":{"id":"call_1","name":"weather","args":{"city":"上海"}},"thoughtSignature":"gemini_call"}]}}]}),
            ),
            data(
                json!({"responseId":"up_1","candidates":[{"index":0,"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":3,"thoughtsTokenCount":2,"cachedContentTokenCount":3}}),
            ),
        ]),
    }
}
async fn collect(s: Session) -> (Result<ResponseHead, RelayError>, String, Completion) {
    let Session {
        head,
        mut body,
        completion,
        ..
    } = s;
    let head = timeout(WAIT, head).await.unwrap().unwrap();
    let mut data = Vec::new();
    while let Some(chunk) = timeout(WAIT, body.next_chunk()).await.unwrap() {
        data.extend(chunk.unwrap());
    }
    let done = timeout(WAIT, completion).await.unwrap().unwrap();
    (head, String::from_utf8(data).unwrap(), done)
}
fn packets(s: &str) -> Vec<Value> {
    s.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter(|s| *s != "[DONE]")
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn assert_output(client: ClientProtocol, stream: bool, body: &str) {
    if client == ClientProtocol::Anthropic {
        let response = if stream {
            let events = packets(body);
            assert_eq!(events.last().unwrap()["type"], "message_stop");
            let mut text = String::new();
            let mut thinking = String::new();
            let mut arguments = String::new();
            for event in &events {
                let delta = &event["delta"];
                if let Some(value) = delta["text"].as_str() {
                    text.push_str(value);
                }
                if let Some(value) = delta["thinking"].as_str() {
                    thinking.push_str(value);
                }
                if let Some(value) = delta["partial_json"].as_str() {
                    arguments.push_str(value);
                }
            }
            assert_eq!(text, "鲸鱼");
            assert_eq!(thinking, "think");
            assert_eq!(
                serde_json::from_str::<Value>(&arguments).unwrap(),
                json!({"city":"上海"})
            );
            let terminal = events
                .iter()
                .find(|e| e["type"] == "message_delta")
                .unwrap();
            assert_eq!(terminal["delta"]["stop_reason"], "tool_use");
            assert_eq!(terminal["usage"]["output_tokens"], 5);
            assert_eq!(terminal["usage"]["input_tokens"], 7);
            assert_eq!(terminal["usage"]["cache_read_input_tokens"], 3);
            assert!(
                terminal["usage"]
                    .get("cache_creation_input_tokens")
                    .is_none()
            );
            events
                .iter()
                .find(|e| e["type"] == "message_start")
                .unwrap()["message"]
                .clone()
        } else {
            let response: Value = serde_json::from_str(body).unwrap();
            assert_eq!(response["stop_reason"], "tool_use");
            let content = response["content"].as_array().unwrap();
            assert!(content.iter().any(|b| b["text"] == "鲸鱼"));
            assert!(content.iter().any(|b| b["thinking"] == "think"));
            let tool = content.iter().find(|b| b["type"] == "tool_use").unwrap();
            assert_eq!(tool["id"], "call_1");
            assert_eq!(tool["name"], "weather");
            assert_eq!(tool["input"], json!({"city":"上海"}));
            assert_eq!(response["usage"]["output_tokens"], 5);
            assert_eq!(response["usage"]["input_tokens"], 7);
            assert_eq!(response["usage"]["cache_read_input_tokens"], 3);
            assert!(
                response["usage"]
                    .get("cache_creation_input_tokens")
                    .is_none()
            );
            response
        };
        assert_eq!(response["model"], "public-model");
        assert!(!body.contains("upstream-test-secret"));
        assert!(!body.contains("upstream-real"));
        return;
    }
    if stream {
        let packets = packets(body);
        let mut text = String::new();
        let mut thinking = String::new();
        let mut args = String::new();
        match client {
            ClientProtocol::Chat => {
                assert!(body.ends_with("data: [DONE]\n\n"));
                for p in &packets {
                    if let Some(c) = p["choices"].as_array().and_then(|c| c.first()) {
                        if let Some(t) = c["delta"]["content"].as_str() {
                            text.push_str(t);
                        }
                        if let Some(t) = c["delta"]["reasoning_content"].as_str() {
                            thinking.push_str(t);
                        }
                        if let Some(calls) = c["delta"]["tool_calls"].as_array() {
                            for c in calls {
                                if let Some(t) = c["function"]["arguments"].as_str() {
                                    args.push_str(t);
                                }
                            }
                        }
                    }
                }
                let usage = packets.last().unwrap();
                assert_eq!(usage["usage"]["prompt_tokens"], 10);
                assert_eq!(usage["usage"]["completion_tokens"], 5);
                assert_eq!(
                    packets[packets.len() - 2]["choices"][0]["finish_reason"],
                    "tool_calls"
                );
            }
            ClientProtocol::Responses => {
                for (i, p) in packets.iter().enumerate() {
                    assert_eq!(p["sequence_number"].as_u64(), Some(i as u64));
                    match p["type"].as_str() {
                        Some("response.output_text.delta") => {
                            text.push_str(p["delta"].as_str().unwrap())
                        }
                        Some("response.reasoning_summary_text.delta") => {
                            thinking.push_str(p["delta"].as_str().unwrap())
                        }
                        Some("response.function_call_arguments.delta") => {
                            args.push_str(p["delta"].as_str().unwrap())
                        }
                        _ => {}
                    }
                }
                let last = packets.last().unwrap();
                assert_eq!(last["type"], "response.completed");
                assert_eq!(last["response"]["usage"]["input_tokens"], 10);
                assert_eq!(last["response"]["usage"]["output_tokens"], 5);
                assert_eq!(last["response"]["output"][2]["call_id"], "call_1");
                assert_ne!(last["response"]["output"][2]["id"], "call_1");
            }

            ClientProtocol::Anthropic | ClientProtocol::Gemini => {
                unreachable!("native clients require native fixtures")
            }
        }
        assert_eq!(text, "鲸鱼");
        assert_eq!(thinking, "think");
        assert_eq!(
            serde_json::from_str::<Value>(&args).unwrap(),
            json!({"city":"上海"})
        );
    } else {
        let v: Value = serde_json::from_str(body).unwrap();
        let args = match client {
            ClientProtocol::Chat => {
                let message = &v["choices"][0]["message"];
                assert_eq!(message["content"], "鲸鱼");
                assert_eq!(message["reasoning_content"], "think");
                assert_eq!(message["tool_calls"][0]["id"], "call_1");
                assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
                assert_eq!(v["usage"]["prompt_tokens"], 10);
                assert_eq!(v["usage"]["completion_tokens"], 5);
                message["tool_calls"][0]["function"]["arguments"]
                    .as_str()
                    .unwrap()
            }
            ClientProtocol::Responses => {
                assert_eq!(v["status"], "completed");
                assert_eq!(v["output"][0]["summary"][0]["text"], "think");
                assert_eq!(v["output"][1]["content"][0]["text"], "鲸鱼");
                assert_eq!(v["output"][2]["call_id"], "call_1");
                assert_eq!(v["usage"]["input_tokens"], 10);
                assert_eq!(v["usage"]["output_tokens"], 5);
                v["output"][2]["arguments"].as_str().unwrap()
            }

            ClientProtocol::Anthropic | ClientProtocol::Gemini => {
                unreachable!("native clients require native fixtures")
            }
        };
        assert_eq!(
            serde_json::from_str::<Value>(args).unwrap(),
            json!({"city":"上海"})
        );
    }
    assert!(!body.contains("upstream-test-secret"));
    assert!(!body.contains("upstream-real"));
}

#[tokio::test]
async fn vision_inputs_reach_four_upstreams_and_preserve_json_sse_usage() {
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
    let url = format!("data:image/png;base64,{png}");
    for protocol in [
        UpstreamProtocol::Chat,
        UpstreamProtocol::Responses,
        UpstreamProtocol::Anthropic,
        UpstreamProtocol::Gemini,
    ] {
        for client in [ClientProtocol::Chat, ClientProtocol::Responses] {
            for stream in [false, true] {
                let upstream = Upstream::start(if stream {
                    Script::sse(streaming(protocol))
                } else {
                    Script::json(full(protocol))
                })
                .await;
                let money = Arc::new(Money::default());
                let limits = Limits::default();
                let relay = relay(&upstream, protocol, limits.clone(), money.clone());
                let mut body = input(client, stream);
                match client {
                    ClientProtocol::Chat => {
                        body["messages"][0]["content"] = json!([
                            {"type":"text", "text":"describe"}, {"type":"image_url", "image_url":{"url":url}}
                        ])
                    }
                    ClientProtocol::Responses => {
                        body["input"] = json!([{"role":"user", "content":[
                            {"type":"input_text", "text":"describe"}, {"type":"input_image", "image_url":url}
                        ]}])
                    }
                    _ => unreachable!(),
                }
                let request =
                    Request::parse(client, &serde_json::to_vec(&body).unwrap(), &limits).unwrap();
                let (head, output, done) = collect(relay.start(request, context()).unwrap()).await;
                assert!(head.is_ok(), "{protocol:?} {client:?} {stream}: {head:?}");
                assert!(
                    done.error.is_none(),
                    "{protocol:?} {client:?} {stream}: {:?}",
                    done.error
                );
                assert_output(client, stream, &output);
                assert_eq!(done.report.usage.input_tokens, Some(10));
                assert_eq!(done.report.usage.output_tokens, Some(5));
                assert_eq!(done.settlement, Settlement::Confirmed);
                assert_eq!(money.reserves.load(Ordering::SeqCst), 1);
                assert_eq!(money.finals.lock().unwrap().len(), 1);
                let requests = upstream.observed.requests.lock().unwrap();
                assert_eq!(requests.len(), 1);
                let payload = &requests[0].1;
                match protocol {
                    UpstreamProtocol::Chat => {
                        assert_eq!(
                            payload["messages"][0]["content"][1]["image_url"]["url"],
                            url
                        );
                        assert_eq!(
                            payload["messages"][0]["content"][1]["image_url"]["detail"],
                            "auto"
                        );
                    }
                    UpstreamProtocol::Responses => {
                        assert_eq!(payload["input"][0]["content"][1]["image_url"], url);
                        assert_eq!(payload["input"][0]["content"][1]["detail"], "auto");
                    }
                    UpstreamProtocol::Anthropic => {
                        assert_eq!(payload["messages"][0]["content"][1]["source"]["data"], png)
                    }
                    UpstreamProtocol::Gemini => assert_eq!(
                        payload["contents"][0]["parts"][1]["inlineData"]["data"],
                        png
                    ),
                }
            }
        }
    }
}

#[tokio::test]
async fn twenty_four_protocol_json_sse_cases_use_real_tcp() {
    for p in [
        UpstreamProtocol::Chat,
        UpstreamProtocol::Responses,
        UpstreamProtocol::Anthropic,
        UpstreamProtocol::Gemini,
    ] {
        for client in [
            ClientProtocol::Chat,
            ClientProtocol::Responses,
            ClientProtocol::Anthropic,
        ] {
            for stream in [false, true] {
                let script = if stream {
                    let mut s = Script::sse(Vec::new());
                    s.steps = streaming(p)
                        .chunks(3)
                        .map(|b| Step::Data(b.to_vec()))
                        .collect();
                    s
                } else {
                    Script::json(full(p))
                };
                let upstream = Upstream::start(script).await;
                let money = Arc::new(Money::default());
                let limits = Limits::default();
                let relay = relay(&upstream, p, limits.clone(), money.clone());
                let id = context();
                let expected_id = id.request_id.clone();
                let (head, body, done) =
                    collect(relay.start(request(client, stream, &limits), id).unwrap()).await;
                // Native thinking requires an Anthropic signature. Other providers'
                // reasoning cannot be silently relabeled as signed Anthropic content.
                if client == ClientProtocol::Anthropic && p != UpstreamProtocol::Anthropic {
                    assert!(
                        matches!(
                            done.error,
                            Some(RelayError::Unsupported(_) | RelayError::Protocol(_))
                        ),
                        "{p:?} {client:?} {stream}: {:?}",
                        done.error
                    );
                    assert!(!body.contains("\"type\":\"message_stop\""));
                    assert_eq!(money.finals.lock().unwrap().len(), 1);
                    assert_eq!(upstream.observed.hits.load(Ordering::SeqCst), 1);
                    println!(
                        "unsupported_reasoning provider={p:?} client={client:?} stream={stream} rejected"
                    );
                    continue;
                }
                assert!(head.is_ok(), "{p:?} {client:?} {stream}: {head:?}");
                assert!(
                    done.error.is_none(),
                    "{p:?} {client:?} {stream}: {:?} {body}",
                    done.error
                );
                assert_eq!(done.settlement, Settlement::Confirmed);
                assert_eq!(done.report.outcome, Outcome::Completed);
                assert_eq!(done.report.usage.input_tokens, Some(10));
                assert_eq!(done.report.usage.output_tokens, Some(5));
                assert_eq!(done.report.finish_reason, Some(FinishReason::ToolCalls));
                assert_eq!(done.report.attempts, 1);
                assert_output(client, stream, &body);
                assert_eq!(money.finals.lock().unwrap().len(), 1);
                if !stream {
                    let proof = money.checkpoints.lock().unwrap();
                    assert_eq!(proof.len(), 1);
                    assert_eq!(proof[0].sequence, 1);
                    assert!(proof[0].terminal_observed);
                    assert_eq!(proof[0].packet_bytes, 0);
                    assert_eq!(proof[0].usage, done.report.usage);
                }
                assert_eq!(money.pinned.lock().unwrap()[0], (expected_id, 42));
                assert_eq!(upstream.observed.hits.load(Ordering::SeqCst), 1);
                let requests = upstream.observed.requests.lock().unwrap();
                let (headers, payload) = &requests[0];
                let auth_name = match p {
                    UpstreamProtocol::Anthropic => "x-api-key",
                    UpstreamProtocol::Gemini => "x-goog-api-key",
                    _ => "authorization",
                };
                assert!(headers.to_ascii_lowercase().contains(auth_name));
                assert!(headers.contains("upstream-test-secret"));
                match p {
                    UpstreamProtocol::Chat => {
                        assert_eq!(payload["model"], "upstream-real");
                        if stream {
                            assert_eq!(payload["stream_options"]["include_usage"], true);
                        }
                    }
                    UpstreamProtocol::Responses | UpstreamProtocol::Anthropic => {
                        assert_eq!(payload["model"], "upstream-real")
                    }
                    UpstreamProtocol::Gemini => {
                        assert!(headers.contains("models/upstream-real:"));
                        assert_eq!(payload["contents"][0]["role"], "user");
                    }
                }
                println!("round_trip provider={p:?} client={client:?} stream={stream} passed");
            }
        }
    }
}

struct ConcurrentRoutes {
    router: lmm_core::routing::Router,
}
impl ConcurrentRoutes {
    fn new(address: &str, protocol: UpstreamProtocol) -> Self {
        use lmm_core::routing::{Endpoint, PriceConfig, PriceSnapshot, RoutingConfig};
        let wire_protocol = match protocol {
            UpstreamProtocol::Chat => "openai_chat",
            UpstreamProtocol::Responses => "openai_responses",
            UpstreamProtocol::Anthropic => "anthropic_messages",
            UpstreamProtocol::Gemini => "gemini",
        };
        let mut config: RoutingConfig = serde_json::from_value(json!({
            "version":42,"groups":[{"name":"load","enabled":true}],
            "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid","protocol":wire_protocol,
                "credential":{"id":1,"revision":1},"enabled":true}],
            "routes":[{"id":1,"model":"public-model","group":"load","enabled":true,"speeds":["standard"],
                "targets":[{"id":1,"upstream_id":1,"upstream_model":"upstream-real","enabled":true,
                    "priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":30000,
                    "client_protocols":["openai_chat","openai_responses"]}],
                "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]
        })).unwrap();
        config.upstreams[0].endpoint = Endpoint::local_loopback(address.into()).unwrap();
        let prices: PriceConfig = serde_json::from_value(json!({"version":42,"rules":[{
            "id":1,"model":"public-model","group":"load","speed":"standard","billing_mode":"tokens",
            "rates":{"input":1000000,"output":2000000,"cache_read":100000,"cache_write":1000000,"request":0},
            "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000
        }]})).unwrap();
        Self {
            router: lmm_core::routing::Router::new(
                config,
                Arc::new(PriceSnapshot::compile(prices).unwrap()),
            )
            .unwrap(),
        }
    }
}
impl RouteProvider for ConcurrentRoutes {
    fn select<'a>(&'a self, _: &'a RequestContext, request: &'a Request) -> RelayFuture<'a, Route> {
        Box::pin(async move {
            use lmm_core::routing::{Protocol, ReplayPermission, RouteRequest};
            let protocol = match request.client {
                ClientProtocol::Chat => Protocol::OpenAiChat,
                ClientProtocol::Responses => Protocol::OpenAiResponses,
                _ => return Err(RelayError::RouteUnavailable),
            };
            let mut plan = self
                .router
                .route(RouteRequest {
                    model: &request.model,
                    group: "load",
                    speed: None,
                    protocol,
                    replay: ReplayPermission::Denied,
                })
                .map_err(|_| RelayError::RouteUnavailable)?;
            let attempt = plan.start().map_err(|_| RelayError::RouteUnavailable)?;
            assert!(attempt.is_available());
            assert_eq!(attempt.upstream_model(), "upstream-real");
            assert_eq!(attempt.quote().version(), 42);
            assert_eq!(attempt.credential_ref().id, 1);
            let protocol = match attempt.protocol() {
                Protocol::OpenAiChat => UpstreamProtocol::Chat,
                Protocol::OpenAiResponses => UpstreamProtocol::Responses,
                Protocol::AnthropicMessages => UpstreamProtocol::Anthropic,
                Protocol::Gemini => UpstreamProtocol::Gemini,
                _ => return Err(RelayError::RouteUnavailable),
            };
            let operation = match protocol {
                UpstreamProtocol::Chat => "v1/chat/completions".to_owned(),
                UpstreamProtocol::Responses => "v1/responses".to_owned(),
                UpstreamProtocol::Anthropic => "v1/messages".to_owned(),
                UpstreamProtocol::Gemini => format!(
                    "v1beta/models/{}:{}",
                    attempt.upstream_model(),
                    if request.stream {
                        "streamGenerateContent?alt=sse"
                    } else {
                        "generateContent"
                    }
                ),
            };
            Route::new(
                attempt.route_key(),
                protocol,
                &format!("{}/{operation}", attempt.endpoint().trim_end_matches('/')),
                attempt.upstream_model().to_owned(),
                "upstream-test-secret",
                attempt.quote().version(),
            )
        })
    }
}

// Explicit load test: real loopback HTTP and protocol conversion, test billing.
// Debug-profile timings are diagnostic and never a production throughput claim.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run explicitly for protocol concurrency and latency measurements"]
async fn concurrent_protocol_json_sse_load() {
    use std::time::Instant;
    let concurrency: usize = std::env::var("LMM_PROTOCOL_CONCURRENCY")
        .map(|v| v.parse().unwrap())
        .unwrap_or(128);
    let payload_bytes: usize = std::env::var("LMM_PROTOCOL_PAYLOAD_BYTES")
        .map(|v| v.parse().unwrap())
        .unwrap_or(0);
    assert!(payload_bytes <= 1024 * 1024);
    let count: usize = std::env::var("LMM_PROTOCOL_REQUESTS")
        .map(|v| v.parse().unwrap())
        .unwrap_or(256);
    assert!((1..=1024).contains(&concurrency) && count >= concurrency);
    let hits = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().route(
        "/{*path}",
        post({
            let hits = hits.clone();
            let active = active.clone();
            let peak = peak.clone();
            move |uri: axum::http::Uri, axum::Json(body): axum::Json<Value>| {
                let (hits, active, peak) = (hits.clone(), active.clone(), peak.clone());
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    // An awaited supplier delay makes overlap observable rather than
                    // merely counting queued tasks in the load generator.
                    sleep(Duration::from_millis(2)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    let path = uri.path();
                    let protocol = if path.contains("chat/completions") {
                        UpstreamProtocol::Chat
                    } else if path.ends_with("responses") {
                        UpstreamProtocol::Responses
                    } else if path.ends_with("messages") {
                        UpstreamProtocol::Anthropic
                    } else {
                        UpstreamProtocol::Gemini
                    };
                    let stream = body["stream"] == true || path.ends_with(":streamGenerateContent");
                    let (content_type, bytes) = if stream {
                        ("text/event-stream", streaming(protocol))
                    } else {
                        (
                            "application/json",
                            serde_json::to_vec(&full(protocol)).unwrap(),
                        )
                    };
                    Response::builder()
                        .header("content-type", content_type)
                        .body(Body::from(bytes))
                        .unwrap()
                }
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let started = Instant::now();
    let mut total = 0;
    for protocol in [
        UpstreamProtocol::Chat,
        UpstreamProtocol::Responses,
        UpstreamProtocol::Anthropic,
        UpstreamProtocol::Gemini,
    ] {
        for stream in [false, true] {
            let money = Arc::new(Money::default());
            let limits = Limits {
                max_in_flight: concurrency,
                ..Limits::default()
            };
            let relay = Arc::new(
                Relay::new(
                    Arc::new(ConcurrentRoutes::new(&address, protocol)),
                    money.clone(),
                    limits.clone(),
                )
                .unwrap(),
            );
            let gate = Arc::new(Semaphore::new(concurrency));
            let mut jobs = tokio::task::JoinSet::new();
            let batch = Instant::now();
            for i in 0..count {
                let permit = gate.clone().acquire_owned().await.unwrap();
                let relay = relay.clone();
                let limits = limits.clone();
                jobs.spawn(async move {
                    let _permit = permit;
                    let client = if i % 2 == 0 {
                        ClientProtocol::Chat
                    } else {
                        ClientProtocol::Responses
                    };
                    let mut body = input(client, stream);
                    if payload_bytes > 0 {
                        let text = "x".repeat(payload_bytes);
                        match client {
                            ClientProtocol::Chat => body["messages"][0]["content"] = json!(text),
                            ClientProtocol::Responses => body["input"] = json!(text),
                            _ => unreachable!(),
                        }
                    }
                    let request =
                        Request::parse(client, &serde_json::to_vec(&body).unwrap(), &limits)
                            .unwrap();
                    let context = context();
                    let expected_id = context.request_id.clone();
                    let start = Instant::now();
                    let (head, body, done) = collect(relay.start(request, context).unwrap()).await;
                    assert_eq!(done.report.request_id, expected_id);
                    assert!(head.is_ok(), "{protocol:?} {stream}: {head:?}");
                    assert!(
                        done.error.is_none(),
                        "{protocol:?} {stream}: {:?}",
                        done.error
                    );
                    assert_eq!(done.settlement, Settlement::Confirmed);
                    assert_eq!(done.report.usage.input_tokens, Some(10));
                    assert_eq!(done.report.usage.output_tokens, Some(5));
                    assert_output(client, stream, &body);
                    start.elapsed().as_micros()
                });
            }
            let mut latency = Vec::with_capacity(count);
            while let Some(result) = jobs.join_next().await {
                latency.push(result.unwrap());
            }
            assert_eq!(latency.len(), count);
            assert_eq!(money.reserves.load(Ordering::SeqCst), count);
            let finalized: std::collections::HashSet<_> = money
                .finals
                .lock()
                .unwrap()
                .iter()
                .map(|report| report.request_id.clone())
                .collect();
            assert_eq!(
                finalized.len(),
                count,
                "each request must finalize exactly once"
            );
            assert_eq!(money.finals.lock().unwrap().len(), count);
            let drained = relay.drain(Duration::from_secs(1)).await;
            assert_eq!((drained.cancelled, drained.remaining), (0, 0));
            assert_eq!(relay.active_requests(), 0);
            latency.sort_unstable();
            println!(
                "protocol_load profile={} upstream={protocol:?} stream={stream} concurrency={concurrency} completed={count} errors=0 elapsed_ms={} requests_per_s={:.1} p50_us={} p95_us={} p99_us={}",
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                },
                batch.elapsed().as_millis(),
                count as f64 / batch.elapsed().as_secs_f64(),
                latency[(count - 1) * 50 / 100],
                latency[(count - 1) * 95 / 100],
                latency[(count - 1) * 99 / 100]
            );
            total += count;
        }
    }
    assert_eq!(hits.load(Ordering::SeqCst), total);
    if concurrency > 1 {
        assert!(peak.load(Ordering::SeqCst) > 1);
    }
    assert!(peak.load(Ordering::SeqCst) <= concurrency);
    println!(
        "protocol_load_total completed={total} upstream_hits={total} peak_upstream={} elapsed_ms={}",
        peak.load(Ordering::SeqCst),
        started.elapsed().as_millis()
    );
    server.abort();
}

#[tokio::test]
async fn gemini_multiple_sse_frames_in_one_chunk_preserve_trailing_finish_and_usage() {
    let frames = vec![
        data(
            json!({"responseId":"up_962","candidates":[{"index":0,"content":{"parts":[{"text":"鲸"}]}}]}),
        ),
        data(
            json!({"responseId":"up_962","candidates":[{"index":0,"content":{"parts":[{"text":"鱼"}]}}]}),
        ),
        data(json!({"responseId":"up_962","candidates":[{"index":0,"finishReason":"STOP"}]})),
        data(
            json!({"responseId":"up_962","usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":3,"thoughtsTokenCount":2,"cachedContentTokenCount":3}}),
        ),
    ];
    let wire = cat(frames.clone());
    let layouts = [
        ("coalesced", vec![wire.clone()]),
        ("per_frame", frames),
        (
            "bytewise",
            wire.chunks(1).map(|bytes| bytes.to_vec()).collect(),
        ),
    ];
    let mut baseline = None;
    for (layout, chunks) in layouts {
        let expected_chunks = chunks.len();
        let mut script = Script::sse(Vec::new());
        script.steps = chunks.into_iter().map(Step::Data).collect();
        let upstream = Upstream::start(script).await;
        let money = Arc::new(Money::default());
        let limits = Limits::default();
        let relay = relay(
            &upstream,
            UpstreamProtocol::Gemini,
            limits.clone(),
            money.clone(),
        );
        let (head, body, done) = collect(
            relay
                .start(request(ClientProtocol::Chat, true, &limits), context())
                .unwrap(),
        )
        .await;
        let head = head.unwrap();
        assert_eq!(head.status, 200, "{layout}");
        assert_eq!(
            head.content_type.split(';').next().unwrap().trim(),
            "text/event-stream",
            "{layout}"
        );
        assert!(done.error.is_none(), "{layout}: {:?} {body}", done.error);
        assert_eq!(done.settlement, Settlement::Confirmed, "{layout}");
        assert_eq!(done.report.outcome, Outcome::Completed, "{layout}");
        assert!(done.report.provider_complete, "{layout}");
        assert_eq!(
            done.report.finish_reason,
            Some(FinishReason::Stop),
            "{layout}"
        );
        assert_eq!(done.report.usage.input_tokens, Some(10), "{layout}");
        assert_eq!(done.report.usage.output_tokens, Some(5), "{layout}");
        assert_eq!(done.report.usage.cached_input_tokens, Some(3), "{layout}");
        assert_eq!(
            done.report.usage.reasoning_output_tokens,
            Some(2),
            "{layout}"
        );
        assert_eq!(done.report.attempts, 1, "{layout}");
        assert_eq!(done.report.upstream_bytes, wire.len(), "{layout}");
        assert_eq!(upstream.observed.hits.load(Ordering::SeqCst), 1, "{layout}");
        assert_eq!(
            upstream.observed.chunks.load(Ordering::SeqCst),
            expected_chunks,
            "{layout}"
        );
        let finals = money.finals.lock().unwrap();
        assert_eq!(finals.len(), 1, "{layout}");
        assert_eq!(finals[0].usage, done.report.usage, "{layout}");
        assert_eq!(
            finals[0].finish_reason,
            Some(FinishReason::Stop),
            "{layout}"
        );
        assert!(finals[0].provider_complete, "{layout}");
        assert_eq!(finals[0].outcome, Outcome::Completed, "{layout}");
        drop(finals);

        assert!(body.ends_with("data: [DONE]\n\n"), "{layout}: {body}");
        assert_eq!(body.matches("data: [DONE]\n\n").count(), 1, "{layout}");
        let mut output = packets(&body);
        let text: String = output
            .iter()
            .filter_map(|packet| packet["choices"][0]["delta"]["content"].as_str())
            .collect();
        assert_eq!(text, "鲸鱼", "{layout}");
        let finishes: Vec<_> = output
            .iter()
            .filter_map(|packet| packet["choices"][0]["finish_reason"].as_str())
            .collect();
        assert_eq!(finishes, ["stop"], "{layout}");
        let usage = output.last().unwrap();
        assert_eq!(usage["choices"], json!([]), "{layout}");
        assert_eq!(usage["usage"], openai_usage(true), "{layout}");
        assert_eq!(
            output[output.len() - 2]["choices"][0]["finish_reason"],
            "stop"
        );
        // Request IDs and creation times differ between sessions; payloads must not.
        for packet in &mut output {
            let packet = packet.as_object_mut().unwrap();
            packet.remove("id");
            packet.remove("created");
        }
        if let Some(expected) = &baseline {
            assert_eq!(&output, expected, "{layout}");
        } else {
            baseline = Some(output);
        }
    }
}
fn first_text(text: &str) -> Vec<u8> {
    data(json!({"id":"live_1","choices":[{"index":0,"delta":{"role":"assistant","content":text}}]}))
}
fn final_text(text: &str) -> Vec<u8> {
    cat(vec![
        data(
            json!({"id":"live_1","choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]}),
        ),
        data(json!({"id":"live_1","choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3}})),
        b"data: [DONE]\n\n".to_vec(),
    ])
}
async fn until(body: &mut reqwest::Response, seen: &mut Vec<u8>, needle: &str) {
    timeout(WAIT, async {
        while !String::from_utf8_lossy(seen).contains(needle) {
            seen.extend(
                body.chunk()
                    .await
                    .unwrap()
                    .expect("stream ended before the expected chunk"),
            );
        }
    })
    .await
    .unwrap();
}
struct Frontend {
    url: String,
    reports: mpsc::Receiver<Completion>,
    task: JoinHandle<()>,
}
impl Drop for Frontend {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Frontend {
    async fn start(relay: Relay, client: ClientProtocol, limits: Limits) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let (tx, reports) = mpsc::channel(4);
        let app = Router::new().route(
            "/model",
            post(move |bytes: Bytes| {
                let relay = relay.clone();
                let limits = limits.clone();
                let tx = tx.clone();
                async move {
                    let result = Request::parse(client, &bytes, &limits)
                        .and_then(|r| relay.start(r, context()));
                    match result {
                        Err(e) => Response::builder()
                            .status(e.status())
                            .body(Body::from(e.json().to_string()))
                            .unwrap(),
                        Ok(session) => {
                            let Session {
                                head,
                                body,
                                completion,
                                ..
                            } = session;
                            tokio::spawn(async move {
                                if let Ok(c) = completion.await {
                                    let _ = tx.send(c).await;
                                }
                            });
                            match head.await.unwrap() {
                                Ok(h) => Response::builder()
                                    .status(h.status)
                                    .header("content-type", h.content_type)
                                    .body(Body::from_stream(body))
                                    .unwrap(),
                                Err(e) => Response::builder()
                                    .status(e.status())
                                    .body(Body::from(e.json().to_string()))
                                    .unwrap(),
                            }
                        }
                    }
                }
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { url, reports, task }
    }
}

#[tokio::test]
async fn downstream_receives_text_before_upstream_completes() {
    let gate = Arc::new(Semaphore::new(0));
    let upstream = Upstream::start(Script {
        steps: vec![
            Step::Data(first_text("before")),
            Step::Wait(gate.clone()),
            Step::Data(final_text("after")),
        ],
        ..Script::sse(Vec::new())
    })
    .await;
    let money = Arc::new(Money::default());
    let l = Limits::default();
    let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
    let mut front = Frontend::start(relay.clone(), ClientProtocol::Chat, l).await;
    let mut response = reqwest::Client::new()
        .post(&front.url)
        .json(&input(ClientProtocol::Chat, true))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut seen = Vec::new();
    until(&mut response, &mut seen, "before").await;
    assert_eq!(relay.active_requests(), 1);
    assert!(money.finals.lock().unwrap().is_empty());
    assert!(!String::from_utf8_lossy(&seen).contains("[DONE]"));
    gate.add_permits(1);
    while let Some(c) = response.chunk().await.unwrap() {
        seen.extend(c);
    }
    let done = timeout(WAIT, front.reports.recv()).await.unwrap().unwrap();
    assert!(done.error.is_none());
    let output = String::from_utf8(seen).unwrap();
    assert!(output.contains("after"));
    assert!(output.contains("[DONE]"));
}

#[tokio::test]
async fn actual_downstream_disconnect_cancels_upstream_and_finalizes_once() {
    let upstream = Upstream::start(Script {
        steps: vec![Step::Data(first_text("before")), Step::Sleep(WAIT)],
        ..Script::sse(Vec::new())
    })
    .await;
    let l = Limits::default();
    let money = Arc::new(Money::default());
    let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
    let mut front = Frontend::start(relay.clone(), ClientProtocol::Chat, l).await;
    let mut response = reqwest::Client::builder()
        .http1_only()
        .build()
        .unwrap()
        .post(&front.url)
        .json(&input(ClientProtocol::Chat, true))
        .send()
        .await
        .unwrap();
    let mut seen = Vec::new();
    until(&mut response, &mut seen, "before").await;
    drop(response);
    let done = timeout(WAIT, front.reports.recv()).await.unwrap().unwrap();
    assert_eq!(done.error, Some(RelayError::Cancelled));
    assert_eq!(done.report.outcome, Outcome::Cancelled);
    assert_eq!(done.report.usage, Usage::default());
    assert_eq!(done.settlement, Settlement::Confirmed);
    assert_eq!(money.finals.lock().unwrap().len(), 1);
    timeout(WAIT, async {
        while !upstream.observed.disconnected.load(Ordering::SeqCst) {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(relay.drain(Duration::ZERO).await.remaining, 0);
}

#[tokio::test]
async fn cancel_during_reservation_waits_for_handle_then_finalizes_without_sending() {
    let gate = Arc::new(Semaphore::new(0));
    let money = Arc::new(Money {
        reserve_wait: Some(gate.clone()),
        ..Money::default()
    });
    let upstream = Upstream::start(Script::json(full(UpstreamProtocol::Chat))).await;
    let l = Limits::default();
    let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
    let session = relay
        .start(request(ClientProtocol::Chat, false, &l), context())
        .unwrap();
    timeout(WAIT, async {
        while money.reserves.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    session.cancel.cancel();
    gate.add_permits(1);
    let (head, _, done) = collect(session).await;
    assert_eq!(head.unwrap_err(), RelayError::Cancelled);
    assert_eq!(done.report.outcome, Outcome::Cancelled);
    assert_eq!(upstream.observed.hits.load(Ordering::SeqCst), 0);
    assert_eq!(money.finals.lock().unwrap().len(), 1);
    assert_eq!(done.settlement, Settlement::Confirmed);
}

#[tokio::test]
async fn admission_and_drain_are_bounded_and_do_not_abort_settlement() {
    let upstream = Upstream::start(Script {
        steps: vec![Step::Data(first_text("live")), Step::Sleep(WAIT)],
        ..Script::sse(Vec::new())
    })
    .await;
    let l = Limits {
        max_in_flight: 1,
        ..Limits::default()
    };
    let money = Arc::new(Money::default());
    let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
    let mut s = relay
        .start(request(ClientProtocol::Chat, true, &l), context())
        .unwrap();
    timeout(WAIT, &mut s.head).await.unwrap().unwrap().unwrap();
    assert!(matches!(
        relay.start(request(ClientProtocol::Chat, true, &l), context()),
        Err(RelayError::Busy)
    ));
    let drained = relay.drain(Duration::ZERO).await;
    assert_eq!(
        drained,
        DrainReport {
            cancelled: 1,
            remaining: 0
        }
    );
    assert!(matches!(
        relay.start(request(ClientProtocol::Chat, true, &l), context()),
        Err(RelayError::Draining)
    ));
    let done = timeout(WAIT, s.completion).await.unwrap().unwrap();
    assert_eq!(done.error, Some(RelayError::Cancelled));
    assert_eq!(money.finals.lock().unwrap().len(), 1);
    assert_eq!(relay.active_requests(), 0);
}

#[tokio::test]
async fn rejected_billing_never_opens_upstream_and_never_changes_readiness() {
    let upstream = Upstream::start(Script::json(full(UpstreamProtocol::Chat))).await;
    let l = Limits::default();
    let money = Arc::new(Money {
        reject: true,
        ..Money::default()
    });
    let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
    let (head, body, done) = collect(
        relay
            .start(request(ClientProtocol::Chat, false, &l), context())
            .unwrap(),
    )
    .await;
    assert_eq!(head.unwrap_err(), RelayError::BillingUnavailable);
    assert!(body.is_empty());
    assert_eq!(upstream.observed.hits.load(Ordering::SeqCst), 0);
    assert!(money.finals.lock().unwrap().is_empty());
    assert!(!done.report.upstream_attempted);
    // The module itself neither registers endpoints nor touches core readiness.
    let public_source = include_str!("../src/http.rs");
    assert!(public_source.contains("SERVICE_UNAVAILABLE"));
}

#[tokio::test]
async fn settlement_failure_suppresses_success_for_both_clients_and_modes() {
    for client in [ClientProtocol::Chat, ClientProtocol::Responses] {
        for stream in [false, true] {
            let upstream = Upstream::start(if stream {
                Script::sse(streaming(UpstreamProtocol::Chat))
            } else {
                Script::json(full(UpstreamProtocol::Chat))
            })
            .await;
            let l = Limits::default();
            let money = Arc::new(Money {
                fail_finalize: true,
                ..Money::default()
            });
            let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
            let (head, body, done) =
                collect(relay.start(request(client, stream, &l), context()).unwrap()).await;
            assert_eq!(done.error, Some(RelayError::SettlementPending));
            assert_eq!(done.settlement, Settlement::Pending);
            assert!(!body.contains("[DONE]"));
            assert!(!body.contains("response.completed"));
            assert_eq!(money.finals.lock().unwrap().len(), 1);
            if stream {
                assert!(head.is_ok());
                assert!(body.contains("settlement_pending"));
            } else {
                assert_eq!(head.unwrap_err(), RelayError::SettlementPending);
                assert!(body.is_empty());
            }
        }
    }
}

#[tokio::test]
async fn non_stream_head_waits_for_settlement_and_hooks_have_a_deadline() {
    let gate = Arc::new(Semaphore::new(0));
    let upstream = Upstream::start(Script::json(full(UpstreamProtocol::Chat))).await;
    let l = Limits {
        hook_timeout: Duration::from_millis(250),
        ..Limits::default()
    };
    let money = Arc::new(Money {
        finalize_wait: Some(gate),
        ..Money::default()
    });
    let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
    let mut session = relay
        .start(request(ClientProtocol::Chat, false, &l), context())
        .unwrap();
    timeout(WAIT, async {
        while money.finals.lock().unwrap().is_empty() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        session.head.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    let (head, body, done) = collect(session).await;
    assert_eq!(head.unwrap_err(), RelayError::SettlementPending);
    assert!(body.is_empty());
    assert_eq!(done.settlement, Settlement::Pending);
}

#[tokio::test]
async fn reservation_timeout_is_ambiguous_not_a_free_request() {
    let upstream = Upstream::start(Script::json(full(UpstreamProtocol::Chat))).await;
    let l = Limits {
        hook_timeout: Duration::from_millis(150),
        ..Limits::default()
    };
    let money = Arc::new(Money {
        reserve_wait: Some(Arc::new(Semaphore::new(0))),
        ..Money::default()
    });
    let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
    let (head, _, done) = collect(
        relay
            .start(request(ClientProtocol::Chat, false, &l), context())
            .unwrap(),
    )
    .await;
    assert_eq!(head.unwrap_err(), RelayError::Timeout(Phase::Reserve));
    assert_eq!(done.settlement, Settlement::Pending);
    assert_eq!(upstream.observed.hits.load(Ordering::SeqCst), 0);
    assert!(money.finals.lock().unwrap().is_empty());
}

#[tokio::test]
async fn http_statuses_and_redirects_are_never_retried_or_leaked() {
    for status in [302, 401, 429, 500, 503] {
        let upstream = Upstream::start(Script {
            status,
            ..Script::json(json!({"error":"secret-upstream-body"}))
        })
        .await;
        let l = Limits {
            connect_retries: 2,
            ..Limits::default()
        };
        let money = Arc::new(Money::default());
        let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
        let (head, body, done) = collect(
            relay
                .start(request(ClientProtocol::Chat, false, &l), context())
                .unwrap(),
        )
        .await;
        let e = head.unwrap_err();
        assert_eq!(e, RelayError::UpstreamStatus(status));
        assert!(!e.to_string().contains("secret"));
        assert!(body.is_empty());
        assert_eq!(done.report.attempts, 1);
        assert_eq!(upstream.observed.hits.load(Ordering::SeqCst), 1);
        assert_eq!(money.finals.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn connect_retries_are_limited_to_the_configured_number() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let l = Limits {
        connect_retries: 2,
        retry_backoff: Duration::from_millis(20),
        ..Limits::default()
    };
    let money = Arc::new(Money::default());
    let relay = Relay::new(
        Arc::new(Routes {
            address,
            protocol: UpstreamProtocol::Chat,
        }),
        money.clone(),
        l.clone(),
    )
    .unwrap();
    let (_, _, done) = collect(
        relay
            .start(request(ClientProtocol::Chat, false, &l), context())
            .unwrap(),
    )
    .await;
    assert_eq!(done.error, Some(RelayError::UpstreamConnection));
    assert_eq!(done.report.attempts, 3);
    assert_eq!(money.finals.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn header_read_and_total_deadlines_are_distinct() {
    let cases = [
        (
            Phase::Headers,
            Script {
                header_delay: Duration::from_secs(2),
                ..Script::sse(Vec::new())
            },
            Limits {
                header_timeout: Duration::from_millis(150),
                ..Limits::default()
            },
        ),
        (
            Phase::Read,
            Script {
                steps: vec![
                    Step::Data(first_text("live")),
                    Step::Sleep(Duration::from_secs(2)),
                ],
                ..Script::sse(Vec::new())
            },
            Limits {
                read_timeout: Duration::from_millis(150),
                ..Limits::default()
            },
        ),
        (
            Phase::Total,
            Script {
                steps: (0..20)
                    .flat_map(|_| {
                        [
                            Step::Data(b": heartbeat\n\n".to_vec()),
                            Step::Sleep(Duration::from_millis(50)),
                        ]
                    })
                    .collect(),
                ..Script::sse(Vec::new())
            },
            Limits {
                total_timeout: Duration::from_millis(250),
                read_timeout: Duration::from_millis(200),
                ..Limits::default()
            },
        ),
    ];
    for (phase, script, l) in cases {
        let upstream = Upstream::start(script).await;
        let money = Arc::new(Money::default());
        let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
        let (_, body, done) = collect(
            relay
                .start(request(ClientProtocol::Chat, true, &l), context())
                .unwrap(),
        )
        .await;
        assert_eq!(done.error, Some(RelayError::Timeout(phase)));
        assert_eq!(done.report.outcome, Outcome::TimedOut);
        assert_eq!(done.report.attempts, 1);
        assert!(!body.contains("[DONE]"));
        assert_eq!(money.finals.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn a_slow_client_cannot_create_an_unbounded_queue() {
    let upstream = Upstream::start(Script::sse(streaming(UpstreamProtocol::Chat))).await;
    let l = Limits {
        queue_chunks: 1,
        chunk_bytes: 32,
        write_timeout: Duration::from_millis(150),
        ..Limits::default()
    };
    let money = Arc::new(Money::default());
    let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
    let mut session = relay
        .start(request(ClientProtocol::Chat, true, &l), context())
        .unwrap();
    timeout(WAIT, &mut session.head)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    // Deliberately retain, but do not poll, the body until the worker terminates.
    let done = timeout(WAIT, &mut session.completion)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.error, Some(RelayError::Timeout(Phase::Write)));
    assert!(done.report.emitted_bytes <= 32);
    let mut count = 0;
    while let Some(chunk) = session.body.next_chunk().await {
        assert!(chunk.unwrap().len() <= 32);
        count += 1;
    }
    assert_eq!(count, 1);
    assert_eq!(relay.drain(Duration::ZERO).await.remaining, 0);
    assert_eq!(money.finals.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn event_output_tool_and_raw_body_limits_are_enforced() {
    let base = Limits {
        output_bytes: 2048,
        event_bytes: 1024,
        tool_argument_bytes: 64,
        ..Limits::default()
    };
    let mut argument = full(UpstreamProtocol::Chat);
    argument["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] =
        json!({"city":"x".repeat(100)}).to_string().into();
    let cases = [
        (
            Script::sse([b"data: ".as_slice(), vec![b'x'; 2048].as_slice()].concat()),
            true,
            base.clone(),
        ),
        (
            Script::sse(cat((0..30).map(|_| first_text(&"x".repeat(100))).collect())),
            true,
            base.clone(),
        ),
        (Script::json(argument), false, base.clone()),
        (
            Script::json(json!({"padding":"x".repeat(4096)})),
            false,
            base.clone(),
        ),
    ];
    for (script, stream, l) in cases {
        let upstream = Upstream::start(script).await;
        let money = Arc::new(Money::default());
        let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
        let (_, body, done) = collect(
            relay
                .start(request(ClientProtocol::Chat, stream, &l), context())
                .unwrap(),
        )
        .await;
        assert!(
            matches!(done.error, Some(RelayError::Limit(_))),
            "{:?}",
            done.error
        );
        assert!(!body.contains("[DONE]"));
        assert_eq!(money.finals.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn missing_terminal_invalid_utf8_and_broken_http_fail_without_replay() {
    let cases = [
        (Script::sse(first_text("partial")), RelayError::Truncated),
        (
            Script::sse(b"data: {\"id\":\"unfinished".to_vec()),
            RelayError::Truncated,
        ),
        (
            Script {
                complete: false,
                steps: vec![Step::Data(first_text("partial"))],
                ..Script::sse(Vec::new())
            },
            RelayError::UpstreamConnection,
        ),
        (
            Script::sse(b"data: \xff\n\n".to_vec()),
            RelayError::Protocol("SSE is not UTF-8"),
        ),
        (
            Script {
                content_type: "text/html",
                ..Script::sse(b"secret error page".to_vec())
            },
            RelayError::Protocol("unexpected upstream content type"),
        ),
    ];
    for (script, expected) in cases {
        let upstream = Upstream::start(script).await;
        let l = Limits {
            connect_retries: 2,
            ..Limits::default()
        };
        let money = Arc::new(Money::default());
        let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
        let (head, body, done) = collect(
            relay
                .start(request(ClientProtocol::Chat, true, &l), context())
                .unwrap(),
        )
        .await;
        assert_eq!(done.error, Some(expected.clone()));
        assert_eq!(done.report.outcome, Outcome::Failed);
        assert!(!done.report.provider_complete);
        assert_eq!(expected.status(), 502);
        assert_ne!(expected.code(), RelayError::Cancelled.code());
        match head {
            Ok(_) => {
                let error = packets(&body).pop().unwrap();
                assert_eq!(error["error"]["code"], expected.code());
            }
            Err(error) => assert_eq!(error, expected),
        }
        assert_eq!(done.report.attempts, 1);
        assert!(!body.contains("[DONE]"));
        assert!(!body.contains("secret error page"));
        let finals = money.finals.lock().unwrap();
        assert_eq!(finals.len(), 1);
        assert_eq!(finals[0].outcome, Outcome::Failed);
        assert!(!finals[0].provider_complete);
        assert_eq!(finals[0].usage, Usage::default());
    }
}

#[tokio::test]
async fn missing_usage_stays_unknown_and_partial_tools_only_allow_incomplete_finish() {
    for reason in ["length", "tool_calls"] {
        let mut v = full(UpstreamProtocol::Chat);
        v.as_object_mut().unwrap().remove("usage");
        v["choices"][0]["finish_reason"] = json!(reason);
        v["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] = json!("{\"city\":");
        let upstream = Upstream::start(Script::json(v)).await;
        let l = Limits::default();
        let money = Arc::new(Money::default());
        let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
        let (_, body, done) = collect(
            relay
                .start(request(ClientProtocol::Responses, false, &l), context())
                .unwrap(),
        )
        .await;
        assert_eq!(done.report.usage, Usage::default());
        assert_eq!(money.finals.lock().unwrap()[0].usage, Usage::default());
        if reason == "length" {
            assert!(done.error.is_none());
            let v: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["status"], "incomplete");
            assert!(v["usage"].is_null());
        } else {
            assert!(matches!(done.error, Some(RelayError::Protocol(_))));
        }
    }
}

#[tokio::test]
async fn immutable_request_history_is_converted_without_losing_tool_results() {
    for p in [
        UpstreamProtocol::Chat,
        UpstreamProtocol::Responses,
        UpstreamProtocol::Anthropic,
        UpstreamProtocol::Gemini,
    ] {
        let upstream = Upstream::start(Script::json(full(p))).await;
        let l = Limits::default();
        let money = Arc::new(Money::default());
        let relay = relay(&upstream, p, l.clone(), money);
        let body = json!({"model":"public-model","messages":[{"role":"system","content":"rules"},{"role":"user","content":"weather?"},
            {"role":"assistant","tool_calls":[{"type":"function","id":"call_old","function":{"name":"weather","arguments":"{\"city\":\"上海\"}"}}]},
            {"role":"tool","tool_call_id":"call_old","content":"sunny"}],"max_tokens":100});
        let r = Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&body).unwrap(),
            &l,
        )
        .unwrap();
        let (_, _, done) = collect(relay.start(r, context()).unwrap()).await;
        assert!(done.error.is_none(), "{p:?}: {:?}", done.error);
        let observed = upstream.observed.requests.lock().unwrap();
        let v = &observed[0].1;
        match p {
            UpstreamProtocol::Chat => {
                assert_eq!(v["messages"][3]["tool_call_id"], "call_old");
                assert_eq!(v["messages"][3]["content"], "sunny");
            }
            UpstreamProtocol::Responses => {
                assert_eq!(v["input"][3]["call_id"], "call_old");
                assert_eq!(v["input"][3]["output"], "sunny");
            }
            UpstreamProtocol::Anthropic => {
                assert_eq!(v["system"][0]["text"], "rules");
                assert_eq!(v["messages"][2]["content"][0]["tool_use_id"], "call_old");
            }
            UpstreamProtocol::Gemini => {
                assert_eq!(v["systemInstruction"]["parts"][0]["text"], "rules");
                assert_eq!(
                    v["contents"][2]["parts"][0]["functionResponse"]["name"],
                    "weather"
                );
                assert_eq!(
                    v["contents"][2]["parts"][0]["functionResponse"]["response"]["output"],
                    "sunny"
                );
            }
        }
    }
}

#[test]
fn unsupported_or_ambiguous_input_and_invalid_routes_fail_closed() {
    let l = Limits::default();
    for v in [
        json!({"model":"x","messages":[],"n":2}),
        json!({"model":"x","messages":[],"unknown":true}),
        json!({"model":"x","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"file:///tmp/unbound-image"}}]}]}),
        json!({"model":"x","messages":[{"role":"tool","tool_call_id":"orphan","content":"result"}]}),
        json!({"model":"x","messages":[],"max_tokens":1,"max_completion_tokens":2}),
        json!({"model":"x","messages":[],"store":true}),
    ] {
        assert!(
            Request::parse(ClientProtocol::Chat, &serde_json::to_vec(&v).unwrap(), &l).is_err()
        );
    }
    for url in [
        "file:///tmp/key",
        "http://example.com/v1",
        "https://user:password@example.com/v1",
        "https://example.com/v1?key=secret",
        "https://example.com/v1#fragment",
    ] {
        assert!(
            Route::new(
                "r".into(),
                UpstreamProtocol::Chat,
                url,
                "m".into(),
                "test",
                1
            )
            .is_err()
        );
    }
    let route = Route::new(
        "r".into(),
        UpstreamProtocol::Chat,
        "https://example.com/v1",
        "m".into(),
        "hidden-key",
        1,
    )
    .unwrap();
    assert!(!format!("{route:?}").contains("hidden-key"));
    assert!(!format!("{route:?}").contains("example.com"));
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::test]
async fn stopping_the_actual_go_extension_does_not_cut_a_rust_http_stream() {
    let binary = std::env::var("LMM_RELAY_EXTENSION_BIN").expect("build services/extensions/cmd/extensions and set LMM_RELAY_EXTENSION_BIN; this acceptance test must not be skipped");
    let dir = std::env::temp_dir().join(format!(
        "lmm-relay-go-{}-{}",
        std::process::id(),
        IDS.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let token = dir.join("extension-token");
    let mut token_options = std::fs::OpenOptions::new();
    token_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        token_options.mode(0o600);
    }
    let mut token_file = token_options.open(&token).unwrap();
    std::io::Write::write_all(
        &mut token_file,
        b"relay-test-only-credential-0123456789abcdef",
    )
    .unwrap();
    drop(token_file);
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = port.local_addr().unwrap();
    drop(port);
    let mut command = Command::new(binary);
    let configuration = dir.join("extensions.toml");
    std::fs::write(
        &configuration,
        format!(
            "version=1\n[host]\nlisten={:?}\ntoken_file={token:?}\nmodules=['none']\n",
            address.to_string()
        ),
    )
    .unwrap();
    command
        .args(["--config", configuration.to_str().unwrap()])
        .env_clear()
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let mut process = Process(command.spawn().unwrap());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    timeout(WAIT, async {
        loop {
            assert!(
                process.0.try_wait().unwrap().is_none(),
                "the real Go extension exited before the test"
            );
            if client
                .get(format!("http://{address}/health/live"))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let gate = Arc::new(Semaphore::new(0));
    let upstream = Upstream::start(Script {
        steps: vec![
            Step::Data(first_text("before_go_stop")),
            Step::Wait(gate.clone()),
            Step::Data(final_text("after_go_stop")),
        ],
        ..Script::sse(Vec::new())
    })
    .await;
    let l = Limits::default();
    let money = Arc::new(Money::default());
    let relay = relay(&upstream, UpstreamProtocol::Chat, l.clone(), money.clone());
    let mut front = Frontend::start(relay.clone(), ClientProtocol::Chat, l).await;
    let mut response = reqwest::Client::new()
        .post(&front.url)
        .json(&input(ClientProtocol::Chat, true))
        .send()
        .await
        .unwrap();
    let mut seen = Vec::new();
    until(&mut response, &mut seen, "before_go_stop").await;
    assert_eq!(relay.active_requests(), 1);
    let pid = process.0.id();
    process.0.kill().unwrap();
    assert!(!process.0.wait().unwrap().success());
    assert_eq!(relay.active_requests(), 1);
    gate.add_permits(1);
    while let Some(bytes) = timeout(WAIT, response.chunk()).await.unwrap().unwrap() {
        seen.extend(bytes);
    }
    let done = timeout(WAIT, front.reports.recv()).await.unwrap().unwrap();
    let text = String::from_utf8(seen).unwrap();
    assert!(text.contains("after_go_stop"));
    assert!(text.ends_with("data: [DONE]\n\n"));
    assert!(done.error.is_none());
    assert_eq!(done.report.usage.input_tokens, Some(2));
    assert_eq!(done.report.usage.output_tokens, Some(3));
    assert_eq!(money.finals.lock().unwrap().len(), 1);
    assert_eq!(relay.drain(Duration::ZERO).await.remaining, 0);
    std::fs::remove_dir_all(dir).unwrap();
    println!(
        "actual Go extension pid={pid} killed after first HTTP output; Rust emitted later output and settled once; no Rust restart was performed"
    );
}

#[tokio::test]
async fn error_packets_obey_the_same_chunk_limit_as_model_output() {
    let upstream = Upstream::start(Script::sse(first_text("partial"))).await;
    let limits = Limits {
        queue_chunks: 2,
        chunk_bytes: 17,
        ..Limits::default()
    };
    let money = Arc::new(Money::default());
    let relay = relay(&upstream, UpstreamProtocol::Chat, limits.clone(), money);
    let Session {
        head,
        mut body,
        completion,
        ..
    } = relay
        .start(request(ClientProtocol::Chat, true, &limits), context())
        .unwrap();
    timeout(WAIT, head).await.unwrap().unwrap().unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = timeout(WAIT, body.next_chunk()).await.unwrap() {
        let chunk = chunk.unwrap();
        assert!(chunk.len() <= 17);
        bytes.extend(chunk);
    }
    let done = timeout(WAIT, completion).await.unwrap().unwrap();
    assert_eq!(done.error, Some(RelayError::Truncated));
    let body = String::from_utf8(bytes).unwrap();
    assert!(body.contains("upstream_truncated"));
    assert!(!body.contains("[DONE]"));
}

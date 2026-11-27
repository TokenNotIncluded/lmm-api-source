//! Native CLIs reach a disposable Core with an ordinary API key and a sole dynamically named model.
use super::*;
use axum::middleware::{self, Next};
use std::{
    io::Write,
    process::{Command, Stdio},
};

// Only protocol vocabulary is observable. Paths inside message text, custom
// schemas, arbitrary function arguments and metadata never leave this fixture.
fn schema_name(name: &str) -> &str {
    match name {
        "request"
        | "model"
        | "input"
        | "instructions"
        | "messages"
        | "system"
        | "role"
        | "content"
        | "type"
        | "text"
        | "verbosity"
        | "format"
        | "tools"
        | "name"
        | "description"
        | "parameters"
        | "input_schema"
        | "strict"
        | "function"
        | "tool_choice"
        | "tool_use_id"
        | "stream"
        | "stream_options"
        | "include_usage"
        | "include_obfuscation"
        | "include"
        | "parallel_tool_calls"
        | "store"
        | "reasoning"
        | "effort"
        | "summary"
        | "thinking"
        | "metadata"
        | "user_id"
        | "client_metadata"
        | "access_programs"
        | "prompt_cache_key"
        | "prompt_cache_options"
        | "safety_identifier"
        | "previous_response_id"
        | "truncation"
        | "service_tier"
        | "cache_control"
        | "ttl"
        | "context_management"
        | "output_config"
        | "output_format"
        | "max_tokens"
        | "max_output_tokens"
        | "temperature"
        | "top_p"
        | "top_k"
        | "stop_sequences"
        | "betas"
        | "anthropic_version"
        | "container"
        | "speed"
        | "mcp_servers"
        | "defer_loading"
        | "allowed_callers"
        | "input_examples"
        | "id"
        | "phase"
        | "status"
        | "external_web_access"
        | "filters"
        | "allowed_domains"
        | "user_location"
        | "namespace"
        | "tool_search"
        | "cache_creation"
        | "sequence_number" => name,
        _ => "other",
    }
}

fn request_shape(value: &Value) -> std::collections::BTreeSet<String> {
    fn walk(
        value: &Value,
        prefix: &str,
        depth: usize,
        out: &mut std::collections::BTreeSet<String>,
    ) {
        if depth > 4 || out.len() >= 96 {
            return;
        }
        if let Some(items) = value.as_array() {
            for item in items.iter().take(12) {
                walk(item, &format!("{prefix}[]"), depth + 1, out);
            }
        } else if let Some(fields) = value.as_object() {
            for (key, field) in fields {
                if out.len() >= 96 {
                    break;
                }
                let key = schema_name(key);
                let path = format!("{prefix}.{key}");
                out.insert(path.clone());
                if key == "client_metadata"
                    && let Some(metadata) = field.as_object()
                {
                    for name in metadata.keys() {
                        let label = match name.as_str() {
                            "session_id"
                            | "thread_id"
                            | "turn_id"
                            | "root_turn_id"
                            | "parent_turn_id"
                            | "x-codex-window-id"
                            | "x-codex-installation-id"
                            | "x-codex-turn-metadata"
                            | "x-codex-parent-thread-id"
                            | "x-openai-subagent" => name.as_str(),
                            _ => "other",
                        };
                        out.insert(format!("{path}.{label}"));
                    }
                }
                if key == "type"
                    && let Some(kind) = field.as_str()
                {
                    let kind = match kind {
                        "function"
                        | "custom"
                        | "namespace"
                        | "tool_search"
                        | "web_search"
                        | "web_search_preview"
                        | "message"
                        | "input_text"
                        | "output_text"
                        | "text"
                        | "function_call"
                        | "function_call_output"
                        | "reasoning"
                        | "compaction"
                        | "tool_use"
                        | "tool_result"
                        | "thinking"
                        | "ephemeral" => kind,
                        _ => "other",
                    };
                    out.insert(format!("{path}={kind}"));
                }
                if !matches!(
                    key,
                    "other"
                        | "parameters"
                        | "input_schema"
                        | "metadata"
                        | "client_metadata"
                        | "description"
                ) && (field.is_array() || field.is_object())
                {
                    walk(field, &path, depth + 1, out);
                }
            }
        }
    }
    let mut out = std::collections::BTreeSet::new();
    walk(value, "request", 0, &mut out);
    out
}

// The Core returns fixed rejection reasons, except for UnsupportedField which can
// contain an arbitrary client-supplied path. Never print the raw response or that path.
fn rejection_summary(body: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let Some((code, detail)) = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .and_then(|text| text.split_once(": "))
    else {
        return "unclassified".into();
    };
    if !matches!(
        code,
        "invalid_request" | "unsupported_feature" | "relay_limit_exceeded"
    ) {
        return "unclassified".into();
    }
    if let Some(path) = detail.strip_prefix("unsupported field ") {
        let label = path
            .trim_start_matches("$.")
            .split('.')
            .take(4)
            .map(|segment| schema_name(segment.trim_end_matches("[]")))
            .collect::<Vec<_>>()
            .join(".");
        return format!("{code}:field:{label}");
    }
    // Only a digest of the bounded reason is retained. It can be matched against
    // Core's source literals without adding prompts or provider payloads to CI logs.
    let digest = Sha256::digest(detail.as_bytes());
    let mut hex = [0; 64];
    zhex::encode_lower(&digest[..], &mut hex).expect("SHA-256 hex output capacity");
    let hex = std::str::from_utf8(&hex).expect("hexadecimal ASCII");
    format!("{code}:reason-sha256:{hex}")
}

async fn client_case(pool: PgPool, protocol: Protocol, client: &'static str) {
    let (identity, ledger, _, secret, key_id, account) = fixture(&pool, 1_000_000).await;
    let actor = identity.authorize(&secret).await.unwrap().user_id;
    let suffix: String =
        sqlx::query_scalar("SELECT left(replace(gen_random_uuid()::text,'-',''),16)")
            .fetch_one(&pool)
            .await
            .unwrap();
    let model = format!("native-{suffix}");
    let group = format!("group-{suffix}");
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES($1,$2,true,'synthetic:cli')")
        .bind(actor).bind(&group).execute(&pool).await.unwrap();
    let provider = server(protocol, false).await;
    let gateway = scoped_gateway(
        &pool,
        &provider,
        protocol,
        key_id,
        false,
        CliRoute {
            model: &model,
            group: &group,
            input: 65_536,
            output: 32_768,
        },
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let capture = seen.clone();
    let path = if protocol == Protocol::AnthropicMessages {
        "/v1/messages"
    } else {
        "/v1/responses"
    };
    let app = gateway.router().layer(middleware::from_fn(
        move |req: axum::extract::Request, next: Next| {
            let capture = capture.clone();
            async move {
                let inference =
                    req.method() == axum::http::Method::POST && req.uri().path() == path;
                if !inference {
                    return next.run(req).await;
                }
                let (request_parts, body) = req.into_parts();
                let wire = to_bytes(body, 4 * 1024 * 1024)
                    .await
                    .expect("bounded local CLI request");
                let value: Value = serde_json::from_slice(&wire).unwrap_or(Value::Null);
                let req = axum::extract::Request::from_parts(request_parts, Body::from(wire));
                let mut response = next.run(req).await;
                {
                    let id = response
                        .headers()
                        .get("x-request-id")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    let status = response.status();
                    let rejection = if status.is_client_error() {
                        let (parts, body) = response.into_parts();
                        let bytes = to_bytes(body, 64 * 1024)
                            .await
                            .expect("bounded Core rejection");
                        let summary = rejection_summary(&bytes);
                        response = Response::from_parts(parts, Body::from(bytes));
                        Some(format!("{summary}; schema={:?}", request_shape(&value)))
                    } else {
                        None
                    };
                    capture
                        .lock()
                        .unwrap()
                        .push((status.as_u16(), id, rejection));
                }
                response
            }
        },
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let core = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let before = ledger.balance(account).await.unwrap();
    let output = tokio::task::spawn_blocking(move || {
        let mut child = Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../scripts/ci/native-cli.mjs"
            ))
            .arg(client)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(json!({"core":core,"key":secret}).to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    handle.abort();
    let seen = seen.lock().unwrap().clone();
    assert!(
        output.status.success(),
        "{client} failed: {}; Core model response statuses {:?}; supplier hits {}",
        String::from_utf8_lossy(&output.stderr),
        seen.iter()
            .map(|(status, _, rejection)| (status, rejection))
            .collect::<Vec<_>>(),
        provider.hits.load(Ordering::SeqCst)
    );
    let proof: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(proof["selected"], model.as_str());
    assert_eq!(proof["completed"], true);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, 200);
    let id = seen[0]
        .1
        .as_deref()
        .expect("request ID on actual client ingress");
    assert_eq!(provider.hits.load(Ordering::SeqCst), 1);
    type ChargeRow = (String, String, i64, i64, i64, i64, i64, Value);
    let charges: Vec<ChargeRow> = sqlx::query_as(
        "SELECT id,state,key_id,actor_user_id,payer_account_id,settled,output_tokens,price FROM core_charging.requests"
    ).fetch_all(&pool).await.unwrap();
    assert_eq!(charges.len(), 1);
    let (charge, state, key, user, payer, settled, tokens, price) = &charges[0];
    assert_eq!(
        (charge.as_str(), state.as_str(), *key, *user, *payer),
        (id, "settled", key_id, actor, account)
    );
    assert_eq!(price["model"], model.as_str());
    assert_eq!(price["group"], group.as_str());
    assert_eq!(*tokens, 20);
    let holds: Vec<(bool, i64, Option<i64>)> = sqlx::query_as(
        "SELECT closed,captured,capture_journal_id FROM core_charging.ledger_holds WHERE request_id=$1"
    ).bind(id).fetch_all(&pool).await.unwrap();
    assert_eq!(holds.len(), 1);
    assert!(holds[0].0 && holds[0].2.is_some() && *settled > 0);
    assert_eq!(holds[0].1, *settled);
    let after = ledger.balance(account).await.unwrap();
    assert_eq!(before.available_units - after.available_units, *settled);
    assert_eq!(after.reserved_units, 0);
    let record = json!({"client":client,"version":proof["version"],"model":model,"group":group,
        "request_id":id,"path":path,"key_id":key,"payer_account_id":payer,"captured_credits":settled,
        "capture_journal":holds[0].2,"output_tokens":tokens,"terminal":true});
    std::fs::write(
        std::env::var("NATIVE_CLI_RECEIPT").expect("cloud receipt path"),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires the pinned official Codex CLI"]
async fn codex_exec_settles_one_native_responses_request(pool: PgPool) {
    client_case(pool, Protocol::OpenAiResponses, "codex").await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires the pinned official Claude Code CLI"]
async fn claude_print_settles_one_native_messages_request(pool: PgPool) {
    client_case(pool, Protocol::AnthropicMessages, "claude").await;
}

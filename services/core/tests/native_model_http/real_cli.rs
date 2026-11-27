//! Native CLIs reach a disposable Core with an ordinary API key and a sole dynamically named model.
use super::*;
use axum::middleware::{self, Next};
use std::{
    io::Write,
    process::{Command, Stdio},
};

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
                let response = next.run(req).await;
                if inference {
                    let id = response
                        .headers()
                        .get("x-request-id")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    capture
                        .lock()
                        .unwrap()
                        .push((response.status().as_u16(), id));
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
        seen.iter().map(|(status, _)| status).collect::<Vec<_>>(),
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
    let charges: Vec<(String, String, i64, i64, i64, i64, i64, Value)> = sqlx::query_as(
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

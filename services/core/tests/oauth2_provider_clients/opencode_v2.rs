use super::*;

#[sqlx::test(migrations = false)]
#[ignore = "requires packaged OpenCode V2 and loopback TLS on the hosted runner"]
async fn real_v2_oauth_session_settles_one_request(pool: PgPool) {
    pi::with_cli_host_fixture(pool, pi::CliHost::OpenCode, |f| async move {
        let before = f.ledger.balance(f.account).await.unwrap();
        let config = json!({"coreBase":f.base,"issuer":f.issuer.profile().resource().strip_suffix("/api/oauth2").unwrap(),
            "refresh":f.refresh,"cap":before.available_units});
        let proof: Value = tokio::task::spawn_blocking(move || {
            use std::{io::Write,process::{Command,Stdio}};
            let mut child = Command::new("node");
            child.arg(concat!(env!("CARGO_MANIFEST_DIR"),"/../../scripts/ci/opencode-v2-core.mjs")).env_clear();
            for key in ["PATH","OPENCODE_V2_BIN","OPENCODE_V2_PLUGIN","FIXTURE_TLS_CERT","FIXTURE_TLS_KEY","NODE_EXTRA_CA_CERTS"] {
                child.env(key,std::env::var_os(key).expect("missing OpenCode host fixture environment"));
            }
            let mut child=child.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(&serde_json::to_vec(&config).unwrap()).unwrap();
            let output=child.wait_with_output().unwrap();
            assert!(output.status.success(),"OpenCode V2 host failed");
            serde_json::from_slice(&output.stdout).unwrap()
        }).await.unwrap();
        let id=proof["requestId"].as_str().unwrap();
        assert_eq!(proof["terminal"],true);
        assert_eq!(f.provider.hits.load(Ordering::SeqCst),1);
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.requests").fetch_one(&f.owner).await.unwrap();
        assert_eq!(count,1);
        let (family, price, input, cached, output, settled, closed, captured, journal):
            (String,Value,i64,i64,i64,i64,bool,i64,i64)=sqlx::query_as(
            "SELECT r.oauth_family_id,r.price,r.input_tokens,r.cached_tokens,r.output_tokens,r.settled,h.closed,h.captured,h.capture_journal_id
             FROM core_charging.requests r JOIN core_charging.ledger_holds h ON h.request_id=r.id
             JOIN core_oauth.families f ON f.id=r.oauth_family_id
             WHERE r.id=$1 AND r.state='settled' AND r.oauth_client_id='lmm-opencode'
             AND r.oauth_source->>'client_id'=r.oauth_client_id AND r.key_id IS NULL
             AND r.payer_account_id=$2 AND r.actor_user_id=$3 AND f.client_id=r.oauth_client_id
             AND f.user_id=r.actor_user_id AND f.owner_account_id=r.payer_account_id
             AND f.issuance_kind='browser' AND f.purpose='normal_model' AND f.budget_credits=$4")
            .bind(id).bind(f.account).bind(f.user).bind(before.available_units).fetch_one(&f.owner).await.unwrap();
        assert_eq!((input,cached,output,settled,closed,captured),(10,2,20,49,true,49));
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD,Engine};
        let group=URL_SAFE_NO_PAD.encode(price["group"].as_str().unwrap());
        let model=URL_SAFE_NO_PAD.encode(price["model"].as_str().unwrap());
        assert_eq!(proof["modelId"],format!("lmm:{group}:{model}"));
        assert_eq!(proof["group"],group);
        let after=f.ledger.balance(f.account).await.unwrap();
        assert_eq!(before.available_units-after.available_units,settled);
        assert_eq!((before.reserved_units,after.reserved_units),(0,0));
        let receipt=json!({"commit":std::env::var("GITHUB_SHA").unwrap(),"version":proof["version"],"client":"lmm-opencode",
            "endpoint":"/v1/chat/completions","request_id":id,"family_id":family,"actor":f.user,"payer":f.account,"api_key":false,
            "terminal":true,"group":price["group"],"model":price["model"],"input_tokens":input,"cached_tokens":cached,
            "output_tokens":output,"cap":before.available_units,"settled":settled,"capture_journal":journal,
            "supplier_hits":1,"reserved_after":after.reserved_units,"external_paid_calls":0});
        std::fs::write(std::env::var("OPENCODE_V2_RECEIPT").unwrap(),serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    }).await;
}

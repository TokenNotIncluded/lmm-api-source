use super::*;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub(super) struct Case {
    pub host: pi::CliHost,
    pub client: &'static str,
    pub script: &'static str,
    pub env: &'static [&'static str],
    pub receipt_env: &'static str,
    pub scope: &'static str,
}

pub(super) async fn verify(pool: PgPool, case: Case) {
    let Case {
        host,
        client,
        script,
        env,
        receipt_env,
        scope,
    } = case;
    pi::with_cli_host_fixture(pool, host, move |f| async move {
        let before = f.ledger.balance(f.account).await.unwrap();
        let config = json!({
            "coreBase": f.base,
            "issuer": f.issuer.profile().resource().strip_suffix("/api/oauth2").unwrap(),
            "refresh": f.refresh, "cap": before.available_units
        });
        let proof: Value = tokio::task::spawn_blocking(move || {
            let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(script);
            let mut command = Command::new("node");
            command.arg(driver).env_clear();
            for &name in ["PATH", "FIXTURE_TLS_KEY", "FIXTURE_TLS_CERT", "NODE_EXTRA_CA_CERTS"]
                .iter().chain(env)
            {
                command.env(name, std::env::var_os(name).expect("missing provider host environment"));
            }
            let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(&serde_json::to_vec(&config).unwrap()).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "{client} native host failed");
            serde_json::from_slice(&output.stdout).unwrap()
        }).await.unwrap();
        let id = proof["requestId"].as_str().unwrap();
        assert_eq!(proof["terminal"], true);
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
            .fetch_one(&f.owner).await.unwrap();
        assert_eq!(count, 1);
        let (family, price, input, cached, output, settled, closed, captured, journal):
            (String, Value, i64, i64, i64, i64, bool, i64, i64) = sqlx::query_as(
            "SELECT r.oauth_family_id,r.price,r.input_tokens,r.cached_tokens,r.output_tokens,r.settled,h.closed,h.captured,h.capture_journal_id
             FROM core_charging.requests r JOIN core_charging.ledger_holds h ON h.request_id=r.id
             JOIN core_oauth.families f ON f.id=r.oauth_family_id
             WHERE r.id=$1 AND r.state='settled' AND r.oauth_client_id=$5
             AND r.oauth_source->>'client_id'=r.oauth_client_id AND r.key_id IS NULL
             AND r.payer_account_id=$2 AND r.actor_user_id=$3 AND f.client_id=r.oauth_client_id
             AND f.user_id=r.actor_user_id AND f.owner_account_id=r.payer_account_id
             AND f.issuance_kind='browser' AND f.purpose='normal_model' AND f.budget_credits=$4")
            .bind(id).bind(f.account).bind(f.user).bind(before.available_units).bind(client)
            .fetch_one(&f.owner).await.unwrap();
        assert_eq!((input, cached, output, settled, closed, captured), (10, 2, 20, 49, true, 49));
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        let group = URL_SAFE_NO_PAD.encode(price["group"].as_str().unwrap());
        let model = URL_SAFE_NO_PAD.encode(price["model"].as_str().unwrap());
        assert_eq!(proof["modelId"], format!("lmm:{group}:{model}"));
        assert_eq!(proof["group"], group);
        let after = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(before.available_units - after.available_units, settled);
        assert_eq!((before.reserved_units, after.reserved_units), (0, 0));
        let receipt = json!({
            "commit": std::env::var("GITHUB_SHA").unwrap(), "version": proof["version"], "client": client,
            "acceptance_scope": scope, "endpoint": "/v1/chat/completions", "request_id": id,
            "family_id": family, "actor": f.user, "payer": f.account, "api_key": false, "terminal": true,
            "group": price["group"], "model": price["model"], "input_tokens": input, "cached_tokens": cached,
            "output_tokens": output, "cap": before.available_units, "settled": settled, "capture_journal": journal,
            "supplier_hits": 1, "reserved_after": after.reserved_units, "external_paid_calls": 0
        });
        std::fs::write(std::env::var(receipt_env).unwrap(), serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    }).await;
}

//! Opt-in integration with a real headless ssmcpd, the Core ledger and Go store.
//! LMM_TEST_SSMCPD_BINARY selects the daemon under test. All data is disposable.
use super::{Client, StatusCode, Value, address, hex_codec, json, private, success};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

pub struct Relay {
    child: Child,
    pub settings: String,
}
impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn digest(b: &str) -> String {
    hex_codec::encode(Sha256::digest(b.as_bytes()))
}
impl Relay {
    pub async fn start(root: &Path, user: i64) -> Option<Self> {
        let binary = std::env::var_os("LMM_TEST_SSMCPD_BINARY")?;
        assert!(Path::new(&binary).is_absolute());
        let addr = address();
        let origin = format!("http://{addr}");
        let key = || {
            format!(
                "{:032x}{:032x}",
                rand::random::<u128>(),
                rand::random::<u128>()
            )
        };
        let billing = key();
        let token = private(root, "traffic-accounting-token", billing.as_bytes());
        let config = format!(
            "[server]\nlisten='{addr}'\npublic_url='https://relay.invalid'\n[relay]\ndevice_ingress_sha256='{}'\n[relay.principals]\nalice='{}'\n[relay.accounting]\nissuer='https://issuer.invalid'\nstate_dir='{}'\nservice_token_sha256='{}'\nutc_offset_minutes=480\nfree_monthly_bytes=100\npaid_monthly_bytes=100\n[relay.accounting.users]\nalice='{user}'\n",
            digest(&key()),
            digest(&key()),
            root.join("traffic-state").display(),
            digest(&billing)
        );
        let cfg = private(root, "traffic-relay.toml", config.as_bytes());
        let logger = File::create(root.join("traffic-relay.log")).unwrap();
        let child = Command::new(binary)
            .env_clear()
            .args(["serve", "--config"])
            .arg(cfg)
            .stdin(Stdio::null())
            .stdout(logger.try_clone().unwrap())
            .stderr(logger)
            .spawn()
            .unwrap();
        let shop = digest(r#"["shop",{"kind":"personal","id":9},"shop"]"#);
        let variant = digest(r#"["resource",9,"traffic-sku"]"#);
        let settings = format!(
            "[ssmcpd]\nmode='relay'\npublic_origin='https://relay.invalid'\n[ssmcpd.metering]\norigin='{origin}'\ntoken_file='{token}'\n[ssmcpd.commerce]\nshop_id='{shop}'\nvariant_id='{variant}'\nbytes_per_unit=300\n[ssmcpd.commerce.grants]\norigin='{origin}'\ntoken_file='{token}'\n",
            token = token.display()
        );
        let mut result = Self { child, settings };
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        for _ in 0..100 {
            if client
                .get(format!("{origin}/healthz"))
                .send()
                .await
                .is_ok_and(|v| v.status() == StatusCode::OK)
            {
                return Some(result);
            }
            assert!(
                result.child.try_wait().unwrap().is_none(),
                "isolated relay exited before readiness"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("isolated relay did not become ready");
    }
}
async fn api(
    c: &Client,
    ep: &str,
    user: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let url = format!("{ep}/api/plugins/ssmcpd/{path}");
    let req = match body {
        Some(b) => c.post(url).json(&b),
        None => c.get(url),
    };
    let r = req.bearer_auth(user).send().await.unwrap();
    let status = r.status();
    (status, r.json().await.unwrap())
}
pub async fn exercise(
    c: &Client,
    ep: &str,
    host: &str,
    buyer: &str,
    seller: &str,
    other: &str,
    shop: &str,
    product: &Value,
    store_owner: &sqlx::PgPool,
    core: &sqlx::PgPool,
) {
    success(c,ep,host,seller,"/catalog",json!({"action":"create_variant","shop_id":shop,"product_id":product["id"],"title":"Monthly traffic","price":{"amount_credits":17,"unit":"credits"},"quota":9,"key":"traffic-sku"})).await;
    let (code, offer) = api(c, ep, buyer, "offer", None).await;
    assert_eq!(code, StatusCode::OK, "{offer}");
    let q = &offer["data"];
    let cmd = json!({"request_key":"first-test-order","quantity":2,"unit_credits":q["unit_credits"],"bytes_per_unit":q["bytes_per_unit"],"period":q["period"]});
    let (code, buy) = api(c, ep, buyer, "orders", Some(cmd.clone())).await;
    assert_eq!(code, StatusCode::OK, "{buy}");
    let id = buy["data"]["order_id"].as_str().unwrap();
    assert_eq!(buy["data"]["credits"], "34");
    let again = api(c, ep, buyer, "orders", Some(cmd.clone())).await;
    assert_eq!(again.1["data"]["order_id"], id);
    let mut changed = cmd;
    changed["quantity"] = json!(3);
    assert_eq!(
        api(c, ep, buyer, "orders", Some(changed)).await.0,
        StatusCode::CONFLICT
    );
    let path = format!("orders/{id}/redeem");
    assert_eq!(
        api(c, ep, other, &path, Some(json!({}))).await.0,
        StatusCode::NOT_FOUND
    );
    // Drop only the local ACK after the real daemon has durably granted the
    // original voucher. A subsequent call must reconcile that same order.
    sqlx::raw_sql("CREATE FUNCTION lmm_store.lose_traffic_ack() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture lost entitlement ACK'; END $$; CREATE TRIGGER lose_traffic_ack BEFORE UPDATE ON lmm_store.orders FOR EACH ROW WHEN (NEW.body->'service_entitlement'->>'dispatch'='granted') EXECUTE FUNCTION lmm_store.lose_traffic_ack()").execute(store_owner).await.unwrap();
    let (lost, _) = api(c, ep, buyer, &path, Some(json!({}))).await;
    assert!(lost.is_server_error() || lost == StatusCode::ACCEPTED);
    let (_, observed) = api(c, ep, buyer, &format!("orders/{id}"), None).await;
    assert_eq!(observed["data"]["grant_state"], "unknown");
    let (_, usage) = api(c, ep, buyer, "usage", None).await;
    assert_eq!(usage["data"]["paid_bytes"], "600");
    sqlx::raw_sql("DROP TRIGGER lose_traffic_ack ON lmm_store.orders; DROP FUNCTION lmm_store.lose_traffic_ack()").execute(store_owner).await.unwrap();
    let (_, first) = api(c, ep, buyer, &path, Some(json!({}))).await;
    assert_eq!(first["data"]["grant_state"], "granted", "{first}");
    assert_eq!(first["data"]["money_state"], "settled");
    assert_eq!(first["data"]["bytes"], "600");
    let (_, replay) = api(c, ep, buyer, &path, Some(json!({}))).await;
    assert_eq!(replay["data"], first["data"]);
    let (code, usage) = api(c, ep, buyer, "usage", None).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(usage["data"]["paid_bytes"], "600");
    let facts:(i64,i64) = sqlx::query_as("SELECT o.credits,count(op.operation_key) FROM core_commerce.orders o JOIN core_commerce.operations op ON op.order_id=o.id WHERE o.order_id=$1 AND o.state='settled' GROUP BY o.credits").bind(id).fetch_one(core).await.unwrap();
    assert_eq!(facts, (34, 2));
    let (code, asked) = api(
        c,
        ep,
        buyer,
        &format!("orders/{id}/refund-request"),
        Some(json!({"reason":"fixture"})),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{asked}");
    assert_eq!(asked["data"]["refund_state"], "requested");
    success(
        c,
        ep,
        host,
        seller,
        "/orders/change",
        json!({"shop_id":shop,"order_id":id,"action":"refund","text":"fixture"}),
    )
    .await;
    let (_, after) = api(c, ep, buyer, &path, Some(json!({}))).await;
    assert_eq!(after["data"]["money_state"], "refunded");
    assert_eq!(after["data"]["grant_state"], "granted");
    let (_, usage) = api(c, ep, buyer, "usage", None).await;
    assert_eq!(usage["data"]["paid_bytes"], "600");
}

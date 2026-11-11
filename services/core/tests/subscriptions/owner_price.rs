use super::*;
use lmm_core::payments::{Channel, PaymentsStore};
use lmm_core::subscriptions::PublishPancakePrice;
use rsa::{RsaPrivateKey, pkcs8::EncodePublicKey};
use sqlx::postgres::PgPoolOptions;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};

#[sqlx::test(migrations = false)]
async fn owner_price_cli_uses_real_set_role_and_exact_existing_plan(pool: PgPool) {
    let suffix = format!("{:016x}", rand::random::<u64>());
    let owner = format!("price_owner_{suffix}");
    let installer = format!("price_installer_{suffix}");
    let runtime = format!("price_runtime_{suffix}");
    let database = pool.connect_options().get_database().unwrap().to_owned();
    assert!(
        database
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    );
    sqlx::raw_sql(&format!("CREATE ROLE {owner} NOLOGIN; CREATE ROLE {installer} LOGIN NOINHERIT; CREATE ROLE {runtime} LOGIN NOINHERIT; GRANT {owner} TO {installer}; ALTER DATABASE \"{database}\" OWNER TO {owner};"))
        .execute(&pool).await.unwrap();
    let set_role = format!("SET ROLE {owner}");
    let owner_pool = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(move |c, _| {
            let sql = set_role.clone();
            Box::pin(async move {
                sqlx::query(&sql).execute(c).await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone().username(&installer))
        .await
        .unwrap();
    let f = fixture(&owner_pool, true).await;
    let mut input = create("owner_existing_plan_01");
    input.terms.allow_balance_pay = false;
    let plan = f
        .store
        .save_plan(&f.admin, None, input.clone())
        .await
        .unwrap();
    input.operation_key = "http_existing_plan_02".into();
    let http_plan = f.store.save_plan(&f.admin, None, input).await.unwrap();
    let public = RsaPrivateKey::new(&mut rand::thread_rng(), 2048)
        .unwrap()
        .to_public_key()
        .to_public_key_pem(Default::default())
        .unwrap();
    let channel = Channel::pancake_isolated_test(
        5003,
        "merchant-fixture".into(),
        public.clone(),
        "STO_fixture".into(),
        "PROD_fixture".into(),
        "CNY".into(),
        100_000,
    )
    .unwrap();
    let payments = PaymentsStore::new(owner_pool.clone(), vec![channel]).unwrap();
    let request: PublishPancakePrice = serde_json::from_value(serde_json::json!({
        "operation_key":"owner_price_publish_01","expected_plan":plan,
        "price":{"plan_id":plan.id.to_string(),"plan_revision":plan.revision.to_string(),
            "channel_id":"5003","currency":"CNY","amount_minor":"990",
            "store_id":"STO_fixture","product_id":"PROD_fixture","product_type":"one_time",
            "billing_period":"","provider_plan_reference":"PLAN_fixture_owner","return_url":"https://example.test/return",
            "checkout_region":"china","checkout_language":"zh","tax_category":"digital_goods"}
    })).unwrap();
    let price = request.price.clone();
    let mut http_price = price.clone();
    http_price.plan_id = http_plan.id;
    assert_eq!(
        f.store
            .save_pancake_price(&payments, &f.admin, "normal_http_price_01", &http_price)
            .await
            .unwrap(),
        http_price
    );
    assert_eq!(
        f.store
            .save_pancake_price(&payments, &f.user, "normal_http_denied_01", &http_price)
            .await,
        Err(Error::Forbidden)
    );
    let mut wrong = request.clone();
    wrong.expected_plan.terms.limit_units = "999".into();
    assert_eq!(
        f.store
            .publish_pancake_price_as_owner(&payments, &wrong)
            .await,
        Err(Error::Conflict)
    );
    wrong = request.clone();
    wrong.price.return_url = "http://example.test/return".into();
    assert_eq!(
        f.store
            .publish_pancake_price_as_owner(&payments, &wrong)
            .await,
        Err(Error::Invalid)
    );
    wrong = request.clone();
    wrong.price.product_type = "subscription".into();
    wrong.price.billing_period = "monthly".into();
    assert_eq!(
        f.store
            .publish_pancake_price_as_owner(&payments, &wrong)
            .await,
        Err(Error::Invalid)
    );
    for field in [
        "provider_plan_reference",
        "checkout_region",
        "checkout_language",
        "tax_category",
    ] {
        let mut raw = serde_json::to_value(&price).unwrap();
        raw[field] = "".into();
        let mut invalid = request.clone();
        invalid.price = serde_json::from_value(raw).unwrap();
        assert_eq!(
            f.store
                .publish_pancake_price_as_owner(&payments, &invalid)
                .await,
            Err(Error::Invalid)
        );
        assert_eq!(
            f.store
                .save_pancake_price(
                    &payments,
                    &f.admin,
                    "invalid_price_metadata_01",
                    &invalid.price
                )
                .await,
            Err(Error::Invalid)
        );
    }
    sqlx::raw_sql(&format!("GRANT CONNECT ON DATABASE \"{database}\" TO {runtime}; GRANT USAGE ON SCHEMA core_identity,core_subscriptions TO {runtime}; GRANT EXECUTE ON FUNCTION core_identity.is_identity_relation_owner() TO {runtime};"))
        .execute(&pool).await.unwrap();
    let runtime_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone().username(&runtime))
        .await
        .unwrap();
    assert_eq!(
        SubscriptionStore::new(runtime_pool.clone())
            .publish_pancake_price_as_owner(&payments, &request)
            .await,
        Err(Error::Forbidden)
    );
    runtime_pool.close().await;
    // Publication remains possible while the imported publisher is dormant.
    sqlx::query("UPDATE core_identity.users SET active=false WHERE id=8301")
        .execute(&owner_pool)
        .await
        .unwrap();
    let before:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_identity.users),(SELECT count(*) FROM core_identity.credentials),(SELECT count(*) FROM core_subscriptions.plans),(SELECT count(*) FROM core_billing.ledger_entries)")
        .fetch_one(&owner_pool).await.unwrap();
    let dir = std::env::temp_dir().join(format!("lmm-owner-price-{suffix}"));
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    };
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_username(&installer).unwrap();
    url.set_path(&format!("/{database}"));
    url.query_pairs_mut()
        .append_pair("options", &format!("-c role={owner}"));
    let dbfile = write("owner-url", url.as_str().as_bytes());
    let pubfile = write("provider.pem", public.as_bytes());
    let tokenfile = write("rpc-token", b"synthetic-rpc-token-owner-publication");
    let payment = format!(
        "version=2\n[permissions]\n[[channels]]\nid=5003\nprovider='waffo_pancake'\nmerchant='merchant-fixture'\nenvironment='test'\ncurrency='CNY'\ncredits_per_minor=5000\nmax_amount_minor=100000\nwebhook_secret_file={}\nstore_id='STO_fixture'\nproduct_id='PROD_fixture'\n",
        serde_json::to_string(&pubfile).unwrap()
    );
    let payfile = write("payments.toml", payment.as_bytes());
    let config = format!(
        "version=1\n[database]\nurl_file={}\n[payments]\nconfig_file={}\ntoken_file={}\n",
        serde_json::to_string(&dbfile).unwrap(),
        serde_json::to_string(&payfile).unwrap(),
        serde_json::to_string(&tokenfile).unwrap()
    );
    let cfgfile = write("owner.toml", config.as_bytes());
    let command = || {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lmm-core"))
            .args([
                "--config",
                cfgfile.to_str().unwrap(),
                "publish-subscription-pancake-price",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(&serde_json::to_vec(&serde_json::json!({"operation_key":request.operation_key,"expected_plan":request.expected_plan,"price":request.price})).unwrap()).unwrap();
        child.wait_with_output().unwrap()
    };
    for _ in 0..2 {
        let output = command();
        assert!(
            output.status.success(),
            "owner CLI failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(reply["status"], "published");
    }
    wrong = request.clone();
    wrong.price.amount_minor = "991".into();
    assert_eq!(
        f.store
            .publish_pancake_price_as_owner(&payments, &wrong)
            .await,
        Err(Error::Conflict)
    );
    let after:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_identity.users),(SELECT count(*) FROM core_identity.credentials),(SELECT count(*) FROM core_subscriptions.plans),(SELECT count(*) FROM core_billing.ledger_entries)")
        .fetch_one(&owner_pool).await.unwrap();
    assert_eq!(before, after);
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_subscriptions.external_prices),(SELECT count(*) FROM core_subscriptions.pancake_prices)").fetch_one(&owner_pool).await.unwrap();
    assert_eq!(counts, (2, 2));
    owner_pool.close().await;
    fs::remove_dir_all(dir).unwrap();
}

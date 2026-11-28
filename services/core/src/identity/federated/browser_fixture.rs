//! Hosted browser fixture for the native provider binding. Only provider endpoints
//! and credentials are synthetic. OAuth flow storage, exchange and ownership run in Core.
use super::{ProviderConfiguration, ProviderKind, network};
use crate::identity::IdentityStore;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Startup {
    authorization_origin: String,
    api_origin: String,
    client_id: String,
    secret: String,
    slug: String,
    subject: String,
    spki: String,
    certificate_file: String,
}

pub(crate) struct BrowserProvider {
    child: Child,
    id: i64,
    name: String,
    data: Startup,
    users_before: i64,
}
impl Drop for BrowserProvider {
    fn drop(&mut self) {
        // EOF closes both listeners and disposes of the short-lived private CA.
        drop(self.child.stdin.take());
        for _ in 0..30 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) async fn start(
    identity: IdentityStore,
    pool: &PgPool,
    nonce: &str,
    web_origin: &str,
) -> (IdentityStore, BrowserProvider) {
    let mut child = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/console/browser_acceptance/account-link-provider.mjs"
        ))
        .arg(web_origin)
        .arg(nonce)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("RUNNER_TEMP", std::env::var_os("RUNNER_TEMP").unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("private account-link provider");
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    assert!(output.read_line(&mut line).unwrap() > 0);
    let mut data: Startup = serde_json::from_str(&line).unwrap();
    let mut store = identity
        .with_account_links(true)
        .with_federated_keys(
            "browser-fixture".into(),
            vec![("browser-fixture".into(), vec![79; 32])],
            vec![80; 32],
        )
        .unwrap();
    Arc::get_mut(store.federated.as_mut().unwrap())
        .unwrap()
        .network = network::ProviderNetwork::local_for_test();
    let kind = ProviderKind::CustomOauth;
    let mut configuration = ProviderConfiguration {
        client_id: data.client_id.clone(),
        authorization_endpoint: format!("{}/authorize", data.authorization_origin),
        token_endpoint: format!("{}/token", data.api_origin),
        user_info_endpoint: format!("{}/user", data.api_origin),
        scopes: "profile".into(),
        pkce_s256: true,
        ..Default::default()
    };
    store
        .validate_federated_configuration(kind, &mut configuration)
        .unwrap();
    let context = Sha256::digest(nonce.as_bytes()).to_vec();
    let (key, sealed) = store
        .seal_federated_secret(&context, 1, &data.secret)
        .unwrap();
    zeroize::Zeroize::zeroize(&mut data.secret);
    zeroize::Zeroize::zeroize(&mut line);
    let name = format!("Browser identity {nonce}");
    let id = sqlx::query_scalar("INSERT INTO core_federated_auth.providers(kind,slug,name,enabled,configuration,secret_context,secret_key_id,sealed_secret) VALUES($1,$2,$3,true,$4,$5,$6,$7) RETURNING id")
        .bind(kind.as_str()).bind(&data.slug).bind(&name).bind(sqlx::types::Json(configuration))
        .bind(context).bind(key).bind(sealed).fetch_one(pool).await.unwrap();
    let users_before = sqlx::query_scalar("SELECT count(*) FROM core_identity.users")
        .fetch_one(pool)
        .await
        .unwrap();
    (
        store,
        BrowserProvider {
            child,
            id,
            name,
            data,
            users_before,
        },
    )
}

impl BrowserProvider {
    pub(crate) fn descriptor(&self) -> Value {
        json!({"name":self.name,"id":self.id.to_string(),"slug":self.data.slug,"subject":self.data.subject,
            "authorizationOrigin":self.data.authorization_origin,"certificateFile":self.data.certificate_file,"spki":self.data.spki})
    }

    pub(crate) async fn verify(&self, pool: &PgPool, expected_user: i64) -> Value {
        let owners: Vec<(i64, String, i64, bool)> = sqlx::query_as("SELECT user_id,subject,revision,revoked_at IS NULL FROM core_federated_auth.identities WHERE provider_id=$1")
            .bind(self.id).fetch_all(pool).await.unwrap();
        assert_eq!(
            owners.as_slice(),
            &[(expected_user, self.data.subject.clone(), 1, true)]
        );
        let users: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.users")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(users, self.users_before);
        let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.audit WHERE actor_user_id=$1 AND action='account.federated_link' AND target->>'user_id'=$1::bigint::text AND target ? 'user_version'")
            .bind(expected_user).fetch_one(pool).await.unwrap();
        assert_eq!(audited, 1);
        let calls: Value = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
            .get(format!("{}/counts", self.data.api_origin))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(calls, json!({"approved":1, "token":1, "userinfo":1}));
        json!({"provider_id":self.id,"owner_user_id":expected_user,"active_bindings":owners.len(),"revision":1,
            "provider_calls":calls,"new_core_users":users-self.users_before,"authority_rotation_audits":audited})
    }
}

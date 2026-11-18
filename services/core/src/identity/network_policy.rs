//! User-owned source networks, intersected with platform and API-key policy.
use super::*;
use ipnet::IpNet;
use serde::Deserialize;
use std::{future::Future, net::IpAddr};

const KEY: &str = "ip_allowlist";
const MAX_NETWORKS: usize = 128;

tokio::task_local! {
    static REQUEST_IP: Option<IpAddr>;
}

/// Scope trusted transport metadata to this HTTP request, including its awaits.
/// Internal callers without an HTTP scope retain their existing authority path.
pub async fn scope_request_ip<F: Future>(peer: Option<IpAddr>, request: F) -> F::Output {
    REQUEST_IP.scope(peer.map(canonical_ip), request).await
}

pub(crate) async fn authorize_request_ip(c: &mut PgConnection, user_id: i64) -> Result<()> {
    match REQUEST_IP.try_with(|peer| *peer) {
        Ok(peer) => authorize_user_ip(c, user_id, peer).await,
        Err(_) => Ok(()),
    }
}

#[derive(Debug)]
pub(crate) enum AllowlistError {
    Identity(IdentityError),
    Invalid,
    Lockout,
}
impl From<IdentityError> for AllowlistError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}
impl From<sqlx::Error> for AllowlistError {
    fn from(error: sqlx::Error) -> Self {
        Self::Identity(error.into())
    }
}
type PolicyResult<T> = std::result::Result<T, AllowlistError>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AllowlistUpdate {
    pub allow_ips: Vec<String>,
}
#[derive(Serialize)]
pub(crate) struct AllowlistView {
    pub allow_ips: Vec<String>,
    pub client_ip: Option<IpAddr>,
}

pub(crate) fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4),
        _ => ip,
    }
}

fn network(raw: &str) -> PolicyResult<IpNet> {
    if raw.len() > 64 {
        return Err(AllowlistError::Invalid);
    }
    let raw = raw.trim();
    let net = if let Ok(ip) = raw.parse::<IpAddr>() {
        IpNet::from(canonical_ip(ip))
    } else {
        let mut net: IpNet = raw.parse().map_err(|_| AllowlistError::Invalid)?;
        if let IpNet::V6(v6) = net
            && let Some(v4) = v6.addr().to_ipv4_mapped()
        {
            let prefix = v6
                .prefix_len()
                .checked_sub(96)
                .ok_or(AllowlistError::Invalid)?;
            net = IpNet::new(IpAddr::V4(v4), prefix).map_err(|_| AllowlistError::Invalid)?;
        }
        net
    };
    Ok(net.trunc())
}
fn parse_networks(input: &[String]) -> PolicyResult<Vec<IpNet>> {
    if input.len() > MAX_NETWORKS {
        return Err(AllowlistError::Invalid);
    }
    let mut result = Vec::with_capacity(input.len());
    for raw in input {
        let net = network(raw)?;
        if !result.contains(&net) {
            result.push(net);
        }
    }
    Ok(result)
}
fn names(networks: &[IpNet]) -> Vec<String> {
    networks
        .iter()
        .map(|net| {
            if net.prefix_len() == if net.addr().is_ipv4() { 32 } else { 128 } {
                net.addr().to_string()
            } else {
                net.to_string()
            }
        })
        .collect()
}
fn allows(networks: &[IpNet], peer: Option<IpAddr>) -> bool {
    networks.is_empty()
        || peer.is_some_and(|ip| {
            let ip = canonical_ip(ip);
            networks.iter().any(|network| network.contains(&ip))
        })
}
fn document(raw: &str) -> Result<Value> {
    // Legacy imports preserve empty settings as an unconfigured profile.
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(Value::is_object)
        .ok_or(IdentityError::Storage)
}
fn stored_networks(settings: &Value) -> Result<Vec<IpNet>> {
    let Some(value) = settings.get(KEY) else {
        return Ok(Vec::new());
    };
    let values: Vec<String> =
        serde_json::from_value(value.clone()).map_err(|_| IdentityError::Storage)?;
    parse_networks(&values).map_err(|_| IdentityError::Storage)
}

/// Caller supplies the user from verified authority and the actual request peer.
pub(crate) async fn authorize_user_ip(
    c: &mut PgConnection,
    user_id: i64,
    peer: Option<IpAddr>,
) -> Result<()> {
    let raw: String =
        sqlx::query_scalar("SELECT setting FROM core_identity.user_profiles WHERE user_id=$1")
            .bind(user_id)
            .fetch_one(c)
            .await?;
    if allows(&stored_networks(&document(&raw)?)?, peer) {
        Ok(())
    } else {
        Err(IdentityError::Forbidden)
    }
}

impl IdentityStore {
    pub(crate) async fn user_ip_allowlist(
        &self,
        secret: &str,
        peer: Option<IpAddr>,
    ) -> PolicyResult<AllowlistView> {
        let mut tx = self.pool.begin().await?;
        let actor = session(&mut tx, secret).await?;
        let raw: String =
            sqlx::query_scalar("SELECT setting FROM core_identity.user_profiles WHERE user_id=$1")
                .bind(actor.user_id)
                .fetch_one(&mut *tx)
                .await?;
        let networks = stored_networks(&document(&raw)?)?;
        if !allows(&networks, peer) {
            return Err(IdentityError::Forbidden.into());
        }
        tx.commit().await?;
        Ok(AllowlistView {
            allow_ips: names(&networks),
            client_ip: peer.map(canonical_ip),
        })
    }

    pub(crate) async fn update_user_ip_allowlist(
        &self,
        secret: &str,
        input: &AllowlistUpdate,
        peer: Option<IpAddr>,
    ) -> PolicyResult<AllowlistView> {
        let networks = parse_networks(&input.allow_ips)?;
        if !allows(&networks, peer) {
            return Err(AllowlistError::Lockout);
        }
        let mut tx = self.pool.begin().await?;
        let actor = session(&mut tx, secret).await?;
        let raw: String = sqlx::query_scalar(
            "SELECT setting FROM core_identity.user_profiles WHERE user_id=$1 FOR UPDATE",
        )
        .bind(actor.user_id)
        .fetch_one(&mut *tx)
        .await?;
        let mut settings = document(&raw)?;
        if !allows(&stored_networks(&settings)?, peer) {
            return Err(IdentityError::Forbidden.into());
        }
        let allow_ips = names(&networks);
        settings[KEY] = json!(allow_ips);
        sqlx::query("UPDATE core_identity.user_profiles SET setting=$2 WHERE user_id=$1")
            .bind(actor.user_id)
            .bind(settings.to_string())
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            &actor,
            "user.ip_allowlist.updated",
            json!({"user_id":actor.user_id,"network_count":allow_ips.len()}),
        )
        .await?;
        tx.commit().await?;
        Ok(AllowlistView {
            allow_ips,
            client_ip: peer.map(canonical_ip),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(values: &[&str]) -> Vec<IpNet> {
        parse_networks(
            &values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }
    #[test]
    fn user_ip_allowlist_normalizes_hosts_networks_and_mapped_ipv6() {
        let networks = parsed(&[
            "192.0.2.7",
            "192.0.2.7/32",
            "2001:db8::abcd/64",
            "::ffff:198.51.100.4/120",
            "::ffff:192.0.2.7",
        ]);
        assert_eq!(
            names(&networks),
            ["192.0.2.7", "2001:db8::/64", "198.51.100.0/24"]
        );
        for ip in [
            "192.0.2.7",
            "::ffff:192.0.2.7",
            "2001:db8::ffff",
            "198.51.100.255",
        ] {
            assert!(allows(&networks, Some(ip.parse().unwrap())), "{ip}");
        }
        assert!(!allows(&networks, Some("192.0.2.8".parse().unwrap())));
        assert!(!allows(&networks, None));
        assert!(allows(&[], None));
    }
    #[test]
    fn user_ip_allowlist_rejects_invalid_networks_and_bounds_input() {
        for value in [
            "",
            "localhost",
            "192.0.2.1:443",
            "192.0.2.0/33",
            "::/129",
            "192.0.2.1,192.0.2.2",
            "::ffff:192.0.2.1/95",
            "2001:db8::1%eth0",
        ] {
            assert!(parse_networks(&[value.into()]).is_err(), "{value}");
        }
        assert!(parse_networks(&vec!["192.0.2.1".into(); 129]).is_err());
        assert!(parse_networks(&vec!["192.0.2.1".into(); 128]).is_ok());
        assert_eq!(document(" \t\n").unwrap(), json!({}));
        for raw in ["{invalid", "null", "[]"] {
            assert!(document(raw).is_err());
        }
        for value in [
            json!(null),
            json!("192.0.2.1"),
            json!([123]),
            json!(["invalid"]),
        ] {
            assert!(stored_networks(&json!({KEY:value})).is_err());
        }
    }

    #[sqlx::test(migrations = false)]
    async fn user_ip_allowlist_persists_isolates_users_and_prevents_lockout(pool: PgPool) {
        let store = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
        store.init_database().await.unwrap();
        let session = store
            .register(&LoginRequest {
                login_name: "ip-policy-user".into(),
                password: "test password 123".into(),
            })
            .await
            .unwrap();
        let actor = store.authorize(&session.secret).await.unwrap();
        let peer = Some("192.0.2.7".parse().unwrap());
        sqlx::query("UPDATE core_identity.user_profiles SET setting='' WHERE user_id=$1")
            .bind(actor.user_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            scope_request_ip(peer, store.authorize(&session.secret))
                .await
                .is_ok()
        );
        assert!(
            store
                .user_ip_allowlist(&session.secret, peer)
                .await
                .unwrap()
                .allow_ips
                .is_empty()
        );
        sqlx::query("UPDATE core_identity.user_profiles SET setting=$2 WHERE user_id=$1")
            .bind(actor.user_id)
            .bind(r#"{"language":"zh-CN"}"#)
            .execute(&pool)
            .await
            .unwrap();
        let update = AllowlistUpdate {
            allow_ips: vec!["192.0.2.0/24".into(), "2001:db8::/64".into()],
        };
        let saved = store
            .update_user_ip_allowlist(&session.secret, &update, peer)
            .await
            .unwrap();
        assert_eq!(saved.allow_ips, update.allow_ips);
        let reopened = IdentityStore::from_pool(pool.clone());
        assert_eq!(
            reopened
                .user_ip_allowlist(&session.secret, peer)
                .await
                .unwrap()
                .allow_ips,
            update.allow_ips
        );
        let blocked = Some("198.51.100.8".parse().unwrap());
        assert!(matches!(
            reopened.user_ip_allowlist(&session.secret, blocked).await,
            Err(AllowlistError::Identity(IdentityError::Forbidden))
        ));
        assert!(matches!(
            reopened
                .update_user_ip_allowlist(
                    &session.secret,
                    &AllowlistUpdate {
                        allow_ips: vec!["198.51.100.0/24".into()]
                    },
                    peer
                )
                .await,
            Err(AllowlistError::Lockout)
        ));
        assert!(matches!(
            reopened
                .update_user_ip_allowlist(
                    &session.secret,
                    &AllowlistUpdate { allow_ips: vec![] },
                    blocked
                )
                .await,
            Err(AllowlistError::Identity(IdentityError::Forbidden))
        ));
        let raw: String =
            sqlx::query_scalar("SELECT setting FROM core_identity.user_profiles WHERE user_id=$1")
                .bind(actor.user_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(document(&raw).unwrap()["language"], "zh-CN");
        let mut tx = pool.begin().await.unwrap();
        assert!(
            authorize_user_ip(&mut tx, actor.user_id, peer)
                .await
                .is_ok()
        );
        assert_eq!(
            authorize_user_ip(&mut tx, actor.user_id, blocked).await,
            Err(IdentityError::Forbidden)
        );
        tx.rollback().await.unwrap();
        let other = store
            .register(&LoginRequest {
                login_name: "other-ip-user".into(),
                password: "test password 456".into(),
            })
            .await
            .unwrap();
        assert!(
            store
                .user_ip_allowlist(&other.secret, blocked)
                .await
                .unwrap()
                .allow_ips
                .is_empty()
        );
        reopened
            .update_user_ip_allowlist(
                &session.secret,
                &AllowlistUpdate { allow_ips: vec![] },
                peer,
            )
            .await
            .unwrap();
        assert!(
            reopened
                .user_ip_allowlist(&session.secret, blocked)
                .await
                .unwrap()
                .allow_ips
                .is_empty()
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_identity.audit WHERE action='user.ip_allowlist.updated'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 2);
    }
    #[tokio::test]
    async fn user_ip_allowlist_scope_is_restored_and_not_inherited_by_background_tasks() {
        let peer = Some("192.0.2.7".parse().unwrap());
        assert!(REQUEST_IP.try_with(|value| *value).is_err());
        scope_request_ip(peer, async {
            tokio::task::yield_now().await;
            assert_eq!(REQUEST_IP.with(|value| *value), peer);
            scope_request_ip(None, async {
                assert_eq!(REQUEST_IP.with(|value| *value), None);
            })
            .await;
            assert_eq!(REQUEST_IP.with(|value| *value), peer);
            assert!(
                tokio::spawn(async { REQUEST_IP.try_with(|value| *value).is_err() })
                    .await
                    .unwrap()
            );
        })
        .await;
        assert!(REQUEST_IP.try_with(|value| *value).is_err());
    }

    #[sqlx::test(migrations = false)]
    async fn user_ip_allowlist_protects_sessions_login_refresh_and_cookie_recovery(pool: PgPool) {
        let store = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
        store.init_database().await.unwrap();
        let input = LoginRequest {
            login_name: "network-session-user".into(),
            password: "test password 123".into(),
        };
        let session = store.register(&input).await.unwrap();
        let browser = BrowserAuth::new(store.clone(), &[42; 32]).unwrap();
        let bundle = browser
            .login(
                &input.login_name,
                &input.password,
                &BrowserSessionMetadata {
                    ip: "192.0.2.7".into(),
                    user_agent: "isolated network-policy test".into(),
                },
            )
            .await
            .unwrap();
        let peer = Some("192.0.2.7".parse().unwrap());
        let blocked = Some("198.51.100.8".parse().unwrap());
        store
            .update_user_ip_allowlist(
                &session.secret,
                &AllowlistUpdate {
                    allow_ips: vec!["192.0.2.0/24".into()],
                },
                peer,
            )
            .await
            .unwrap();
        assert!(
            scope_request_ip(peer, store.authorize(&session.secret))
                .await
                .is_ok()
        );
        assert!(matches!(
            scope_request_ip(blocked, store.authorize(&session.secret)).await,
            Err(IdentityError::Forbidden)
        ));
        assert!(matches!(
            scope_request_ip(None, store.authorize(&session.secret)).await,
            Err(IdentityError::Forbidden)
        ));
        let before: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.credentials")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(matches!(
            scope_request_ip(blocked, store.login(&input)).await,
            Err(IdentityError::Forbidden)
        ));
        assert!(matches!(
            scope_request_ip(
                blocked,
                browser.login(
                    &input.login_name,
                    &input.password,
                    &BrowserSessionMetadata::default()
                )
            )
            .await,
            Err(BrowserAuthError::Identity(IdentityError::Forbidden))
        ));
        assert!(matches!(
            scope_request_ip(blocked, browser.refresh(&bundle.refresh_token, None)).await,
            Err(BrowserAuthError::Identity(IdentityError::Forbidden))
        ));
        assert!(matches!(
            scope_request_ip(
                blocked,
                browser.reconcile_session(Some(&bundle.refresh_token), None)
            )
            .await,
            Err(BrowserAuthError::Identity(IdentityError::Forbidden))
        ));
        let mut tx = pool.begin().await.unwrap();
        assert!(matches!(
            scope_request_ip(
                blocked,
                BrowserAuth::oauth_identity_in_transaction(&mut tx, &bundle.refresh_token)
            )
            .await,
            Err(IdentityError::Forbidden)
        ));
        tx.rollback().await.unwrap();
        let after: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.credentials")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before, after);
        // Rejected requests leave the existing cookie/session usable from an allowed IP.
        assert!(
            scope_request_ip(peer, browser.refresh(&bundle.refresh_token, None))
                .await
                .is_ok()
        );
        assert!(scope_request_ip(peer, store.login(&input)).await.is_ok());
        assert!(store.authorize(&session.secret).await.is_ok());
    }

    #[sqlx::test(migrations = false)]
    async fn user_ip_allowlist_intersects_key_policy_without_http_context(pool: PgPool) {
        let store = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
        store.init_database().await.unwrap();
        // Issuing a key also reads the abuse policy; identity-only installation
        // deliberately does not create that separate component.
        sqlx::raw_sql(sqlx::AssertSqlSafe(crate::abuse::SCHEMA))
            .execute(&pool)
            .await
            .unwrap();
        let session = store
            .register(&LoginRequest {
                login_name: "network-key-user".into(),
                password: "test password 123".into(),
            })
            .await
            .unwrap();
        let actor = store.authorize(&session.secret).await.unwrap();
        let key = store
            .issue_key(
                &session.secret,
                Account {
                    kind: AccountKind::Personal,
                    id: actor.user_id,
                },
                FundingPolicy::new_personal_key(),
                3600,
            )
            .await
            .unwrap();
        sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,allow_ips) VALUES ($1,'network-policy test','default',ARRAY['192.0.2.7'::inet])")
            .bind(key.id).execute(&pool).await.unwrap();
        store
            .update_user_ip_allowlist(
                &session.secret,
                &AllowlistUpdate {
                    allow_ips: vec!["192.0.2.0/24".into()],
                },
                Some("192.0.2.7".parse().unwrap()),
            )
            .await
            .unwrap();
        assert!(
            store
                .authorize_model_request(
                    &key.secret,
                    "model",
                    Some("192.0.2.7".parse().unwrap()),
                    "default"
                )
                .await
                .is_ok()
        );
        for ip in ["192.0.2.8", "198.51.100.8"] {
            assert!(matches!(
                store
                    .authorize_model_request(
                        &key.secret,
                        "model",
                        Some(ip.parse().unwrap()),
                        "default"
                    )
                    .await,
                Err(IdentityError::Forbidden)
            ));
        }
        assert!(matches!(
            store
                .authorize_model_request(&key.secret, "model", None, "default")
                .await,
            Err(IdentityError::Forbidden)
        ));
        assert!(matches!(
            store
                .user_ip_allowlist(&key.secret, Some("192.0.2.7".parse().unwrap()))
                .await,
            Err(AllowlistError::Identity(IdentityError::Forbidden))
        ));
    }
}

//! Continuation of source-owned cc52 decisions. This module cannot create an
//! imported offer, qualify a historical user as Google-verified, or credit an
//! already claimed source gift. Only an authenticated claim of an immutable
//! offered row may append one canonical ledger transaction.
use super::{Error, Receipt};
use crate::billing::{
    Credits,
    ledger::{Action, Outcome, Request, Wallet},
};
use sqlx::{PgConnection, Row, types::Json};
use std::net::IpAddr;

#[path = "legacy_text.rs"]
mod text;

pub const SCHEMA: &str = include_str!("../../schema/gift_legacy_continuation.sql");

#[cfg(test)]
#[path = "legacy_tests.rs"]
mod tests;

struct Installation {
    namespace: String,
    secret: Vec<u8>,
    oidc_issuer: Option<String>,
    enabled: bool,
    cap: i64,
}

async fn installation(c: &mut PgConnection) -> Result<Option<Installation>, Error> {
    let row = sqlx::query("SELECT i.namespace,i.risk_secret,i.oidc_issuer,i.source_enabled AND p.enabled AS enabled,LEAST(i.inclusive_cap+1,p.exclusive_cap) AS cap,COALESCE(r.input_sha256=i.input_sha256,FALSE) AS committed FROM core_gift.legacy_installations i CROSS JOIN core_gift.policy p LEFT JOIN core_gift.legacy_imports r ON r.namespace=i.namespace WHERE i.singleton AND p.singleton")
        .fetch_optional(c).await?;
    row.map(|r| {
        if !r.try_get::<bool, _>("committed")? {
            return Err(Error::Unavailable);
        }
        Ok(Installation {
            namespace: r.try_get("namespace")?,
            secret: r.try_get("risk_secret")?,
            oidc_issuer: r.try_get("oidc_issuer")?,
            enabled: r.try_get("enabled")?,
            cap: r.try_get("cap")?,
        })
    })
    .transpose()
}

async fn subject(c: &mut PgConnection, namespace: &str, user: i64) -> Result<String, Error> {
    let source: Option<i64> = sqlx::query_scalar("SELECT b.source_user_id FROM core_gift.legacy_bindings b JOIN core_identity.users u ON u.id=b.user_id AND u.personal_account_id=b.account_id WHERE b.namespace=$1 AND b.user_id=$2")
        .bind(namespace).bind(user).fetch_optional(c).await?;
    Ok(match source {
        Some(id) => format!("source:{id}"),
        None => format!("native:{user}"),
    })
}

async fn identity(
    c: &mut PgConnection,
    i: &Installation,
    user: i64,
) -> Result<Option<Vec<u8>>, Error> {
    let email: Option<String> = sqlx::query_scalar("SELECT p.email FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id=$1 AND u.active AND a.active AND a.kind='personal' AND u.platform_role='user' FOR SHARE OF u,a,p")
        .bind(user).fetch_optional(&mut *c).await?;
    let Some(email) = email else {
        return Ok(None);
    };
    if let Some(value) = text::canonical_email(&email) {
        return Ok(Some(text::hash(
            &i.secret,
            "assistant-gift-identity-v1:",
            &value,
        )?));
    }
    let oauth = sqlx::query("SELECT provider,subject FROM core_gift.legacy_oauth_subject($1,$2)")
        .bind(user)
        .bind(&i.oidc_issuer)
        .fetch_optional(c)
        .await?;
    oauth
        .map(|r| {
            text::hash(
                &i.secret,
                "assistant-registration-oauth-v1:",
                &format!(
                    "{}:{}",
                    r.try_get::<String, _>("provider")?,
                    r.try_get::<String, _>("subject")?
                ),
            )
        })
        .transpose()
}

fn network(i: &Installation, ip: IpAddr) -> Result<Vec<u8>, Error> {
    let ip = match ip {
        IpAddr::V6(v) => v.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        _ => ip,
    };
    text::hash(&i.secret, "assistant-gift-network-v1:", &ip.to_string())
}

async fn now(c: &mut PgConnection) -> Result<i64, Error> {
    Ok(
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(c)
            .await?,
    )
}

/// The caller authenticated the original Session and verified the edge proof.
/// Absence of an imported installation is an explicitly unconfigured feature.
pub(super) async fn observe(c: &mut PgConnection, user: i64, ip: IpAddr) -> Result<(), Error> {
    let Some(i) = installation(c).await? else {
        return Ok(());
    };
    let Some(identity) = identity(c, &i, user).await? else {
        return Ok(());
    };
    let subject = subject(c, &i.namespace, user).await?;
    let network = network(&i, ip)?;
    let now = now(c).await?;
    sqlx::query("INSERT INTO core_gift.legacy_profiles(namespace,subject,identity_hash,network_hash,observed_at) VALUES($1,$2,$3,$4,$5) ON CONFLICT(namespace,subject) DO UPDATE SET identity_hash=EXCLUDED.identity_hash,network_hash=EXCLUDED.network_hash,observed_at=EXCLUDED.observed_at")
        .bind(i.namespace).bind(subject).bind(identity).bind(network).bind(now).execute(c).await?;
    Ok(())
}

pub(super) async fn record_text(
    c: &mut PgConnection,
    user: i64,
    message: &str,
) -> Result<(), Error> {
    let Some(i) = installation(c).await? else {
        return Ok(());
    };
    let subject = subject(c, &i.namespace, user).await?;
    let now = now(c).await?;
    if profile(c, &i, user, &subject, None, now).await?.is_none() {
        return Ok(());
    }
    let (digest, tokens) = text::fingerprints(&i.secret, message)?;
    for token in tokens {
        sqlx::query("INSERT INTO core_gift.legacy_fingerprints(namespace,subject,message_digest,token,created_at) VALUES($1,$2,$3,$4,$5) ON CONFLICT(namespace,subject,message_digest,token) DO NOTHING")
            .bind(&i.namespace).bind(&subject).bind(&digest).bind(token).bind(now).execute(&mut *c).await?;
    }
    sqlx::query("DELETE FROM core_gift.legacy_fingerprints WHERE namespace=$1 AND subject=$2 AND (created_at<$3-604800 OR id IN(SELECT id FROM core_gift.legacy_fingerprints WHERE namespace=$1 AND subject=$2 ORDER BY id DESC OFFSET 256))")
        .bind(i.namespace).bind(subject).bind(now).execute(c).await?;
    Ok(())
}

pub(super) async fn has_decision(c: &mut PgConnection, user: i64) -> Result<bool, Error> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_gift.legacy_decisions d JOIN core_gift.legacy_bindings b USING(namespace,source_user_id) WHERE b.user_id=$1)")
        .bind(user).fetch_one(c).await?)
}

struct Profile {
    identity: Vec<u8>,
    network: Vec<u8>,
    observed: i64,
}

async fn profile(
    c: &mut PgConnection,
    i: &Installation,
    user: i64,
    subject: &str,
    peer: Option<IpAddr>,
    time: i64,
) -> Result<Option<Profile>, Error> {
    let Some(current) = identity(c, i, user).await? else {
        return Ok(None);
    };
    if let Some(ip) = peer {
        return Ok(Some(Profile {
            identity: current,
            network: network(i, ip)?,
            observed: time,
        }));
    }
    let row = sqlx::query("SELECT identity_hash,network_hash,observed_at FROM core_gift.legacy_profiles WHERE namespace=$1 AND subject=$2")
        .bind(&i.namespace).bind(subject).fetch_optional(c).await?;
    let Some(r) = row else {
        return Ok(None);
    };
    let stored = Profile {
        identity: r.try_get("identity_hash")?,
        network: r.try_get("network_hash")?,
        observed: r.try_get("observed_at")?,
    };
    if stored.identity != current || stored.observed > time + 60 || time - stored.observed > 900 {
        return Ok(None);
    }
    Ok(Some(stored))
}

async fn campaign_hold(
    c: &mut PgConnection,
    i: &Installation,
    subject: &str,
    network: &[u8],
    time: i64,
) -> Result<bool, Error> {
    // Exact source >= boundaries. Existing offers do not require two messages;
    // with fewer messages this correlated campaign predicate simply is false.
    let row = sqlx::query("WITH own AS(SELECT DISTINCT message_digest FROM core_gift.legacy_fingerprints WHERE namespace=$1 AND subject=$2 AND created_at>=$4-604800), peers AS(SELECT f.subject FROM core_gift.legacy_fingerprints f WHERE f.namespace=$1 AND f.subject<>$2 AND f.created_at>=$4-604800 AND f.token IN(SELECT token FROM core_gift.legacy_fingerprints WHERE namespace=$1 AND subject=$2 AND created_at>=$4-604800) AND (SELECT count(*) FROM own)>=2 GROUP BY f.subject HAVING count(DISTINCT f.message_digest)>=2 AND count(DISTINCT f.token)>=4) SELECT count(*) AS templates,count(*) FILTER(WHERE EXISTS(SELECT 1 FROM core_gift.legacy_profiles p WHERE p.namespace=$1 AND p.subject=peers.subject AND p.network_hash=$3 AND p.observed_at>=$4-604800)) AS networks FROM peers")
        .bind(&i.namespace).bind(subject).bind(network).bind(time).fetch_one(c).await?;
    Ok(row.try_get::<i64, _>("templates")? >= 4 && row.try_get::<i64, _>("networks")? >= 3)
}

/// Imported decisions (including declines) and original identity memories also
/// participate in new native gift admission. They are never converted into a
/// native qualification or synthetic model request merely to enforce this.
pub(super) async fn native_admission(
    c: &mut PgConnection,
    user: i64,
) -> Result<(String, i64), Error> {
    let Some(i) = installation(c).await? else {
        return Ok((String::new(), 0));
    };
    if has_decision(c, user).await? {
        return Ok(("reward_used".into(), 0));
    }
    let subject = subject(c, &i.namespace, user).await?;
    let time = now(c).await?;
    let Some(p) = profile(c, &i, user, &subject, None, time).await? else {
        return Ok(("qualification_missing".into(), 0));
    };
    let used: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_gift.legacy_risk_memories WHERE namespace=$1 AND key_hash=$2 AND kind='identity' AND decision_count>0)")
        .bind(&i.namespace).bind(&p.identity).fetch_one(&mut *c).await?;
    if used {
        return Ok(("identity_used".into(), 0));
    }
    if campaign_hold(c, &i, &subject, &p.network, time).await? {
        return Ok(("registration_hold".into(), 0));
    }
    Ok((String::new(), network_count(c, &i, &p.network, time).await?))
}

async fn network_count(
    c: &mut PgConnection,
    i: &Installation,
    network: &[u8],
    time: i64,
) -> Result<i64, Error> {
    let count: Option<i64> = sqlx::query_scalar("SELECT decision_count FROM core_gift.legacy_risk_memories WHERE namespace=$1 AND key_hash=$2 AND kind='network' AND $3-window_started_at<2592000")
        .bind(&i.namespace).bind(network).bind(time).fetch_optional(c).await?;
    Ok(count.unwrap_or(0))
}

pub(super) async fn read(
    c: &mut PgConnection,
    user: i64,
    peer: Option<IpAddr>,
) -> Result<Option<Receipt>, Error> {
    let Some(i) = installation(c).await? else {
        return Ok(None);
    };
    let row = sqlx::query("SELECT d.*,c.claimed_at AS native_claimed_at,c.ledger_journal_id FROM core_gift.legacy_decisions d JOIN core_gift.legacy_bindings b USING(namespace,source_user_id) JOIN core_identity.users u ON u.id=b.user_id AND u.personal_account_id=b.account_id LEFT JOIN core_gift.legacy_claims c ON c.namespace=d.namespace AND c.source_id=d.source_id WHERE d.namespace=$1 AND b.user_id=$2")
        .bind(&i.namespace).bind(user).fetch_optional(&mut *c).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let state: String = row.try_get("source_status")?;
    if state == "declined" {
        return Ok(None);
    }
    let amount: i64 = row.try_get("amount_credits")?;
    let claimed: Option<i64> = row.try_get("native_claimed_at")?;
    let journal = row.try_get("ledger_journal_id")?;
    let mut result = Receipt {
        gift_id: row.try_get("gift_id")?,
        owner_user_id: user,
        credit_amount: amount,
        created_at_unix: row.try_get("created_at")?,
        claimed_at_unix: claimed,
        ledger_transaction_id: journal,
        max_credit_amount: i.cap,
        claim_available: false,
        claim_blocked_code: String::new(),
    };
    if state == "claimed" {
        result.claimed_at_unix = Some(row.try_get("claimed_at")?);
        result.claim_blocked_code = "already_claimed_legacy".into();
    } else if claimed.is_some() {
        result.claim_blocked_code = "already_claimed".into();
    } else if !i.enabled {
        result.claim_blocked_code = "disabled".into();
    } else if amount <= 0 || amount >= i.cap {
        result.claim_blocked_code = "cap_changed".into();
    } else {
        let subject = subject(c, &i.namespace, user).await?;
        let time = now(c).await?;
        match profile(c, &i, user, &subject, peer, time).await? {
            None => result.claim_blocked_code = "qualification_missing".into(),
            Some(p) => {
                if campaign_hold(c, &i, &subject, &p.network, time).await? {
                    result.claim_blocked_code = "registration_hold".into();
                } else {
                    result.claim_available = true;
                }
            }
        }
    }
    Ok(Some(result))
}

/// Called under the same global admission lock as native gift decisions.
pub(super) async fn claim(
    c: &mut PgConnection,
    user: i64,
    account: i64,
    peer: Option<IpAddr>,
) -> Result<Option<(Receipt, bool)>, Error> {
    let Some(old) = read(c, user, peer).await? else {
        return Ok(None);
    };
    if old.claimed_at_unix.is_some() {
        return Ok(Some((old, true)));
    }
    let Some(ip) = peer else {
        return Err(Error::Forbidden);
    };
    observe(c, user, ip).await?;
    let old = read(c, user, None).await?.ok_or(Error::Unavailable)?;
    if !old.claim_available {
        return Err(Error::Forbidden);
    }
    let _: Json<Wallet> = sqlx::query_scalar("SELECT core_billing.open_ledger_wallet($1)")
        .bind(account)
        .fetch_one(&mut *c)
        .await?;
    let request = Request {
        scope: "core-assistant-legacy-gift".into(),
        operation_key: old.gift_id.clone(),
        actor_user_id: Some(user),
        reason: "Claim of preserved source assistant gift".into(),
        action: Action::Gift {
            account_id: account,
            amount_units: Credits::try_from(old.credit_amount).map_err(|_| Error::Invalid)?,
        },
    };
    request.validate().map_err(|_| Error::Invalid)?;
    let Json(out): Json<Outcome> = sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
        .bind(Json(request))
        .fetch_one(&mut *c)
        .await?;
    let Outcome::Posted { journal_id, .. } = out else {
        return Err(Error::Conflict);
    };
    let count = sqlx::query("INSERT INTO core_gift.legacy_claims(namespace,source_id,user_id,account_id,amount_credits,claimed_at,ledger_journal_id) SELECT d.namespace,d.source_id,$2,$3,d.amount_credits,floor(extract(epoch FROM clock_timestamp()))::bigint,$4 FROM core_gift.legacy_decisions d JOIN core_gift.legacy_bindings b USING(namespace,source_user_id) WHERE d.gift_id=$1 AND d.source_status='offered' AND b.user_id=$2 AND b.account_id=$3")
        .bind(&old.gift_id).bind(user).bind(account).bind(journal_id).execute(&mut *c).await?.rows_affected();
    if count != 1 {
        return Err(Error::Conflict);
    }
    Ok(Some((
        read(c, user, None).await?.ok_or(Error::Unavailable)?,
        false,
    )))
}

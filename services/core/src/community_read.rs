//! Read-only community aggregates for the Go community extension: public usage
//! rankings, opted-in profile cards, onboarding facts and public display names.
//! No spending authority, no writes, no per-request records leave Core.
pub mod http;

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unauthorized,
    TooDense,
    Unavailable,
}
impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
pub type Result<T> = std::result::Result<T, Error>;

const JS_SAFE: i64 = 9_007_199_254_740_991;
const MAX_WINDOW: i64 = 400 * 86_400;
const MAX_ROWS: usize = 20_000;

/// Settled native requests (actor is the user; tokens are input+output; credits
/// are net settled) plus the imported legacy source. When the legacy hourly
/// aggregates are installed they supersede per-log history, which they already
/// contain; otherwise consume logs (type 2) from complete sources are used.
const NATIVE: &str = r#"SELECT r.actor_user_id AS user_id, r.authorized_at AS at, btrim(coalesce(r.price->>'model','')) AS model,
  coalesce(r.input_tokens,0)+coalesce(r.output_tokens,0) AS tokens, 1::bigint AS requests,
  greatest(r.settled-r.refunded,0) AS credits
  FROM core_charging.requests r WHERE r.state='settled' AND r.authorized_at BETWEEN $1 AND $2"#;
const HISTORY_LOGS: &str = r#"SELECT l.user_id, l.created_at AS at, btrim(coalesce(l.model_name,'')) AS model,
  greatest(coalesce(l.prompt_tokens,0),0)+greatest(coalesce(l.completion_tokens,0),0) AS tokens, 1::bigint AS requests,
  CASE WHEN l.pricing_credits_per_usd>0 THEN (greatest(coalesce(l.quota,0),0)::numeric*500000/l.pricing_credits_per_usd)::bigint ELSE 0 END AS credits
  FROM core_history.log_records l JOIN core_history.sources s ON s.source_id=l.source_id
  WHERE s.complete AND s.contract_version='core.history-source.v1' AND l.log_type=2 AND l.created_at BETWEEN $1 AND $2"#;
const HISTORY_AGGREGATES: &str = r#"SELECT q.user_id, q.created_at AS at, btrim(q.model_name) AS model,
  q.token_used AS tokens, q.request_count AS requests, q.quota_credits AS credits
  FROM core_history.quota_aggregates q JOIN core_history.sources s ON s.source_id=q.source_id
  WHERE s.complete AND q.created_at BETWEEN $1 AND $2"#;

#[derive(Clone)]
pub struct CommunityRead {
    pool: PgPool,
    usage: String,
    history_source: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub start: i64,
    pub end: i64,
    #[serde(default)]
    pub bucket_seconds: i64,
}
impl Window {
    fn validate(&self) -> Result<()> {
        if self.start < 0
            || self.end < self.start
            || self.end > 253_402_300_799
            || self.end - self.start > MAX_WINDOW
            || !matches!(self.bucket_seconds, 0 | 3600 | 86_400 | 604_800)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelBucket {
    pub bucket: i64,
    pub model_name: String,
    pub tokens: i64,
    pub requests: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsersRequest {
    pub start: i64,
    pub end: i64,
    pub limit: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct RankedUser {
    pub user_id: i64,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub name: String,
    pub anonymous: bool,
    pub tokens: i64,
    pub requests: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct UserRankings {
    pub total_tokens: i64,
    pub total_requests: i64,
    pub participant_count: i64,
    pub anonymous_participant_count: i64,
    pub users: Vec<RankedUser>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserUsageRequest {
    pub user_id: i64,
    pub start: i64,
    pub end: i64,
    #[serde(default)]
    pub bucket_seconds: i64,
    #[serde(default)]
    pub model_limit: i64,
}
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct UsageTotals {
    pub tokens: i64,
    pub requests: i64,
    pub credits: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct UsageBucket {
    pub bucket: i64,
    pub tokens: i64,
    pub requests: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ModelUsage {
    pub model_name: String,
    pub tokens: i64,
    pub requests: i64,
    pub credits: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct UserUsage {
    pub user_id: i64,
    pub active: bool,
    pub name: String,
    pub username: String,
    pub platform_role: String,
    pub totals: UsageTotals,
    pub model_count: i64,
    pub buckets: Vec<UsageBucket>,
    pub models: Vec<ModelUsage>,
    pub credits_per_usd: i64,
    pub history_source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsersInfoRequest {
    pub user_ids: Vec<i64>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct UserInfo {
    pub user_id: i64,
    pub active: bool,
    pub name: String,
    pub developer_access: bool,
    pub has_active_key: bool,
    pub last_api_activity_at: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRequest {
    pub user_id: i64,
    pub credential_id: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct KeyInfo {
    pub active: bool,
    pub group: String,
    pub auto_groups: Vec<String>,
}

fn safe(value: i64) -> Result<i64> {
    if (0..=JS_SAFE).contains(&value) {
        Ok(value)
    } else {
        Err(Error::Unavailable)
    }
}

/// Legacy semantics: malformed or blank visibility is anonymous; only an
/// explicit "public" shows a name and "hidden" removes the user.
pub fn leaderboard_visibility(setting: Option<&str>) -> &'static str {
    let value = setting
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|v| {
            v.get("usage_leaderboard_visibility")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_ascii_lowercase())
        });
    match value.as_deref() {
        Some("public") => "public",
        Some("hidden") => "hidden",
        _ => "anonymous",
    }
}

impl CommunityRead {
    /// Startup verification: every relation used is readable, with no probe write.
    pub async fn new(pool: PgPool) -> Result<Self> {
        let mut tx = pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await?;
        let aggregates: bool = sqlx::query_scalar("SELECT to_regclass('core_history.quota_aggregates') IS NOT NULL AND has_table_privilege('core_history.quota_aggregates','SELECT')")
            .fetch_one(&mut *tx)
            .await?;
        let history = if aggregates { HISTORY_AGGREGATES } else { HISTORY_LOGS };
        let usage = format!("({NATIVE}) UNION ALL ({history})");
        sqlx::query(&format!("SELECT * FROM ({usage}) u WHERE FALSE"))
            .bind(0_i64)
            .bind(0_i64)
            .fetch_all(&mut *tx)
            .await?;
        sqlx::query("SELECT u.id,u.active,u.platform_role,p.username,p.display_name,p.setting,d.granted,k.group_name,k.auto_groups,k.enabled,c.kind,c.revoked_at,c.expires_at FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id LEFT JOIN core_identity.developer_access d ON d.user_id=u.id JOIN core_identity.credentials c ON c.user_id=u.id JOIN core_identity.api_key_settings k ON k.credential_id=c.id WHERE FALSE")
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Self {
            pool,
            usage,
            history_source: if aggregates {
                "quota_aggregates"
            } else {
                "log_records"
            },
        })
    }

    async fn snapshot(&self) -> Result<Transaction<'static, Postgres>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='8s'")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    /// Site-wide tokens per (bucket, model). bucket_seconds=0 gives totals.
    pub async fn model_buckets(&self, w: &Window) -> Result<Vec<ModelBucket>> {
        w.validate()?;
        let mut tx = self.snapshot().await?;
        let rows = sqlx::query(&format!(
            "SELECT CASE WHEN $3>0 THEN (u.at/$3)*$3 ELSE 0 END AS bucket, u.model, sum(u.tokens)::bigint AS tokens, sum(u.requests)::bigint AS requests
             FROM ({}) u WHERE u.model<>'' GROUP BY 1,2 HAVING sum(u.tokens)>0 ORDER BY 1,2 LIMIT {}",
            self.usage,
            MAX_ROWS + 1
        ))
        .bind(w.start)
        .bind(w.end)
        .bind(w.bucket_seconds)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        if rows.len() > MAX_ROWS {
            return Err(Error::TooDense);
        }
        rows.into_iter()
            .map(|r| {
                Ok(ModelBucket {
                    bucket: r.try_get("bucket")?,
                    model_name: r.try_get("model")?,
                    tokens: safe(r.try_get("tokens")?)?,
                    requests: safe(r.try_get("requests")?)?,
                })
            })
            .collect()
    }

    /// Public user leaderboard over active users who did not hide themselves.
    pub async fn user_rankings(&self, r: &UsersRequest) -> Result<UserRankings> {
        Window {
            start: r.start,
            end: r.end,
            bucket_seconds: 0,
        }
        .validate()?;
        if !(1..=100).contains(&r.limit) {
            return Err(Error::Invalid);
        }
        let mut tx = self.snapshot().await?;
        let rows = sqlx::query(&format!(
            "WITH a AS (SELECT u.user_id, sum(u.tokens)::bigint AS tokens, sum(u.requests)::bigint AS requests FROM ({}) u
               WHERE u.user_id>0 GROUP BY 1 HAVING sum(u.requests)>0 OR sum(u.tokens)>0)
             SELECT a.user_id,a.tokens,a.requests,p.username,p.display_name,p.setting FROM a
             JOIN core_identity.users x ON x.id=a.user_id AND x.active
             JOIN core_identity.user_profiles p ON p.user_id=a.user_id",
            self.usage
        ))
        .bind(r.start)
        .bind(r.end)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut out = UserRankings {
            total_tokens: 0,
            total_requests: 0,
            participant_count: 0,
            anonymous_participant_count: 0,
            users: Vec::new(),
        };
        let mut candidates = Vec::new();
        for row in rows {
            let setting: Option<String> = row.try_get("setting")?;
            let visibility = leaderboard_visibility(setting.as_deref());
            if visibility == "hidden" {
                continue;
            }
            let tokens = safe(row.try_get("tokens")?)?;
            let requests = safe(row.try_get("requests")?)?;
            let mut user = RankedUser {
                user_id: row.try_get("user_id")?,
                name: String::new(),
                anonymous: visibility == "anonymous",
                tokens,
                requests,
            };
            if !user.anonymous {
                let display: String = row.try_get("display_name")?;
                let username: String = row.try_get("username")?;
                user.name = if display.trim().is_empty() {
                    username.trim().to_owned()
                } else {
                    display.trim().to_owned()
                };
                if user.name.is_empty() {
                    continue;
                }
            }
            out.participant_count += 1;
            if user.anonymous {
                out.anonymous_participant_count += 1;
            }
            out.total_tokens = safe(out.total_tokens.saturating_add(tokens))?;
            out.total_requests = safe(out.total_requests.saturating_add(requests))?;
            candidates.push(user);
        }
        candidates.sort_by(|a, b| {
            b.tokens
                .cmp(&a.tokens)
                .then(b.requests.cmp(&a.requests))
                .then(a.anonymous.cmp(&b.anonymous))
                .then(a.name.cmp(&b.name))
                .then(a.user_id.cmp(&b.user_id))
        });
        candidates.truncate(r.limit as usize);
        out.users = candidates;
        Ok(out)
    }

    /// One user's totals, optional buckets and top models (profile cards).
    pub async fn user_usage(&self, r: &UserUsageRequest) -> Result<UserUsage> {
        Window {
            start: r.start,
            end: r.end,
            bucket_seconds: r.bucket_seconds,
        }
        .validate()?;
        if r.user_id <= 0 || !(0..=12).contains(&r.model_limit) {
            return Err(Error::Invalid);
        }
        let mut tx = self.snapshot().await?;
        let user = sqlx::query("SELECT u.active, u.platform_role, p.username, coalesce(nullif(btrim(p.display_name),''),btrim(p.username),'') AS name FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id=$1")
            .bind(r.user_id)
            .fetch_optional(&mut *tx)
            .await?;
        let (active, name, username, platform_role) = match user {
            Some(row) => (
                row.try_get("active")?,
                row.try_get("name")?,
                row.try_get("username")?,
                row.try_get("platform_role")?,
            ),
            None => (false, String::new(), String::new(), String::new()),
        };
        let scoped = format!("SELECT * FROM ({}) u WHERE u.user_id=$3", self.usage);
        let totals = sqlx::query(&format!(
            "SELECT coalesce(sum(tokens),0)::bigint AS tokens, coalesce(sum(requests),0)::bigint AS requests, coalesce(sum(credits),0)::bigint AS credits,
              count(DISTINCT coalesce(nullif(model,''),'unknown'))::bigint AS models FROM ({scoped}) s"
        ))
        .bind(r.start)
        .bind(r.end)
        .bind(r.user_id)
        .fetch_one(&mut *tx)
        .await?;
        let mut buckets = Vec::new();
        if r.bucket_seconds > 0 {
            for row in sqlx::query(&format!(
                "SELECT (at/$4)*$4 AS bucket, sum(tokens)::bigint AS tokens, sum(requests)::bigint AS requests FROM ({scoped}) s GROUP BY 1 ORDER BY 1 LIMIT 1000"
            ))
            .bind(r.start)
            .bind(r.end)
            .bind(r.user_id)
            .bind(r.bucket_seconds)
            .fetch_all(&mut *tx)
            .await?
            {
                buckets.push(UsageBucket {
                    bucket: row.try_get("bucket")?,
                    tokens: safe(row.try_get("tokens")?)?,
                    requests: safe(row.try_get("requests")?)?,
                });
            }
        }
        let mut models = Vec::new();
        if r.model_limit > 0 {
            for row in sqlx::query(&format!(
                "SELECT coalesce(nullif(model,''),'unknown') AS model, sum(tokens)::bigint AS tokens, sum(requests)::bigint AS requests, sum(credits)::bigint AS credits
                 FROM ({scoped}) s GROUP BY 1 ORDER BY credits DESC, tokens DESC, requests DESC, model LIMIT $4"
            ))
            .bind(r.start)
            .bind(r.end)
            .bind(r.user_id)
            .bind(r.model_limit)
            .fetch_all(&mut *tx)
            .await?
            {
                models.push(ModelUsage {
                    model_name: row.try_get("model")?,
                    tokens: safe(row.try_get("tokens")?)?,
                    requests: safe(row.try_get("requests")?)?,
                    credits: safe(row.try_get("credits")?)?,
                });
            }
        }
        tx.commit().await?;
        Ok(UserUsage {
            user_id: r.user_id,
            active,
            name,
            username,
            platform_role,
            totals: UsageTotals {
                tokens: safe(totals.try_get("tokens")?)?,
                requests: safe(totals.try_get("requests")?)?,
                credits: safe(totals.try_get("credits")?)?,
            },
            model_count: totals.try_get("models")?,
            buckets,
            models,
            credits_per_usd: crate::billing::CREDITS_PER_USD,
            history_source: self.history_source.into(),
        })
    }

    /// Public names and onboarding facts for a bounded set of users.
    pub async fn users(&self, r: &UsersInfoRequest) -> Result<Vec<UserInfo>> {
        if r.user_ids.is_empty() || r.user_ids.len() > 100 || r.user_ids.iter().any(|id| *id <= 0) {
            return Err(Error::Invalid);
        }
        let history = if self.history_source == "quota_aggregates" {
            "(SELECT max(q.created_at) FROM core_history.quota_aggregates q WHERE q.user_id=u.id AND q.request_count>0)"
        } else {
            "(SELECT max(l.created_at) FROM core_history.log_records l WHERE l.user_id=u.id AND l.log_type=2)"
        };
        let mut tx = self.snapshot().await?;
        let rows = sqlx::query(&format!(
            "SELECT u.id, u.active, coalesce(nullif(btrim(p.display_name),''),btrim(p.username),'') AS name,
              (u.platform_role IN ('admin','superadmin') OR coalesce(d.granted,false)) AS developer,
              EXISTS(SELECT FROM core_identity.credentials c JOIN core_identity.api_key_settings k ON k.credential_id=c.id
                WHERE c.user_id=u.id AND c.kind='api_key' AND k.enabled AND c.revoked_at IS NULL AND c.expires_at>clock_timestamp()) AS has_key,
              greatest(coalesce((SELECT max(r.authorized_at) FROM core_charging.requests r WHERE r.actor_user_id=u.id AND r.state='settled' AND r.outcome='completed'),0),
                       coalesce({history},0))::bigint AS last_api
             FROM core_identity.users u LEFT JOIN core_identity.user_profiles p ON p.user_id=u.id
             LEFT JOIN core_identity.developer_access d ON d.user_id=u.id
             WHERE u.id=ANY($1) ORDER BY u.id"
        ))
        .bind(&r.user_ids)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(UserInfo {
                    user_id: row.try_get("id")?,
                    active: row.try_get("active")?,
                    name: row.try_get("name")?,
                    developer_access: row.try_get("developer")?,
                    has_active_key: row.try_get("has_key")?,
                    last_api_activity_at: row.try_get("last_api")?,
                })
            })
            .collect()
    }

    /// Group of one of the user's own API keys (onboarding configuration proof).
    pub async fn key(&self, r: &KeyRequest) -> Result<Option<KeyInfo>> {
        if r.user_id <= 0 || r.credential_id <= 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.snapshot().await?;
        let row = sqlx::query("SELECT (k.enabled AND c.revoked_at IS NULL AND c.expires_at>clock_timestamp()) AS active, k.group_name, k.auto_groups
             FROM core_identity.credentials c JOIN core_identity.api_key_settings k ON k.credential_id=c.id
             WHERE c.id=$2 AND c.user_id=$1 AND c.kind='api_key'")
            .bind(r.user_id)
            .bind(r.credential_id)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        row.map(|row| {
            Ok(KeyInfo {
                active: row.try_get("active")?,
                group: row.try_get("group_name")?,
                auto_groups: row.try_get("auto_groups")?,
            })
        })
        .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_defaults_private() {
        assert_eq!(leaderboard_visibility(None), "anonymous");
        assert_eq!(leaderboard_visibility(Some("not json")), "anonymous");
        assert_eq!(leaderboard_visibility(Some(r#"{"usage_leaderboard_visibility":" Public "}"#)), "public");
        assert_eq!(leaderboard_visibility(Some(r#"{"usage_leaderboard_visibility":"hidden"}"#)), "hidden");
        assert_eq!(leaderboard_visibility(Some(r#"{"usage_leaderboard_visibility":"x"}"#)), "anonymous");
    }

    #[test]
    fn windows_are_bounded() {
        let ok = Window { start: 0, end: 86_400, bucket_seconds: 3600 };
        assert!(ok.validate().is_ok());
        for bad in [
            Window { start: 10, end: 5, bucket_seconds: 0 },
            Window { start: 0, end: MAX_WINDOW + 1, bucket_seconds: 0 },
            Window { start: 0, end: 10, bucket_seconds: 60 },
        ] {
            assert_eq!(bad.validate(), Err(Error::Invalid));
        }
    }
}

//! Session-owned inbox and account notification preferences. Delivery never
//! changes a balance; reminders read the same authoritative wallet projection.
use super::{ConsoleCompat, ConsoleError, bearer, error, session, success};
use crate::identity;
use axum::{
    Json,
    extract::{Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use std::collections::BTreeMap;

const CATEGORIES: &[&str] = &[
    "all",
    "account_action",
    "security_incident",
    "moderation",
    "human_support",
    "open_source_bounty_review",
    "open_source_bounty",
];

pub struct NotificationSweep {
    pub next_user: Option<i64>,
    pub failed_accounts: usize,
}

fn exact_numbers(value: Value) -> Value {
    match value {
        Value::Number(number) => Value::String(number.to_string()),
        Value::Array(values) => Value::Array(values.into_iter().map(exact_numbers).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, exact_numbers(value)))
                .collect(),
        ),
        value => value,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TodoQuery {
    #[serde(default = "all")]
    category: String,
    #[serde(default = "one")]
    p: i64,
    #[serde(default = "page_size")]
    page_size: i64,
}
fn all() -> String {
    "all".into()
}
fn one() -> i64 {
    1
}
fn page_size() -> i64 {
    50
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadInput {
    category: String,
    ids: Vec<i64>,
    all: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreferenceInput {
    revision: i64,
    enabled: bool,
    low_balance_units: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RatioQuery {
    before: Option<String>,
}
impl PreferenceInput {
    fn threshold(&self) -> Result<i64, ConsoleError> {
        let value = self
            .low_balance_units
            .parse::<i64>()
            .map_err(|_| ConsoleError::Invalid)?;
        if self.revision < 0
            || !(0..=9_007_199_254_740_991).contains(&value)
            || value.to_string() != self.low_balance_units
        {
            return Err(ConsoleError::Invalid);
        }
        Ok(value)
    }
}

async fn preference(c: &mut PgConnection, user: i64) -> Result<Value, ConsoleError> {
    let row = sqlx::query("SELECT enabled,low_balance_units,revision FROM core_notifications.preferences WHERE user_id=$1")
        .bind(user).fetch_optional(c).await?;
    Ok(match row {
        Some(row) => json!({"enabled":row.try_get::<bool,_>("enabled")?,
            "low_balance_units":row.try_get::<i64,_>("low_balance_units")?.to_string(),
            "revision":row.try_get::<i64,_>("revision")?,"delivery":"in_app"}),
        None => json!({"enabled":true,"low_balance_units":"0","revision":0,"delivery":"in_app"}),
    })
}

// A single reminder is created for each low-balance episode. Reading the feed
// again does not create another reminder; replenishing above the threshold rearms it.
async fn refresh_balance(c: &mut PgConnection, user: i64) -> Result<(), ConsoleError> {
    let row = sqlx::query("SELECT enabled,low_balance_units,below_threshold FROM core_notifications.preferences WHERE user_id=$1 FOR UPDATE")
        .bind(user).fetch_optional(&mut *c).await?;
    let Some(row) = row else {
        return Ok(());
    };
    let threshold: i64 = row.try_get("low_balance_units")?;
    let enabled: bool = row.try_get("enabled")?;
    if !enabled || threshold == 0 {
        return Ok(());
    }
    let wallet = match crate::financial_profile::read_wallet(c, user).await {
        Ok(wallet) => wallet,
        Err(crate::financial_profile::ProfileError::Unavailable) => return Ok(()),
        Err(_) => return Err(ConsoleError::Storage),
    };
    let below = wallet.quota < threshold;
    if below == row.try_get::<bool, _>("below_threshold")? {
        return Ok(());
    }
    sqlx::query("UPDATE core_notifications.preferences SET below_threshold=$2 WHERE user_id=$1")
        .bind(user)
        .bind(below)
        .execute(&mut *c)
        .await?;
    if below {
        let key = format!("low-balance:{:032x}", rand::random::<u128>());
        sqlx::query("INSERT INTO core_notifications.inbox(user_id,source_key,category,kind,title,summary,details) VALUES($1,$2,'account_action','low_balance','Low balance','Your wallet balance is below your notification threshold.',$3)")
            .bind(user).bind(key).bind(json!({"balance_units":wallet.quota.to_string(),"threshold_units":threshold.to_string(),"unit":"credit_500k_usd"}))
            .execute(c).await?;
    }
    Ok(())
}

impl ConsoleCompat {
    /// The host runs one bounded page per tick. Row locks make multiple Core
    /// instances safe; only a committed threshold transition creates a notice.
    pub async fn run_notifications(&self, mut stopped: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut after_user = 0;
        loop {
            tokio::select! {
                _ = stopped.changed() => break,
                _ = interval.tick() => {},
            }
            if *stopped.borrow() {
                break;
            }
            match self.notification_sweep(after_user).await {
                Ok(page) => {
                    after_user = page.next_user.unwrap_or(0);
                    if page.failed_accounts > 0 {
                        eprintln!(
                            "notification sweep deferred {} accounts",
                            page.failed_accounts
                        );
                    }
                }
                Err(_) => {
                    eprintln!("notification sweep unavailable; retrying on the next tick");
                }
            }
        }
    }

    /// One bounded pass for the host's background worker. A failed account
    /// does not prevent the next pass from retrying it; no delivery marks or
    /// balance changes are committed on failure.
    pub async fn notification_sweep(
        &self,
        after_user: i64,
    ) -> Result<NotificationSweep, ConsoleError> {
        if after_user < 0 {
            return Err(ConsoleError::Invalid);
        }
        let users: Vec<i64> = sqlx::query_scalar("SELECT p.user_id FROM core_notifications.preferences p JOIN core_identity.users u ON u.id=p.user_id WHERE p.enabled AND p.low_balance_units>0 AND u.active AND p.user_id>$1 ORDER BY p.user_id LIMIT 100")
            .bind(after_user).fetch_all(&self.pool).await?;
        let mut failed_accounts = 0;
        for user in &users {
            let result: Result<(), ConsoleError> = async {
                let mut tx = self.pool.begin().await?;
                sqlx::query("SET LOCAL statement_timeout='5s'")
                    .execute(&mut *tx)
                    .await?;
                refresh_balance(&mut tx, *user).await?;
                tx.commit().await?;
                Ok(())
            }
            .await;
            if result.is_err() {
                failed_accounts += 1;
            }
        }
        Ok(NotificationSweep {
            next_user: if users.len() == 100 {
                users.last().copied()
            } else {
                None
            },
            failed_accounts,
        })
    }

    async fn ratio_page(&self, secret: &str, input: RatioQuery) -> Result<Value, ConsoleError> {
        let before = match input.before {
            None => i64::MAX,
            Some(value) => {
                let number = value.parse::<i64>().map_err(|_| ConsoleError::Invalid)?;
                if number <= 0 || number.to_string() != value {
                    return Err(ConsoleError::Invalid);
                }
                number
            }
        };
        let mut tx = self.pool.begin().await?;
        let actor = session(identity::authorize_in_transaction(&mut tx, secret).await?)?;
        let models = crate::abuse::public_models_in_transaction(&mut tx, actor.user_id)
            .await
            .map_err(|_| ConsoleError::Storage)?;
        let rows=sqlx::query("SELECT e.id,e.release_version,e.price_version,e.model,e.group_name,e.speed,e.previous_price,e.current_price,extract(epoch FROM e.created_at)::bigint AS effective_at FROM core_notifications.price_events e WHERE e.id<$2 AND EXISTS(SELECT 1 FROM core_identity.user_routing_groups g WHERE g.user_id=$1 AND g.enabled AND g.group_name=e.group_name) AND ($3::text[] IS NULL OR e.model=ANY($3)) ORDER BY e.id DESC LIMIT 51")
            .bind(actor.user_id).bind(before).bind(models).fetch_all(&mut *tx).await?;
        let more = rows.len() > 50;
        let mut events = Vec::new();
        let mut cursor = None;
        for row in rows.into_iter().take(50) {
            let id: i64 = row.try_get("id")?;
            cursor = Some(id.to_string());
            events.push(json!({"event_id":format!("core-price:{id}"),"effective_at":row.try_get::<i64,_>("effective_at")?,
                "release_version":row.try_get::<i64,_>("release_version")?.to_string(),
                "price_version":row.try_get::<i64,_>("price_version")?.to_string(),
                "changes":[{"option":"PublishedPrice","model":row.try_get::<String,_>("model")?,
                    "group":row.try_get::<String,_>("group_name")?,"speed":row.try_get::<String,_>("speed")?,
                    "old":row.try_get::<Option<Value>,_>("previous_price")?.map(exact_numbers),
                    "new":row.try_get::<Option<Value>,_>("current_price")?.map(exact_numbers)}]}));
        }
        tx.commit().await?;
        Ok(json!({"success":true,"data":events,"next":if more {cursor}else{None}}))
    }
    async fn notification_page(&self, secret: &str, q: TodoQuery) -> Result<Value, ConsoleError> {
        if !CATEGORIES.contains(&q.category.as_str())
            || !(1..=10000).contains(&q.p)
            || !(1..=100).contains(&q.page_size)
        {
            return Err(ConsoleError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let actor = session(identity::authorize_in_transaction(&mut tx, secret).await?)?;
        refresh_balance(&mut tx, actor.user_id).await?;
        let counts=sqlx::query("SELECT category,count(*)::bigint AS total,count(*) FILTER(WHERE read_at IS NULL)::bigint AS unread FROM core_notifications.inbox WHERE user_id=$1 GROUP BY category ORDER BY category")
            .bind(actor.user_id).fetch_all(&mut *tx).await?;
        let mut categories = Vec::new();
        let mut unread_by_category = BTreeMap::new();
        let (mut total, mut unread, mut total_unread) = (0_i64, 0_i64, 0_i64);
        for row in counts {
            let category: String = row.try_get("category")?;
            let count: i64 = row.try_get("total")?;
            let unread_count: i64 = row.try_get("unread")?;
            total_unread += unread_count;
            if q.category == "all" || q.category == category {
                total += count;
                unread += unread_count;
            }
            categories.push(json!({"key":category,"total":count,"unread":unread_count}));
            unread_by_category.insert(category, unread_count);
        }
        let rows=sqlx::query("SELECT id,category,kind,title,summary,details,read_at IS NOT NULL AS read,extract(epoch FROM created_at)::bigint AS created_at FROM core_notifications.inbox WHERE user_id=$1 AND ($2='all' OR category=$2) ORDER BY id DESC LIMIT $3 OFFSET $4")
            .bind(actor.user_id).bind(&q.category).bind(q.page_size).bind((q.p-1)*q.page_size).fetch_all(&mut *tx).await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let id: i64 = row.try_get("id")?;
            let created: i64 = row.try_get("created_at")?;
            items.push(json!({"id":format!("notification:{id}"),"source_id":id,
                "category":row.try_get::<String,_>("category")?,"type":row.try_get::<String,_>("kind")?,
                "title":row.try_get::<String,_>("title")?,"summary":row.try_get::<String,_>("summary")?,
                "details":row.try_get::<Value,_>("details")?,"read":row.try_get::<bool,_>("read")?,
                "created_at":created,"updated_at":created}));
        }
        tx.commit().await?;
        Ok(
            json!({"items":items,"page":q.p,"page_size":q.page_size,"total":total,
            "category":q.category,"unread_count":unread,"total_unread_count":total_unread,
            "unread_by_category":unread_by_category,"categories":categories}),
        )
    }
    async fn mark_notifications(
        &self,
        secret: &str,
        input: ReadInput,
    ) -> Result<u64, ConsoleError> {
        if !CATEGORIES.contains(&input.category.as_str())
            || input.ids.len() > 100
            || input.ids.iter().any(|id| *id <= 0)
            || input.all && !input.ids.is_empty()
            || !input.all && input.ids.is_empty()
        {
            return Err(ConsoleError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let actor = session(identity::authorize_in_transaction(&mut tx, secret).await?)?;
        let marked=sqlx::query("UPDATE core_notifications.inbox SET read_at=clock_timestamp() WHERE user_id=$1 AND read_at IS NULL AND ($2='all' OR category=$2) AND ($3 OR id=ANY($4))")
            .bind(actor.user_id).bind(input.category).bind(input.all).bind(input.ids).execute(&mut *tx).await?.rows_affected();
        tx.commit().await?;
        Ok(marked)
    }
    async fn notification_preferences(
        &self,
        secret: &str,
        input: Option<PreferenceInput>,
    ) -> Result<Value, ConsoleError> {
        let mut tx = self.pool.begin().await?;
        let actor = session(identity::authorize_in_transaction(&mut tx, secret).await?)?;
        if let Some(input) = input {
            let threshold = input.threshold()?;
            let changed=sqlx::query("INSERT INTO core_notifications.preferences(user_id,enabled,low_balance_units) SELECT $1,$2,$3 WHERE $4=0 ON CONFLICT DO NOTHING")
                .bind(actor.user_id).bind(input.enabled).bind(threshold).bind(input.revision).execute(&mut *tx).await?.rows_affected();
            if changed == 0 {
                let changed=sqlx::query("UPDATE core_notifications.preferences SET below_threshold=CASE WHEN enabled=$2 AND low_balance_units=$3 THEN below_threshold ELSE false END,enabled=$2,low_balance_units=$3,revision=revision+1,updated_at=clock_timestamp() WHERE user_id=$1 AND revision=$4")
                    .bind(actor.user_id).bind(input.enabled).bind(threshold).bind(input.revision).execute(&mut *tx).await?.rows_affected();
                if changed != 1 {
                    return Err(ConsoleError::Order);
                }
            }
        }
        let value = preference(&mut tx, actor.user_id).await?;
        tx.commit().await?;
        Ok(value)
    }
}

pub(super) async fn ratio_notifications(
    State(state): State<ConsoleCompat>,
    headers: HeaderMap,
    Query(query): Query<RatioQuery>,
) -> Response {
    use axum::response::IntoResponse;
    let result = async { state.ratio_page(bearer(&headers)?, query).await }.await;
    match result {
        Ok(value) => Json(value).into_response(),
        Err(err) => error(err),
    }
}

pub(super) async fn todos(
    State(state): State<ConsoleCompat>,
    headers: HeaderMap,
    Query(query): Query<TodoQuery>,
) -> Response {
    let result = async { state.notification_page(bearer(&headers)?, query).await }.await;
    match result {
        Ok(value) => success(value),
        Err(err) => error(err),
    }
}
pub(super) async fn mark_read(
    State(state): State<ConsoleCompat>,
    headers: HeaderMap,
    body: Result<Json<ReadInput>, JsonRejection>,
) -> Response {
    let result = async {
        state
            .mark_notifications(
                bearer(&headers)?,
                body.map_err(|_| ConsoleError::Invalid)?.0,
            )
            .await
    }
    .await;
    match result {
        Ok(marked) => success(json!({"marked":marked})),
        Err(err) => error(err),
    }
}
pub(super) async fn preferences(
    State(state): State<ConsoleCompat>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        state
            .notification_preferences(bearer(&headers)?, None)
            .await
    }
    .await;
    match result {
        Ok(value) => success(value),
        Err(err) => error(err),
    }
}
pub(super) async fn update_preferences(
    State(state): State<ConsoleCompat>,
    headers: HeaderMap,
    body: Result<Json<PreferenceInput>, JsonRejection>,
) -> Response {
    let result = async {
        state
            .notification_preferences(
                bearer(&headers)?,
                Some(body.map_err(|_| ConsoleError::Invalid)?.0),
            )
            .await
    }
    .await;
    match result {
        Ok(value) => success(value),
        Err(ConsoleError::Order) => super::failure(
            StatusCode::CONFLICT,
            "NOTIFICATION_PREFERENCES_CHANGED",
            "Reload notification preferences before saving.",
        ),
        Err(err) => error(err),
    }
}

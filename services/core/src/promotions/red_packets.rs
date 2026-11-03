//! Red packets: share links that hand out existing Core redemption codes.
//!
//! A packet never mints value. Its items bind redemption codes whose budget was
//! already reserved when the codes were issued (or imported). A claim posts
//! exactly that code's canonical promotion grant to the claimant in the same
//! transaction, so the code, the packet item and the ledger journal are spent
//! together and at most once. Releasing an unclaimed item (delete/replace)
//! returns the code to the administrator's pool; revoking it through the
//! promotions admin releases the reserved budget.
use super::{Campaign, Error, Principal, PromotionStore, actor, campaign, hex, lock, now};
use crate::identity;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use rand::{Rng, rngs::OsRng};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, Postgres, Row, Transaction};

pub const SCHEMA: &str = include_str!("../../schema/red_packets.sql");
const MAX_COVER_BYTES: usize = 3 << 20;
const MAX_ITEMS: usize = 1000;

#[derive(Debug, PartialEq, Eq)]
pub enum RedPacketError {
    Core(Error),
    NotFound,
    NotStarted,
    Expired,
    Exhausted,
    ClaimLimit,
    InvalidItem,
    DuplicatedItem,
    HasClaims,
    Invalid(&'static str),
}
impl From<Error> for RedPacketError {
    fn from(e: Error) -> Self {
        Self::Core(e)
    }
}
impl From<sqlx::Error> for RedPacketError {
    fn from(e: sqlx::Error) -> Self {
        Self::Core(e.into())
    }
}
type RpResult<T> = std::result::Result<T, RedPacketError>;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ItemInput {
    pub item_type: String,
    /// Hex SHA-256 code id from the promotions code list.
    pub code_id: Option<String>,
    /// Legacy redemption/discount row id. Not resolvable in Core.
    pub source_id: Option<i64>,
    pub weight: i64,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Mutation {
    pub title: String,
    pub description: String,
    pub cover_image: String,
    pub cover_prompt: String,
    pub draw_mode: String,
    pub per_user_limit: i64,
    pub start_at: i64,
    pub end_at: i64,
    pub enabled: Option<bool>,
    pub items: Option<Vec<ItemInput>>,
}
impl Mutation {
    fn validate(&mut self, require_items: bool) -> RpResult<()> {
        self.title = self.title.trim().to_owned();
        self.description = self.description.trim().to_owned();
        self.cover_prompt = self.cover_prompt.trim().to_owned();
        let title = self.title.chars().count();
        if !(1..=80).contains(&title) {
            return Err(RedPacketError::Invalid("红包标题长度必须为 1-80 个字符"));
        }
        if self.description.chars().count() > 500 {
            return Err(RedPacketError::Invalid("红包说明不能超过 500 个字符"));
        }
        if self.cover_prompt.chars().count() > 1000 {
            return Err(RedPacketError::Invalid("红包封面提示词不能超过 1000 个字符"));
        }
        if self.per_user_limit <= 0 {
            self.per_user_limit = 1;
        }
        if self.per_user_limit > 100 {
            return Err(RedPacketError::Invalid("单用户领取次数不能超过 100"));
        }
        if self.start_at < 0
            || self.end_at < 0
            || (self.start_at > 0 && self.end_at > 0 && self.end_at <= self.start_at)
        {
            return Err(RedPacketError::Invalid("红包有效时间无效"));
        }
        if self.cover_image.len() > MAX_COVER_BYTES {
            return Err(RedPacketError::Invalid("红包封面不能超过 3MB"));
        }
        if !self.cover_image.is_empty()
            && !["data:image/", "https://", "http://"]
                .iter()
                .any(|p| self.cover_image.starts_with(p))
        {
            return Err(RedPacketError::Invalid(
                "红包封面必须是图片 data URL 或 http(s) URL",
            ));
        }
        let items = self.items.as_ref().map_or(0, Vec::len);
        if require_items && items == 0 {
            return Err(RedPacketError::Invalid("红包至少需要一个奖励"));
        }
        if items > MAX_ITEMS {
            return Err(RedPacketError::Invalid("单个红包最多包含 1000 个奖励"));
        }
        self.draw_mode = match self.draw_mode.trim().to_ascii_lowercase().as_str() {
            m @ ("random" | "weighted" | "sequence") => m.to_owned(),
            _ => "random".to_owned(),
        };
        Ok(())
    }
}

struct Packet {
    id: i64,
    draw_mode: String,
    per_user_limit: i64,
    start_at: i64,
    end_at: i64,
    enabled: bool,
    deleted: bool,
}
fn packet_row(r: &sqlx::postgres::PgRow) -> RpResult<Packet> {
    Ok(Packet {
        id: r.try_get("id")?,
        draw_mode: r.try_get("draw_mode")?,
        per_user_limit: i64::from(r.try_get::<i32, _>("per_user_limit")?),
        start_at: r.try_get("start_at")?,
        end_at: r.try_get("end_at")?,
        enabled: r.try_get("enabled")?,
        deleted: r.try_get::<Option<i64>, _>("deleted_at")?.is_some(),
    })
}
const PACKET_COLUMNS: &str = "id,slug,title,description,cover_image,cover_prompt,draw_mode,per_user_limit,start_at,end_at,enabled,created_by,created_at,updated_at,deleted_at";

/// Items still claimable: bound, unclaimed, and backed by a live code.
const AVAILABLE: &str = "SELECT i.id,i.weight,i.code_digest,d.campaign_id FROM core_promotions.red_packet_items i
 JOIN core_promotions.codes d ON d.digest=i.code_digest
 JOIN core_promotions.campaigns c ON c.id=d.campaign_id
 WHERE i.packet_id=$1 AND i.released_at IS NULL AND NOT d.revoked
   AND c.enabled AND c.starts_at<=$2 AND c.ends_at>$2
   AND NOT EXISTS(SELECT 1 FROM core_promotions.red_packet_claims r WHERE r.item_id=i.id)
   AND NOT EXISTS(SELECT 1 FROM core_promotions.claims l WHERE l.code_digest=i.code_digest)
 ORDER BY i.id";

fn active(p: &Packet, time: i64) -> RpResult<()> {
    if p.deleted || !p.enabled {
        return Err(RedPacketError::NotFound);
    }
    if p.start_at > 0 && time < p.start_at {
        return Err(RedPacketError::NotStarted);
    }
    if p.end_at > 0 && time >= p.end_at {
        return Err(RedPacketError::Expired);
    }
    Ok(())
}

fn pick(mode: &str, weights: &[i64]) -> usize {
    match mode {
        "sequence" => 0,
        "weighted" => {
            let total: i64 = weights.iter().sum();
            let mut cursor = OsRng.gen_range(0..total);
            for (i, w) in weights.iter().enumerate() {
                if cursor < *w {
                    return i;
                }
                cursor -= w;
            }
            weights.len() - 1
        }
        _ => OsRng.gen_range(0..weights.len()),
    }
}

fn parse_digest(value: &str) -> RpResult<Vec<u8>> {
    super::parse_hex(value).map_err(|_| RedPacketError::InvalidItem)
}

async fn counts(c: &mut PgConnection, packet: i64, time: i64) -> RpResult<(i64, i64, i64)> {
    let (total, claims): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_promotions.red_packet_items WHERE packet_id=$1 AND released_at IS NULL)
              + (SELECT count(*) FROM core_promotions.red_packet_claims WHERE packet_id=$1 AND item_id IS NULL),
                (SELECT count(*) FROM core_promotions.red_packet_claims WHERE packet_id=$1)",
    )
    .bind(packet)
    .fetch_one(&mut *c)
    .await?;
    let remaining: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM ({AVAILABLE}) a"))
        .bind(packet)
        .bind(time)
        .fetch_one(&mut *c)
        .await?;
    Ok((total, remaining, claims))
}

fn reward(r: &sqlx::postgres::PgRow) -> RpResult<Value> {
    let mut value: Value = r.try_get::<sqlx::types::Json<Value>, _>("reward")?.0;
    let object = value.as_object_mut().ok_or(Error::Unavailable)?;
    object.insert("claim_id".into(), json!(r.try_get::<i64, _>("id")?));
    object.insert("claimed_at".into(), json!(r.try_get::<i64, _>("created_at")?));
    Ok(value)
}

async fn insert_items(
    c: &mut PgConnection,
    packet: i64,
    items: &[ItemInput],
    time: i64,
) -> RpResult<()> {
    let mut seen = std::collections::HashSet::new();
    for item in items {
        if item.item_type.trim().to_ascii_lowercase() != "redemption" {
            // Core has no discount-code authority; reject rather than drop.
            return Err(RedPacketError::InvalidItem);
        }
        let Some(code_id) = item.code_id.as_deref() else {
            return Err(RedPacketError::Invalid(
                "红包奖励需要提供兑换码 code_id（旧 source_id 不再可用）",
            ));
        };
        let digest = parse_digest(code_id)?;
        if !seen.insert(digest.clone()) {
            return Err(RedPacketError::DuplicatedItem);
        }
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core_promotions.codes d JOIN core_promotions.campaigns c ON c.id=d.campaign_id
              WHERE d.digest=$1 AND NOT d.revoked AND c.kind='redemption' AND c.enabled AND c.ends_at>$2
                AND NOT EXISTS(SELECT 1 FROM core_promotions.claims l WHERE l.code_digest=d.digest))",
        )
        .bind(&digest)
        .bind(time)
        .fetch_one(&mut *c)
        .await?;
        if !live {
            return Err(RedPacketError::InvalidItem);
        }
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core_promotions.red_packet_items WHERE code_digest=$1 AND released_at IS NULL)",
        )
        .bind(&digest)
        .fetch_one(&mut *c)
        .await?;
        if taken {
            return Err(RedPacketError::DuplicatedItem);
        }
        let weight = if item.weight <= 0 { 1 } else { item.weight.min(1_000_000) };
        sqlx::query("INSERT INTO core_promotions.red_packet_items(packet_id,code_digest,weight,created_at) VALUES($1,$2,$3,$4)")
            .bind(packet)
            .bind(&digest)
            .bind(weight as i32)
            .bind(time)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}

impl PromotionStore {
    async fn begin_red_packet(&self) -> RpResult<Transaction<'_, Postgres>> {
        // Row locks (packet, then campaign) serialize claims; read committed
        // lets a waiter re-read committed claims after acquiring the lock.
        let mut tx = self.pool.begin().await?;
        for sql in [
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
            "SET LOCAL statement_timeout='10s'",
            "SET LOCAL lock_timeout='5s'",
            "SET LOCAL synchronous_commit=on",
        ] {
            sqlx::query(sql).execute(&mut *tx).await?;
        }
        Ok(tx)
    }

    pub async fn red_packet_public(&self, slug: &str) -> RpResult<Value> {
        let mut tx = self.begin_red_packet().await?;
        let row = sqlx::query(&format!(
            "SELECT {PACKET_COLUMNS} FROM core_promotions.red_packets WHERE slug=$1"
        ))
        .bind(slug.trim())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RedPacketError::NotFound)?;
        let p = packet_row(&row)?;
        let time = now(&mut tx).await?;
        let (total, mut remaining, claims) = counts(&mut tx, p.id, time).await?;
        if p.deleted {
            remaining = 0;
        }
        tx.commit().await?;
        Ok(json!({
            "slug": row.try_get::<String,_>("slug")?,
            "title": row.try_get::<String,_>("title")?,
            "description": row.try_get::<String,_>("description")?,
            "cover_image": row.try_get::<String,_>("cover_image")?,
            "draw_mode": p.draw_mode,
            "per_user_limit": p.per_user_limit,
            "start_at": p.start_at,
            "end_at": p.end_at,
            "enabled": p.enabled && !p.deleted,
            "total_items": total,
            "remaining_items": remaining,
            "claim_count": claims,
        }))
    }

    pub async fn red_packet_my_claims(&self, secret: &str, slug: &str) -> RpResult<Vec<Value>> {
        let mut tx = self.begin_red_packet().await?;
        let a = actor(&mut tx, secret, false).await?;
        let id: i64 = sqlx::query_scalar("SELECT id FROM core_promotions.red_packets WHERE slug=$1")
            .bind(slug.trim())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(RedPacketError::NotFound)?;
        let rows = sqlx::query("SELECT id,reward,created_at FROM core_promotions.red_packet_claims WHERE packet_id=$1 AND user_id=$2 ORDER BY claim_index")
            .bind(id)
            .bind(a.user_id)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        rows.iter().map(reward).collect()
    }

    pub async fn red_packet_claim(&self, secret: &str, slug: &str) -> RpResult<Value> {
        let mut tx = self.begin_red_packet().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        let row = sqlx::query(&format!(
            "SELECT {PACKET_COLUMNS} FROM core_promotions.red_packets WHERE slug=$1 FOR UPDATE"
        ))
        .bind(slug.trim())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RedPacketError::NotFound)?;
        let p = packet_row(&row)?;
        let time = now(&mut tx).await?;
        active(&p, time)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_promotions.red_packet_claims WHERE packet_id=$1 AND user_id=$2",
        )
        .bind(p.id)
        .bind(a.user_id)
        .fetch_one(&mut *tx)
        .await?;
        if count >= p.per_user_limit {
            return Err(RedPacketError::ClaimLimit);
        }
        let items = sqlx::query(AVAILABLE)
            .bind(p.id)
            .bind(time)
            .fetch_all(&mut *tx)
            .await?;
        if items.is_empty() {
            return Err(RedPacketError::Exhausted);
        }
        let weights = items
            .iter()
            .map(|r| r.try_get::<i32, _>("weight").map(i64::from))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let chosen = &items[pick(&p.draw_mode, &weights)];
        let item_id: i64 = chosen.try_get("id")?;
        let digest: Vec<u8> = chosen.try_get("code_digest")?;
        let camp: Campaign = campaign(&mut tx, &chosen.try_get::<String, _>("campaign_id")?).await?;
        let receipt = super::post_grant(
            &mut tx,
            &a,
            &camp,
            "redemption",
            hex(&digest),
            Some(digest),
            time,
        )
        .await?;
        let stored = json!({
            "item_type": "redemption",
            "code": "",
            "name": camp.spec.title,
            "reward_type": "quota",
            "quota": camp.spec.award_units.parse::<i64>().map_err(|_| Error::Unavailable)?,
            "credited": true,
            "journal_id": receipt.journal_id,
        });
        let claim = sqlx::query("INSERT INTO core_promotions.red_packet_claims(packet_id,user_id,claim_index,item_id,promotion_claim_id,reward,created_at) VALUES($1,$2,$3,$4,$5,$6,$7) RETURNING id,reward,created_at")
            .bind(p.id)
            .bind(a.user_id)
            .bind((count + 1) as i32)
            .bind(item_id)
            .bind(&receipt.id)
            .bind(sqlx::types::Json(&stored))
            .bind(time)
            .fetch_one(&mut *tx)
            .await?;
        let result = reward(&claim)?;
        audit(
            &mut tx,
            &a,
            "promotion.red_packet.claim",
            json!({"packet_id":p.id,"item_id":item_id,"promotion_claim_id":receipt.id,"journal_id":receipt.journal_id}),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn red_packet_admin_list(&self, secret: &str) -> RpResult<Vec<Value>> {
        let mut tx = self.begin_red_packet().await?;
        actor(&mut tx, secret, true).await?;
        let time = now(&mut tx).await?;
        let rows = sqlx::query(&format!(
            "SELECT {PACKET_COLUMNS} FROM core_promotions.red_packets WHERE deleted_at IS NULL ORDER BY id DESC LIMIT 1000"
        ))
        .fetch_all(&mut *tx)
        .await?;
        let mut result = Vec::with_capacity(rows.len());
        for row in &rows {
            let p = packet_row(row)?;
            let (total, remaining, claims) = counts(&mut tx, p.id, time).await?;
            result.push(json!({
                "id": p.id,
                "slug": row.try_get::<String,_>("slug")?,
                "title": row.try_get::<String,_>("title")?,
                "description": row.try_get::<String,_>("description")?,
                "cover_image": row.try_get::<String,_>("cover_image")?,
                "cover_prompt": row.try_get::<String,_>("cover_prompt")?,
                "draw_mode": p.draw_mode,
                "per_user_limit": p.per_user_limit,
                "start_at": p.start_at,
                "end_at": p.end_at,
                "enabled": p.enabled,
                "created_by": row.try_get::<i64,_>("created_by")?,
                "created_at": row.try_get::<i64,_>("created_at")?,
                "updated_at": row.try_get::<i64,_>("updated_at")?,
                "total_items": total,
                "remaining_items": remaining,
                "claim_count": claims,
            }));
        }
        tx.commit().await?;
        Ok(result)
    }

    pub async fn red_packet_create(&self, secret: &str, mut input: Mutation) -> RpResult<Value> {
        input.validate(true)?;
        let mut tx = self.begin_red_packet().await?;
        let a = actor(&mut tx, secret, true).await?;
        let time = now(&mut tx).await?;
        let mut slug = [0u8; 16];
        rand::RngCore::fill_bytes(&mut OsRng, &mut slug);
        let slug = hex(&slug);
        let id: i64 = sqlx::query_scalar("INSERT INTO core_promotions.red_packets(slug,title,description,cover_image,cover_prompt,draw_mode,per_user_limit,start_at,end_at,enabled,created_by,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$12) RETURNING id")
            .bind(&slug).bind(&input.title).bind(&input.description).bind(&input.cover_image).bind(&input.cover_prompt)
            .bind(&input.draw_mode).bind(input.per_user_limit as i32).bind(input.start_at).bind(input.end_at)
            .bind(input.enabled.unwrap_or(true)).bind(a.user_id).bind(time)
            .fetch_one(&mut *tx).await?;
        let items = input.items.unwrap_or_default();
        insert_items(&mut tx, id, &items, time).await?;
        audit(&mut tx, &a, "promotion.red_packet.create", json!({"id":id,"slug":slug,"items":items.len(),"draw_mode":input.draw_mode,"per_user_limit":input.per_user_limit})).await?;
        tx.commit().await?;
        Ok(json!({
            "id": id, "slug": slug, "title": input.title, "description": input.description,
            "cover_image": input.cover_image, "cover_prompt": input.cover_prompt,
            "draw_mode": input.draw_mode, "per_user_limit": input.per_user_limit,
            "start_at": input.start_at, "end_at": input.end_at,
            "enabled": input.enabled.unwrap_or(true), "created_by": a.user_id,
            "created_at": time, "updated_at": time,
        }))
    }

    pub async fn red_packet_update(&self, secret: &str, id: i64, mut input: Mutation) -> RpResult<()> {
        input.validate(false)?;
        let mut tx = self.begin_red_packet().await?;
        let a = actor(&mut tx, secret, true).await?;
        let time = now(&mut tx).await?;
        let deleted: Option<i64> = sqlx::query_scalar(
            "SELECT deleted_at FROM core_promotions.red_packets WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RedPacketError::NotFound)?;
        if deleted.is_some() {
            return Err(RedPacketError::NotFound);
        }
        sqlx::query("UPDATE core_promotions.red_packets SET title=$2,description=$3,cover_image=$4,cover_prompt=$5,draw_mode=$6,per_user_limit=$7,start_at=$8,end_at=$9,enabled=$10,updated_at=$11 WHERE id=$1")
            .bind(id).bind(&input.title).bind(&input.description).bind(&input.cover_image).bind(&input.cover_prompt)
            .bind(&input.draw_mode).bind(input.per_user_limit as i32).bind(input.start_at).bind(input.end_at)
            .bind(input.enabled.unwrap_or(true)).bind(time)
            .execute(&mut *tx).await?;
        if let Some(items) = &input.items {
            if items.is_empty() {
                return Err(RedPacketError::InvalidItem);
            }
            let claims: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM core_promotions.red_packet_claims WHERE packet_id=$1",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            if claims > 0 {
                return Err(RedPacketError::HasClaims);
            }
            sqlx::query("UPDATE core_promotions.red_packet_items SET released_at=$2 WHERE packet_id=$1 AND released_at IS NULL")
                .bind(id)
                .bind(time)
                .execute(&mut *tx)
                .await?;
            insert_items(&mut tx, id, items, time).await?;
        }
        audit(&mut tx, &a, "promotion.red_packet.update", json!({"id":id,"items":input.items.as_ref().map_or(0, Vec::len)})).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Soft-deletes the packet and releases unclaimed bindings. Claim history,
    /// claimed items and the underlying codes stay intact; repeat is a no-op.
    pub async fn red_packet_delete(&self, secret: &str, id: i64) -> RpResult<()> {
        let mut tx = self.begin_red_packet().await?;
        let a = actor(&mut tx, secret, true).await?;
        let time = now(&mut tx).await?;
        let deleted: Option<Option<i64>> = sqlx::query_scalar(
            "SELECT deleted_at FROM core_promotions.red_packets WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        if matches!(deleted, Some(None)) {
            let released = sqlx::query("UPDATE core_promotions.red_packet_items i SET released_at=$2 WHERE i.packet_id=$1 AND i.released_at IS NULL AND NOT EXISTS(SELECT 1 FROM core_promotions.red_packet_claims r WHERE r.item_id=i.id)")
                .bind(id)
                .bind(time)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            sqlx::query("UPDATE core_promotions.red_packets SET enabled=false,updated_at=$2,deleted_at=$2 WHERE id=$1")
                .bind(id)
                .bind(time)
                .execute(&mut *tx)
                .await?;
            audit(&mut tx, &a, "promotion.red_packet.delete", json!({"id":id,"released_items":released})).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

async fn audit(c: &mut PgConnection, a: &Principal, action: &str, target: Value) -> RpResult<()> {
    identity::audit(c, a, action, target)
        .await
        .map_err(|_| RedPacketError::Core(Error::Unavailable))
}

// ---------------------------------------------------------------- HTTP

fn bearer(h: &HeaderMap) -> RpResult<&str> {
    let mut values = h.get_all(header::AUTHORIZATION).iter();
    let raw = values.next().ok_or(Error::Unauthorized)?;
    if values.next().is_some() {
        return Err(Error::Unauthorized.into());
    }
    raw.to_str()
        .ok()
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, secret)| {
            scheme.eq_ignore_ascii_case("Bearer")
                && !secret.is_empty()
                && !secret.bytes().any(|b| b.is_ascii_whitespace())
        })
        .map(|(_, secret)| secret)
        .ok_or(Error::Unauthorized.into())
}

impl IntoResponse for RedPacketError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Core(Error::Unauthorized) => (StatusCode::UNAUTHORIZED, "未登录或登录已过期"),
            Self::Core(Error::Forbidden) => (StatusCode::FORBIDDEN, "无权进行此操作"),
            Self::Core(Error::Unavailable) => (StatusCode::SERVICE_UNAVAILABLE, "红包服务暂不可用"),
            Self::Core(Error::Invalid) => (StatusCode::OK, "请求参数无效"),
            Self::Core(Error::Conflict) => (StatusCode::OK, "奖励状态已变化，请重试"),
            Self::Core(Error::Exhausted) | Self::Exhausted => (StatusCode::OK, "红包已经领完"),
            Self::Core(Error::NotFound) | Self::NotFound => (StatusCode::OK, "红包不存在"),
            Self::NotStarted => (StatusCode::OK, "红包尚未开始"),
            Self::Expired => (StatusCode::OK, "红包已结束"),
            Self::ClaimLimit => (StatusCode::OK, "已达到该红包的领取次数上限"),
            Self::InvalidItem => (StatusCode::OK, "红包包含无效奖励"),
            Self::DuplicatedItem => (StatusCode::OK, "奖励已被其他红包占用"),
            Self::HasClaims => (StatusCode::OK, "已有领取记录的红包不能替换奖励库存"),
            Self::Invalid(message) => (StatusCode::OK, message),
        };
        (status, Json(json!({"success":false,"message":message}))).into_response()
    }
}
fn ok(data: Value) -> Response {
    Json(json!({"success":true,"message":"","data":data})).into_response()
}
fn respond(result: RpResult<Value>) -> Response {
    match result {
        Ok(data) => ok(data),
        Err(e) => e.into_response(),
    }
}
fn packet_id(raw: &str) -> RpResult<i64> {
    raw.parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or(RedPacketError::Invalid("无效的红包 ID"))
}

pub fn router(pool: sqlx::PgPool) -> Router {
    let mutation_limit = DefaultBodyLimit::max(4 << 20);
    Router::new()
        .route("/api/red-packet/public/{slug}", get(public))
        .route("/api/red-packet/public/{slug}/claims/me", get(my_claims))
        .route(
            "/api/red-packet/public/{slug}/claim",
            post(claim).layer(DefaultBodyLimit::max(4 << 10)),
        )
        .route(
            "/api/red-packet/admin",
            get(admin_list).post(admin_create).layer(mutation_limit.clone()),
        )
        .route(
            "/api/red-packet/admin/{id}",
            put(admin_update).delete(admin_delete).layer(mutation_limit),
        )
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
                response
            },
        ))
        .with_state(PromotionStore::new(pool))
}
async fn public(State(s): State<PromotionStore>, Path(slug): Path<String>) -> Response {
    respond(s.red_packet_public(&slug).await)
}
async fn my_claims(State(s): State<PromotionStore>, Path(slug): Path<String>, h: HeaderMap) -> Response {
    respond(async { Ok(json!(s.red_packet_my_claims(bearer(&h)?, &slug).await?)) }.await)
}
async fn claim(State(s): State<PromotionStore>, Path(slug): Path<String>, h: HeaderMap) -> Response {
    respond(async { s.red_packet_claim(bearer(&h)?, &slug).await }.await)
}
async fn admin_list(State(s): State<PromotionStore>, h: HeaderMap) -> Response {
    respond(async { Ok(json!(s.red_packet_admin_list(bearer(&h)?).await?)) }.await)
}
async fn admin_create(
    State(s): State<PromotionStore>,
    h: HeaderMap,
    j: std::result::Result<Json<Mutation>, JsonRejection>,
) -> Response {
    respond(
        async {
            let secret = bearer(&h)?;
            let Json(input) = j.map_err(|_| Error::Invalid)?;
            s.red_packet_create(secret, input).await
        }
        .await,
    )
}
async fn admin_update(
    State(s): State<PromotionStore>,
    Path(id): Path<String>,
    h: HeaderMap,
    j: std::result::Result<Json<Mutation>, JsonRejection>,
) -> Response {
    respond(
        async {
            let secret = bearer(&h)?;
            let id = packet_id(&id)?;
            let Json(input) = j.map_err(|_| Error::Invalid)?;
            s.red_packet_update(secret, id, input).await?;
            Ok(Value::Null)
        }
        .await,
    )
}
async fn admin_delete(State(s): State<PromotionStore>, Path(id): Path<String>, h: HeaderMap) -> Response {
    respond(
        async {
            let secret = bearer(&h)?;
            s.red_packet_delete(secret, packet_id(&id)?).await?;
            Ok(Value::Null)
        }
        .await,
    )
}

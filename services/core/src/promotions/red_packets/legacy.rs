//! Imported packet rights distribute the original code, as the source did.
//! The quota, reset and discount stores remain the only redemption authority.
use super::{
    Error, PACKET_COLUMNS, Principal, RedPacketError, RpResult, active, audit, now, packet_row,
    pick, reward,
};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};

pub(super) async fn contains(c: &mut PgConnection, packet: i64) -> RpResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM core_promotions.legacy_red_packet_bases WHERE packet_id=$1)",
    )
    .bind(packet)
    .fetch_one(c)
    .await?)
}

const AVAILABLE: &str = "SELECT i.id,i.weight,i.reward_kind,i.code_digest,i.discount_code_id,i.reward
 FROM core_promotions.legacy_red_packet_items i
 WHERE i.packet_id=$1 AND i.released_at IS NULL AND i.source_admission_enabled
   AND NOT EXISTS(SELECT FROM core_promotions.legacy_red_packet_claim_links l WHERE l.item_id=i.id)
   AND CASE i.reward_kind
     WHEN 'quota' THEN EXISTS(SELECT FROM core_promotions.codes d
       JOIN core_promotions.campaigns c ON c.id=d.campaign_id
       WHERE d.digest=i.code_digest AND NOT d.revoked AND c.kind='redemption' AND c.enabled
         AND c.starts_at<=$2 AND c.ends_at>$2
         AND NOT EXISTS(SELECT FROM core_promotions.claims a WHERE a.code_digest=d.digest))
     WHEN 'reset_voucher' THEN EXISTS(SELECT FROM core_subscriptions.reset_codes c
       WHERE c.digest=i.code_digest AND c.enabled AND (c.expires_at=0 OR c.expires_at>=$2)
         AND c.voucher_expires_at>$2
         AND NOT EXISTS(SELECT FROM core_subscriptions.reset_vouchers v WHERE v.code_digest=c.digest))
     WHEN 'discount' THEN EXISTS(SELECT FROM core_payments.discount_codes d
       JOIN core_payments.discount_code_usage u ON u.code_id=d.id
       WHERE d.id=i.discount_code_id AND d.enabled AND d.starts_at<=$2
         AND (d.ends_at=0 OR d.ends_at>$2)
         AND (d.max_uses=0 OR u.used_count<d.max_uses)
         AND NOT EXISTS(SELECT FROM core_payments.discount_code_tombstones t WHERE t.code_id=d.id)
         AND NOT EXISTS(SELECT FROM core_payments.legacy_discount_owners o
                        WHERE o.code_id=d.id AND o.owner_user_id IS NOT NULL))
     ELSE FALSE END
 ORDER BY i.source_item_id";

pub(super) async fn counts(
    c: &mut PgConnection,
    packet: i64,
    time: i64,
) -> RpResult<Option<(i64, i64, i64)>> {
    if !contains(c, packet).await? {
        return Ok(None);
    }
    let (total, claims): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_promotions.legacy_red_packet_items WHERE packet_id=$1 AND released_at IS NULL),
                (SELECT count(*) FROM core_promotions.red_packet_claims WHERE packet_id=$1)",
    )
    .bind(packet)
    .fetch_one(&mut *c)
    .await?;
    let remaining: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM ({AVAILABLE}) available"
    )))
    .bind(packet)
    .bind(time)
    .fetch_one(c)
    .await?;
    Ok(Some((total, remaining, claims)))
}

async fn lock_authority(c: &mut PgConnection, row: &sqlx::postgres::PgRow) -> RpResult<()> {
    match row.try_get::<String, _>("reward_kind")?.as_str() {
        "quota" => {
            // Match ordinary redemption/revocation lock order. No grant is
            // posted here: the still-valid bearer code is only distributed.
            sqlx::query("SELECT c.id FROM core_promotions.codes d JOIN core_promotions.campaigns c ON c.id=d.campaign_id WHERE d.digest=$1 FOR UPDATE OF c")
                .bind(row.try_get::<Vec<u8>, _>("code_digest")?)
                .fetch_optional(c)
                .await?;
        }
        "reset_voucher" => {
            // Match the reset redemption transaction's exact advisory key.
            // This needs no UPDATE permission on immutable reset-code terms.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('core-reset-code:'||encode($1::bytea,'hex'),0))")
                .bind(row.try_get::<Vec<u8>, _>("code_digest")?)
                .execute(c)
                .await?;
        }
        "discount" => {
            sqlx::query("SELECT core_payments.lock_legacy_discount_code($1)")
                .bind(row.try_get::<i64, _>("discount_code_id")?)
                .execute(c)
                .await?;
        }
        _ => return Err(RedPacketError::InvalidItem),
    }
    Ok(())
}

async fn discount_reward(c: &mut PgConnection, id: i64, value: &mut Value) -> RpResult<()> {
    let row = sqlx::query(
        "SELECT d.code,d.name,d.discount_percent,d.min_credit_units,d.ends_at,b.min_amount_unit
         FROM core_payments.discount_codes d JOIN core_payments.legacy_discount_bases b ON b.code_id=d.id WHERE d.id=$1",
    )
    .bind(id)
    .fetch_one(c)
    .await?;
    let object = value.as_object_mut().ok_or(Error::Unavailable)?;
    object.insert("code".into(), json!(row.try_get::<String, _>("code")?));
    object.insert("name".into(), json!(row.try_get::<String, _>("name")?));
    object.insert(
        "discount_percent".into(),
        json!(row.try_get::<i32, _>("discount_percent")?),
    );
    let minimum: i64 = row.try_get("min_credit_units")?;
    object.insert("min_credit_units".into(), json!(minimum.to_string()));
    let divisor = if row.try_get::<String, _>("min_amount_unit")? == "ledger_quota" {
        1
    } else {
        500_000
    };
    if minimum % divisor == 0 {
        object.insert("min_amount".into(), json!(minimum / divisor));
    } else {
        // A later native administrator can choose a raw-credit minimum that
        // has no exact integer source projection. The exact native field wins.
        object.remove("min_amount");
    }
    let ends: i64 = row.try_get("ends_at")?;
    object.insert(
        "expires_at".into(),
        json!(if ends == 0 { 0 } else { ends - 1 }),
    );
    Ok(())
}

pub(super) async fn claim(
    c: &mut PgConnection,
    actor: &Principal,
    packet: i64,
    draw_mode: &str,
    next_index: i64,
    time: i64,
) -> RpResult<Option<Value>> {
    if !contains(c, packet).await? {
        return Ok(None);
    }
    let mut items = sqlx::query(AVAILABLE)
        .bind(packet)
        .bind(time)
        .fetch_all(&mut *c)
        .await?;
    while !items.is_empty() {
        let weights = items
            .iter()
            .map(|row| row.try_get::<i32, _>("weight").map(i64::from))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let selected = pick(draw_mode, &weights);
        let row = items.remove(selected);
        let item: i64 = row.try_get("id")?;
        lock_authority(c, &row).await?;
        // Code locks can wait across a packet or code expiry. Re-read the
        // database clock and the already locked packet before distribution.
        let current_time = now(c).await?;
        let current_packet = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {PACKET_COLUMNS} FROM core_promotions.red_packets WHERE id=$1"
        )))
        .bind(packet)
        .fetch_one(&mut *c)
        .await?;
        active(&packet_row(&current_packet)?, current_time)?;
        // Another transaction may have redeemed or revoked the code while
        // this request waited for its authority lock. Recheck the actual row.
        let available: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT EXISTS(SELECT FROM ({AVAILABLE}) eligible WHERE id=$3)"
        )))
        .bind(packet)
        .bind(current_time)
        .bind(item)
        .fetch_one(&mut *c)
        .await?;
        if !available {
            continue;
        }
        let mut value = row.try_get::<sqlx::types::Json<Value>, _>("reward")?.0;
        if row.try_get::<String, _>("reward_kind")? == "discount" {
            discount_reward(c, row.try_get("discount_code_id")?, &mut value).await?;
        }
        let inserted = sqlx::query(
            "INSERT INTO core_promotions.red_packet_claims(packet_id,user_id,claim_index,item_id,promotion_claim_id,reward,created_at)
             VALUES($1,$2,$3,NULL,NULL,$4,$5) RETURNING id,reward,created_at",
        )
        .bind(packet)
        .bind(actor.user_id)
        .bind(i32::try_from(next_index).map_err(|_| RedPacketError::ClaimLimit)?)
        .bind(sqlx::types::Json(&value))
        .bind(current_time)
        .fetch_one(&mut *c)
        .await?;
        let claim_id: i64 = inserted.try_get("id")?;
        sqlx::query("INSERT INTO core_promotions.legacy_red_packet_claim_links(claim_id,item_id) VALUES($1,$2)")
            .bind(claim_id)
            .bind(item)
            .execute(&mut *c)
            .await?;
        // For a discount this committed link is its owner assignment. The
        // payment validator observes it under the same discount-code row lock.
        audit(c, actor, "promotion.legacy_red_packet.claim",
            json!({"packet_id":packet,"item_id":item,"claim_id":claim_id,"wallet_credit_posted":false})).await?;
        return Ok(Some(reward(&inserted)?));
    }
    Err(RedPacketError::Exhausted)
}

pub(super) async fn history_reward(
    c: &mut PgConnection,
    claim_id: i64,
    value: &mut Value,
) -> RpResult<()> {
    let code_id: Option<i64> = sqlx::query_scalar(
        "SELECT i.discount_code_id FROM core_promotions.legacy_red_packet_claim_links l
         JOIN core_promotions.legacy_red_packet_items i ON i.id=l.item_id
         WHERE l.claim_id=$1 AND i.reward_kind='discount'",
    )
    .bind(claim_id)
    .fetch_optional(&mut *c)
    .await?;
    if let Some(code_id) = code_id {
        discount_reward(c, code_id, value).await?;
    }
    Ok(())
}

pub(super) async fn release_unclaimed(
    c: &mut PgConnection,
    packet: i64,
    time: i64,
) -> RpResult<u64> {
    Ok(sqlx::query(
        "UPDATE core_promotions.legacy_red_packet_items i SET released_at=$2
         WHERE i.packet_id=$1 AND i.released_at IS NULL
           AND NOT EXISTS(SELECT FROM core_promotions.legacy_red_packet_claim_links l WHERE l.item_id=i.id)",
    )
    .bind(packet)
    .bind(time)
    .execute(c)
    .await?
    .rows_affected())
}

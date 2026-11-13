//! Core-only settlement discounts. Quantities are raw credit_500k_usd integers.
//! Pending externally payable intents retain their slot: time alone cannot prove
//! that a late provider credit is impossible. Refunds do not replenish coupons.
use super::*;
use crate::accounts::AccountKind;
use serde::{Deserialize, Serialize};

mod admin;
pub mod http;
pub use admin::{Batch, CodeView, CodesPage, CodesQuery};
pub const MANAGEMENT_SCHEMA: &str = include_str!("../../schema/payment_discount_management.sql");
pub const SCHEMA: &str = include_str!("../../schema/payment_discounts.sql");

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct Code {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub discount_percent: i32,
    pub min_credit_units: i64,
    pub max_uses: i64,
    pub enabled: bool,
    pub starts_at: i64,
    pub ends_at: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub code: String,
    pub name: String,
    pub discount_percent: i32,
    #[serde(deserialize_with = "admin::integer")]
    pub min_credit_units: i64,
    #[serde(deserialize_with = "admin::integer")]
    pub max_uses: i64,
    pub enabled: bool,
    #[serde(deserialize_with = "admin::integer")]
    pub starts_at: i64,
    #[serde(deserialize_with = "admin::integer")]
    pub ends_at: i64,
}
#[derive(Debug, Serialize)]
pub struct Quote {
    pub payment_channel_id: i64,
    pub currency: String,
    pub amount_minor: i64,
    pub credit_units: i64,
    pub credit_unit: &'static str,
    pub discount_code: String,
    pub discount_percent: i32,
}

pub(super) fn normalize_optional(raw: &str) -> Result<String> {
    let code = raw.trim().to_ascii_uppercase();
    if code.is_empty() {
        return Ok(code);
    }
    if !(3..=64).contains(&code.len())
        || !code.as_bytes()[0].is_ascii_alphanumeric()
        || !code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"_-".contains(&b))
    {
        return Err(Error::Invalid);
    }
    Ok(code)
}
async fn session(
    c: &mut PgConnection,
    credential: &str,
    admin: bool,
) -> Result<identity::Principal> {
    let actor = identity::authorize_in_transaction(c, credential).await?;
    if actor.credential_kind != CredentialKind::Session
        || actor.owner.kind != AccountKind::Personal
        || actor.owner.id != actor.user_id
    {
        return Err(Error::Denied);
    }
    if admin {
        let role: String = sqlx::query_scalar(
            "SELECT platform_role FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id)
        .fetch_one(&mut *c)
        .await?;
        if !matches!(role.as_str(), "admin" | "superadmin") {
            return Err(Error::Denied);
        }
        // Fixed go-v0.2.118 uses static AdminAuth here. The nine catalog
        // overrides govern their own actions; never manufacture an allow for
        // coupon management or reinterpret an unrelated catalog deny.
        if identity::has_privileged_import_facts(c, actor.user_id)
            .await
            .map_err(|_| Error::Unavailable)?
        {
            identity::load_complete_policy(c, actor.user_id)
                .await
                .map_err(|_| Error::Denied)?;
        }
    }
    Ok(actor)
}

pub(super) async fn eligible(c: &mut PgConnection, code: &str, credits: i64) -> Result<Code> {
    let row: Code = sqlx::query_as("SELECT id,code,name,discount_percent,min_credit_units,max_uses,enabled,starts_at,ends_at FROM core_payments.discount_codes WHERE code=$1 FOR UPDATE")
        .bind(code).fetch_optional(&mut *c).await?.ok_or(Error::NotFound)?;
    let deleted: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM core_payments.discount_code_tombstones WHERE code_id=$1)",
    )
    .bind(row.id)
    .fetch_one(&mut *c)
    .await?;
    if deleted {
        return Err(Error::NotFound);
    }
    let timestamp: i64 = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
        .fetch_one(&mut *c)
        .await?;
    if !row.enabled
        || timestamp < row.starts_at
        || (row.ends_at != 0 && timestamp >= row.ends_at)
        || credits < row.min_credit_units
    {
        return Err(Error::Denied);
    }
    if row.max_uses > 0 {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_payments.discount_reservations WHERE code_id=$1",
        )
        .bind(row.id)
        .fetch_one(c)
        .await?;
        if count >= row.max_uses {
            return Err(Error::Conflict);
        }
    }
    Ok(row)
}
impl PaymentsStore {
    pub async fn save_discount(
        &self,
        credential: &str,
        id: Option<i64>,
        input: Definition,
    ) -> Result<Code> {
        let code = normalize_optional(&input.code)?;
        validate_definition(&input, &code)?;
        let mut tx = self.begin().await?;
        let actor = session(&mut tx, credential, true).await?;
        let row = save_in_transaction(&mut tx, &actor, id, input, code).await?;
        tx.commit().await?;
        Ok(row)
    }
    pub async fn discount_codes(&self, credential: &str, after_id: i64) -> Result<Vec<Code>> {
        if after_id < 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        session(&mut tx, credential, true).await?;
        let rows = sqlx::query_as("SELECT id,code,name,discount_percent,min_credit_units,max_uses,enabled,starts_at,ends_at FROM core_payments.discount_codes WHERE id>$1 AND NOT EXISTS(SELECT FROM core_payments.discount_code_tombstones t WHERE t.code_id=discount_codes.id) ORDER BY id LIMIT 100")
            .bind(after_id).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(rows)
    }
    pub async fn quote_discount(
        &self,
        credential: &str,
        channel_id: i64,
        credits: i64,
        raw_code: &str,
    ) -> Result<Quote> {
        units(credits)?;
        let code = normalize_optional(raw_code)?;
        let channel = self.channels.get(&channel_id).ok_or(Error::Unavailable)?;
        let policy = channel.credit_quote.as_ref().ok_or(Error::Invalid)?;
        let mut tx = self.begin().await?;
        session(&mut tx, credential, false).await?;
        let mut quote = policy.freeze(credits)?;
        let percent = if code.is_empty() {
            0
        } else {
            let row = eligible(&mut tx, &code, credits).await?;
            quote.apply_code_percent(row.discount_percent)?;
            row.discount_percent
        };
        let amount_minor = quote.amount_minor(credits)?;
        if amount_minor > channel.max_amount_minor || credits < amount_minor {
            return Err(Error::Invalid);
        }
        tx.commit().await?;
        Ok(Quote {
            payment_channel_id: channel_id,
            currency: channel.currency.clone(),
            amount_minor,
            credit_units: credits,
            credit_unit: CREDIT_UNIT,
            discount_code: code,
            discount_percent: percent,
        })
    }
    pub async fn prepare_personal_discount(
        &self,
        credential: &str,
        request: pb::PreparePersonalTopupRequest,
        code: &str,
    ) -> Result<pb::PaymentIntent> {
        let mut tx = self.begin().await?;
        let actor = session(&mut tx, credential, false).await?;
        tx.commit().await?;
        self.prepare_with_discount(
            credential,
            pb::PrepareTopupRequest {
                mutation: request.mutation,
                account_id: actor.owner_account_id,
                payment_channel_id: request.payment_channel_id,
                currency: request.currency,
                amount_minor: request.amount_minor,
                requested_credit_units: request.requested_credit_units,
            },
            code,
        )
        .await
    }
}

async fn save_in_transaction(
    c: &mut PgConnection,
    actor: &identity::Principal,
    id: Option<i64>,
    input: Definition,
    code: String,
) -> Result<Code> {
    // Definitions may change; already prepared intents retain frozen terms.
    let row = if let Some(id) = id {
        let locked: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM core_payments.discount_codes WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *c)
        .await?;
        let deleted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM core_payments.discount_code_tombstones WHERE code_id=$1)",
        )
        .bind(id)
        .fetch_one(&mut *c)
        .await?;
        if deleted {
            return Err(Error::NotFound);
        }
        if locked.is_none() {
            return Err(Error::NotFound);
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_payments.discount_reservations WHERE code_id=$1",
        )
        .bind(id)
        .fetch_one(&mut *c)
        .await?;
        if input.max_uses != 0 && input.max_uses < count {
            return Err(Error::Conflict);
        }
        sqlx::query_as::<_, Code>("UPDATE core_payments.discount_codes SET code=$9,name=$2,discount_percent=$3,min_credit_units=$4,max_uses=$5,enabled=$6,starts_at=$7,ends_at=$8 WHERE id=$1 RETURNING id,code,name,discount_percent,min_credit_units,max_uses,enabled,starts_at,ends_at")
                .bind(id).bind(input.name).bind(input.discount_percent).bind(input.min_credit_units).bind(input.max_uses).bind(input.enabled).bind(input.starts_at).bind(input.ends_at).bind(code)
                .fetch_optional(&mut *c).await?.ok_or(Error::Conflict)?
    } else {
        sqlx::query_as::<_, Code>("INSERT INTO core_payments.discount_codes(code,name,discount_percent,min_credit_units,max_uses,enabled,starts_at,ends_at,created_by) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(code) DO NOTHING RETURNING id,code,name,discount_percent,min_credit_units,max_uses,enabled,starts_at,ends_at")
                .bind(code).bind(input.name).bind(input.discount_percent).bind(input.min_credit_units).bind(input.max_uses).bind(input.enabled).bind(input.starts_at).bind(input.ends_at).bind(actor.user_id)
                .fetch_optional(&mut *c).await?.ok_or(Error::Conflict)?
    };
    identity::audit(
        c,
        actor,
        if id.is_some() {
            "payment_discount.update"
        } else {
            "payment_discount.create"
        },
        json!({
            "code_id": row.id, "discount_percent": row.discount_percent,
            "min_credit_units": row.min_credit_units, "max_uses": row.max_uses,
            "enabled": row.enabled, "starts_at": row.starts_at, "ends_at": row.ends_at
        }),
    )
    .await?;
    Ok(row)
}

fn validate_definition(input: &Definition, code: &str) -> Result<()> {
    if code.is_empty()
        || input.name.trim().is_empty()
        || input.name.chars().count() > 120
        || !(1..100).contains(&input.discount_percent)
        || input.min_credit_units < 0
        || input.max_uses < 0
        || input.starts_at < 0
        || (input.ends_at != 0 && input.ends_at <= input.starts_at)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

use super::*;
use rand::TryRng as _;
use rand::rngs::SysRng;
use sqlx::Row;

// Accept exact JSON integers and decimal strings, never floating point quantities.
pub(super) fn integer<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Input {
        Number(i64),
        Text(String),
    }
    match Input::deserialize(d)? {
        Input::Number(v) => Ok(v),
        Input::Text(v)
            if !v.is_empty()
                && v.bytes()
                    .enumerate()
                    .all(|(i, b)| b.is_ascii_digit() || (i == 0 && b == b'-')) =>
        {
            v.parse().map_err(serde::de::Error::custom)
        }
        _ => Err(serde::de::Error::custom("expected a decimal i64")),
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeView {
    pub id: String,
    pub code: String,
    pub name: String,
    pub discount_percent: i32,
    pub min_credit_units: String,
    pub max_uses: String,
    pub enabled: bool,
    pub starts_at: String,
    pub ends_at: String,
    pub used_count: String,
    pub reserved_count: String,
}
impl CodeView {
    pub(super) fn new(c: Code, used: i64, reserved: i64) -> Self {
        Self {
            id: c.id.to_string(),
            code: c.code,
            name: c.name,
            discount_percent: c.discount_percent,
            min_credit_units: c.min_credit_units.to_string(),
            max_uses: c.max_uses.to_string(),
            enabled: c.enabled,
            starts_at: c.starts_at.to_string(),
            ends_at: c.ends_at.to_string(),
            used_count: used.to_string(),
            reserved_count: reserved.to_string(),
        }
    }
}
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodesQuery {
    pub after_id: i64,
    pub keyword: Option<String>,
    pub enabled: Option<bool>,
    pub limit: i64,
}
impl Default for CodesQuery {
    fn default() -> Self {
        Self {
            after_id: 0,
            keyword: None,
            enabled: None,
            limit: 20,
        }
    }
}
#[derive(Serialize)]
pub struct CodesPage {
    pub items: Vec<CodeView>,
    pub next_after: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub request_key: String,
    pub count: i32,
    pub name: String,
    pub discount_percent: i32,
    #[serde(deserialize_with = "integer")]
    pub min_credit_units: i64,
    #[serde(deserialize_with = "integer")]
    pub max_uses: i64,
    pub enabled: bool,
    #[serde(deserialize_with = "integer")]
    pub starts_at: i64,
    #[serde(deserialize_with = "integer")]
    pub ends_at: i64,
}
impl Batch {
    fn definition(&self, code: String) -> Definition {
        Definition {
            code,
            name: self.name.clone(),
            discount_percent: self.discount_percent,
            min_credit_units: self.min_credit_units,
            max_uses: self.max_uses,
            enabled: self.enabled,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
        }
    }
}
const SELECT: &str = "SELECT c.id,c.code,c.name,c.discount_percent,c.min_credit_units,c.max_uses,c.enabled,c.starts_at,c.ends_at,(SELECT count(*) FROM core_payments.discount_reservations r WHERE r.code_id=c.id AND r.consumed) AS used_count,(SELECT count(*) FROM core_payments.discount_reservations r WHERE r.code_id=c.id AND NOT r.consumed) AS reserved_count FROM core_payments.discount_codes c";
fn view(row: sqlx::postgres::PgRow) -> Result<CodeView> {
    use sqlx::FromRow;
    Ok(CodeView::new(
        Code::from_row(&row)?,
        row.try_get("used_count")?,
        row.try_get("reserved_count")?,
    ))
}
impl PaymentsStore {
    pub async fn list_discount_codes(
        &self,
        credential: &str,
        query: &CodesQuery,
    ) -> Result<CodesPage> {
        if query.after_id < 0 || !(1..=100).contains(&query.limit) {
            return Err(Error::Invalid);
        }
        let keyword = query
            .keyword
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        let mut tx = self.begin().await?;
        session(&mut tx, credential, true).await?;
        let rows=sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE ($1=0 OR c.id<$1) AND ($2='' OR c.code LIKE $2||'%' OR c.name ILIKE '%'||$2||'%') AND ($3::boolean IS NULL OR c.enabled=$3) AND NOT EXISTS(SELECT FROM core_payments.discount_code_tombstones t WHERE t.code_id=c.id) ORDER BY c.id DESC LIMIT $4")))
            .bind(query.after_id).bind(keyword).bind(query.enabled).bind(query.limit+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > query.limit as usize;
        let items = rows
            .into_iter()
            .take(query.limit as usize)
            .map(view)
            .collect::<Result<Vec<_>>>()?;
        let next_after = if more {
            items.last().map(|c| c.id.clone())
        } else {
            None
        };
        tx.commit().await?;
        Ok(CodesPage { items, next_after })
    }
    pub async fn discount_code_view(&self, credential: &str, id: i64) -> Result<CodeView> {
        let mut tx = self.begin().await?;
        session(&mut tx, credential, true).await?;
        let row=sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE c.id=$1 AND NOT EXISTS(SELECT FROM core_payments.discount_code_tombstones t WHERE t.code_id=c.id)")))
            .bind(id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        let result = view(row)?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn generate_discount_codes(
        &self,
        credential: &str,
        input: Batch,
    ) -> Result<Vec<CodeView>> {
        // UUID parsing uses the established UUID type already used by WebAuthn.
        let key = webauthn_rs::prelude::Uuid::parse_str(&input.request_key)
            .map_err(|_| Error::Invalid)?;
        if input.request_key.len() != 36 {
            return Err(Error::Invalid);
        }
        let mut normalized = input.clone();
        normalized.request_key = key.to_string();
        let request = serde_json::to_value(&normalized).map_err(|_| Error::Invalid)?;
        let mut tx = self.begin().await?;
        let actor = session(&mut tx, credential, true).await?;
        // Serializes competing copies before looking for a committed receipt.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("payment_discount_batch:{}:{}", actor.user_id, key))
            .execute(&mut *tx)
            .await?;
        if let Some(row)=sqlx::query("SELECT request,result FROM core_payments.discount_management_batches WHERE actor_id=$1 AND request_key=$2::text::uuid")
            .bind(actor.user_id).bind(key.to_string()).fetch_optional(&mut *tx).await? {
            if row.try_get::<serde_json::Value,_>("request")? != request { return Err(Error::Conflict); }
            let result=serde_json::from_value(row.try_get("result")?).map_err(|_|Error::Storage)?;
            tx.commit().await?;
            return Ok(result);
        }
        if !(1..=100).contains(&input.count) {
            return Err(Error::Invalid);
        }
        validate_definition(&input.definition("BATCH".into()), "BATCH")?;
        let mut items = Vec::with_capacity(input.count as usize);
        for _ in 0..input.count {
            let mut bytes = [0u8; 16];
            SysRng
                .try_fill_bytes(&mut bytes)
                .map_err(|_| Error::Unavailable)?;
            let code = format!(
                "LMM-{}",
                crate::hex_codec::encode(bytes).to_ascii_uppercase()
            );
            let row =
                save_in_transaction(&mut tx, &actor, None, input.definition(code.clone()), code)
                    .await?;
            items.push(CodeView::new(row, 0, 0));
        }
        sqlx::query("INSERT INTO core_payments.discount_management_batches(actor_id,request_key,request,result) VALUES($1,$2::text::uuid,$3,$4)")
            .bind(actor.user_id).bind(key.to_string()).bind(request).bind(serde_json::to_value(&items).map_err(|_|Error::Storage)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(items)
    }
    pub async fn set_discount_status(
        &self,
        credential: &str,
        id: i64,
        enabled: bool,
    ) -> Result<CodeView> {
        if id <= 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let actor = session(&mut tx, credential, true).await?;
        let row: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM core_payments.discount_codes WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let deleted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM core_payments.discount_code_tombstones WHERE code_id=$1)",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if row.is_none() || deleted {
            return Err(Error::NotFound);
        }
        sqlx::query("UPDATE core_payments.discount_codes SET enabled=$2 WHERE id=$1")
            .bind(id)
            .bind(enabled)
            .execute(&mut *tx)
            .await?;
        identity::audit(
            &mut tx,
            &actor,
            "payment_discount.status",
            json!({"code_id":id,"enabled":enabled}),
        )
        .await?;
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE c.id=$1")))
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let result = view(row)?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn delete_discount_code(&self, credential: &str, id: i64) -> Result<()> {
        if id <= 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let actor = session(&mut tx, credential, true).await?;
        // Same row lock as eligibility: committed tombstones block all new holds.
        sqlx::query("SELECT id FROM core_payments.discount_codes WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        let changed=sqlx::query("INSERT INTO core_payments.discount_code_tombstones(code_id,deleted_by) SELECT id,$2 FROM core_payments.discount_codes WHERE id=$1 ON CONFLICT DO NOTHING")
            .bind(id).bind(actor.user_id).execute(&mut *tx).await?;
        if changed.rows_affected() > 0 {
            identity::audit(
                &mut tx,
                &actor,
                "payment_discount.delete",
                json!({"code_id":id}),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn delete_exhausted_discount_codes(&self, credential: &str) -> Result<i64> {
        let mut tx = self.begin().await?;
        let actor = session(&mut tx, credential, true).await?;
        let count:i64=sqlx::query_scalar("WITH selected AS MATERIALIZED (SELECT c.id FROM core_payments.discount_codes c WHERE c.max_uses>0 AND (SELECT count(*) FROM core_payments.discount_reservations r WHERE r.code_id=c.id AND r.consumed)>=c.max_uses AND NOT EXISTS(SELECT FROM core_payments.discount_code_tombstones t WHERE t.code_id=c.id) ORDER BY c.id FOR UPDATE), inserted AS (INSERT INTO core_payments.discount_code_tombstones(code_id,deleted_by) SELECT id,$1 FROM selected ON CONFLICT DO NOTHING RETURNING code_id) SELECT count(*) FROM inserted")
            .bind(actor.user_id).fetch_one(&mut *tx).await?;
        identity::audit(
            &mut tx,
            &actor,
            "payment_discount.cleanup",
            json!({"count":count}),
        )
        .await?;
        tx.commit().await?;
        Ok(count)
    }
}

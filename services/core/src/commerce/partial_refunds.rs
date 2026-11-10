//! Per-refund identities over the ORIGINAL authenticated commerce payment.
//! No provider request, new payer, quotation, account or generic entry list.
use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundMode {
    Full,
    Quantity,
    Amount,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefundSpec {
    pub id: String,
    pub request_key: String,
    pub requested_by: Account,
    pub role: String,
    pub mode: RefundMode,
    pub quantity: i64,
    pub retired_quantity: i64,
    pub amount: i64,
    pub items_digest: String,
}

impl RefundSpec {
    pub(super) fn validate(&self, cmd: &MoneyCommand) -> Result<()> {
        let key = &self.request_key;
        if cmd.kind != "refund"
            || key.is_empty()
            || key.len() > 64
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !hash_id(&self.items_digest)
            || self.amount <= 0
            || self.quantity < 0
            || self.retired_quantity < 0
            || !((self.role == "buyer" && self.requested_by == cmd.buyer)
                || (self.role == "seller" && self.requested_by == cmd.seller))
            || self.id
                != id(&serde_json::json!([
                    "store-refund-v2",
                    cmd.shop_id,
                    cmd.order_id,
                    self.requested_by,
                    self.role,
                    self.request_key
                ]))
            || cmd.key
                != id(&serde_json::json!([
                    "store-refund-money-v1",
                    cmd.shop_id,
                    cmd.order_id,
                    self.id
                ]))
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

#[derive(Default)]
struct Totals {
    native: i64,
    units: i64,
    retired: i64,
    quantity: i64,
    quantity_native: i64,
}

fn money_error(error: sqlx::Error) -> Error {
    if error.as_database_error().and_then(|e| e.code()).as_deref() == Some("P1003") {
        Error::Pending
    } else {
        error.into()
    }
}

// All inputs come from the original order and the locked Core cumulative row.
// Go/browser amounts are checked against this result, never used as a quote.
fn plan(cmd: &MoneyCommand, t: &Totals) -> Result<(i64, i64)> {
    let r = cmd.refund.as_ref().ok_or(Error::Invalid)?;
    let native = match &cmd.total {
        Price::Usd(v) => v.amount_minor,
        Price::Credits(v) => v.amount_credits,
    };
    let quantity = cmd.order_snapshot.quantity;
    if t.native < 0
        || t.native > native
        || t.units < 0
        || t.units > cmd.total.credits()?
        || t.retired < 0
        || t.retired > quantity
        || t.quantity < 0
        || t.quantity > quantity
    {
        return Err(Error::Conflict);
    }
    let left = native - t.native;
    let amount = match r.mode {
        RefundMode::Full if r.quantity == quantity - t.retired => left,
        RefundMode::Quantity if r.quantity > 0 && r.quantity <= quantity - t.retired => {
            let q = t.quantity.checked_add(r.quantity).ok_or(Error::Invalid)?;
            if q > quantity {
                return Err(Error::Conflict);
            }
            i64::try_from(i128::from(native) * i128::from(q) / i128::from(quantity))
                .map_err(|_| Error::Invalid)?
                .checked_sub(t.quantity_native)
                .ok_or(Error::Conflict)?
        }
        RefundMode::Amount if r.quantity == 0 => r.amount,
        _ => return Err(Error::Invalid),
    };
    if amount <= 0 || amount > left || amount != r.amount {
        return Err(Error::Conflict);
    }
    let retired = if amount == left {
        quantity - t.retired
    } else if r.mode == RefundMode::Quantity {
        r.quantity
    } else {
        0
    };
    if retired != r.retired_quantity {
        return Err(Error::Conflict);
    }
    let units = match &cmd.total {
        Price::Usd(_) => amount.checked_mul(5000).ok_or(Error::Invalid)?,
        Price::Credits(_) => amount,
    };
    if units > cmd.total.credits()? - t.units {
        return Err(Error::Conflict);
    }
    Ok((amount, units))
}

impl CommerceStore {
    pub(super) async fn partial_returned(
        tx: &mut Transaction<'_, Postgres>,
        oid: i64,
    ) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT coalesce((SELECT refunded_units FROM core_commerce.refund_totals WHERE order_id=$1),0)")
            .bind(oid).fetch_one(&mut **tx).await?)
    }

    pub(super) async fn capture_partial_remainder(
        tx: &mut Transaction<'_, Postgres>,
        oid: i64,
        cmd: &MoneyCommand,
        actor: i64,
        original: i64,
        returned: i64,
    ) -> Result<Outcome> {
        let units = original
            .checked_sub(returned)
            .filter(|n| *n > 0)
            .ok_or(Error::Conflict)?;
        let journal: Option<i64> =
            sqlx::query_scalar("SELECT core_commerce.post_refund_delta($1,$2,$3,$4,$5,true)")
                .bind(oid)
                .bind(&cmd.key)
                .bind(Json(cmd))
                .bind(actor)
                .bind(units)
                .fetch_one(&mut **tx)
                .await
                .map_err(money_error)?;
        Ok(Outcome::Posted {
            journal_id: journal.ok_or(Error::Storage)?,
            entries: vec![],
        })
    }

    pub(super) async fn once_partial_refund(
        &self,
        secret: &str,
        cmd: &MoneyCommand,
        digest: &[u8],
    ) -> Result<Receipt> {
        let spec = cmd.refund.as_ref().ok_or(Error::Invalid)?;
        spec.validate(cmd)?;
        let mut tx = self.begin().await?;
        let actor = crate::identity::authorize_in_transaction(&mut tx, secret).await?;
        let buyer = native(&mut tx, cmd.buyer, false).await?;
        let seller = native(&mut tx, cmd.seller, true).await?;
        authority(&mut tx, &actor, cmd.seller, seller, false).await?;
        let consumption_id = format!(
            "commerce:{}",
            id(&serde_json::json!([cmd.shop_id, cmd.order_id]))
        );
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(&consumption_id)
            .execute(&mut *tx)
            .await?;
        let order = sqlx::query(
            "SELECT * FROM core_commerce.orders WHERE shop_id=$1 AND order_id=$2 FOR UPDATE",
        )
        .bind(&cmd.shop_id)
        .bind(&cmd.order_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Missing)?;
        let Json(economics): Json<Economics> = order.try_get("economics")?;
        let oid: i64 = order.try_get("id")?;
        if economics != cmd.economics()
            || order.try_get::<i64, _>("buyer_account_id")? != buyer
            || order.try_get::<i64, _>("seller_account_id")? != seller
            || order.try_get::<String, _>("capture_principal")? != self.service_principal
        {
            return Err(Error::Conflict);
        }
        // Recheck authority, then exact-byte receipt BEFORE current state/ref
        // checks: an original successful refund remains replayable after later
        // refunds/capture, without executing any new financial command.
        if let Some(saved)=sqlx::query("SELECT command_sha256,receipt FROM core_commerce.refund_operations WHERE operation_key=$1 AND order_id=$2")
            .bind(&cmd.key).bind(oid).fetch_optional(&mut *tx).await? {
            if saved.try_get::<Vec<u8>,_>("command_sha256")?!=digest {return Err(Error::Conflict);}
            let Json(receipt):Json<Receipt>=saved.try_get("receipt")?;
            tx.commit().await?;return Ok(receipt);
        }
        let reserve = order
            .try_get::<Option<i64>, _>("reserve_journal_id")?
            .ok_or(Error::Conflict)?;
        let capture: Option<i64> = order.try_get("settlement_journal_id")?;
        if cmd.payment_ref != reference(reserve)
            || cmd.settlement_ref != capture.map(reference).unwrap_or_default()
            || !matches!(
                order.try_get::<String, _>("state")?.as_str(),
                "paid" | "settled" | "refunded"
            )
        {
            return Err(Error::Conflict);
        }
        sqlx::query(
            "INSERT INTO core_commerce.refund_totals(order_id) VALUES($1) ON CONFLICT DO NOTHING",
        )
        .bind(oid)
        .execute(&mut *tx)
        .await?;
        let counters =
            sqlx::query("SELECT * FROM core_commerce.refund_totals WHERE order_id=$1 FOR UPDATE")
                .bind(oid)
                .fetch_one(&mut *tx)
                .await?;
        let totals = Totals {
            native: counters.try_get("refunded_native")?,
            units: counters.try_get("refunded_units")?,
            retired: counters.try_get("retired_quantity")?,
            quantity: counters.try_get("quantity_basis")?,
            quantity_native: counters.try_get("quantity_native")?,
        };
        let (amount, units) = match plan(cmd, &totals) {
            Ok(value) => value,
            Err(Error::Conflict | Error::Invalid) => {
                // A known cap/selection rejection has no money effect. Persist
                // the exact command's negative receipt, so a concurrent loser
                // is recoverable rather than becoming permanently UNKNOWN.
                // These columns record its requested delta; only successful
                // receipts change the authoritative cumulative counters.
                let units = match &cmd.total {
                    Price::Usd(_) => spec.amount.checked_mul(5000).ok_or(Error::Invalid)?,
                    Price::Credits(_) => spec.amount,
                };
                let receipt = Receipt {
                    key: cmd.key.clone(),
                    command_hash: hex(digest),
                    reference: format!("commerce:refund:{}", spec.id),
                    outcome: "rejected".into(),
                    rejection_code: "refund_plan_or_limit_conflict".into(),
                };
                sqlx::query("INSERT INTO core_commerce.refund_operations(operation_key,refund_id,order_id,command_sha256,command,actor_user_id,amount_units,amount_native,retired_quantity,journal_id,receipt) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,NULL,$10)")
                    .bind(&cmd.key).bind(&spec.id).bind(oid).bind(digest).bind(Json(cmd)).bind(actor.user_id)
                    .bind(units).bind(spec.amount).bind(spec.retired_quantity).bind(Json(&receipt)).execute(&mut *tx).await?;
                tx.commit().await?;
                return Ok(receipt);
            }
            Err(error) => return Err(error),
        };
        let journal: Option<i64> =
            sqlx::query_scalar("SELECT core_commerce.post_refund_delta($1,$2,$3,$4,$5,false)")
                .bind(oid)
                .bind(&cmd.key)
                .bind(Json(cmd))
                .bind(actor.user_id)
                .bind(units)
                .fetch_one(&mut *tx)
                .await
                .map_err(money_error)?;
        let receipt = Receipt {
            key: cmd.key.clone(),
            command_hash: hex(digest),
            reference: journal
                .map(reference)
                .unwrap_or_else(|| format!("commerce:refund:{}", spec.id)),
            outcome: "succeeded".into(),
            rejection_code: String::new(),
        };
        sqlx::query("INSERT INTO core_commerce.refund_operations(operation_key,refund_id,order_id,command_sha256,command,actor_user_id,amount_units,amount_native,retired_quantity,journal_id,receipt) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(&cmd.key).bind(&spec.id).bind(oid).bind(digest).bind(Json(cmd)).bind(actor.user_id)
            .bind(units).bind(amount).bind(spec.retired_quantity).bind(journal).bind(Json(&receipt)).execute(&mut *tx).await?;
        let version:i64=sqlx::query_scalar("UPDATE core_commerce.refund_totals SET refunded_units=refunded_units+$2,refunded_native=refunded_native+$3,retired_quantity=retired_quantity+$4,quantity_basis=quantity_basis+$5,quantity_native=quantity_native+$6,revision=revision+1 WHERE order_id=$1 RETURNING revision")
            .bind(oid).bind(units).bind(amount).bind(spec.retired_quantity)
            .bind(if spec.mode==RefundMode::Quantity {spec.quantity}else{0})
            .bind(if spec.mode==RefundMode::Quantity {amount}else{0}).fetch_one(&mut *tx).await?;
        let changed=sqlx::query("UPDATE core_charging.external_consumption SET refunded=refunded+$2 WHERE id=$1 AND refunded<=amount_credits-$2")
            .bind(&consumption_id).bind(units).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(Error::Conflict);
        }
        if totals.units + units == cmd.total.credits()? {
            sqlx::query(
                "UPDATE core_commerce.orders SET state='refunded',revision=revision+1 WHERE id=$1",
            )
            .bind(oid)
            .execute(&mut *tx)
            .await?;
        }
        let payload = serde_json::to_vec(&receipt).map_err(|_| Error::Storage)?;
        EventStore::publish(
            &mut tx,
            NewEvent {
                key: &format!("commerce:{}", cmd.key),
                event_type: "commerce.refund.v1",
                schema_version: 1,
                resource_id: oid,
                resource_version: version,
                payload: &payload,
            },
        )
        .await
        .map_err(|e| match e {
            crate::events::EventError::Busy => Error::Retry,
            _ => Error::Storage,
        })?;
        tx.commit().await?;
        Ok(receipt)
    }
}

//! Provider identifiers belong to one original payment subject across products.
use sqlx::PgConnection;

use super::{Channel, Error, Result, stripe, valid_id};

pub(crate) enum Subject<'a> {
    Native(i64),
    Subscription(&'a str),
    Legacy(&'a str),
}

pub(crate) async fn claim(
    connection: &mut PgConnection,
    channel: &Channel,
    subject: Subject<'_>,
    mut identifiers: Vec<(&str, &str)>,
) -> Result<()> {
    identifiers.sort_unstable();
    identifiers.dedup();
    if let Subject::Subscription(id) = subject {
        for (kind, identifier) in identifiers {
            if !matches!(kind, "event" | "order" | "payment") || !valid_id(identifier) {
                return Err(Error::Invalid);
            }
            sqlx::query("INSERT INTO core_payments.provider_identifiers(provider,merchant,environment,kind,identifier,subscription_order_id) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
                .bind(&channel.provider).bind(&channel.merchant).bind(&channel.environment).bind(kind).bind(identifier).bind(id).execute(&mut *connection).await?;
            let owns:bool=sqlx::query_scalar("SELECT subscription_order_id IS NOT DISTINCT FROM $6 FROM core_payments.provider_identifiers WHERE provider=$1 AND merchant=$2 AND environment=$3 AND kind=$4 AND identifier=$5")
                .bind(&channel.provider).bind(&channel.merchant).bind(&channel.environment).bind(kind).bind(identifier).bind(id).fetch_one(&mut *connection).await?;
            if !owns {
                return Err(Error::Conflict);
            }
        }
        return Ok(());
    }
    let (native, legacy) = match subject {
        Subject::Native(id) => (Some(id), None),
        Subject::Legacy(id) => (None, Some(id)),
        Subject::Subscription(_) => unreachable!(),
    };
    for (kind, identifier) in identifiers {
        if !matches!(kind, "event" | "order" | "payment") || !valid_id(identifier) {
            return Err(Error::Invalid);
        }
        sqlx::query("INSERT INTO core_payments.provider_identifiers(provider,merchant,environment,kind,identifier,native_intent_id,legacy_basis_id) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING")
            .bind(&channel.provider).bind(&channel.merchant).bind(&channel.environment)
            .bind(kind).bind(identifier).bind(native).bind(legacy)
            .execute(&mut *connection).await?;
        let owns: bool = sqlx::query_scalar("SELECT native_intent_id IS NOT DISTINCT FROM $6 AND legacy_basis_id IS NOT DISTINCT FROM $7 FROM core_payments.provider_identifiers WHERE provider=$1 AND merchant=$2 AND environment=$3 AND kind=$4 AND identifier=$5")
            .bind(&channel.provider).bind(&channel.merchant).bind(&channel.environment)
            .bind(kind).bind(identifier).bind(native).bind(legacy)
            .fetch_one(&mut *connection).await?;
        if !owns {
            return Err(Error::Conflict);
        }
    }
    Ok(())
}

pub(super) async fn native(
    connection: &mut PgConnection,
    channel: &Channel,
    intent_id: i64,
    verified: &stripe::Verified,
    signed_payload: &[u8],
) -> Result<()> {
    let mut aliases = vec![("event", verified.event_id.as_str())];
    let order;
    if verified.kind == stripe::EventKind::Credit {
        aliases.push(("payment", verified.transaction.as_str()));
        if channel.provider == "waffo_pancake" {
            let event = stripe::unique_json(signed_payload)?;
            order = event
                .get("data")
                .and_then(|v| v.get("orderId"))
                .and_then(serde_json::Value::as_str)
                .ok_or(Error::Invalid)?
                .to_owned();
            aliases.push(("order", order.as_str()));
        }
    }
    claim(connection, channel, Subject::Native(intent_id), aliases).await
}

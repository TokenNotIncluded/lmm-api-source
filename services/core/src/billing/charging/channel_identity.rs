//! Reserve Native routing identities in the caller's existing funding transaction.
use super::{Charge, Error};
use sqlx::PgConnection;
use std::collections::BTreeSet;

pub(in crate::billing::charging) async fn reserve(
    c: &mut PgConnection,
    charge: &Charge,
) -> Result<(), Error> {
    let mut ids = BTreeSet::new();
    if let Some(image) = &charge.price.image {
        let quote = image.clone().restore().map_err(|_| Error::Invalid)?;
        ids.insert(quote.binding().upstream_id);
    } else if let Some(contract) = super::load(c, charge).await? {
        ids.insert(contract.fields.binding.upstream_id);
        ids.extend(
            contract
                .fields
                .fallbacks
                .iter()
                .map(|f| f.binding.upstream_id),
        );
    }
    // Legacy reservations without frozen Native evidence do not invent IDs.
    // All identity locks precede budget/entitlement/ledger writes in hold().
    for id in ids {
        let id = id as i64;
        let inserted: Option<i64> = sqlx::query_scalar(
            "INSERT INTO core_channels.identities(id,origin) VALUES($1,'bootstrap') \
             ON CONFLICT(id) DO NOTHING RETURNING id",
        )
        .bind(id)
        .fetch_optional(&mut *c)
        .await?;
        let retired: bool = sqlx::query_scalar(
            "SELECT retired_at IS NOT NULL FROM core_channels.identities WHERE id=$1 FOR SHARE",
        )
        .bind(id)
        .fetch_one(&mut *c)
        .await?;
        if retired {
            return Err(Error::Conflict);
        }
        if inserted.is_some() {
            sqlx::query(
                "INSERT INTO core_channels.counter_baselines(channel_id,historical_coverage) \
                 VALUES($1,FALSE)",
            )
            .bind(id)
            .execute(&mut *c)
            .await?;
            sqlx::query(
                "INSERT INTO core_channels.balance_observations(channel_id,kind) VALUES($1,'unknown')",
            )
            .bind(id)
            .execute(&mut *c)
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "channel_identity_tests.rs"]
mod tests;

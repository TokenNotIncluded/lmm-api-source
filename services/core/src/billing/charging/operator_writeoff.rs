//! Explicit owner waiver of an unresolved model hold. Provider evidence stays intact.
use super::*;
use serde::Deserialize;

const KIND: &str = "operator-model-writeoff:v1";
const MAX_BATCH: usize = 500;

/// Private input for the offline model write-off command.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorWriteoffRequest {
    pub request_ids: Vec<String>,
    pub reason: String,
    pub confirmation_digest: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    request_id: String,
    model: String,
    group: String,
    payer_account_id: i64,
    revision: i64,
    release_credits: i64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Audit {
    kind: String,
    operator: String,
    reason: String,
    confirmation_digest: String,
    entry: Entry,
}

impl Charging {
    /// Preview or release the listed unknown wallet holds as the database owner.
    ///
    /// # Errors
    ///
    /// Rejects non-owner connections, changed confirmations and requests outside
    /// unresolved native text wallet charging.
    pub async fn operator_writeoff(&self, mut request: OperatorWriteoffRequest) -> Result<Value> {
        if !(1..=MAX_BATCH).contains(&request.request_ids.len())
            || request.request_ids.iter().any(|id| {
                id.len() != 64
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            })
            || request.reason.trim().is_empty()
            || request.reason.len() > 512
            || request.reason.chars().any(char::is_control)
        {
            return Err(Error::Invalid);
        }
        request.request_ids.sort_unstable();
        if request
            .request_ids
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(Error::Invalid);
        }

        let mut tx = self.begin().await?;
        let applying = request.confirmation_digest.is_some();
        if !applying {
            sqlx::query("SET TRANSACTION READ ONLY")
                .execute(&mut *tx)
                .await?;
        }
        let (database, operator, owner): (String, String, bool) = sqlx::query_as(
            "SELECT pg_catalog.current_database()::text, session_user::text, \
             (current_user=session_user OR EXISTS(SELECT FROM pg_catalog.pg_roles \
                 WHERE rolname=current_user AND NOT rolcanlogin)) \
             AND (SELECT pg_catalog.pg_get_userbyid(datdba)=current_user \
                  FROM pg_catalog.pg_database WHERE datname=pg_catalog.current_database()) \
             AND (SELECT pg_catalog.count(*)=4 \
                  AND pg_catalog.bool_and(pg_catalog.pg_get_userbyid(relowner)=current_user) \
                  FROM pg_catalog.pg_class WHERE oid IN ( \
                     pg_catalog.to_regclass('core_charging.requests'), \
                     pg_catalog.to_regclass('core_charging.usage_events'), \
                     pg_catalog.to_regclass('core_charging.ledger_commands'), \
                     pg_catalog.to_regclass('core_charging.ledger_holds')))",
        )
        .fetch_one(&mut *tx)
        .await?;
        if !owner {
            return Err(Error::Forbidden);
        }

        let mut entries = Vec::with_capacity(request.request_ids.len());
        let mut pending = Vec::with_capacity(request.request_ids.len());
        for id in &request.request_ids {
            if applying {
                lock(&mut tx, id).await?;
            }
            let charge = select_stored(&mut tx, id, applying)
                .await?
                .ok_or(Error::NotFound)?
                .charge;
            if let Some(digest) = &request.confirmation_digest
                && charge.state == "released"
            {
                entries.push(replayed(&mut tx, &charge, digest, &request.reason).await?);
                continue;
            }
            entries.push(prepare(&mut tx, &charge).await?);
            pending.push(charge);
        }

        let digest = crate::hex_codec::encode(hash(&(
            KIND,
            &database,
            &operator,
            &request.reason,
            &entries,
        ))?);
        let total = entries.iter().try_fold(0_i64, |sum, entry| {
            sum.checked_add(entry.release_credits).ok_or(Error::Invalid)
        })?;
        let receipt = json!({
            "mode": if applying { "applied" } else { "preview" },
            "kind": KIND,
            "operator": operator,
            "confirmation_digest": digest,
            "reason": request.reason,
            "charge_credits": 0,
            "release_credits": total,
            "entries": entries,
        });
        let Some(confirmed) = &request.confirmation_digest else {
            tx.rollback().await?;
            return Ok(receipt);
        };
        if confirmed != &digest || (!pending.is_empty() && pending.len() != entries.len()) {
            return Err(Error::Conflict);
        }
        for (charge, entry) in pending.iter().zip(entries) {
            self.release_hold(
                &mut tx,
                charge,
                json(&Audit {
                    kind: KIND.into(),
                    operator: operator.clone(),
                    reason: request.reason.clone(),
                    confirmation_digest: digest.clone(),
                    entry,
                })?,
            )
            .await?;
        }
        commit(tx).await?;
        Ok(receipt)
    }
}

async fn prepare(c: &mut PgConnection, charge: &Charge) -> Result<Entry> {
    if charge.state != "reconcile"
        || charge.source != Source::Wallet
        || charge.key_id.is_none()
        || charge.oauth_source.is_some()
        || charge.price.image.is_some()
        || charge.price.audio.is_some()
        || charge.price.tool.is_some()
        || charge.price.text_request.is_some()
        || charge.price.grounding.is_some()
        || charge.usage != Usage::default()
        || charge.settled != 0
        || charge.refunded != 0
        || charge.reserved <= 0
    {
        return Err(Error::WrongState);
    }
    let eligible: bool = sqlx::query_scalar(
        "SELECT COALESCE(sum(reserved),0)=$2 \
         AND COALESCE(bool_and(NOT closed AND captured=0 AND refunded=0),false) \
         FROM core_charging.ledger_holds WHERE request_id=$1",
    )
    .bind(&charge.id)
    .bind(charge.reserved)
    .fetch_one(c)
    .await?;
    if !eligible {
        return Err(Error::Conflict);
    }
    Ok(Entry {
        request_id: charge.id.clone(),
        model: charge.price.model.clone(),
        group: charge.price.group.clone(),
        payer_account_id: charge.payer_account_id,
        revision: charge.revision,
        release_credits: charge.reserved,
    })
}

async fn replayed(
    c: &mut PgConnection,
    charge: &Charge,
    digest: &str,
    reason: &str,
) -> Result<Entry> {
    let event: Value = sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='release'",
    )
    .bind(&charge.id)
    .fetch_optional(c)
    .await?
    .ok_or(Error::Conflict)?;
    let audit: Audit = serde_json::from_value(event).map_err(|_| Error::Conflict)?;
    let entry = audit.entry;
    if audit.kind != KIND
        || audit.confirmation_digest != digest
        || audit.reason != reason
        || entry.request_id != charge.id
        || entry.payer_account_id != charge.payer_account_id
        || entry.release_credits != charge.reserved
        || entry.revision.checked_add(1) != Some(charge.revision)
    {
        return Err(Error::Conflict);
    }
    Ok(entry)
}

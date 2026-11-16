//! Exact native consumption totals using the same receipts as the usage views.
use super::{InvalidReceipt, SELECT_REQUESTS, validate_usage_batch};
use sqlx::PgConnection;

#[derive(Default)]
pub(crate) struct PersonalUsageSummary {
    pub(crate) settled_requests: u128,
    pub(crate) pending_requests: u128,
    pub(crate) gross_settled: u128,
    pub(crate) refunded: u128,
    pub(crate) personal_wallet: u128,
    pub(crate) team_wallet: u128,
    pub(crate) subscription: u128,
    pub(crate) sponsored: u128,
}

/// Caller authorizes the target in this read-only transaction. Actor plus the
/// user's personal owner excludes team-owned keys while retaining personal keys
/// paid by a team. Historical opening counters and imported logs stay separate.
pub(crate) async fn read(
    connection: &mut PgConnection,
    user: i64,
    owner: i64,
) -> Result<Option<PersonalUsageSummary>, InvalidReceipt> {
    let available: bool = sqlx::query_scalar(
        "SELECT count(*)=4 AND COALESCE(bool_and(has_schema_privilege(n.oid,'USAGE') AND has_table_privilege(c.oid,'SELECT')),FALSE)
         FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
         WHERE n.nspname='core_charging' AND c.relkind='r'
         AND c.relname IN ('requests','usage_events','ledger_commands','refunds')",
    )
    .fetch_one(&mut *connection)
    .await?;
    if !available {
        return Ok(None);
    }
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
            .fetch_one(&mut *connection)
            .await?;
    let sql = format!("DECLARE lmm_admin_user_usage_reader NO SCROLL CURSOR FOR {SELECT_REQUESTS}");
    sqlx::query(sqlx::AssertSqlSafe::<&str>(&sql))
        .bind(user)
        .bind(owner)
        .bind(0_i64)
        .bind(now)
        .bind(0_i16)
        .bind("")
        .bind("")
        .bind("")
        .bind("")
        .execute(&mut *connection)
        .await?;
    let mut total = PersonalUsageSummary::default();
    let mut previous_id: Option<String> = None;
    loop {
        let rows = sqlx::query("FETCH FORWARD 64 FROM lmm_admin_user_usage_reader")
            .fetch_all(&mut *connection)
            .await?;
        let count = rows.len();
        for request in validate_usage_batch(connection, rows)
            .await
            .map_err(|_| InvalidReceipt)?
        {
            if previous_id.as_deref() == Some(request.id.as_str()) {
                return Err(InvalidReceipt);
            }
            previous_id = Some(request.id.clone());
            if request.held() {
                total.pending_requests += 1;
            }
            if !request.settled() {
                continue;
            }
            // Receipt validation bounds individual values; u128 retains exact
            // cumulative totals even after they exceed JavaScript's safe range.
            total.settled_requests += 1;
            total.gross_settled += request.amounts.gross as u128;
            total.refunded += request.amounts.refunded as u128;
            let net = request.amounts.net() as u128;
            if request.sponsored {
                total.sponsored += net;
            } else if request.subscription() {
                total.subscription += net;
            } else if request.payer_kind == "team" {
                total.team_wallet += net;
            } else if request.payer_kind == "personal" && request.payer_is_owner {
                total.personal_wallet += net;
            } else {
                return Err(InvalidReceipt);
            }
        }
        if count < 64 {
            break;
        }
    }
    sqlx::query("CLOSE lmm_admin_user_usage_reader")
        .execute(connection)
        .await?;
    Ok(Some(total))
}

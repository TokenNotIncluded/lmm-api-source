//! Native channel consumption, read from one SQL snapshot without ledger writes.
use super::channel_projection;
use serde::Serialize;
use sqlx::PgConnection;

#[derive(Debug, Serialize)]
pub(crate) struct ChannelUsage {
    pub scope: &'static str,
    pub start_timestamp: i64,
    pub end_timestamp: i64,
    pub gross_settled_credits: String,
    pub refunded_credits: String,
    pub net_used_credits: String,
    pub settled_requests: String,
    pub pending_potential_requests: String,
    pub unknown_provenance_requests: String,
    pub credits_per_usd: &'static str,
    pub historical_coverage: bool,
}

#[derive(Debug)]
pub(crate) enum ChannelUsageError {
    Storage,
    InvalidReceipt,
    HistoryAmbiguous,
    Unavailable,
}
impl From<sqlx::Error> for ChannelUsageError {
    fn from(_: sqlx::Error) -> Self {
        Self::Storage
    }
}
/// Caller validates the 31-day window and owns the read-only snapshot. No
/// archived logs, live provider calls, current routes or wallet balances enter it.
pub(crate) async fn channel_usage(
    c: &mut PgConnection,
    channel_id: u64,
    start: i64,
    end: i64,
) -> Result<ChannelUsage, ChannelUsageError> {
    aggregate(c, channel_id, start, end, "native_channel_window").await
}

/// Own Native consumption across all stored facts visible in this snapshot,
/// without a transaction-start or 31-day cutoff. The timestamps describe the
/// full supported time domain; the caller adds any inheritance baseline.
pub(crate) async fn channel_totals(
    c: &mut PgConnection,
    channel_id: u64,
) -> Result<ChannelUsage, ChannelUsageError> {
    aggregate(c, channel_id, 0, 253402300799, "native_channel_totals").await
}

async fn aggregate(
    c: &mut PgConnection,
    channel_id: u64,
    start: i64,
    end: i64,
    scope: &'static str,
) -> Result<ChannelUsage, ChannelUsageError> {
    let ambiguous: Option<bool> =
        sqlx::query_scalar("SELECT history_ambiguous OR retired_at IS NOT NULL FROM core_channels.identities WHERE id=$1")
            .bind(channel_id as i64)
            .fetch_optional(&mut *c)
            .await?;
    if ambiguous == Some(true) {
        return Err(ChannelUsageError::HistoryAmbiguous);
    }
    channel_projection::read_usage(c, channel_id, start, end, scope).await
}

//! Administrator projection of the native personal-usage receipt reader.
use super::*;

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UsageProjection {
    Available(Box<UsageSummary>),
    Unavailable { code: &'static str },
}

#[derive(Serialize)]
pub struct UsageSummary {
    unit: &'static str,
    scope: &'static str,
    historical_imports_included: bool,
    settled_requests: String,
    pending_requests: String,
    gross_settled_units: String,
    refunded_units: String,
    net_consumption_units: String,
    personal_wallet_consumption_units: String,
    team_wallet_consumption_units: String,
    subscription_consumption_units: String,
    sponsored_consumption_units: String,
}

pub(super) async fn read(
    connection: &mut PgConnection,
    user: i64,
    owner: i64,
) -> ReadResult<UsageProjection> {
    let Some(total) = crate::usage::personal_summary::read(connection, user, owner)
        .await
        .map_err(|_| AdminUsersError::Unavailable)?
    else {
        return Ok(UsageProjection::Unavailable {
            code: "USAGE_PROJECTION_UNAVAILABLE",
        });
    };
    Ok(UsageProjection::Available(Box::new(UsageSummary {
        unit: "core_credit",
        scope: "actor_personal_usage",
        historical_imports_included: false,
        settled_requests: total.settled_requests.to_string(),
        pending_requests: total.pending_requests.to_string(),
        gross_settled_units: total.gross_settled.to_string(),
        refunded_units: total.refunded.to_string(),
        net_consumption_units: (total.gross_settled - total.refunded).to_string(),
        personal_wallet_consumption_units: total.personal_wallet.to_string(),
        team_wallet_consumption_units: total.team_wallet.to_string(),
        subscription_consumption_units: total.subscription.to_string(),
        sponsored_consumption_units: total.sponsored.to_string(),
    })))
}

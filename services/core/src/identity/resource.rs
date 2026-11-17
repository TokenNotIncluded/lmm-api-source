//! New resource admission only. This is not a general authentication check.
//! Existing reservations, payment callbacks and refunds retain their authority.
use super::{IdentityError, Result};
use sqlx::PgConnection;

/// Each caller must name the new action being admitted. Never use this helper
/// in shared authorization, key management, read projections or settlement.
pub(crate) enum NewResourceAction {
    CommercePayment,
    PaymentIntent,
    AssistantConversation,
    AssistantTurn,
    MarketAuthorization,
}

/// Caller must already hold canonical credential/user/account SHARE locks in
/// this same transaction, until the new action's durable admission commits.
/// Mark/unmark owns the user's UPDATE lock. Serializable callers also require
/// the mark writer to touch that user row so an older waiting snapshot aborts.
pub(crate) async fn reject_marked_new_action(
    connection: &mut PgConnection,
    user_id: i64,
    _purpose: NewResourceAction,
) -> Result<()> {
    let marked = crate::abuse::is_marked_in_transaction(connection, user_id)
        .await
        .map_err(|error| match error {
            crate::abuse::AbuseError::Conflict => IdentityError::Conflict,
            _ => IdentityError::Storage,
        })?;
    if marked {
        Err(IdentityError::Forbidden)
    } else {
        Ok(())
    }
}

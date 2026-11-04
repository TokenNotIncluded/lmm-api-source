//! Credential creation facts, never login-provider bindings or current key state.
use super::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionMilestone {
    pub user_id: i64,
    pub api_key_created_at: Option<i64>,
    pub oauth_authorized_at: Option<i64>,
    pub history_complete: bool,
    pub application_submitted_at: Option<i64>,
    pub access_approved_at: Option<i64>,
    pub application_history_complete: bool,
    pub approval_history_complete: bool,
    pub history_source: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionMilestones {
    pub users: Vec<ConnectionMilestone>,
}

impl CommunityRead {
    pub async fn connection_milestones(
        &self,
        r: &UsersInfoRequest,
    ) -> Result<ConnectionMilestones> {
        if r.user_ids.is_empty() || r.user_ids.len() > 100 || r.user_ids.iter().any(|id| *id <= 0) {
            return Err(Error::Invalid);
        }
        let mut tx = self.snapshot().await?;
        let rows = sqlx::query(super::connection_history::SQL)
            .bind(&r.user_ids)
            .fetch_all(&mut *tx)
            .await?;
        let users = rows
            .into_iter()
            .map(|row| {
                Ok(ConnectionMilestone {
                    user_id: row.try_get("user_id")?,
                    api_key_created_at: row.try_get("key_at")?,
                    oauth_authorized_at: row.try_get("oauth_at")?,
                    history_complete: row.try_get("complete")?,
                    application_submitted_at: row.try_get("application_at")?,
                    access_approved_at: row.try_get("approval_at")?,
                    application_history_complete: row.try_get("application_complete")?,
                    approval_history_complete: row.try_get("approval_complete")?,
                    history_source: row.try_get("history_source")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(ConnectionMilestones { users })
    }
}

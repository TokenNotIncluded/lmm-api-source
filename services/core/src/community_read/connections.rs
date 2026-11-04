//! Credential creation facts, never login-provider bindings or current key state.
use super::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionMilestone {
    pub user_id: i64,
    pub api_key_created_at: Option<i64>,
    pub oauth_authorized_at: Option<i64>,
    pub history_complete: bool,
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
        // Cutover credentials erase oauth_managed and also contain unsupported
        // managed keys as revoked api_key rows. Never classify those rows by name,
        // revoked state or secret prefix. Until a source-bound historical contract
        // exists, no purported key creation or exact first time leaves Core for
        // imported users. Native browser grants still prove a positive milestone.
        let rows = sqlx::query(
            r#"WITH users AS (
 SELECT u.id,NOT (
 EXISTS(SELECT 1 FROM core_billing.opening_imports o WHERE o.account_id=u.personal_account_id)
 OR EXISTS(SELECT 1 FROM core_migration.import_runs i WHERE
 coalesce(i.manifest->>'scope','identity-only')='identity-only'
 OR (i.manifest->>'scope'='legacy-cutover-identity.v1'
 AND i.manifest->'registration_times' ? u.personal_account_id::text))
 ) AS complete FROM core_identity.users u WHERE u.id=ANY($1)
)
SELECT u.id AS user_id,u.complete,
 CASE WHEN u.complete THEN (SELECT floor(extract(epoch FROM min(c.created_at)))::bigint
 FROM core_identity.credentials c WHERE c.user_id=u.id AND c.kind='api_key'
 AND c.created_at>to_timestamp(0)) END AS key_at,
 (SELECT floor(extract(epoch FROM min(f.created_at)))::bigint FROM (
 SELECT f.created_at FROM core_oauth.families f
 WHERE f.user_id=u.id AND f.issuance_kind='browser' AND f.created_at>to_timestamp(0)
 UNION ALL SELECT m.created_at FROM core_market_oauth.families m
 WHERE m.user_id=u.id AND m.created_at>to_timestamp(0)
 ) f) AS oauth_at
 FROM users u ORDER BY u.id"#,
        )
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
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(ConnectionMilestones { users })
    }
}

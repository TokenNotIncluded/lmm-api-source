//! Exact Core84 -> Core85; existing identities and paid model evidence survive.
use super::*;
pub(super) const PROCEDURE: &str = "core-assistant-confirmed-actions-v1";
const TARGET: &str = include_str!("assistant_confirmed_actions_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::ASSISTANT_CONFIRMED_ACTIONS,
    procedure: PROCEDURE,
    predecessor_count: 84,
    target_count: 85,
    predecessor_manifest: "8d80da1b5b94cf003a58bc4c3887fc66eb94a92447c11a823ae7fa6c250e2802",
    predecessor_catalog: "38813c20f3b2904e552ba148343e1e27f2379fdf85faa6b7565d89ed32cd5b60",
    target_json: TARGET,
    receipt: "core_meta.assistant_confirmed_actions_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("ASSISTANT_CONFIRMED_ACTIONS_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(84).map(|v| v.0) != Some("assistant-confirmed-actions") {
        return Err(reject("ASSISTANT_CONFIRMED_ACTIONS_COMPONENT_CHANGED"));
    }
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    captured()?;
    ENGINE.connected(c, r).await
}

// Only an already enabled confirmation coordinator receives these new column
// writes. Existing table privileges, owner receipts and disabled modules stay
// exactly as recorded by the predecessor.
pub(super) async fn grant_runtime(c: &mut PgConnection, role: &str) -> Result<(), UpgradeError> {
    let enabled: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege($1,'core_assistant_tools','USAGE')
          AND has_table_privilege($1,'core_assistant_tools.confirmations','SELECT')
          AND has_table_privilege($1,'core_assistant_tools.confirmations','INSERT')
          AND NOT EXISTS(SELECT FROM unnest(ARRAY['nonce_sha256','state','confirmed_at']) AS columns(name)
              WHERE NOT has_column_privilege($1,'core_assistant_tools.confirmations',columns.name,'UPDATE'))",
    ).bind(role).fetch_one(&mut *c).await?;
    if enabled {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT UPDATE(execution_receipt,execution_sha256,executed_at) ON core_assistant_tools.confirmations TO {role}; GRANT SELECT,INSERT ON core_assistant_tools.key_creation_cards TO {role}; GRANT UPDATE(nonce_sha256,ciphertext,consumed_at) ON core_assistant_tools.key_creation_cards TO {role}; GRANT EXECUTE ON FUNCTION core_assistant_tools.market_confirmation_bound(TEXT) TO {role}",
            role=quote(role)
        ))).execute(c).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "assistant_confirmed_actions_tests.rs"]
mod tests;

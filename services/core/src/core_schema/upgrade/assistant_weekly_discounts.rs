//! Exact Core87 -> Core88; owner activation and history import remain explicit.
use super::*;
pub(super) const PROCEDURE: &str = "core-assistant-weekly-discounts-v1";
const TARGET: &str = include_str!("assistant_weekly_discounts_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::ASSISTANT_WEEKLY_DISCOUNTS,
    procedure: PROCEDURE,
    predecessor_count: 87,
    target_count: 88,
    predecessor_manifest: "9df112cfc22c9f4493f3903189fbb084896d59e3c36a38981132c4651b6aaca1",
    predecessor_catalog: "f8debeb9f33cd3cca6d303d554bf8e234183e8e1f8faf9d0e0266007a9eb9882",
    target_json: TARGET,
    receipt: "core_meta.assistant_weekly_discount_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("ASSISTANT_WEEKLY_DISCOUNTS_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(87).map(|v| v.0) != Some("assistant-weekly-discounts") {
        return Err(reject("ASSISTANT_WEEKLY_DISCOUNTS_COMPONENT_CHANGED"));
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
pub(super) async fn grant_runtime(c: &mut PgConnection, role: &str) -> Result<(), UpgradeError> {
    // Preserve capability switches from the observed predecessor. No policy
    // updates, history imports, coupon administration or sequence access.
    let (reader, browser, agent): (bool, bool, bool) = sqlx::query_as(
        "SELECT
           has_schema_privilege($1,'core_payments','USAGE')
             AND has_table_privilege($1,'core_payments.discount_codes','SELECT'),
           has_schema_privilege($1,'core_assistant_tools','USAGE')
             AND has_schema_privilege($1,'core_payments','USAGE')
             AND has_table_privilege($1,'core_assistant_tools.confirmations','SELECT')
             AND has_column_privilege($1,'core_assistant_tools.confirmations','state','UPDATE'),
           has_schema_privilege($1,'core_assistant','USAGE')
             AND has_table_privilege($1,'core_assistant.agent_tool_operations','INSERT')",
    )
    .bind(role)
    .fetch_one(&mut *c)
    .await?;
    for (enabled, sql) in [
        (
            reader || browser,
            include_str!("../../../schema/assistant_weekly_discount_owner_grants.sql"),
        ),
        (
            browser,
            include_str!("../../../schema/assistant_weekly_discount_browser_grants.sql"),
        ),
        (
            agent,
            include_str!("../../../schema/assistant_weekly_discount_agent_grants.sql"),
        ),
    ] {
        if enabled {
            let sql = sql.replace(":\"runtime_role\"", &quote(role));
            sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(&sql))
                .execute(&mut *c)
                .await?;
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "assistant_weekly_discounts_tests.rs"]
mod tests;

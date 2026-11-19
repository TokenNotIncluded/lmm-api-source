//! Explicit 76 -> 77 history-import contract; never invoked by startup.
use super::*;

pub(super) const PROCEDURE: &str = "core-connection-history-import-v1";
const TARGET: &str = include_str!("connection_history_import_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::CONNECTION_HISTORY_IMPORT,
    procedure: PROCEDURE,
    predecessor_count: 76,
    target_count: 77,
    predecessor_manifest: "2c14283c2d35580f3fe314b7de6a88e3aec3524e74157310bf5022415367ff8e",
    predecessor_catalog: "bed3e7e3ad2810bda942473b660e4348294dab31e8fdc72e52cb5693507158db",
    target_json: TARGET,
    receipt: "core_meta.connection_history_import_upgrades",
};

pub(super) fn validate(request: &UpgradeRequest) -> Result<(), UpgradeError> {
    ENGINE.validate(request)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.get(76).map(|value| value.0) != Some("connection-history-import") {
        return Err(reject("CONNECTION_HISTORY_IMPORT_COMPONENT_CHANGED"));
    }
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    connection: &mut PgConnection,
    request: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    ENGINE.connected(connection, request).await
}

#[cfg(test)]
#[path = "connection_history_import_tests.rs"]
mod tests;

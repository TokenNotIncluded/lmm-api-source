//! Exact frozen 32-component predecessor to the explicit 43-component successor.
use super::*;
pub(super) const PROCEDURE: &str = "core-functional-completion-v1";
const PRE_MANIFEST: &str = "9c9726a7fa204a150d9fe18139d4580d23feb4cec40145fcb829338331a4cd15";
const PRE_CATALOG: &str = "abe3979b376da869d4a05b32dda8c3ad33197497db7af16970d8c9b1153bc6ce";
const TARGET: &str = include_str!("completion_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    procedure: PROCEDURE,
    predecessor_count: 32,
    predecessor_manifest: PRE_MANIFEST,
    predecessor_catalog: PRE_CATALOG,
    target_json: TARGET,
    receipt: "core_meta.completion_upgrades",
};
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    ENGINE.connected(c, r).await
}

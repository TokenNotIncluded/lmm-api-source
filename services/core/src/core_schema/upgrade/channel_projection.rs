//! Exact frozen 43-component predecessor to the explicit 44-component successor.
use super::*;
pub(super) const PROCEDURE: &str = "core-channel-projection-v1";
const PRE_MANIFEST: &str = "f12d5e81ca219acdc59ac07be931d52e46f0b64974f1e2b3154ad102b5cc0c1c";
const PRE_CATALOG: &str = "1f9ef4951b101320e4016f2e8347985c7a321b4c3405fe756864abba1bf7c680";
const TARGET: &str = include_str!("channel_projection_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    procedure: PROCEDURE,
    predecessor_count: 43,
    target_count: 44,
    predecessor_manifest: PRE_MANIFEST,
    predecessor_catalog: PRE_CATALOG,
    target_json: TARGET,
    receipt: "core_meta.channel_projection_upgrades",
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

pub(super) async fn check_ready(c: &mut PgConnection) -> Result<(), UpgradeError> {
    let ready: Option<bool> =
        sqlx::query_scalar("SELECT ready AND NOT EXISTS(SELECT FROM core_channels.projection_dirty) FROM core_channels.projection_state WHERE singleton")
            .fetch_optional(c)
            .await?;
    if ready != Some(true) {
        return Err(reject("CHANNEL_PROJECTION_BACKFILL_INCOMPLETE"));
    }
    Ok(())
}
pub(super) async fn backfill(c: &mut PgConnection) -> Result<(), UpgradeError> {
    crate::usage::channel_projection::backfill_channel_usage(c)
        .await
        .map_err(|_| UpgradeError::Storage)?;
    check_ready(c).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen43_is_exact_and_current_successor_is_explicit44() {
        let frozen = include_str!("projection_predecessor_manifest.json");
        assert_eq!(hex(&Sha256::digest(frozen.as_bytes())), PRE_MANIFEST);
        let mut compiled: super::super::historical_fixture::Manifest =
            serde_json::from_str(include_str!("projection_predecessor_manifest.json")).unwrap();
        assert_eq!(ENGINE.target_count, 44);
        let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
        assert_ne!(ENGINE.target_manifest().unwrap(), target["manifest_sha256"]);
        compiled.components.truncate(43);
        let mut bytes = serde_json::to_vec(&compiled).unwrap();
        bytes.push(b'\n');
        assert_eq!(bytes, frozen.as_bytes());
        let pre: serde_json::Value =
            serde_json::from_str(include_str!("projection_predecessor_target.json")).unwrap();
        assert_eq!(pre["catalog_sha256"], PRE_CATALOG);
        for target in [
            include_str!("completion_target.json"),
            include_str!("account_appeals_target.json"),
            include_str!("channel_accounting_target.json"),
        ] {
            assert_eq!(TARGET, target);
        }
        assert!(matches!(
            authorize_target(),
            Err(UpgradeError::Rejected(
                "CHANNEL_PROJECTION_COMPILED_PREDECESSOR_CHANGED"
            ))
        ));
    }
    #[test]
    fn accounting43_request_cannot_be_reinterpreted_as_projection44() {
        let mut request:UpgradeRequest=serde_json::from_value(serde_json::json!({
            "version":1,"procedure":PROCEDURE,"upgrade_id":"synthetic-projection",
            "database":{"name":"synthetic_projection","oid":1,"system_identifier":"1","server_version_num":180006},
            "roles":{"installer":"projection_installer","owner":"projection_owner","runtime":"projection_runtime","extensions":"projection_extensions"},
            "predecessor_manifest_sha256":PRE_MANIFEST,"predecessor_catalog_sha256":PRE_CATALOG,
            "successor_manifest_sha256":ENGINE.target_manifest().unwrap()
        })).unwrap();
        validate(&request).unwrap();
        request.successor_manifest_sha256 = PRE_MANIFEST.into();
        assert!(validate(&request).is_err());
        request.successor_manifest_sha256 = ENGINE.target_manifest().unwrap();
        request.procedure = "core-channel-accounting-v1".into();
        assert!(validate(&request).is_err());
        request.procedure = PROCEDURE.into();
        request.roles.runtime = request.roles.owner.clone();
        assert!(validate(&request).is_err());
    }
}

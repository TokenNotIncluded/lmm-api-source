//! Current target binding never changes the separately frozen upgrade targets.
use super::*;
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    manifest_sha256: String,
    catalog_sha256: Option<String>,
}
fn target() -> Result<Target, IdentityError> {
    let target: Target =
        serde_json::from_str(include_str!("upgrade/channel_observations_target.json"))
            .map_err(|_| IdentityError::Storage)?;
    let mut bytes = serde_json::to_vec(&schema_manifest()).map_err(|_| IdentityError::Storage)?;
    bytes.push(b'\n');
    if COMPONENTS.len() != 46 || target.manifest_sha256 != format!("{:x}", Sha256::digest(bytes)) {
        return Err(IdentityError::Storage);
    }
    Ok(target)
}
/// A pending capture permits only owner-driven fresh construction, never startup
/// or an upgrade. Once captured, even fresh construction must match the pin.
pub(super) fn check_install_target(catalog: &[u8]) -> Result<(), IdentityError> {
    if target()?.catalog_sha256.is_some() {
        check_target(catalog)?;
    }
    Ok(())
}
pub(super) fn check_target(catalog: &[u8]) -> Result<(), IdentityError> {
    let actual = catalog
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if target()?.catalog_sha256.as_deref() != Some(actual.as_str()) {
        return Err(IdentityError::Storage);
    }
    Ok(())
}

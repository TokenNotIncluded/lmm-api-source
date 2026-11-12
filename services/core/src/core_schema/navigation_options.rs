//! Current target binding never changes the separately frozen upgrade targets.
//! Each offline procedure keeps its own historical successor. Pending current
//! pins grant no startup or upgrade authority.
use super::*;
use serde::Deserialize;
// Root's registry must use this combined component, leaving producer SQL intact.
pub(super) const SUBSCRIPTION_STRIPE_WITH_UPGRADE_RECEIPT: &str = concat!(
    include_str!("../../schema/subscription_stripe.sql"),
    "\n",
    include_str!("upgrade/core59_to62_receipt.sql")
);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    component_count: usize,
    manifest_sha256: Option<String>,
    catalog_sha256: Option<String>,
}
fn target() -> Result<Target, IdentityError> {
    let target: Target = serde_json::from_str(include_str!("upgrade/fresh_target.json"))
        .map_err(|_| IdentityError::Storage)?;
    let mut bytes = serde_json::to_vec(&schema_manifest()).map_err(|_| IdentityError::Storage)?;
    bytes.push(b'\n');
    target.validate(COMPONENTS.len(), &format!("{:x}", Sha256::digest(bytes)))?;
    Ok(target)
}
impl Target {
    fn validate(&self, count: usize, manifest_sha256: &str) -> Result<(), IdentityError> {
        if self.component_count != count
            || self.manifest_sha256.as_deref() != Some(manifest_sha256)
            || self.catalog_sha256.as_ref().is_some_and(|value| {
                value.len() != 64
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err(IdentityError::Storage);
        }
        Ok(())
    }
}
/// A reviewed manifest with a pending catalog permits owner-driven construction
/// only. A pending manifest rejects even construction; startup always requires
/// both exact pins. Capture proposals are reviewed before being compiled in.
pub(super) fn check_install_target(catalog: &[u8]) -> Result<(), IdentityError> {
    if target()?.catalog_sha256.is_some() {
        check_target(catalog)?;
    }
    Ok(())
}
pub(super) fn check_target(catalog: &[u8]) -> Result<(), IdentityError> {
    let actual = crate::hex_codec::encode(catalog);
    if target()?.catalog_sha256.as_deref() != Some(actual.as_str()) {
        return Err(IdentityError::Storage);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reviewed() -> Target {
        Target {
            component_count: 62,
            manifest_sha256: Some("ab".repeat(32)),
            catalog_sha256: None,
        }
    }
    #[test]
    fn pending_manifest_grants_no_install_authority() {
        let mut value = reviewed();
        value.manifest_sha256 = None;
        assert!(value.validate(62, &"ab".repeat(32)).is_err());
    }
    #[test]
    fn reviewed_manifest_binds_full_count_and_exact_bytes() {
        let value = reviewed();
        assert!(value.validate(62, &"ab".repeat(32)).is_ok());
        assert!(value.validate(59, &"ab".repeat(32)).is_err());
        assert!(value.validate(62, &"cd".repeat(32)).is_err());
    }
    #[test]
    fn malformed_catalog_cannot_become_a_capture_pin() {
        for pin in ["", "abc", &"AB".repeat(32), &"gg".repeat(32)] {
            let mut value = reviewed();
            value.catalog_sha256 = Some(pin.to_owned());
            assert!(value.validate(62, &"ab".repeat(32)).is_err());
        }
        let mut value = reviewed();
        value.catalog_sha256 = Some("cd".repeat(32));
        assert!(value.validate(62, &"ab".repeat(32)).is_ok());
    }
}

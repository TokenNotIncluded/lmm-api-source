//! Historical price evidence used only to decode and settle already admitted
//! requests. New requests use their configured group and channel prices.
mod types;
pub use types::{DiscountRatio, FrozenTrustPrice};
pub const SCHEMA: &str = concat!(
    include_str!("../../schema/trust_pricing.sql"),
    "\n",
    include_str!("../../schema/image_trust_extension.sql")
);
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "trust pricing {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;

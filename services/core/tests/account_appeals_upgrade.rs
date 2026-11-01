//! Exact 41 -> 43 offline upgrade through the public API.
const ERROR_PREFIX: &str = "ACCOUNT_APPEALS";
const PROCEDURE: &str = "core-account-appeals-v1";
const PRE_MANIFEST: &str = "12c971ce84ca71eb5605a7fe284535216a5777858d5aaf766c3876655bf8aee6";
const PRE_CATALOG: &str = "05d1aa6913236fc12cc099640ead1ac2d88a1bde073923ce8353907cfc46c8bb";
#[path = "common/appeals_predecessor.rs"]
mod predecessor;

const RECEIPT: &str = "core_meta.account_appeal_upgrades";
#[path = "common/channel_accounting_history.rs"]
mod history;
use history::{check_history, seed_history};
#[path = "common/suffix_upgrade.rs"]
mod harness;

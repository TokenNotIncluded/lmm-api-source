//! Stable model-request core. No extension runtime or Go dependency belongs here.
//!
//! PostgreSQL identity is usable in isolation. Public paid model endpoints stay
//! disabled until durable billing and routing adapters are integrated.
pub mod abuse;
pub mod accounts;
pub mod assistant_model;
pub mod assistant_tools_core;
pub mod billing;
pub mod bounties;
pub mod builtins;
pub mod channel_http;
#[cfg(unix)]
pub mod commerce;
pub mod config;
pub mod console_compat;
pub mod core_schema;
pub mod edge_access;
pub mod events;
pub mod finance_export;
pub mod financial_profile;
pub mod funding;
pub mod gift;
pub mod history_http;
pub mod history_read;
pub mod http;
pub mod identity;
pub mod identity_http;
#[cfg(unix)]
pub mod internal_rpc;
pub mod legacy_auth;
pub mod lifecycle;
pub mod log_redact;
pub mod market_oauth;
pub mod model_http;
pub mod native_key_read;
pub mod oauth2_balance;
pub mod oauth2_http;
pub mod operations;
pub mod options_http;
#[cfg(unix)]
pub mod payments;
pub mod pricing_http;
pub mod promotions;
pub mod public_server;
pub mod relay;
pub mod remote_control;
pub mod request_limits;
pub mod routing;
pub mod runtime_database;
pub mod subscriptions;
pub mod support_projection;
pub mod system_info;
#[cfg(test)]
#[path = "../tests/common/runtime.rs"]
pub(crate) mod test_runtime;
pub mod tool_market;
pub mod topup_info_http;
pub mod upgrade_history;
#[cfg(unix)]
pub mod upgrade_offline;
pub mod upstream_cost;
pub mod usage;
pub mod wallet;

pub mod humangrant;

#[cfg(test)]
mod independent_966_log_tests;

#[path = "../staged/email_delivery/mod.rs"]
pub mod email_delivery;
pub mod email_http;

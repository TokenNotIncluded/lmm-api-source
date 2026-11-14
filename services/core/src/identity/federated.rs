//! Native external login clients, separate from the site's OAuth/OIDC server.
#[cfg(all(test, target_os = "linux"))]
mod independent_auth_http;

mod bridge_network;
mod crypto;
mod flows;
mod management;
mod network;
mod policy;
mod protocol;
mod provider;
mod service;

pub(crate) use crypto::FederatedKeys;
pub(crate) use flows::{
    ExchangeResult, FederatedStart, FlowConsumeInput, FlowOwner, FlowPurpose, FlowStartInput,
};
pub(crate) use management::ProviderWrite;
pub(crate) use provider::{FederatedProvider, ProviderConfiguration, ProviderKind};
pub(crate) use service::FederatedAuth;
pub(crate) use service::public_catalog;

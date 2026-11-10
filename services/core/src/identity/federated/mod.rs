//! Native external login clients, separate from the site's OAuth/OIDC server.
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
pub(crate) use flows::{ExchangeResult, FederatedStart, FlowOwner, FlowPurpose};
pub(crate) use management::ProviderWrite;
pub(crate) use provider::{FederatedProvider, ProviderConfiguration, ProviderKind};
pub(crate) use service::FederatedAuth;
pub(crate) use service::public_catalog;

pub(crate) const SCHEMA: &str = include_str!("../../../schema/federated_auth.sql");

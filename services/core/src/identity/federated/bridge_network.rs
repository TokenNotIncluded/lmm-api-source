//! The old WeChat helper accepts public HTTP as well as HTTPS. Preserve only
//! that explicit bridge transport; other provider endpoints retain HTTPS.
use crate::{
    identity::{IdentityError, Result},
    relay::{Limits, honeypot_network},
};
use reqwest::{
    Client, Url,
    dns::{Name, Resolve, Resolving},
};
use std::{
    collections::BTreeSet,
    io,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};
use tokio::sync::{Semaphore, oneshot};

pub(super) struct BridgeConnection {
    pub client: Client,
    peers: BTreeSet<SocketAddr>,
}
impl BridgeConnection {
    pub fn matches_peer(&self, peer: Option<SocketAddr>) -> bool {
        peer.is_some_and(|peer| {
            self.peers
                .contains(&SocketAddr::new(peer.ip().to_canonical(), peer.port()))
        })
    }
}
struct RejectDns;
impl Resolve for RejectDns {
    fn resolve(&self, _: Name) -> Resolving {
        Box::pin(async {
            Err(Box::new(io::Error::other("unconfigured bridge DNS lookup"))
                as Box<dyn std::error::Error + Send + Sync>)
        })
    }
}
fn checked(addresses: impl IntoIterator<Item = SocketAddr>) -> Result<BTreeSet<SocketAddr>> {
    let mut peers = BTreeSet::new();
    for (index, address) in addresses.into_iter().enumerate() {
        if index >= 16 || !honeypot_network::public_address(address.ip()) {
            return Err(IdentityError::Unauthorized);
        }
        peers.insert(SocketAddr::new(address.ip().to_canonical(), address.port()));
    }
    if peers.is_empty() {
        return Err(IdentityError::Unauthorized);
    }
    Ok(peers)
}
pub(super) async fn prepare(
    url: &Url,
    limits: &Limits,
    slots: Arc<Semaphore>,
) -> Result<BridgeConnection> {
    if url.scheme() != "http" {
        return Err(IdentityError::Invalid);
    }
    let host = url.host_str().ok_or(IdentityError::Invalid)?;
    let port = url.port_or_known_default().ok_or(IdentityError::Invalid)?;
    let peers = if let Some(ip) = honeypot_network::literal_ip(url) {
        checked([SocketAddr::new(ip, port)])?
    } else {
        let permit = slots
            .try_acquire_owned()
            .map_err(|_| IdentityError::Unauthorized)?;
        let host = host.to_owned();
        let (sender, receiver) = oneshot::channel();
        // The thread retains its capacity even if its request is cancelled.
        std::thread::Builder::new()
            .name("lmm-auth-bridge-dns".into())
            .spawn(move || {
                let _permit = permit;
                let result = (host.as_str(), port)
                    .to_socket_addrs()
                    .map_err(|_| IdentityError::Unauthorized)
                    .and_then(checked);
                let _ = sender.send(result);
            })
            .map_err(|_| IdentityError::Storage)?;
        receiver.await.map_err(|_| IdentityError::Unauthorized)??
    };
    let addresses: Vec<_> = peers.iter().copied().collect();
    let client = honeypot_network::client_builder(limits)
        .dns_resolver(Arc::new(RejectDns))
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| IdentityError::Storage)?;
    Ok(BridgeConnection { client, peers })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_rejects_mixed_and_private_dns_answers() {
        let public = SocketAddr::from(([8, 8, 8, 8], 80));
        assert!(checked([public]).is_ok());
        assert!(checked([public, SocketAddr::from(([127, 0, 0, 1], 80))]).is_err());
        assert!(checked([SocketAddr::from(([169, 254, 169, 254], 80))]).is_err());
        assert!(checked([]).is_err());
    }
}

//! Connection pooling and admission lifecycle for native model APIs.

use crate::relay::provider_network::ProviderTransport;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(super) struct Host {
    pub(super) gate: Arc<Semaphore>,
    pub(super) transport: ProviderTransport,
    parallelism: usize,
}

impl Host {
    pub(super) fn new(parallelism: usize) -> Self {
        Self {
            gate: Arc::new(Semaphore::new(parallelism)),
            transport: ProviderTransport::new(parallelism),
            parallelism,
        }
    }

    pub(super) async fn drain(&self, grace: Duration) -> usize {
        self.gate.close();
        let deadline = tokio::time::Instant::now() + grace;
        while self.gate.available_permits() < self.parallelism
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.parallelism
            .saturating_sub(self.gate.available_permits())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn drain_closes_admission_and_counts_outstanding_calls() {
        for parallelism in [0, 1, 2, 4] {
            let host = Host::new(parallelism);
            let calls: Vec<_> = (0..parallelism)
                .map(|_| host.gate.clone().try_acquire_owned().unwrap())
                .collect();
            assert!(host.gate.clone().try_acquire_owned().is_err());
            assert_eq!(host.drain(Duration::ZERO).await, parallelism);
            drop(calls);
            assert_eq!(host.drain(Duration::ZERO).await, 0);
            assert!(host.gate.clone().try_acquire_owned().is_err());
        }
    }

    #[tokio::test]
    async fn drain_waits_for_an_existing_call_to_finish() {
        let host = Host::new(2);
        let call = host.gate.clone().try_acquire_owned().unwrap();
        let (outstanding, ()) =
            tokio::join!(host.drain(Duration::from_secs(1)), async { drop(call) });
        assert_eq!(outstanding, 0);
        assert!(host.gate.clone().try_acquire_owned().is_err());
    }
}

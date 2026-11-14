//! Audio SSE only: no text-usage normalization, estimates or retry after dispatch.
use super::*;
use crate::{
    billing::audio::journal::{MAX_PACKETS, MAX_STREAM_BYTES, TerminalState, frame_end},
    relay::provider_network::ProviderConnection,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

type Result<T> = std::result::Result<T, RelayError>;

pub(super) struct StreamStart {
    pub(super) gateway: ModelGateway,
    pub(super) id: String,
    pub(super) worker: [u8; 32],
    pub(super) price: charging::Price,
    pub(super) model: String,
    pub(super) secret: String,
    pub(super) connection: ProviderConnection,
    pub(super) upstream: reqwest::Response,
    pub(super) permit: OwnedSemaphorePermit,
}
pub(super) fn launch(start: StreamStart) -> Result<Response> {
    let StreamStart {
        gateway,
        id,
        worker,
        price,
        model,
        secret,
        connection,
        upstream,
        permit,
    } = start;
    if !connection.matches_peer(upstream.remote_addr())
        || upstream.headers().contains_key(header::CONTENT_ENCODING)
        || !upstream
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| {
                h.split(';')
                    .next()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/event-stream"))
            })
    {
        return Err(RelayError::SettlementPending);
    }
    let (tx, rx) = mpsc::channel::<std::result::Result<Bytes, std::io::Error>>(
        gateway.limits.queue_chunks.max(1),
    );
    tokio::spawn(async move {
        let _permit = permit;
        let _connection = connection;
        let result = tokio::time::timeout(
            gateway.limits.total_timeout,
            pump(
                StreamContext {
                    gateway: &gateway,
                    id: &id,
                    worker: &worker,
                    price: &price,
                    model: &model,
                    secret: &secret,
                },
                upstream,
                &tx,
            ),
        )
        .await;
        if !matches!(result, Ok(Ok(()))) {
            let _ = billing::retry(|| gateway.bridge.charging.mark_unknown(&id, &worker)).await;
            let _ = tokio::time::timeout(
                gateway.limits.write_timeout,
                tx.send(Err(std::io::Error::other("audio settlement pending"))),
            )
            .await;
        }
    });
    let mut response = Response::new(Body::from_stream(ReceiverStream::new(rx)));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn send(
    tx: &mpsc::Sender<std::result::Result<Bytes, std::io::Error>>,
    bytes: Vec<u8>,
    limits: &Limits,
) -> Result<()> {
    tokio::time::timeout(limits.write_timeout, tx.send(Ok(Bytes::from(bytes))))
        .await
        .map_err(|_| RelayError::SettlementPending)?
        .map_err(|_| RelayError::SettlementPending)
}
struct StreamContext<'a> {
    gateway: &'a ModelGateway,
    id: &'a str,
    worker: &'a [u8; 32],
    price: &'a charging::Price,
    model: &'a str,
    secret: &'a str,
}
async fn pump(
    context: StreamContext<'_>,
    mut upstream: reqwest::Response,
    tx: &mpsc::Sender<std::result::Result<Bytes, std::io::Error>>,
) -> Result<()> {
    let StreamContext {
        gateway,
        id,
        worker,
        price,
        model,
        secret,
    } = context;
    let format = price
        .audio
        .as_ref()
        .ok_or(RelayError::SettlementPending)?
        .usage_format;
    let limits = &gateway.limits;
    let cap = limits
        .upstream_bytes
        .min(limits.output_bytes)
        .min(MAX_STREAM_BYTES);
    let mut buffer = Vec::new();
    let mut total = 0usize;
    let mut sequence = 0u64;
    let mut digest = Sha256::new();
    let mut secret_tail = Vec::new();
    let mut state = TerminalState::default();
    let mut done_packet = None;
    loop {
        let chunk = tokio::time::timeout(limits.read_timeout, upstream.chunk())
            .await
            .map_err(|_| RelayError::SettlementPending)?
            .map_err(|_| RelayError::SettlementPending)?;
        let Some(chunk) = chunk else {
            break;
        };
        total = total
            .checked_add(chunk.len())
            .ok_or(RelayError::SettlementPending)?;
        if total > cap {
            return Err(RelayError::SettlementPending);
        }
        secret_tail.extend_from_slice(&chunk);
        if !secret.is_empty()
            && secret_tail
                .windows(secret.len())
                .any(|w| w == secret.as_bytes())
        {
            return Err(RelayError::SettlementPending);
        }
        let keep = secret.len().saturating_sub(1).min(secret_tail.len());
        secret_tail.drain(..secret_tail.len() - keep);
        buffer.extend_from_slice(&chunk);
        while let Some(end) = frame_end(&buffer) {
            if end > limits.event_bytes {
                return Err(RelayError::SettlementPending);
            }
            let packet: Vec<u8> = buffer.drain(..end).collect();
            sequence += 1;
            if sequence > MAX_PACKETS {
                return Err(RelayError::SettlementPending);
            }
            let terminal = state
                .observe(&packet, model, format)
                .map_err(|_| RelayError::SettlementPending)?;
            digest.update(&packet);
            let stream_sha: [u8; 32] = digest.clone().finalize().into();
            if terminal {
                // DONE is not HTTP EOF. Never persist final authority until the
                // bounded transport confirms EOF with no later bytes or errors.
                done_packet = Some(packet);
            } else {
                billing::retry(|| {
                    gateway
                        .bridge
                        .charging
                        .checkpoint_audio_packet(id, worker, sequence, &packet, stream_sha, false)
                })
                .await
                .map_err(billing::error)?;
                if !state.has_usage() {
                    send(tx, packet, limits).await?;
                }
            }
        }
        if buffer.len() > limits.event_bytes {
            return Err(RelayError::SettlementPending);
        }
    }
    if !buffer.is_empty() || !state.is_done() {
        return Err(RelayError::SettlementPending);
    }
    let packet = done_packet.ok_or(RelayError::SettlementPending)?;
    let stream_sha: [u8; 32] = digest.finalize().into();
    // DONE+EOF evidence and the raw usage receipt commit in one ledger transaction.
    let checkpoint = billing::retry(|| {
        gateway
            .bridge
            .charging
            .checkpoint_audio_stream(id, worker, sequence, &packet, stream_sha)
    })
    .await
    .map_err(billing::error)?;
    let settled = billing::retry(|| {
        gateway
            .bridge
            .charging
            .settle(id, worker, checkpoint.usage, Outcome::Completed)
    })
    .await;
    let settled = match settled {
        Ok(c) => Some(c),
        Err(charging::Error::CommitUnknown) => gateway
            .bridge
            .charging
            .resolve(id)
            .await
            .map_err(billing::error)?,
        _ => return Err(RelayError::SettlementPending),
    };
    if !settled.as_ref().is_some_and(|c| {
        billing::confirmed_settlement(c, id, price, checkpoint.usage, Outcome::Completed)
    }) {
        return Err(RelayError::SettlementPending);
    }
    send(
        tx,
        state
            .into_usage_packet()
            .ok_or(RelayError::SettlementPending)?,
        limits,
    )
    .await?;
    send(tx, packet, limits).await
}

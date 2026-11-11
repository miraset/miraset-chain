//! Open-loop generators and sender funding.
//!
//! Each sender (and each receipt stream) is pinned to one node, because nodes do not gossip
//! their mempools: an item is proposed only when the node that admitted it leads a view.
//! A generator sends at its own fixed rate; it has at most one request in flight, so a slow
//! node shows up as generator lag rather than as unbounded client-side queueing.

use super::http::{HttpError, RpcClient};
use super::tracker::{Kind, Tracker, lock};
use super::workload::{self, TRANSFER_AMOUNT, TRANSFER_FEE};
use crate::core::types::{AccountState, ComputeReceipt, Transaction};
use crate::rpc::{ApiResponse, TaskSettlementResponse};
use ed25519_dalek::SigningKey;
use hyper::StatusCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::time::Instant;

const FUNDING_POLL: Duration = Duration::from_millis(200);

/// What a generator sends and where.
pub struct Generator {
    pub node_url: String,
    pub interval: Duration,
    pub end: Instant,
}

/// Largest delay between a scheduled send and the generator getting to it.
#[derive(Default)]
pub struct Lag(Mutex<Duration>);

impl Lag {
    fn record(&self, lag: Duration) {
        let mut max = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *max = (*max).max(lag);
    }

    pub fn max(&self) -> Duration {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// An item whose send failed in transport: it may or may not have been admitted (or even
/// committed). Before it is re-sent, committed state is checked; a re-send that the node
/// answers as a conflict (409: already pending or already committed) was admitted by the
/// earlier attempt.
struct Retry<T> {
    item: T,
    scheduled: Instant,
}

enum Outcome {
    Admitted,
    Rejected(String),
    Unknown,
}

fn classify(result: Result<StatusCode, HttpError>, is_retry: bool) -> Outcome {
    match result {
        Ok(status) if status.is_success() => Outcome::Admitted,
        // 409 means already pending or already committed: our earlier attempt got in.
        Ok(StatusCode::CONFLICT) if is_retry => Outcome::Admitted,
        Ok(status) => Outcome::Rejected(format!("status {}", status.as_u16())),
        Err(_) => Outcome::Unknown,
    }
}

/// Wait for the next scheduled send; returns its scheduled time, or `None` once the run is over.
async fn next_tick(next: &mut Instant, generator: &Generator, lag: &Lag) -> Option<Instant> {
    if *next >= generator.end {
        return None;
    }
    let scheduled = *next;
    tokio::time::sleep_until(scheduled).await;
    lag.record(Instant::now().saturating_duration_since(scheduled));
    *next += generator.interval;
    Some(scheduled)
}

pub struct TransferStream {
    pub index: usize,
    pub key: SigningKey,
    pub recipient: String,
}

/// Committed nonce of `address`, if readable.
async fn committed_nonce(client: &RpcClient, node_url: &str, address: &str) -> Option<u64> {
    let url = format!("{node_url}/v1/accounts/{address}");
    let response: ApiResponse<AccountState> = client.get_json(&url).await.ok()?;
    Some(response.data?.nonce)
}

pub async fn run_transfers(
    client: RpcClient,
    generator: Generator,
    stream: TransferStream,
    tracker: Arc<Mutex<Tracker>>,
    lag: Arc<Lag>,
) {
    let url = format!("{}/v1/transactions", generator.node_url);
    let sender = workload::address(&stream.key);
    let mut nonce = 0;
    let mut retry: Option<Retry<Transaction>> = None;
    let mut next = Instant::now();
    while let Some(tick) = next_tick(&mut next, &generator, &lag).await {
        let (item, scheduled, is_retry) = match retry.take() {
            Some(Retry { item, scheduled }) => {
                let committed = committed_nonce(&client, &generator.node_url, &sender).await;
                if committed.is_some_and(|c| c > item.nonce) {
                    // Got in and already committed: re-sending would re-insert it.
                    lock(&tracker).record_admitted_tx(stream.index, nonce, scheduled);
                    nonce += 1;
                    continue;
                }
                (item, scheduled, true)
            }
            None => {
                let item = workload::signed_transfer(
                    &stream.key,
                    nonce,
                    &stream.recipient,
                    TRANSFER_AMOUNT,
                    TRANSFER_FEE,
                );
                (item, tick, false)
            }
        };
        match classify(client.post_json(&url, &item).await, is_retry) {
            Outcome::Admitted => {
                lock(&tracker).record_admitted_tx(stream.index, nonce, scheduled);
                nonce += 1;
            }
            // Not admitted: the nonce is free, the next transfer reuses it.
            Outcome::Rejected(reason) => {
                lock(&tracker).record_rejected(Kind::Transaction, scheduled, reason)
            }
            Outcome::Unknown => retry = Some(Retry { item, scheduled }),
        }
    }
    if let Some(Retry { scheduled, .. }) = retry {
        lock(&tracker).record_rejected(Kind::Transaction, scheduled, "transport");
    }
}

pub struct ReceiptStream {
    pub index: usize,
    pub run_id: u64,
    pub coordinator: SigningKey,
    /// Funded client accounts, used in turn.
    pub clients: Vec<String>,
    pub worker_id: String,
}

/// Whether task `task_id` is settled, if readable.
pub async fn task_settled(client: &RpcClient, node_url: &str, task_id: &str) -> Option<bool> {
    let url = format!("{node_url}/v1/tasks/{task_id}");
    let response: ApiResponse<TaskSettlementResponse> = client.get_json(&url).await.ok()?;
    Some(response.data?.settled)
}

pub async fn run_receipts(
    client: RpcClient,
    generator: Generator,
    stream: ReceiptStream,
    tracker: Arc<Mutex<Tracker>>,
    lag: Arc<Lag>,
) {
    let url = format!("{}/v1/receipts", generator.node_url);
    let mut seq: u64 = 0;
    let mut retry: Option<Retry<ComputeReceipt>> = None;
    let mut next = Instant::now();
    while let Some(tick) = next_tick(&mut next, &generator, &lag).await {
        let (item, scheduled, is_retry) = match retry.take() {
            Some(Retry { item, scheduled }) => {
                if task_settled(&client, &generator.node_url, &item.task_id).await == Some(true) {
                    lock(&tracker).record_admitted_receipt(stream.index, item.task_id, scheduled);
                    seq += 1;
                    continue;
                }
                (item, scheduled, true)
            }
            None => {
                let client_address = &stream.clients[seq as usize % stream.clients.len()];
                let task_id = workload::task_id(stream.run_id, stream.index, seq);
                let item = workload::signed_receipt(
                    &stream.coordinator,
                    task_id,
                    client_address,
                    &stream.worker_id,
                );
                (item, tick, false)
            }
        };
        match classify(client.post_json(&url, &item).await, is_retry) {
            Outcome::Admitted => {
                lock(&tracker).record_admitted_receipt(stream.index, item.task_id, scheduled);
                seq += 1;
            }
            // A rejected task ID is never reused: the next receipt gets the next sequence.
            Outcome::Rejected(reason) => {
                lock(&tracker).record_rejected(Kind::Receipt, scheduled, reason);
                seq += 1;
            }
            Outcome::Unknown => retry = Some(Retry { item, scheduled }),
        }
    }
    if let Some(Retry { scheduled, .. }) = retry {
        lock(&tracker).record_rejected(Kind::Receipt, scheduled, "transport");
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FundingError {
    #[error("faucet account not readable at {url}: {source}")]
    Faucet { url: String, source: HttpError },
    #[error("faucet balance {balance} cannot fund {needed}")]
    Insufficient { balance: u64, needed: u64 },
    #[error("funding transfer {nonce} not admitted: {reason}")]
    Rejected { nonce: u64, reason: String },
    #[error("funding transfers not committed within {0:?}")]
    Timeout(Duration),
}

/// Send `amount` from the faucet to each address through `node_url` and wait until all are
/// committed.
pub async fn fund(
    client: &RpcClient,
    node_url: &str,
    faucet: &SigningKey,
    recipients: &[String],
    amount: u64,
    timeout: Duration,
) -> Result<(), FundingError> {
    let faucet_address = workload::address(faucet);
    let start = account(client, node_url, &faucet_address).await?;
    let needed = amount
        .saturating_add(TRANSFER_FEE)
        .saturating_mul(recipients.len() as u64);
    if start.balance < needed {
        return Err(FundingError::Insufficient {
            balance: start.balance,
            needed,
        });
    }
    let submit = format!("{node_url}/v1/transactions");
    for (offset, recipient) in recipients.iter().enumerate() {
        let nonce = start.nonce + offset as u64;
        let tx = workload::signed_transfer(faucet, nonce, recipient, amount, TRANSFER_FEE);
        match client.post_json(&submit, &tx).await {
            Ok(status) if status.is_success() => {}
            Ok(status) => {
                return Err(FundingError::Rejected {
                    nonce,
                    reason: format!("status {status}"),
                });
            }
            Err(e) => {
                return Err(FundingError::Rejected {
                    nonce,
                    reason: e.to_string(),
                });
            }
        }
    }
    let target = start.nonce + recipients.len() as u64;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if account(client, node_url, &faucet_address).await?.nonce >= target {
            return Ok(());
        }
        tokio::time::sleep(FUNDING_POLL).await;
    }
    Err(FundingError::Timeout(timeout))
}

async fn account(
    client: &RpcClient,
    node_url: &str,
    address: &str,
) -> Result<AccountState, FundingError> {
    let url = format!("{node_url}/v1/accounts/{address}");
    let response: ApiResponse<AccountState> =
        client
            .get_json(&url)
            .await
            .map_err(|source| FundingError::Faucet {
                url: url.clone(),
                source,
            })?;
    response.data.ok_or_else(|| FundingError::Faucet {
        url,
        source: HttpError::Body("account response without data".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duplicate_answer_counts_as_admission_only_on_a_retry() {
        assert!(matches!(
            classify(Ok(StatusCode::CONFLICT), true),
            Outcome::Admitted
        ));
        assert!(matches!(
            classify(Ok(StatusCode::CONFLICT), false),
            Outcome::Rejected(_)
        ));
        assert!(matches!(
            classify(Ok(StatusCode::BAD_REQUEST), true),
            Outcome::Rejected(_)
        ));
        assert!(matches!(
            classify(Ok(StatusCode::OK), false),
            Outcome::Admitted
        ));
        assert!(matches!(
            classify(Ok(StatusCode::SERVICE_UNAVAILABLE), false),
            Outcome::Rejected(r) if r == "status 503"
        ));
        assert!(matches!(
            classify(Err(HttpError::Timeout(Duration::ZERO)), false),
            Outcome::Unknown
        ));
    }
}

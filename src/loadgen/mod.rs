//! Devnet load generator for measured throughput and finality evidence (TODO 8.5.3).
//!
//! Offers an open-loop mix of signed transfers and coordinator-signed compute receipts to the
//! node RPCs at a fixed rate, and reports admission outcomes, committed throughput and
//! admission-to-commit latency for items submitted in the measurement window.
//!
//! This measures a devnet; it is not a production target or a production-readiness claim
//! (PRD: no throughput/latency target until workload, hardware and procedure are approved).

pub mod driver;
pub mod http;
pub mod observer;
pub mod stats;
pub mod tracker;
pub mod workload;

use crate::core::types::MIN_WORKER_BOND;
use driver::{Generator, Lag, ReceiptStream, TransferStream};
use ed25519_dalek::SigningKey;
use http::RpcClient;
use observer::{BlockReport, Observer};
use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;
use tracker::{Kind, KindReport, Tracker, Window, lock};

const DRAIN_POLL: Duration = Duration::from_millis(250);
const FUNDING_TIMEOUT: Duration = Duration::from_secs(120);
/// Time for the commit stream to connect before load starts.
const SUBSCRIBE_SETTLE: Duration = Duration::from_millis(500);
const RECONNECT_BACKOFF: Duration = Duration::from_millis(500);
const PROGRESS_INTERVAL: Duration = Duration::from_secs(60);

pub struct LoadConfig {
    /// Node RPC base URLs (`http://host:port`); the first one funds senders and observes commits.
    pub rpc_urls: Vec<String>,
    pub faucet: SigningKey,
    pub senders: usize,
    /// Offered items (transfers + receipts) per second, all nodes together.
    pub rate: f64,
    /// Fraction of offered items that are receipts.
    pub receipt_share: f64,
    pub duration: Duration,
    /// Initial part of `duration` excluded from the report.
    pub warmup: Duration,
    /// After load stops, how long to wait for admitted items to commit.
    pub drain_timeout: Duration,
    pub fund_amount: u64,
    pub run_id: u64,
    /// RPC API token sent as `Authorization: Bearer` on submissions, for nodes started with
    /// `--rpc-api-keys`.
    pub api_key: Option<String>,
    /// A genesis coordinator key; required when receipts are offered (TODO 8.5.1).
    pub coordinator: Option<SigningKey>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("invalid load configuration: {0}")]
    Config(&'static str),
    #[error(transparent)]
    Funding(#[from] driver::FundingError),
    #[error("commit observer failed: {0}")]
    Observer(String),
}

impl LoadConfig {
    pub fn validate(&self) -> Result<(), LoadError> {
        let invalid = |m| Err(LoadError::Config(m));
        if self.rpc_urls.is_empty() {
            return invalid("at least one RPC URL is required");
        }
        if self.rpc_urls.iter().any(|u| !u.starts_with("http://")) {
            return invalid("RPC URLs must be plain http:// URLs");
        }
        if self.senders == 0 {
            return invalid("senders must be at least 1");
        }
        if !(self.rate.is_finite() && self.rate > 0.0) {
            return invalid("rate must be a positive number");
        }
        if !(0.0..=1.0).contains(&self.receipt_share) {
            return invalid("receipt share must be within 0..=1");
        }
        if self.warmup >= self.duration {
            return invalid("warm-up must be shorter than the duration");
        }
        if self.fund_amount == 0 {
            return invalid("fund amount must be positive");
        }
        if self.receipt_share > 0.0 && self.coordinator.is_none() {
            return invalid("receipts need a genesis coordinator key");
        }
        Ok(())
    }

    fn transfer_interval(&self) -> Option<Duration> {
        let per_sender = self.rate * (1.0 - self.receipt_share) / self.senders as f64;
        (per_sender > 0.0).then(|| Duration::from_secs_f64(1.0 / per_sender))
    }

    fn receipt_interval(&self) -> Option<Duration> {
        let per_stream = self.rate * self.receipt_share / self.rpc_urls.len() as f64;
        (per_stream > 0.0).then(|| Duration::from_secs_f64(1.0 / per_stream))
    }
}

#[derive(Debug, Serialize)]
pub struct LoadReport {
    pub run_id: String,
    pub nodes: usize,
    pub senders: usize,
    pub offered_rate: f64,
    pub receipt_share: f64,
    pub duration_secs: f64,
    pub warmup_secs: f64,
    pub window_secs: f64,
    pub transactions: KindReport,
    pub receipts: KindReport,
    /// Commits of both kinds observed in the window, per second.
    pub committed_per_sec: f64,
    pub blocks: BlockReport,
    /// Longest time between a scheduled send and the generator getting to it (latency is
    /// measured from the scheduled time, so this delay is inside the latency figures).
    pub max_generator_lag_ms: f64,
    /// Longest and mean observer read round (bound the latency stamping error).
    pub max_observer_round_ms: f64,
    pub mean_observer_round_ms: Option<f64>,
    pub observer_read_errors: usize,
    /// Times the commit stream to the observer node dropped and was re-established.
    pub observer_reconnects: usize,
    /// Admitted items still uncommitted when the drain timeout ran out (all windows).
    pub pending_at_end: usize,
    pub oldest_pending_secs: Option<f64>,
}

fn is_loopback_url(url: &str) -> bool {
    let authority = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("");
    let host = match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Fund senders, offer the load, drain, and report.
pub async fn run(config: &LoadConfig) -> Result<LoadReport, LoadError> {
    config.validate()?;
    if config.api_key.is_some() {
        for url in config.rpc_urls.iter().filter(|url| !is_loopback_url(url)) {
            tracing::warn!(%url, "sending the RPC API token over plain HTTP to a non-loopback node");
        }
    }
    let client = RpcClient::new().with_bearer(config.api_key.clone());
    let observer_url = config.rpc_urls[0].clone();
    let sender_keys: Vec<SigningKey> = (0..config.senders)
        .map(|i| workload::derive_key(config.run_id, "sender", i))
        .collect();
    let senders: Vec<String> = sender_keys.iter().map(workload::address).collect();
    tracing::info!(run_id = %format!("{:016x}", config.run_id), senders = senders.len(), "funding senders");
    driver::fund(
        &client,
        &observer_url,
        &config.faucet,
        &senders,
        config.fund_amount,
        FUNDING_TIMEOUT,
    )
    .await?;
    let workers = prepare_receipt_workers(config, &client, &observer_url).await?;

    // Load starts after the commit stream has had time to connect.
    let start = Instant::now() + SUBSCRIBE_SETTLE;
    let end = start + config.duration;
    let window = Window {
        start: start + config.warmup,
        end,
    };
    let receipt_streams = if config.receipt_interval().is_some() {
        config.rpc_urls.len()
    } else {
        0
    };
    let tracker = Arc::new(Mutex::new(Tracker::new(
        window,
        senders.len(),
        receipt_streams,
    )));
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (stop_tx, stop_rx) = watch::channel(false);
    let reconnects = Arc::new(AtomicUsize::new(0));
    let subscription = tokio::spawn(subscribe(
        client.clone(),
        format!("{observer_url}/v1/events/blocks"),
        event_tx,
        Arc::clone(&reconnects),
    ));
    let observer = Observer {
        client: client.clone(),
        node_url: observer_url,
        senders: senders.clone(),
        tracker: Arc::clone(&tracker),
        window,
    };
    let observing = tokio::spawn(observer.run(event_rx, stop_rx));
    let progress = tokio::spawn(log_progress(Arc::clone(&tracker)));
    tokio::time::sleep_until(start).await;

    let lag = Arc::new(Lag::default());
    tracing::info!(
        rate = config.rate,
        duration_secs = config.duration.as_secs_f64(),
        "offering load"
    );
    let mut generators = spawn_generators(
        config,
        &client,
        sender_keys,
        &senders,
        workers,
        end,
        &tracker,
        &lag,
    );
    while generators.join_next().await.is_some() {}

    let drain_deadline = Instant::now() + config.drain_timeout;
    while lock(&tracker).pending() > 0 && Instant::now() < drain_deadline {
        tokio::time::sleep(DRAIN_POLL).await;
    }
    let _ = stop_tx.send(true);
    let observed = observing.await;
    subscription.abort();
    progress.abort();
    let observed = observed.map_err(|e| LoadError::Observer(e.to_string()))?;

    let tracker = lock(&tracker);
    let transactions = tracker.report(Kind::Transaction);
    let receipts = tracker.report(Kind::Receipt);
    let at_end = tracker.progress(Instant::now());
    Ok(LoadReport {
        run_id: format!("{:016x}", config.run_id),
        nodes: config.rpc_urls.len(),
        senders: config.senders,
        offered_rate: config.rate,
        receipt_share: config.receipt_share,
        duration_secs: config.duration.as_secs_f64(),
        warmup_secs: config.warmup.as_secs_f64(),
        window_secs: (config.duration - config.warmup).as_secs_f64(),
        committed_per_sec: round2(transactions.committed_per_sec + receipts.committed_per_sec),
        transactions,
        receipts,
        blocks: observed.blocks.report(),
        max_generator_lag_ms: ms(lag.max()),
        max_observer_round_ms: ms(observed.max_round),
        mean_observer_round_ms: (observed.rounds > 0)
            .then(|| ms(observed.total_round / observed.rounds as u32)),
        observer_read_errors: observed.read_errors,
        observer_reconnects: reconnects.load(Ordering::Relaxed),
        pending_at_end: at_end.pending,
        oldest_pending_secs: at_end.oldest_pending_secs,
    })
}

/// Keep the observer's commit stream connected for the whole run: a dropped stream (node
/// restart, proxy idle timeout, malformed event) is re-established, and the next commit event
/// triggers a full read round, so commits during the gap are still found (stamped late).
async fn subscribe(
    client: RpcClient,
    url: String,
    sink: mpsc::UnboundedSender<(Instant, crate::core::types::CommitEvent)>,
    reconnects: Arc<AtomicUsize>,
) {
    loop {
        let result = client.stream_commits(&url, sink.clone()).await;
        if sink.is_closed() {
            return;
        }
        reconnects.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(?result, "commit stream ended; reconnecting");
        tokio::time::sleep(RECONNECT_BACKOFF).await;
    }
}

/// Periodic progress on stderr, so a long soak leaves evidence even if it never finishes.
async fn log_progress(tracker: Arc<Mutex<Tracker>>) {
    let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let p = lock(&tracker).progress(Instant::now());
        tracing::info!(
            admitted = p.admitted,
            rejected = p.rejected,
            pending = p.pending,
            oldest_pending_secs = ?p.oldest_pending_secs,
            "load progress"
        );
    }
}

/// Owners and the price for receipt streams: one worker per node, funded with the minimum
/// bond and registered before load starts (TODO 8.5.1).
struct ReceiptWorkers {
    owners: Vec<SigningKey>,
    /// One client account per stream (receipts are paid by it, out of its allowance).
    clients: Vec<String>,
    price_per_token: u64,
}

async fn prepare_receipt_workers(
    config: &LoadConfig,
    client: &RpcClient,
    observer_url: &str,
) -> Result<Option<ReceiptWorkers>, LoadError> {
    if config.receipt_interval().is_none() {
        return Ok(None);
    }
    let params = driver::protocol_params(client, observer_url).await?;
    let owners: Vec<SigningKey> = (0..config.rpc_urls.len())
        .map(|i| workload::derive_key(config.run_id, "worker", i))
        .collect();
    let addresses: Vec<String> = owners.iter().map(workload::address).collect();
    tracing::info!(
        workers = owners.len(),
        "funding and registering receipt workers"
    );
    driver::fund(
        client,
        observer_url,
        &config.faucet,
        &addresses,
        MIN_WORKER_BOND + workload::TRANSFER_FEE,
        FUNDING_TIMEOUT,
    )
    .await?;
    let registrations: Vec<(SigningKey, String)> = owners
        .iter()
        .enumerate()
        .map(|(i, owner)| (owner.clone(), workload::worker_id(config.run_id, i)))
        .collect();
    driver::register_workers(client, observer_url, &registrations, FUNDING_TIMEOUT).await?;

    // Receipt clients: funded like senders, each allowing the coordinator its whole balance.
    let Some(coordinator) = &config.coordinator else {
        return Err(LoadError::Config("receipts need a genesis coordinator key"));
    };
    let client_keys: Vec<SigningKey> = (0..config.rpc_urls.len())
        .map(|i| workload::derive_key(config.run_id, "client", i))
        .collect();
    let clients: Vec<String> = client_keys.iter().map(workload::address).collect();
    driver::fund(
        client,
        observer_url,
        &config.faucet,
        &clients,
        config.fund_amount,
        FUNDING_TIMEOUT,
    )
    .await?;
    driver::set_allowances(
        client,
        observer_url,
        &client_keys,
        &workload::address(coordinator),
        config.fund_amount,
        FUNDING_TIMEOUT,
    )
    .await?;
    Ok(Some(ReceiptWorkers {
        owners,
        clients,
        price_per_token: params.price_per_token,
    }))
}

#[allow(clippy::too_many_arguments)]
fn spawn_generators(
    config: &LoadConfig,
    client: &RpcClient,
    sender_keys: Vec<SigningKey>,
    senders: &[String],
    workers: Option<ReceiptWorkers>,
    end: Instant,
    tracker: &Arc<Mutex<Tracker>>,
    lag: &Arc<Lag>,
) -> JoinSet<()> {
    let nodes = config.rpc_urls.len();
    let mut set = JoinSet::new();
    if let Some(interval) = config.transfer_interval() {
        for (index, key) in sender_keys.into_iter().enumerate() {
            let generator = Generator {
                node_url: config.rpc_urls[index % nodes].clone(),
                interval,
                end,
            };
            let stream = TransferStream {
                index,
                key,
                recipient: senders[(index + 1) % senders.len()].clone(),
            };
            set.spawn(driver::run_transfers(
                client.clone(),
                generator,
                stream,
                Arc::clone(tracker),
                Arc::clone(lag),
            ));
        }
    }
    let receipts = config
        .receipt_interval()
        .zip(workers)
        .zip(config.coordinator.clone());
    if let Some(((interval, workers), coordinator)) = receipts {
        for (index, node_url) in config.rpc_urls.iter().enumerate() {
            let generator = Generator {
                node_url: node_url.clone(),
                interval,
                end,
            };
            let stream = ReceiptStream {
                index,
                run_id: config.run_id,
                coordinator: coordinator.clone(),
                worker_owner: workers.owners[index].clone(),
                price_per_token: workers.price_per_token,
                clients: vec![workers.clients[index].clone()],
                worker_id: workload::worker_id(config.run_id, index),
            };
            set.spawn(driver::run_receipts(
                client.clone(),
                generator,
                stream,
                Arc::clone(tracker),
                Arc::clone(lag),
            ));
        }
    }
    set
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 10_000.0).round() / 10.0
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LoadConfig {
        LoadConfig {
            rpc_urls: vec!["http://a:1".into(), "http://b:1".into()],
            faucet: SigningKey::from_bytes(&[1; 32]),
            senders: 4,
            rate: 100.0,
            receipt_share: 0.2,
            duration: Duration::from_secs(10),
            warmup: Duration::from_secs(2),
            drain_timeout: Duration::from_secs(5),
            fund_amount: 1_000,
            run_id: 1,
            api_key: None,
            coordinator: Some(workload::derive_key(1, "coordinator", 0)),
        }
    }

    #[test]
    fn test_rate_is_split_between_senders_and_receipt_streams() {
        let c = config();
        assert!(c.validate().is_ok());
        // 80 transfers/s over 4 senders = 20/s each; 20 receipts/s over 2 nodes = 10/s each.
        assert_eq!(c.transfer_interval(), Some(Duration::from_millis(50)));
        assert_eq!(c.receipt_interval(), Some(Duration::from_millis(100)));
    }

    #[test]
    fn test_pure_mixes_disable_the_other_kind() {
        let transfers_only = LoadConfig {
            receipt_share: 0.0,
            ..config()
        };
        assert_eq!(transfers_only.receipt_interval(), None);
        let receipts_only = LoadConfig {
            receipt_share: 1.0,
            ..config()
        };
        assert_eq!(receipts_only.transfer_interval(), None);
    }

    #[test]
    fn test_invalid_configurations_are_refused() {
        let cases = [
            LoadConfig {
                rpc_urls: vec![],
                ..config()
            },
            LoadConfig {
                rpc_urls: vec!["https://a:1".into()],
                ..config()
            },
            LoadConfig {
                senders: 0,
                ..config()
            },
            LoadConfig {
                rate: 0.0,
                ..config()
            },
            LoadConfig {
                rate: f64::NAN,
                ..config()
            },
            LoadConfig {
                receipt_share: 1.5,
                ..config()
            },
            LoadConfig {
                warmup: Duration::from_secs(10),
                ..config()
            },
            LoadConfig {
                fund_amount: 0,
                ..config()
            },
        ];
        for case in cases {
            assert!(case.validate().is_err());
        }
    }

    #[test]
    fn test_loopback_urls_are_recognised() {
        for url in [
            "http://127.0.0.1:8545",
            "http://localhost:1",
            "http://[::1]:8545/x",
        ] {
            assert!(is_loopback_url(url), "{url}");
        }
        for url in [
            "http://node0:8545",
            "http://10.0.0.1:8545",
            "http://[2001:db8::1]:1",
        ] {
            assert!(!is_loopback_url(url), "{url}");
        }
    }
}

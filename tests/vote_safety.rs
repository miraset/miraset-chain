//! Vote safety (TODO 8.3.2, PRD R07 / AT07).
//!
//! T2: the replica durably records `highest_view_phase_voted` *before* a phase vote reaches the
//! network (vendored `hotstuff_rs` patch #7, `lib/hotstuff_rs/MIRASET_PATCHES.md`).

use ed25519_dalek::SigningKey;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hotstuff_rs::block_tree::pluggables::KVGet;
use hotstuff_rs::hotstuff::messages::HotStuffMessage;
use hotstuff_rs::networking::messages::{Message, ProgressMessage};
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::update_sets::ValidatorSetUpdates;
use hotstuff_rs::types::validator_set::{ValidatorSet, VerifyingKey};

use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::storage::MemKVStore;

const VOTES_REQUIRED: usize = 5;
const TIMEOUT: Duration = Duration::from_secs(20);

/// What the probe saw for one outgoing phase vote.
#[derive(Clone, Copy, Debug)]
struct VoteObservation {
    vote_view: u64,
    recorded_view: Option<u64>,
}

/// `Network` wrapper that, for every outgoing phase vote, reads the vote record from the
/// replica's own store at the moment of sending.
#[derive(Clone)]
struct VoteRecordProbe {
    inner: ChannelNetwork,
    store: MemKVStore,
    observations: Arc<Mutex<Vec<VoteObservation>>>,
}

impl VoteRecordProbe {
    fn observe(&self, message: &Message) {
        let Message::ProgressMessage(ProgressMessage::HotStuffMessage(HotStuffMessage::PhaseVote(
            vote,
        ))) = message
        else {
            return;
        };
        let recorded_view = self
            .store
            .highest_view_phase_voted()
            .expect("vote record readable")
            .map(|view| view.int());
        self.observations
            .lock()
            .expect("probe lock")
            .push(VoteObservation {
                vote_view: vote.view.int(),
                recorded_view,
            });
    }
}

impl Network for VoteRecordProbe {
    fn init_validator_set(&mut self, validator_set: ValidatorSet) {
        self.inner.init_validator_set(validator_set);
    }

    fn update_validator_set(&mut self, updates: ValidatorSetUpdates) {
        self.inner.update_validator_set(updates);
    }

    fn broadcast(&mut self, message: Message) {
        self.observe(&message);
        self.inner.broadcast(message);
    }

    fn send(&mut self, peer: VerifyingKey, message: Message) {
        self.observe(&message);
        self.inner.send(peer, message);
    }

    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        self.inner.recv()
    }
}

#[test]
fn test_phase_vote_is_recorded_before_it_is_sent() {
    let keypair = SigningKey::from_bytes(&[41u8; 32]);
    let vk = keypair.verifying_key();
    let store = MemKVStore::new();
    init_genesis(store.clone(), &[(vk, 100)], &[("alice", 1_000)]);

    let observations = Arc::new(Mutex::new(Vec::new()));
    let probe = VoteRecordProbe {
        inner: ChannelNetwork::new(vk, ChannelNetworkHub::new()),
        store: store.clone(),
        observations: observations.clone(),
    };
    let _node = MirasetNode::start(NodeConfig::devnet(keypair), store, probe, Mempool::new());

    let deadline = Instant::now() + TIMEOUT;
    while observations.lock().expect("probe lock").len() < VOTES_REQUIRED {
        assert!(Instant::now() < deadline, "replica did not vote in time");
        thread::sleep(Duration::from_millis(25));
    }

    let seen = observations.lock().expect("probe lock").clone();
    for observation in &seen {
        let recorded = observation.recorded_view.unwrap_or_else(|| {
            panic!(
                "vote for view {} sent with no vote record",
                observation.vote_view
            )
        });
        assert!(
            recorded >= observation.vote_view,
            "vote for view {} sent while the record was only {recorded}",
            observation.vote_view
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Signing guard end to end (T4, T5).
// ---------------------------------------------------------------------------------------------

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use miraset_chain::consensus::{GuardIdentity, GuardedNetwork, OpenMode, SafetyHalt, SigningGuard};
use miraset_chain::node::genesis_state_commitment;
use miraset_chain::rpc::{HealthResponse, RpcState, create_router};
use miraset_chain::storage::RedbKVStore;
use tower::ServiceExt;

const CHAIN_ID: u64 = 1337;
const ACCOUNTS: [(&str, u64); 1] = [("alice", 1_000)];
const FAST_VIEW: Duration = Duration::from_millis(150);

/// Records every phase vote that got past the guard, i.e. actually left the node.
#[derive(Clone)]
struct VoteTap {
    inner: ChannelNetwork,
    sent: Arc<Mutex<Vec<u64>>>,
}

impl VoteTap {
    fn new(vk: VerifyingKey) -> Self {
        Self {
            inner: ChannelNetwork::new(vk, ChannelNetworkHub::new()),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn sent_views(&self) -> Vec<u64> {
        self.sent.lock().expect("tap lock").clone()
    }

    fn record(&self, message: &Message) {
        if let Message::ProgressMessage(ProgressMessage::HotStuffMessage(
            HotStuffMessage::PhaseVote(vote),
        )) = message
        {
            self.sent.lock().expect("tap lock").push(vote.view.int());
        }
    }
}

impl Network for VoteTap {
    fn init_validator_set(&mut self, validator_set: ValidatorSet) {
        self.inner.init_validator_set(validator_set);
    }
    fn update_validator_set(&mut self, updates: ValidatorSetUpdates) {
        self.inner.update_validator_set(updates);
    }
    fn broadcast(&mut self, message: Message) {
        self.record(&message);
        self.inner.broadcast(message);
    }
    fn send(&mut self, peer: VerifyingKey, message: Message) {
        self.record(&message);
        self.inner.send(peer, message);
    }
    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        self.inner.recv()
    }
}

fn guard_identity(vk: &VerifyingKey) -> GuardIdentity {
    GuardIdentity {
        chain_id: CHAIN_ID,
        validator: vk.to_bytes(),
        genesis_hash: genesis_state_commitment(&ACCOUNTS),
    }
}

fn fast_config(keypair: SigningKey, halt: &SafetyHalt) -> NodeConfig {
    let mut config = NodeConfig::devnet(keypair);
    config.max_view_time = FAST_VIEW;
    config.safety_halt = halt.clone();
    config
}

fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(25));
    }
}

fn last_guard_view(guard: &SigningGuard) -> u64 {
    guard.last_vote().unwrap().map_or(0, |record| record.view)
}

/// T4: chain data wiped, guard kept. Re-running from genesis must not re-sign any view the
/// validator already voted in; votes resume once views pass the guard record.
#[test]
fn test_wiped_chain_data_cannot_resign_old_views() {
    let keypair = SigningKey::from_bytes(&[51u8; 32]);
    let vk = keypair.verifying_key();
    let dir = tempfile::TempDir::new().unwrap();
    let guard_path = dir.path().join("signing-guard.redb");

    // Run 1: vote in some views.
    let first_run_last_view = {
        let guard = SigningGuard::open(&guard_path, guard_identity(&vk), OpenMode::CreateIfMissing)
            .unwrap();
        let store = MemKVStore::new();
        init_genesis(store.clone(), &[(vk, 100)], &ACCOUNTS);
        let halt = SafetyHalt::new();
        let network = GuardedNetwork::new(VoteTap::new(vk), guard.clone(), halt.clone());
        let node = MirasetNode::start(
            fast_config(keypair.clone(), &halt),
            store,
            network,
            Mempool::new(),
        );
        wait_for("first-run votes", || last_guard_view(&guard) >= 10);
        drop(node);
        assert!(!halt.is_halted());
        last_guard_view(&guard)
    };

    // Run 2: fresh (wiped) chain database, same guard file.
    let guard =
        SigningGuard::open(&guard_path, guard_identity(&vk), OpenMode::RequireExisting).unwrap();
    let store = MemKVStore::new();
    init_genesis(store.clone(), &[(vk, 100)], &ACCOUNTS);
    let halt = SafetyHalt::new();
    let tap = VoteTap::new(vk);
    let network = GuardedNetwork::new(tap.clone(), guard.clone(), halt.clone());
    let probe = network.clone();
    let node = MirasetNode::start(fast_config(keypair, &halt), store, network, Mempool::new());
    wait_for("votes to resume past the guard record", || {
        last_guard_view(&guard) > first_run_last_view + 3
    });
    drop(node);

    assert!(
        !halt.is_halted(),
        "stale votes must not halt: {:?}",
        halt.reason()
    );
    assert!(
        probe.stale_votes_dropped() > 0,
        "the restarted replica should have tried to re-vote old views"
    );
    // Reaching the last signed view again on a new chain is refused, not a halt.
    assert!(probe.conflicting_votes_refused() <= 1);
    let resent: Vec<u64> = tap
        .sent_views()
        .into_iter()
        .filter(|view| *view <= first_run_last_view)
        .collect();
    assert!(resent.is_empty(), "re-signed old views {resent:?}");
}

/// T5: a chain-store write failure halts the node visibly: no further votes, `/health` 503.
#[tokio::test(flavor = "multi_thread")]
async fn test_chain_store_write_failure_halts_visibly() {
    let keypair = SigningKey::from_bytes(&[52u8; 32]);
    let vk = keypair.verifying_key();
    let dir = tempfile::TempDir::new().unwrap();
    let halt = SafetyHalt::new();
    let store = RedbKVStore::open(dir.path().join("chain.redb"))
        .unwrap()
        .with_safety_halt(halt.clone());
    init_genesis(store.clone(), &[(vk, 100)], &ACCOUNTS);
    let guard = SigningGuard::open(
        &dir.path().join("signing-guard.redb"),
        guard_identity(&vk),
        OpenMode::CreateIfMissing,
    )
    .unwrap();
    let tap = VoteTap::new(vk);
    let network = GuardedNetwork::new(tap.clone(), guard, halt.clone());
    let node = Arc::new(MirasetNode::start(
        fast_config(keypair, &halt),
        store.clone(),
        network,
        Mempool::new(),
    ));

    wait_for("initial votes", || tap.sent_views().len() >= 3);
    store.set_fail_next_write(true);
    wait_for("safety halt", || halt.is_halted());
    assert!(halt.reason().unwrap().contains("chain store write failed"));

    let votes_at_halt = tap.sent_views().len();
    tokio::time::sleep(FAST_VIEW * 4).await;
    assert_eq!(tap.sent_views().len(), votes_at_halt, "no votes after halt");

    let app = create_router(RpcState {
        node: Arc::clone(&node),
        chain_id: CHAIN_ID,
    });
    let request = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let health: HealthResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(health.status, "halted");
    assert!(health.reason.unwrap().contains("chain store write failed"));

    // The replica's algorithm thread panicked by design; `Replica::drop` would re-panic joining
    // it. The validator binary exits the process instead (see `main.rs`), so do the same here.
    std::mem::forget(node);
}

// ---------------------------------------------------------------------------------------------
// Purpose-bound signer (key custody D1, vendored patch #9).
// ---------------------------------------------------------------------------------------------

use hotstuff_rs::types::data_types::SignatureBytes;
use hotstuff_rs::types::signer::{LocalKeySigner, SignError, SignRequest, ValidatorSigner};

/// Signs everything except phase votes, which it refuses.
struct RefusePhaseVotes(LocalKeySigner);

impl ValidatorSigner for RefusePhaseVotes {
    fn public_key(&self) -> VerifyingKey {
        self.0.public_key()
    }

    fn sign(&self, request: &SignRequest) -> Result<SignatureBytes, SignError> {
        match request {
            SignRequest::PhaseVote { .. } => Err(SignError::Refused("test".into())),
            _ => self.0.sign(request),
        }
    }
}

/// A refusing signer means no phase vote leaves the replica, and the replica keeps running.
#[test]
fn test_signer_refusal_sends_no_phase_vote_and_does_not_halt() {
    let keypair = SigningKey::from_bytes(&[53u8; 32]);
    let vk = keypair.verifying_key();
    let store = MemKVStore::new();
    init_genesis(store.clone(), &[(vk, 100)], &ACCOUNTS);
    let halt = SafetyHalt::new();
    let tap = VoteTap::new(vk);

    let mut config =
        NodeConfig::devnet_with_signer(Arc::new(RefusePhaseVotes(LocalKeySigner::new(keypair))));
    config.max_view_time = FAST_VIEW;
    config.safety_halt = halt.clone();
    let node = MirasetNode::start(config, store, tap.clone(), Mempool::new());

    thread::sleep(FAST_VIEW * 10);
    assert!(
        tap.sent_views().is_empty(),
        "phase votes left: {:?}",
        tap.sent_views()
    );
    assert_eq!(
        node.highest_committed_height(),
        None,
        "no votes, no commits"
    );
    assert!(!halt.is_halted());
}

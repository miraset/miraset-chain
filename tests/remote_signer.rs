//! Remote validator signer over gRPC on a Unix socket (TODO 8.3.4, key custody D3).
//!
//! A real tonic server (`miraset_chain::signer::serve`) runs on its own runtime thread; the
//! client is the `RemoteSigner` the validator uses.

use ed25519_dalek::SigningKey;
use hotstuff_rs::hotstuff::types::Phase;
use hotstuff_rs::types::data_types::{BlockHeight, ChainID, CryptoHash, ViewNumber};
use hotstuff_rs::types::signer::{LocalKeySigner, SignError, SignRequest, ValidatorSigner};
use miraset_chain::consensus::{GuardIdentity, GuardedSigner, OpenMode, SafetyHalt, SigningGuard};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub};
use miraset_chain::node::{MirasetNode, NodeConfig, init_genesis};
use miraset_chain::signer::{
    RemoteSigner, RemoteSignerExpectation, SignerService, bind_socket, serve,
};
use miraset_chain::storage::MemKVStore;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

const CHAIN: u64 = 1337;
const WAIT: Duration = Duration::from_secs(20);

fn key() -> SigningKey {
    SigningKey::from_bytes(&[71u8; 32])
}

struct Server {
    socket: PathBuf,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<Result<(), String>>>,
    _dir: tempfile::TempDir,
}

impl Server {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let guard = SigningGuard::open(
            &dir.path().join("guard.redb"),
            GuardIdentity {
                chain_id: CHAIN,
                validator: key().verifying_key().to_bytes(),
                genesis_hash: [0u8; 32],
            },
            OpenMode::CreateIfMissing,
        )
        .unwrap();
        let signer = GuardedSigner::new(key(), guard).unwrap();
        let socket = dir.path().join("signer.sock");
        let (stop, stopped) = oneshot::channel::<()>();
        let path = socket.clone();
        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let listener = bind_socket(&path)?;
                let service = SignerService::new(signer, SafetyHalt::new());
                serve(listener, path, service, async {
                    let _ = stopped.await;
                })
                .await
            })
        });
        let deadline = Instant::now() + WAIT;
        while !socket.exists() {
            assert!(Instant::now() < deadline, "signer socket never appeared");
            thread::sleep(Duration::from_millis(10));
        }
        Self {
            socket,
            stop: Some(stop),
            thread: Some(thread),
            _dir: dir,
        }
    }

    fn client(&self) -> RemoteSigner {
        RemoteSigner::connect(&self.socket, &expect()).expect("connect")
    }

    /// Wait for the server to stop on its own (halt) and return its result.
    fn join(mut self) -> Result<(), String> {
        self.thread.take().unwrap().join().unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn expect() -> RemoteSignerExpectation {
    RemoteSignerExpectation {
        chain_id: CHAIN,
        validator: Some(key().verifying_key().to_bytes()),
    }
}

fn vote(chain: u64, view: u64, block: u8) -> SignRequest {
    SignRequest::PhaseVote {
        chain_id: ChainID::new(chain),
        view: ViewNumber::new(view),
        block: CryptoHash::new([block; 32]),
        phase: Phase::Generic,
    }
}

#[test]
fn test_remote_signatures_equal_local_signatures() {
    let server = Server::start();
    let remote = server.client();
    let local = LocalKeySigner::new(key());
    assert_eq!(remote.public_key(), local.public_key());

    let requests = [
        vote(CHAIN, 3, 0xaa),
        SignRequest::TimeoutVote {
            chain_id: ChainID::new(CHAIN),
            view: ViewNumber::new(4),
        },
        SignRequest::AdvertiseBlock {
            chain_id: ChainID::new(CHAIN),
            highest_committed_block_height: BlockHeight::new(2),
        },
    ];
    for request in &requests {
        let ours = remote.sign(request).unwrap();
        assert_eq!(ours.bytes(), local.sign(request).unwrap().bytes());
    }
}

#[test]
fn test_refusals_cross_the_wire_as_refusals() {
    let server = Server::start();
    let remote = server.client();
    remote.sign(&vote(CHAIN, 10, 0xaa)).unwrap();
    assert!(
        remote.sign(&vote(CHAIN, 10, 0xaa)).is_ok(),
        "identical re-send"
    );
    assert!(matches!(
        remote.sign(&vote(CHAIN, 9, 0xbb)),
        Err(SignError::Refused(_))
    ));
    assert!(matches!(
        remote.sign(&vote(CHAIN, 10, 0xbb)),
        Err(SignError::Refused(_))
    ));
}

/// K3 at the signer: several nodes (primary, standby, a cloned VM) sharing one signer race to
/// sign conflicting votes for the same views. Each view gets at most one signed block.
#[test]
fn test_concurrent_clients_never_get_two_votes_signed_for_one_view() {
    const CLIENTS: u8 = 4;
    const VIEWS: u64 = 30;
    let server = Server::start();
    let signed: Arc<Mutex<HashMap<u64, HashSet<u8>>>> = Arc::default();

    let workers: Vec<_> = (0..CLIENTS)
        .map(|client_id| {
            let remote = server.client();
            let signed = Arc::clone(&signed);
            thread::spawn(move || {
                for view in 1..=VIEWS {
                    if remote.sign(&vote(CHAIN, view, client_id)).is_ok() {
                        signed
                            .lock()
                            .unwrap()
                            .entry(view)
                            .or_default()
                            .insert(client_id);
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }

    let signed = signed.lock().unwrap();
    assert!(!signed.is_empty(), "nothing was signed");
    for (view, blocks) in signed.iter() {
        assert_eq!(blocks.len(), 1, "view {view} signed for blocks {blocks:?}");
    }
}

#[test]
fn test_signer_fault_halts_the_server_and_it_signs_nothing_more() {
    let server = Server::start();
    let remote = server.client();
    assert!(matches!(
        remote.sign(&vote(CHAIN + 1, 1, 0xaa)),
        Err(SignError::Failed(_))
    ));
    assert!(matches!(
        remote.sign(&vote(CHAIN, 2, 0xaa)),
        Err(SignError::Failed(_))
    ));
    let result = server.join();
    assert!(result.unwrap_err().contains("halted"));
}

#[test]
fn test_connect_rejects_wrong_pin_and_missing_socket() {
    let server = Server::start();
    let wrong_pin = RemoteSignerExpectation {
        chain_id: CHAIN,
        validator: Some([1u8; 32]),
    };
    assert!(RemoteSigner::connect(&server.socket, &wrong_pin).is_err());
    let wrong_chain = RemoteSignerExpectation {
        chain_id: CHAIN + 1,
        validator: None,
    };
    assert!(RemoteSigner::connect(&server.socket, &wrong_chain).is_err());
    assert!(RemoteSigner::connect(Path::new("/nonexistent/signer.sock"), &expect()).is_err());
}

/// The validator holds no key: it signs every message through the remote signer and finalizes.
#[test]
fn test_validator_with_remote_signer_finalizes_blocks() {
    let server = Server::start();
    let remote = Arc::new(server.client());
    let vk = remote.public_key();
    let store = MemKVStore::new();
    init_genesis(store.clone(), &[(vk, 100)], &[("alice", 1_000)]);
    let config = NodeConfig::devnet_with_signer(remote);
    let network = ChannelNetwork::new(vk, ChannelNetworkHub::new());
    let node = MirasetNode::start(config, store, network, Mempool::new());

    let deadline = Instant::now() + WAIT;
    while node.highest_committed_height().unwrap_or(0) < 3 {
        assert!(Instant::now() < deadline, "no finality with remote signer");
        thread::sleep(Duration::from_millis(25));
    }
}

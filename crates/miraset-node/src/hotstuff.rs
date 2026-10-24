use borsh::{BorshDeserialize, BorshSerialize};
use chrono::{DateTime, Utc};
use hotstuff_rs::{
    app::{
        App, ProduceBlockRequest, ProduceBlockResponse, ValidateBlockRequest, ValidateBlockResponse,
    },
    block_tree::{
        accessors::app::AppBlockTreeView,
        pluggables::{KVGet, KVGetError, KVStore, Key, WriteBatch},
    },
    events::CommitBlockEvent,
    networking::{messages::Message, network::Network},
    replica::{Configuration, Replica, ReplicaSpec},
    types::{
        crypto_primitives::{CryptoHasher, Digest},
        data_types::{
            BlockHeight, BufferSize, ChainID, CryptoHash, Data, Datum, EpochLength, Power,
        },
        update_sets::{AppStateUpdates, ValidatorSetUpdates},
        validator_set::{SigningKey, ValidatorSet, ValidatorSetState, VerifyingKey},
    },
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::{State, Storage};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_INBOUND_CONNECTIONS: usize = 64;
const NETWORK_QUEUE_CAPACITY: usize = 4096;
const HOTSTUFF_TREE: &str = "hotstuff_block_tree";

#[derive(Clone, Debug)]
pub struct ValidatorPeer {
    pub public_key: [u8; 32],
    pub address: SocketAddr,
}

#[derive(Clone, Debug)]
pub struct HotStuffConfig {
    pub bind_address: SocketAddr,
    pub signing_key: [u8; 32],
    pub validators: Vec<ValidatorPeer>,
    pub chain_id: u64,
    pub max_view_time: Duration,
}

pub struct HotStuffNode {
    _replica: Replica<HotStuffStore>,
    _network_runtime: NetworkRuntime,
}

pub fn start_hotstuff(
    state: State,
    storage: &Storage,
    config: HotStuffConfig,
) -> anyhow::Result<HotStuffNode> {
    anyhow::ensure!(
        !config.validators.is_empty(),
        "HotStuff validator set must not be empty"
    );
    anyhow::ensure!(
        config.max_view_time >= Duration::from_millis(500),
        "HotStuff max view time must be at least 500 ms"
    );

    let signing_key = SigningKey::from_bytes(&config.signing_key);
    let local_key = signing_key.verifying_key();
    let mut validator_set = ValidatorSet::new();
    let mut peer_addresses = HashMap::new();
    for validator in &config.validators {
        let key = VerifyingKey::from_bytes(&validator.public_key)?;
        anyhow::ensure!(
            peer_addresses.insert(key, validator.address).is_none(),
            "duplicate HotStuff validator public key"
        );
        validator_set.put(&key, Power::new(1));
    }
    anyhow::ensure!(
        validator_set.contains(&local_key),
        "local HotStuff signing key is not in the validator set"
    );
    anyhow::ensure!(
        peer_addresses.get(&local_key).map(SocketAddr::port) == Some(config.bind_address.port()),
        "local HotStuff validator port must match the consensus bind address port"
    );

    let (network, network_runtime) =
        TcpHotStuffNetwork::bind(local_key, config.bind_address, peer_addresses)?;
    let tree = storage.open_tree(HOTSTUFF_TREE)?;
    let kv_store = HotStuffStore::new(tree);

    if kv_store.is_empty() {
        let mut updates = ValidatorSetUpdates::new();
        for key in validator_set.validators() {
            updates.insert(*key, Power::new(1));
        }
        let validator_state =
            ValidatorSetState::new(validator_set.clone(), validator_set, None, true);
        Replica::initialize(kv_store.clone(), AppStateUpdates::new(), validator_state);
    }

    let configuration = Configuration::builder()
        .me(signing_key)
        .chain_id(ChainID::new(config.chain_id))
        .block_sync_request_limit(128)
        .block_sync_server_advertise_time(Duration::from_secs(2))
        .block_sync_response_timeout(Duration::from_secs(3))
        .block_sync_blacklist_expiry_time(Duration::from_secs(30))
        .block_sync_trigger_min_view_difference(1)
        .block_sync_trigger_timeout(Duration::from_secs(10))
        .progress_msg_buffer_capacity(BufferSize::new(16 * 1024 * 1024))
        .epoch_length(EpochLength::new(50))
        .max_view_time(config.max_view_time)
        .log_events(false)
        .build();

    let halted = Arc::new(AtomicBool::new(false));
    let commit_lock = Arc::new(Mutex::new(()));
    let commit_halted = Arc::clone(&halted);
    let commit_store = kv_store.clone();
    let commit_state = state.clone();
    let callback_lock = Arc::clone(&commit_lock);
    let watcher_state = state.clone();
    let watcher_store = kv_store.clone();
    let watcher_halted = Arc::clone(&halted);
    let watcher_lock = Arc::clone(&commit_lock);
    let watcher_shutdown = Arc::clone(&network_runtime.shutdown);
    let mut network_runtime = network_runtime;
    network_runtime.threads.push(thread::spawn(move || {
        while !watcher_shutdown.load(Ordering::Acquire) {
            sync_committed_hotstuff_blocks(
                &watcher_state,
                &watcher_store,
                &watcher_halted,
                &watcher_lock,
            );
            thread::sleep(Duration::from_millis(50));
        }
    }));

    let replica = ReplicaSpec::builder()
        .app(MirasetApp::new(
            state,
            kv_store.clone(),
            halted,
            commit_lock,
        ))
        .network(network)
        .kv_store(kv_store)
        .configuration(configuration)
        .on_commit_block(move |_event: &CommitBlockEvent| {
            sync_committed_hotstuff_blocks(
                &commit_state,
                &commit_store,
                &commit_halted,
                &callback_lock,
            );
        })
        .build()
        .start();

    Ok(HotStuffNode {
        _replica: replica,
        _network_runtime: network_runtime,
    })
}

#[derive(Serialize, Deserialize)]
struct MirasetProposal {
    transactions: Vec<miraset_core::Transaction>,
    timestamp_seconds: i64,
    timestamp_subsec_nanos: u32,
    expected_block_height: u64,
    expected_block_hash: [u8; 32],
}

impl MirasetProposal {
    fn timestamp(&self) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(self.timestamp_seconds, self.timestamp_subsec_nanos)
    }
}

struct MirasetApp {
    state: State,
    store: HotStuffStore,
    halted: Arc<AtomicBool>,
    commit_lock: Arc<Mutex<()>>,
}

impl MirasetApp {
    fn new(
        state: State,
        store: HotStuffStore,
        halted: Arc<AtomicBool>,
        commit_lock: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            state,
            store,
            halted,
            commit_lock,
        }
    }

    fn catch_up_committed(&self) {
        sync_committed_hotstuff_blocks(&self.state, &self.store, &self.halted, &self.commit_lock);
    }

    fn state_at_parent(
        &self,
        block_tree: &AppBlockTreeView<'_, HotStuffStore>,
        parent_block: Option<CryptoHash>,
    ) -> Option<(State, HashSet<[u8; 32]>)> {
        self.catch_up_committed();
        if self.halted.load(Ordering::Acquire) {
            return None;
        }
        let committed_height = self.state.get_latest_block().ok()?.height;
        let mut ancestors = Vec::new();
        let mut cursor = parent_block;
        while let Some(hash) = cursor {
            let height = block_tree.block_height(&hash).ok()??.int();
            if height < committed_height {
                break;
            }
            let block = block_tree.block(&hash).ok()??;
            cursor = if block.justify.is_genesis_pc() {
                None
            } else {
                Some(block.justify.block)
            };
            ancestors.push(block);
        }
        ancestors.reverse();

        let preview = self.state.preview_state();
        let mut in_flight_txs = HashSet::new();
        for ancestor in ancestors {
            let proposal = decode_proposal_from_block(&ancestor)?;
            let timestamp = proposal.timestamp()?;
            for tx in &proposal.transactions {
                if let Ok(tx_hash) = tx.hash() {
                    in_flight_txs.insert(tx_hash);
                }
            }
            let applied = preview
                .produce_block_with_transactions(proposal.transactions, timestamp)
                .ok()?;
            if applied.height != proposal.expected_block_height
                || applied.hash().ok() != Some(proposal.expected_block_hash)
            {
                return None;
            }
        }
        Some((preview, in_flight_txs))
    }

    fn validate_proposal(
        &self,
        request: &ValidateBlockRequest<'_, '_, HotStuffStore>,
    ) -> ValidateBlockResponse {
        let block = request.proposed_block();
        let Some(proposal) = decode_proposal_from_block(block) else {
            return ValidateBlockResponse::Invalid;
        };
        let Some(timestamp) = proposal.timestamp() else {
            return ValidateBlockResponse::Invalid;
        };
        let parent_block = if block.justify.is_genesis_pc() {
            None
        } else {
            Some(block.justify.block)
        };
        let Some((parent_state, _)) = self.state_at_parent(request.block_tree(), parent_block)
        else {
            return ValidateBlockResponse::Invalid;
        };
        let Ok(expected_block) =
            parent_state.produce_block_with_transactions(proposal.transactions, timestamp)
        else {
            return ValidateBlockResponse::Invalid;
        };
        if expected_block.height != proposal.expected_block_height
            || expected_block.hash().ok() != Some(proposal.expected_block_hash)
        {
            return ValidateBlockResponse::Invalid;
        }

        ValidateBlockResponse::Valid {
            app_state_updates: None,
            validator_set_updates: None,
        }
    }
}

impl App<HotStuffStore> for MirasetApp {
    fn produce_block(
        &mut self,
        request: ProduceBlockRequest<HotStuffStore>,
    ) -> ProduceBlockResponse {
        let timestamp = Utc::now();
        let Some((parent_state, in_flight_txs)) =
            self.state_at_parent(request.block_tree(), request.parent_block())
        else {
            tracing::error!("failed to reconstruct parent state for HotStuff proposal");
            return proposal_response(Vec::new(), timestamp, 0, [0; 32]);
        };

        let transactions: Vec<_> = self
            .state
            .pending_transactions()
            .into_iter()
            .filter(|tx| tx.hash().is_ok_and(|hash| !in_flight_txs.contains(&hash)))
            .collect();

        let candidate_state = parent_state.preview_state();
        let expected_block = match candidate_state
            .produce_block_with_transactions(transactions.clone(), timestamp)
        {
            Ok(block) => block,
            Err(error) => {
                tracing::error!(%error, "failed to preview HotStuff proposal; proposing an empty batch");
                let empty =
                    match parent_state.produce_block_with_transactions(Vec::new(), timestamp) {
                        Ok(block) => block,
                        Err(error) => {
                            tracing::error!(%error, "failed to preview empty HotStuff proposal");
                            return proposal_response(Vec::new(), timestamp, 0, [0; 32]);
                        }
                    };
                let hash = match empty.hash() {
                    Ok(hash) => hash,
                    Err(error) => {
                        tracing::error!(%error, "failed to hash empty HotStuff proposal");
                        [0; 32]
                    }
                };
                return proposal_response(Vec::new(), timestamp, empty.height, hash);
            }
        };

        let expected_block_hash = match expected_block.hash() {
            Ok(hash) => hash,
            Err(error) => {
                tracing::error!(%error, "failed to hash HotStuff proposal");
                [0; 32]
            }
        };
        proposal_response(
            transactions,
            timestamp,
            expected_block.height,
            expected_block_hash,
        )
    }

    fn validate_block(
        &mut self,
        request: ValidateBlockRequest<'_, '_, HotStuffStore>,
    ) -> ValidateBlockResponse {
        self.validate_proposal(&request)
    }

    fn validate_block_for_sync(
        &mut self,
        request: ValidateBlockRequest<'_, '_, HotStuffStore>,
    ) -> ValidateBlockResponse {
        self.validate_proposal(&request)
    }
}

fn decode_proposal_from_block(block: &hotstuff_rs::types::block::Block) -> Option<MirasetProposal> {
    if block.data.vec().len() != 1 {
        return None;
    }
    let bytes = block.data.vec()[0].bytes();
    let mut hasher = CryptoHasher::new();
    hasher.update(&bytes);
    let expected_data_hash = CryptoHash::new(hasher.finalize().into());
    if block.data_hash != expected_data_hash
        || hotstuff_rs::types::block::Block::hash(block.height, &block.justify, &block.data_hash)
            != block.hash
    {
        return None;
    }
    serde_json::from_slice::<MirasetProposal>(&bytes).ok()
}

fn proposal_response(
    transactions: Vec<miraset_core::Transaction>,
    timestamp: DateTime<Utc>,
    expected_block_height: u64,
    expected_block_hash: [u8; 32],
) -> ProduceBlockResponse {
    let proposal = MirasetProposal {
        transactions,
        timestamp_seconds: timestamp.timestamp(),
        timestamp_subsec_nanos: timestamp.timestamp_subsec_nanos(),
        expected_block_height,
        expected_block_hash,
    };
    let data_bytes = match serde_json::to_vec(&proposal) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::error!(%error, "failed to serialize HotStuff proposal");
            Vec::new()
        }
    };
    let mut hasher = CryptoHasher::new();
    hasher.update(&data_bytes);
    let data_hash = CryptoHash::new(hasher.finalize().into());
    ProduceBlockResponse {
        data_hash,
        data: Data::new(vec![Datum::new(data_bytes)]),
        app_state_updates: None,
        validator_set_updates: None,
    }
}

fn sync_committed_hotstuff_blocks(
    state: &State,
    store: &HotStuffStore,
    halted: &AtomicBool,
    commit_lock: &Mutex<()>,
) {
    let _guard = commit_lock.lock();
    if halted.load(Ordering::Acquire) {
        return;
    }
    let highest_committed_hash = match store.highest_committed_block() {
        Ok(Some(hash)) => hash,
        Ok(None) => return,
        Err(error) => {
            tracing::error!(?error, "failed to read highest committed HotStuff block");
            halted.store(true, Ordering::Release);
            return;
        }
    };
    let highest_committed_height = match store.block_height(&highest_committed_hash) {
        Ok(Some(height)) => height.int(),
        Ok(None) => {
            tracing::error!(block = %highest_committed_hash, "highest committed HotStuff block is missing height");
            halted.store(true, Ordering::Release);
            return;
        }
        Err(error) => {
            tracing::error!(?error, block = %highest_committed_hash, "failed to read highest committed HotStuff block height");
            halted.store(true, Ordering::Release);
            return;
        }
    };
    let local_miraset_height = match state.get_latest_block() {
        Ok(latest) => latest.height,
        Err(error) => {
            tracing::error!(%error, "failed to read latest Miraset block before HotStuff commit sync");
            halted.store(true, Ordering::Release);
            return;
        }
    };

    for hs_height in local_miraset_height..=highest_committed_height {
        let block_hash = match store.block_at_height(BlockHeight::new(hs_height)) {
            Ok(Some(hash)) => hash,
            Ok(None) => {
                tracing::error!(
                    hs_height,
                    "committed HotStuff block hash is missing at height"
                );
                halted.store(true, Ordering::Release);
                return;
            }
            Err(error) => {
                tracing::error!(
                    ?error,
                    hs_height,
                    "failed to read committed HotStuff block hash at height"
                );
                halted.store(true, Ordering::Release);
                return;
            }
        };
        if !apply_single_committed_hotstuff_block(state, store, &block_hash, halted) {
            return;
        }
    }
}

fn apply_single_committed_hotstuff_block(
    state: &State,
    store: &HotStuffStore,
    block_hash: &CryptoHash,
    halted: &AtomicBool,
) -> bool {
    let block = match store.block(block_hash) {
        Ok(Some(block)) => block,
        Ok(None) => {
            tracing::error!(block = %block_hash, "committed HotStuff block is missing from local block tree");
            halted.store(true, Ordering::Release);
            return false;
        }
        Err(error) => {
            tracing::error!(?error, block = %block_hash, "failed to read committed HotStuff block");
            halted.store(true, Ordering::Release);
            return false;
        }
    };
    let Some(payload) = decode_proposal_from_block(&block) else {
        tracing::error!(block = %block_hash, "committed HotStuff block has invalid Miraset payload or hash");
        halted.store(true, Ordering::Release);
        return false;
    };
    let Some(timestamp) = payload.timestamp() else {
        tracing::error!(block = %block_hash, "committed HotStuff block has an invalid timestamp");
        halted.store(true, Ordering::Release);
        return false;
    };

    match state.get_block(payload.expected_block_height) {
        Some(existing) if existing.hash().ok() == Some(payload.expected_block_hash) => return true,
        Some(_) => {
            tracing::error!(
                height = payload.expected_block_height,
                "HotStuff committed block conflicts with persisted Miraset history"
            );
            halted.store(true, Ordering::Release);
            return false;
        }
        None => {}
    }
    match state.get_latest_block() {
        Ok(latest) if latest.height + 1 != payload.expected_block_height => {
            tracing::error!(
                latest_height = latest.height,
                committed_height = payload.expected_block_height,
                "HotStuff commit is out of order with local Miraset history"
            );
            halted.store(true, Ordering::Release);
            return false;
        }
        Ok(_) => {}
        Err(error) => {
            tracing::error!(%error, "failed to read latest Miraset block before HotStuff commit");
            halted.store(true, Ordering::Release);
            return false;
        }
    }

    match state.produce_block_with_transactions(payload.transactions.clone(), timestamp) {
        Ok(applied) if applied.hash().ok() == Some(payload.expected_block_hash) => {
            if let Err(error) = state.remove_committed_transactions(&payload.transactions) {
                tracing::error!(%error, "failed to remove committed transactions from mempool");
                halted.store(true, Ordering::Release);
                return false;
            }
            tracing::info!(
                height = applied.height,
                block_hash = %hex::encode(payload.expected_block_hash),
                "applied HotStuff-committed Miraset block"
            );
            true
        }
        Ok(applied) => {
            tracing::error!(
                height = applied.height,
                "applied Miraset block hash differs from HotStuff payload; node state is inconsistent"
            );
            halted.store(true, Ordering::Release);
            false
        }
        Err(error) => {
            tracing::error!(%error, "failed to apply HotStuff-committed Miraset block; node state is inconsistent");
            halted.store(true, Ordering::Release);
            false
        }
    }
}

fn read_block_data_for_kv<G: KVGet + ?Sized>(
    getter: &G,
    block: &CryptoHash,
) -> Result<Option<Data>, KVGetError> {
    let Some(len) = getter.block_data_len(block)? else {
        return Ok(None);
    };
    let mut datums = Vec::with_capacity(len.int() as usize);
    for index in 0..len.int() {
        let Some(datum) = getter.block_datum(block, index) else {
            return Err(KVGetError::ValueExpectedButNotFound {
                key: Key::BlockData { block: *block },
            });
        };
        datums.push(datum);
    }
    Ok(Some(Data::new(datums)))
}

#[derive(Clone)]
struct HotStuffStore {
    tree: sled::Tree,
}

impl HotStuffStore {
    fn new(tree: sled::Tree) -> Self {
        Self { tree }
    }

    fn is_empty(&self) -> bool {
        match self.tree.iter().next() {
            None => true,
            Some(Ok(_)) => false,
            Some(Err(error)) => {
                tracing::error!(%error, "failed to inspect HotStuff storage");
                panic!("HotStuff storage inspection failed");
            }
        }
    }
}

impl KVGet for HotStuffStore {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        match self.tree.get(key) {
            Ok(value) => value.map(|value| value.to_vec()),
            Err(error) => {
                tracing::error!(%error, "failed to read HotStuff storage");
                panic!("HotStuff storage read failed");
            }
        }
    }

    fn block_data(&self, block: &CryptoHash) -> Result<Option<Data>, KVGetError> {
        read_block_data_for_kv(self, block)
    }
}

impl KVStore for HotStuffStore {
    type WriteBatch = HotStuffWriteBatch;
    type Snapshot<'a> = HotStuffSnapshot;

    fn write(&mut self, batch: Self::WriteBatch) {
        let mut sled_batch = sled::Batch::default();
        for (key, value) in batch.updates {
            match value {
                Some(value) => sled_batch.insert(key, value),
                None => sled_batch.remove(key),
            }
        }
        if let Err(error) = self.tree.apply_batch(sled_batch) {
            tracing::error!(%error, "failed to apply HotStuff storage batch");
            panic!("HotStuff storage write failed");
        }
        if let Err(error) = self.tree.flush() {
            tracing::error!(%error, "failed to flush HotStuff storage");
            panic!("HotStuff storage flush failed");
        }
    }

    fn clear(&mut self) {
        if let Err(error) = self.tree.clear() {
            tracing::error!(%error, "failed to clear HotStuff storage");
            panic!("HotStuff storage clear failed");
        }
        if let Err(error) = self.tree.flush() {
            tracing::error!(%error, "failed to flush cleared HotStuff storage");
            panic!("HotStuff storage flush failed");
        }
    }

    fn snapshot<'a>(&'a self) -> Self::Snapshot<'a> {
        let mut values = HashMap::new();
        for entry in self.tree.iter() {
            match entry {
                Ok((key, value)) => {
                    values.insert(key.to_vec(), value.to_vec());
                }
                Err(error) => {
                    tracing::error!(%error, "failed to create HotStuff storage snapshot");
                    panic!("HotStuff storage snapshot failed");
                }
            }
        }
        HotStuffSnapshot(values)
    }
}

struct HotStuffSnapshot(HashMap<Vec<u8>, Vec<u8>>);

impl KVGet for HotStuffSnapshot {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.get(key).cloned()
    }

    fn block_data(&self, block: &CryptoHash) -> Result<Option<Data>, KVGetError> {
        read_block_data_for_kv(self, block)
    }
}

struct HotStuffWriteBatch {
    updates: HashMap<Vec<u8>, Option<Vec<u8>>>,
}

impl WriteBatch for HotStuffWriteBatch {
    fn new() -> Self {
        Self {
            updates: HashMap::new(),
        }
    }

    fn set(&mut self, key: &[u8], value: &[u8]) {
        self.updates.insert(key.to_vec(), Some(value.to_vec()));
    }

    fn delete(&mut self, key: &[u8]) {
        self.updates.insert(key.to_vec(), None);
    }
}

#[derive(Clone)]
struct OutboundMessage {
    destination: Option<VerifyingKey>,
    message: Message,
}

#[derive(Clone)]
struct TcpHotStuffNetwork {
    peer_addresses: Arc<HashMap<VerifyingKey, SocketAddr>>,
    outbound: SyncSender<OutboundMessage>,
    inbound: Arc<Mutex<Receiver<(VerifyingKey, Message)>>>,
}

struct NetworkRuntime {
    shutdown: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Drop for NetworkRuntime {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                tracing::warn!("HotStuff network thread panicked during shutdown");
            }
        }
    }
}

impl TcpHotStuffNetwork {
    fn bind(
        local_key: VerifyingKey,
        address: SocketAddr,
        peer_addresses: HashMap<VerifyingKey, SocketAddr>,
    ) -> io::Result<(Self, NetworkRuntime)> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let (outbound_tx, outbound_rx) = mpsc::sync_channel(NETWORK_QUEUE_CAPACITY);
        let (inbound_tx, inbound_rx) = mpsc::sync_channel(NETWORK_QUEUE_CAPACITY);
        let peer_addresses = Arc::new(peer_addresses);
        let shutdown = Arc::new(AtomicBool::new(false));

        let sender_peers = Arc::clone(&peer_addresses);
        let sender_shutdown = Arc::clone(&shutdown);
        let local_inbound = inbound_tx.clone();
        let sender_thread = thread::Builder::new()
            .name("hotstuff-network-send".to_string())
            .spawn(move || {
                run_outbound(
                    outbound_rx,
                    sender_peers,
                    local_key,
                    local_inbound,
                    sender_shutdown,
                );
            })?;

        let receiver_peers = Arc::clone(&peer_addresses);
        let receiver_shutdown = Arc::clone(&shutdown);
        let receiver_thread = thread::Builder::new()
            .name("hotstuff-network-recv".to_string())
            .spawn(move || {
                run_inbound(listener, inbound_tx, receiver_peers, receiver_shutdown);
            })?;

        Ok((
            Self {
                peer_addresses,
                outbound: outbound_tx,
                inbound: Arc::new(Mutex::new(inbound_rx)),
            },
            NetworkRuntime {
                shutdown,
                threads: vec![sender_thread, receiver_thread],
            },
        ))
    }

    fn enqueue(&self, destination: Option<VerifyingKey>, message: Message) {
        if let Err(error) = self.outbound.try_send(OutboundMessage {
            destination,
            message,
        }) {
            match error {
                TrySendError::Full(_) => tracing::warn!("HotStuff outbound queue is full"),
                TrySendError::Disconnected(_) => {
                    tracing::error!("HotStuff outbound queue is closed")
                }
            }
        }
    }
}

impl Network for TcpHotStuffNetwork {
    fn init_validator_set(&mut self, validators: ValidatorSet) {
        for validator in validators.validators() {
            if !self.peer_addresses.contains_key(validator) {
                tracing::error!(
                    validator = %hex::encode(validator.to_bytes()),
                    "HotStuff validator has no configured network address"
                );
            }
        }
    }

    fn update_validator_set(&mut self, _updates: ValidatorSetUpdates) {}

    fn broadcast(&mut self, message: Message) {
        self.enqueue(None, message);
    }

    fn send(&mut self, peer: VerifyingKey, message: Message) {
        self.enqueue(Some(peer), message);
    }

    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        match self.inbound.lock().try_recv() {
            Ok(message) => Some(message),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                tracing::error!("HotStuff inbound queue is closed");
                None
            }
        }
    }
}

fn run_outbound(
    receiver: Receiver<OutboundMessage>,
    peers: Arc<HashMap<VerifyingKey, SocketAddr>>,
    local_key: VerifyingKey,
    local_inbound: SyncSender<(VerifyingKey, Message)>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Acquire) {
        let outbound = match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let broadcast = outbound.destination.is_none();
        let destinations: Vec<_> = match outbound.destination {
            Some(key) if key != local_key => peers.get(&key).copied().into_iter().collect(),
            Some(_) => Vec::new(),
            None => peers
                .iter()
                .filter(|(key, _)| **key != local_key)
                .map(|(_, address)| *address)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect(),
        };
        if broadcast || outbound.destination == Some(local_key) {
            match local_inbound.try_send((local_key, outbound.message.clone())) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    tracing::warn!("HotStuff local inbound queue is full");
                }
                Err(TrySendError::Disconnected(_)) => {
                    tracing::error!("HotStuff local inbound queue is closed");
                }
            }
        }
        let message_bytes = match outbound.message.try_to_vec() {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::error!(%error, "failed to serialize HotStuff network message");
                continue;
            }
        };
        for destination in destinations {
            if let Err(error) = send_frame(destination, local_key.to_bytes(), &message_bytes) {
                tracing::debug!(%error, %destination, "failed to send HotStuff message");
            } else {
                tracing::debug!(%destination, "sent HotStuff network message");
            }
        }
    }
}

fn send_frame(address: SocketAddr, sender: [u8; 32], message: &[u8]) -> io::Result<()> {
    let frame_len = sender.len().saturating_add(message.len());
    if frame_len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HotStuff frame exceeds maximum size",
        ));
    }
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(&(frame_len as u32).to_be_bytes())?;
    stream.write_all(&sender)?;
    stream.write_all(message)?;
    Ok(())
}

fn run_inbound(
    listener: TcpListener,
    sender: SyncSender<(VerifyingKey, Message)>,
    peers: Arc<HashMap<VerifyingKey, SocketAddr>>,
    shutdown: Arc<AtomicBool>,
) {
    let active_connections = Arc::new(AtomicUsize::new(0));
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                if active_connections.fetch_add(1, Ordering::AcqRel) >= MAX_INBOUND_CONNECTIONS {
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                    continue;
                }
                let sender = sender.clone();
                let peers = Arc::clone(&peers);
                let shutdown = Arc::clone(&shutdown);
                let connection_counter = Arc::clone(&active_connections);
                let spawn_result = thread::Builder::new()
                    .name("hotstuff-network-peer".to_string())
                    .spawn(move || {
                        let _active = ActiveConnection(connection_counter);
                        if let Err(error) = receive_frame(stream, sender, &peers, &shutdown) {
                            tracing::debug!(%error, "rejected HotStuff network frame");
                        }
                    });
                if let Err(error) = spawn_result {
                    tracing::error!(%error, "failed to spawn HotStuff peer reader");
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => {
                tracing::error!(%error, "HotStuff network listener failed");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

struct ActiveConnection(Arc<AtomicUsize>);

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn receive_frame(
    mut stream: TcpStream,
    sender: SyncSender<(VerifyingKey, Message)>,
    peers: &HashMap<VerifyingKey, SocketAddr>,
    shutdown: &AtomicBool,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut length_bytes = [0; 4];
    stream.read_exact(&mut length_bytes)?;
    let length = u32::from_be_bytes(length_bytes) as usize;
    if !(32..=MAX_FRAME_BYTES).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HotStuff frame length is invalid",
        ));
    }
    let mut frame = vec![0; length];
    stream.read_exact(&mut frame)?;
    let sender_bytes: [u8; 32] = frame[..32]
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid sender key"))?;
    let peer = VerifyingKey::from_bytes(&sender_bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !peers.contains_key(&peer) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "sender is not in the configured validator set",
        ));
    }
    let message = Message::try_from_slice(&frame[32..])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if shutdown.load(Ordering::Acquire) {
        return Ok(());
    }
    match sender.try_send((peer, message)) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "HotStuff inbound queue is full",
        )),
        Err(TrySendError::Disconnected(_)) => Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "HotStuff inbound queue is closed",
        )),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{HotStuffConfig, ValidatorPeer, start_hotstuff};
    use crate::{State, Storage};
    use hotstuff_rs::types::validator_set::SigningKey;
    use miraset_core::{KeyPair, Transaction, sign_transaction};
    use std::{net::TcpListener, thread, time::Instant};

    #[test]
    fn four_replicas_commit_the_same_miraset_block() {
        let seeds = [[0x11; 32], [0x22; 32], [0x33; 32], [0x44; 32]];
        let keys: Vec<_> = seeds.iter().map(SigningKey::from_bytes).collect();
        let addresses: Vec<_> = (0..4)
            .map(|_| {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                listener.local_addr().unwrap()
            })
            .collect();
        let validators: Vec<_> = keys
            .iter()
            .zip(&addresses)
            .map(|(key, address)| ValidatorPeer {
                public_key: key.verifying_key().to_bytes(),
                address: *address,
            })
            .collect();
        let directory = tempfile::tempdir().unwrap();
        let sender = KeyPair::from_bytes(&[7; 32]);
        let recipient = KeyPair::from_bytes(&[8; 32]);
        let mut transaction = Transaction::Transfer {
            from: sender.address(),
            to: recipient.address(),
            amount: 100,
            nonce: 0,
            signature: [0; 64],
        };
        sign_transaction(&mut transaction, &sender).unwrap();

        let states: Vec<_> = (0..4)
            .map(|_| {
                let state = State::new();
                state.add_balance(&sender.address(), 100_000_000);
                state
            })
            .collect();
        states[0].submit_transaction(transaction).unwrap();

        let mut nodes = Vec::new();
        for (index, state) in states.iter().enumerate() {
            let storage = Storage::open(directory.path().join(format!("node-{index}"))).unwrap();
            nodes.push(
                start_hotstuff(
                    state.clone(),
                    &storage,
                    HotStuffConfig {
                        bind_address: addresses[index],
                        signing_key: seeds[index],
                        validators: validators.clone(),
                        chain_id: 901,
                        max_view_time: std::time::Duration::from_secs(3),
                    },
                )
                .unwrap(),
            );
        }

        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let committed = states
                .iter()
                .all(|state| state.get_balance(&recipient.address()) == 100);
            let same_tip = states
                .iter()
                .map(State::get_latest_block)
                .collect::<Result<Vec<_>, _>>()
                .map(|blocks| {
                    blocks
                        .first()
                        .and_then(|first| first.hash().ok())
                        .is_some_and(|hash| {
                            blocks.iter().all(|block| block.hash().ok() == Some(hash))
                        })
                })
                .unwrap_or(false);
            if committed && same_tip {
                break;
            }
            if Instant::now() >= deadline {
                eprintln!(
                    "HotStuff convergence timeout: heights={:?}, balances={:?}, pending={:?}, committed={:?}, views={:?}",
                    states
                        .iter()
                        .map(|state| state.get_latest_block().map(|block| block.height))
                        .collect::<Vec<_>>(),
                    states
                        .iter()
                        .map(|state| state.get_balance(&recipient.address()))
                        .collect::<Vec<_>>(),
                    states
                        .iter()
                        .map(State::pending_transactions)
                        .map(|transactions| transactions.len())
                        .collect::<Vec<_>>(),
                    nodes
                        .iter()
                        .map(|node| {
                            let snapshot = node._replica.block_tree_camera().snapshot();
                            snapshot.highest_committed_block().and_then(|hash| {
                                hash.map_or(Ok(None), |hash| snapshot.block_height(&hash))
                            })
                        })
                        .collect::<Vec<_>>(),
                    nodes
                        .iter()
                        .map(|node| {
                            node._replica
                                .block_tree_camera()
                                .snapshot()
                                .highest_view_entered()
                        })
                        .collect::<Vec<_>>(),
                );
                panic!("HotStuff replicas did not converge");
            }
            thread::sleep(std::time::Duration::from_millis(100));
        }

        drop(nodes);
    }
}

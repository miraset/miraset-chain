use hotstuff_rs::networking::messages::Message;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::crypto_primitives::VerifyingKey;
use hotstuff_rs::types::update_sets::ValidatorSetUpdates;
use hotstuff_rs::types::validator_set::ValidatorSet;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// Back-off applied when a mailbox is empty. `hotstuff_rs` polls `Network::recv` in a
/// `yield_now` loop, so an instantly-empty in-memory mailbox would busy-spin a core per node.
const EMPTY_MAILBOX_BACKOFF: Duration = Duration::from_micros(500);

type Mailbox = VecDeque<(VerifyingKey, Message)>;
/// One mailbox per replica registered under a key. More than one only when a test runs clones
/// of a validator (the same key on several replicas), which models a copied key (custody K3).
type MailboxMap = HashMap<VerifyingKey, Vec<Mailbox>>;

/// Shared in-memory hub routing messages between replicas, supporting deterministic
/// partition and fault simulation (PRD R06).
#[derive(Default, Clone)]
pub struct ChannelNetworkHub {
    mailboxes: Arc<Mutex<MailboxMap>>,
    isolated_peers: Arc<Mutex<HashSet<VerifyingKey>>>,
    blocked_links: Arc<Mutex<HashSet<(VerifyingKey, VerifyingKey)>>>,
    /// Blocked `(key, replica, to)` links: one clone of a key cannot reach `to`.
    blocked_clone_links: Arc<Mutex<HashSet<(VerifyingKey, usize, VerifyingKey)>>>,
    /// Deliveries refused by an isolation or a blocked link (fault tests check it rises).
    dropped: Arc<AtomicU64>,
}

impl ChannelNetworkHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure `key` has its primary mailbox (index 0). Idempotent, so a replica restarted on the
    /// same hub keeps its mailbox.
    pub fn register(&self, key: VerifyingKey) -> usize {
        let mut lock = self.mailboxes.lock().expect("hub lock poisoned");
        let replicas = lock.entry(key).or_default();
        if replicas.is_empty() {
            replicas.push(Mailbox::new());
        }
        0
    }

    /// Add another replica mailbox for `key` and return its index: a clone of the validator (a
    /// copied key, custody threat K3). Every message addressed to the key reaches all its
    /// replicas, so clones each receive peers' replies and requests too. Equivocation tests only.
    pub fn register_clone(&self, key: VerifyingKey) -> usize {
        let mut lock = self.mailboxes.lock().expect("hub lock poisoned");
        let replicas = lock.entry(key).or_default();
        replicas.push(Mailbox::new());
        replicas.len() - 1
    }

    /// Deliveries refused by an isolation or a blocked link so far.
    pub fn dropped_deliveries(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Isolate a validator peer so all non-self messages to/from it are dropped.
    pub fn isolate_peer(&self, peer: VerifyingKey) {
        self.isolated_peers
            .lock()
            .expect("isolated_peers lock poisoned")
            .insert(peer);
    }

    /// Restore connectivity for a previously isolated validator peer.
    pub fn heal_peer(&self, peer: &VerifyingKey) {
        self.isolated_peers
            .lock()
            .expect("isolated_peers lock poisoned")
            .remove(peer);
    }

    /// Block directional link `from -> to`.
    pub fn block_link(&self, from: VerifyingKey, to: VerifyingKey) {
        self.blocked_links
            .lock()
            .expect("blocked_links lock poisoned")
            .insert((from, to));
    }

    /// Block messages from replica `from_mailbox` of key `from` to `to`, leaving the key's other
    /// replicas connected: each clone of a copied key reaches a different set of peers.
    pub fn block_clone_link(&self, from: VerifyingKey, from_mailbox: usize, to: VerifyingKey) {
        self.blocked_clone_links
            .lock()
            .expect("blocked_clone_links lock poisoned")
            .insert((from, from_mailbox, to));
    }

    /// Clear all simulated network faults and partitions.
    pub fn clear_faults(&self) {
        self.isolated_peers
            .lock()
            .expect("isolated_peers lock poisoned")
            .clear();
        self.blocked_links
            .lock()
            .expect("blocked_links lock poisoned")
            .clear();
        self.blocked_clone_links
            .lock()
            .expect("blocked_clone_links lock poisoned")
            .clear();
    }

    fn is_delivery_allowed(&self, from: &VerifyingKey, to: &VerifyingKey) -> bool {
        if from == to {
            return true;
        }
        let isolated = self
            .isolated_peers
            .lock()
            .expect("isolated_peers lock poisoned");
        if isolated.contains(from) || isolated.contains(to) {
            return false;
        }
        drop(isolated);
        let blocked = self
            .blocked_links
            .lock()
            .expect("blocked_links lock poisoned");
        !blocked.contains(&(*from, *to))
    }

    /// Deliver `msg` from replica `from_mailbox` of key `from` to every replica of `to`. A
    /// message a key sends to itself stays on the sending replica, as local delivery would on
    /// a real host; clones of a key therefore do not see each other's self-addressed messages.
    pub fn send_to(
        &self,
        from: VerifyingKey,
        from_mailbox: usize,
        to: &VerifyingKey,
        msg: Message,
    ) {
        let clone_blocked = self
            .blocked_clone_links
            .lock()
            .expect("blocked_clone_links lock poisoned")
            .contains(&(from, from_mailbox, *to));
        if clone_blocked || !self.is_delivery_allowed(&from, to) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let mut lock = self.mailboxes.lock().expect("hub lock poisoned");
        let Some(replicas) = lock.get_mut(to) else {
            return;
        };
        if from == *to {
            if let Some(queue) = replicas.get_mut(from_mailbox) {
                queue.push_back((from, msg));
            }
            return;
        }
        for queue in replicas.iter_mut() {
            queue.push_back((from, msg.clone()));
        }
    }

    pub fn broadcast(
        &self,
        from: VerifyingKey,
        from_mailbox: usize,
        recipients: &[VerifyingKey],
        msg: Message,
    ) {
        for peer in recipients {
            self.send_to(from, from_mailbox, peer, msg.clone());
        }
    }

    /// Next message for replica `index` registered under `key`.
    pub fn pop(&self, key: &VerifyingKey, index: usize) -> Option<(VerifyingKey, Message)> {
        let mut lock = self.mailboxes.lock().expect("hub lock poisoned");
        lock.get_mut(key)?.get_mut(index)?.pop_front()
    }
}

/// Node network adapter implementing `hotstuff_rs::networking::network::Network`.
#[derive(Clone)]
pub struct ChannelNetwork {
    my_key: VerifyingKey,
    mailbox: usize,
    hub: ChannelNetworkHub,
    validator_set: Arc<RwLock<Option<ValidatorSet>>>,
}

impl ChannelNetwork {
    /// The replica for `my_key` (its primary mailbox; reused across restarts on one hub).
    pub fn new(my_key: VerifyingKey, hub: ChannelNetworkHub) -> Self {
        let mailbox = hub.register(my_key);
        Self::with_mailbox(my_key, hub, mailbox)
    }

    /// An additional replica for `my_key` with its own mailbox (see
    /// [`ChannelNetworkHub::register_clone`]). Equivocation tests only.
    pub fn new_clone(my_key: VerifyingKey, hub: ChannelNetworkHub) -> Self {
        let mailbox = hub.register_clone(my_key);
        Self::with_mailbox(my_key, hub, mailbox)
    }

    fn with_mailbox(my_key: VerifyingKey, hub: ChannelNetworkHub, mailbox: usize) -> Self {
        Self {
            my_key,
            mailbox,
            hub,
            validator_set: Arc::new(RwLock::new(None)),
        }
    }

    pub fn my_key(&self) -> &VerifyingKey {
        &self.my_key
    }
}

impl Network for ChannelNetwork {
    fn init_validator_set(&mut self, validator_set: ValidatorSet) {
        let mut lock = self.validator_set.write().expect("lock poisoned");
        *lock = Some(validator_set);
    }

    fn update_validator_set(&mut self, updates: ValidatorSetUpdates) {
        let mut lock = self.validator_set.write().expect("lock poisoned");
        if let Some(ref mut vs) = *lock {
            vs.apply_updates(&updates);
        }
    }

    fn broadcast(&mut self, message: Message) {
        let lock = self.validator_set.read().expect("lock poisoned");
        if let Some(ref vs) = *lock {
            let peers: Vec<VerifyingKey> = vs.validators().copied().collect();
            drop(lock);
            self.hub
                .broadcast(self.my_key, self.mailbox, &peers, message);
        }
    }

    fn send(&mut self, peer: VerifyingKey, message: Message) {
        self.hub.send_to(self.my_key, self.mailbox, &peer, message);
    }

    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        let message = self.hub.pop(&self.my_key, self.mailbox);
        if message.is_none() {
            std::thread::sleep(EMPTY_MAILBOX_BACKOFF);
        }
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use hotstuff_rs::block_sync::messages::{BlockSyncMessage, BlockSyncRequest};
    use hotstuff_rs::types::data_types::{BlockHeight, ChainID, Power};

    fn generate_key_with_seed(seed: u8) -> (SigningKey, VerifyingKey) {
        let bytes = [seed; 32];
        let sk = SigningKey::from_bytes(&bytes);
        let vk = sk.verifying_key();
        (sk, vk)
    }

    #[test]
    fn test_channel_network_direct_send() {
        let hub = ChannelNetworkHub::new();
        let (_sk1, vk1) = generate_key_with_seed(1);
        let (_sk2, vk2) = generate_key_with_seed(2);

        let mut net1 = ChannelNetwork::new(vk1, hub.clone());
        let mut net2 = ChannelNetwork::new(vk2, hub);

        let test_req = BlockSyncRequest {
            chain_id: ChainID::new(1),
            start_height: BlockHeight::new(1),
            limit: 10,
        };
        let test_msg = Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(test_req));

        net1.send(vk2, test_msg.clone());

        // net1 should have nothing
        assert!(net1.recv().is_none());

        // net2 should receive message from vk1
        let (sender, received) = net2.recv().expect("message expected");
        assert_eq!(sender, vk1);
        match received {
            Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(req)) => {
                assert_eq!(req.start_height, BlockHeight::new(1));
                assert_eq!(req.limit, 10);
            }
            _ => panic!("unexpected message variant"),
        }
    }

    #[test]
    fn test_channel_network_broadcast() {
        let hub = ChannelNetworkHub::new();
        let (_sk1, vk1) = generate_key_with_seed(10);
        let (_sk2, vk2) = generate_key_with_seed(20);
        let (_sk3, vk3) = generate_key_with_seed(30);

        let mut net1 = ChannelNetwork::new(vk1, hub.clone());
        let mut net2 = ChannelNetwork::new(vk2, hub.clone());
        let mut net3 = ChannelNetwork::new(vk3, hub);

        let mut vs = ValidatorSet::new();
        vs.put(&vk1, Power::new(100));
        vs.put(&vk2, Power::new(100));
        vs.put(&vk3, Power::new(100));

        net1.init_validator_set(vs.clone());
        net2.init_validator_set(vs.clone());
        net3.init_validator_set(vs);

        let test_req = BlockSyncRequest {
            chain_id: ChainID::new(1),
            start_height: BlockHeight::new(5),
            limit: 2,
        };
        let test_msg = Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(test_req));

        net1.broadcast(test_msg);

        // Sender also receives own broadcast in HotStuff consensus model
        let (sender1, _) = net1.recv().expect("sender receives own broadcast");
        assert_eq!(sender1, vk1);

        // Both peer 2 and peer 3 receive the message
        let (sender2, msg2) = net2.recv().expect("peer 2 should receive");
        assert_eq!(sender2, vk1);
        match msg2 {
            Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(req)) => {
                assert_eq!(req.start_height, BlockHeight::new(5))
            }
            _ => panic!("wrong message"),
        }

        let (sender3, msg3) = net3.recv().expect("peer 3 should receive");
        assert_eq!(sender3, vk1);
        match msg3 {
            Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(req)) => {
                assert_eq!(req.start_height, BlockHeight::new(5))
            }
            _ => panic!("wrong message"),
        }
    }

    #[test]
    fn test_channel_network_clones_each_receive_every_message() {
        let hub = ChannelNetworkHub::new();
        let (_sk, vk_sender) = generate_key_with_seed(40);
        let (_sk, vk_clone) = generate_key_with_seed(41);
        let mut sender = ChannelNetwork::new(vk_sender, hub.clone());
        let mut clone_a = ChannelNetwork::new(vk_clone, hub.clone());
        let mut clone_b = ChannelNetwork::new_clone(vk_clone, hub.clone());

        let request = BlockSyncRequest {
            chain_id: ChainID::new(1),
            start_height: BlockHeight::new(3),
            limit: 1,
        };
        sender.send(
            vk_clone,
            Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(request)),
        );

        assert_eq!(clone_a.recv().expect("clone a").0, vk_sender);
        assert_eq!(clone_b.recv().expect("clone b").0, vk_sender);
        assert!(clone_a.recv().is_none());
        assert!(clone_b.recv().is_none());

        // A clone's message to its own key stays on that clone.
        let request = BlockSyncRequest {
            chain_id: ChainID::new(1),
            start_height: BlockHeight::new(9),
            limit: 1,
        };
        clone_a.send(
            vk_clone,
            Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(request)),
        );
        assert_eq!(clone_a.recv().expect("local delivery").0, vk_clone);
        assert!(clone_b.recv().is_none());

        // Isolating the key cuts off every clone.
        hub.isolate_peer(vk_clone);
        let request = BlockSyncRequest {
            chain_id: ChainID::new(1),
            start_height: BlockHeight::new(4),
            limit: 1,
        };
        sender.send(
            vk_clone,
            Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(request)),
        );
        assert!(clone_a.recv().is_none() && clone_b.recv().is_none());
        assert_eq!(hub.dropped_deliveries(), 1);
    }

    #[test]
    fn test_channel_network_reregistering_a_key_reuses_its_mailbox() {
        let hub = ChannelNetworkHub::new();
        let (_sk, vk) = generate_key_with_seed(50);
        let (_sk, vk_peer) = generate_key_with_seed(51);
        let mut peer = ChannelNetwork::new(vk_peer, hub.clone());
        let before_restart = ChannelNetwork::new(vk, hub.clone());
        drop(before_restart);
        let request = BlockSyncRequest {
            chain_id: ChainID::new(1),
            start_height: BlockHeight::new(1),
            limit: 1,
        };
        peer.send(
            vk,
            Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(request)),
        );
        let mut restarted = ChannelNetwork::new(vk, hub.clone());
        assert_eq!(restarted.recv().expect("queued before restart").0, vk_peer);
        assert!(restarted.recv().is_none());
    }
}

use hotstuff_rs::networking::messages::Message;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::crypto_primitives::VerifyingKey;
use hotstuff_rs::types::update_sets::ValidatorSetUpdates;
use hotstuff_rs::types::validator_set::ValidatorSet;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock};

type MailboxMap = HashMap<VerifyingKey, VecDeque<(VerifyingKey, Message)>>;

/// Shared in-memory hub routing messages between replicas, supporting deterministic
/// partition and fault simulation (PRD R06).
#[derive(Default, Clone)]
pub struct ChannelNetworkHub {
    mailboxes: Arc<Mutex<MailboxMap>>,
    isolated_peers: Arc<Mutex<HashSet<VerifyingKey>>>,
    blocked_links: Arc<Mutex<HashSet<(VerifyingKey, VerifyingKey)>>>,
}

impl ChannelNetworkHub {
    pub fn new() -> Self {
        Self {
            mailboxes: Arc::new(Mutex::new(HashMap::new())),
            isolated_peers: Arc::new(Mutex::new(HashSet::new())),
            blocked_links: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn register(&self, key: VerifyingKey) {
        let mut lock = self.mailboxes.lock().expect("hub lock poisoned");
        lock.entry(key).or_default();
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

    pub fn send_to(&self, from: VerifyingKey, to: &VerifyingKey, msg: Message) {
        if !self.is_delivery_allowed(&from, to) {
            return;
        }
        let mut lock = self.mailboxes.lock().expect("hub lock poisoned");
        if let Some(queue) = lock.get_mut(to) {
            queue.push_back((from, msg));
        }
    }

    pub fn broadcast(&self, from: VerifyingKey, recipients: &[VerifyingKey], msg: Message) {
        for peer in recipients {
            if self.is_delivery_allowed(&from, peer) {
                let mut lock = self.mailboxes.lock().expect("hub lock poisoned");
                if let Some(queue) = lock.get_mut(peer) {
                    queue.push_back((from, msg.clone()));
                }
            }
        }
    }

    pub fn pop(&self, key: &VerifyingKey) -> Option<(VerifyingKey, Message)> {
        let mut lock = self.mailboxes.lock().expect("hub lock poisoned");
        lock.get_mut(key).and_then(|queue| queue.pop_front())
    }
}

/// Node network adapter implementing `hotstuff_rs::networking::network::Network`.
#[derive(Clone)]
pub struct ChannelNetwork {
    my_key: VerifyingKey,
    hub: ChannelNetworkHub,
    validator_set: Arc<RwLock<Option<ValidatorSet>>>,
}

impl ChannelNetwork {
    pub fn new(my_key: VerifyingKey, hub: ChannelNetworkHub) -> Self {
        hub.register(my_key);
        Self {
            my_key,
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
            self.hub.broadcast(self.my_key, &peers, message);
        }
    }

    fn send(&mut self, peer: VerifyingKey, message: Message) {
        self.hub.send_to(self.my_key, &peer, message);
    }

    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        self.hub.pop(&self.my_key)
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
}

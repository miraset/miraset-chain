//! PoCC settlement parties for tests (TODO 8.5.1): a genesis coordinator and governance key, and
//! a worker whose owner key registers a bond and signs receipts.
#![allow(dead_code)]

use ed25519_dalek::SigningKey;
use miraset_chain::core::types::{
    AllowanceAction, ComputeReceipt, MIN_WORKER_BOND, ProtocolParams, Transaction, WorkerBondAction,
};

pub const PRICE_PER_TOKEN: u64 = 2;
pub const EPOCH_REWARD_BUDGET: u64 = 6_944;
/// Short epochs so tests can close one; production default is 3600.
pub const EPOCH_SECONDS: u64 = 1;
/// Fee on fixture transactions.
pub const FEE: u64 = 1;

pub fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

pub fn address(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

/// Coordinator (seed 200), governance (201) and one worker whose owner is seed 202.
pub struct Pocc {
    pub coordinator: SigningKey,
    pub governance: SigningKey,
    pub worker_owner: SigningKey,
    pub worker_id: String,
}

impl Default for Pocc {
    fn default() -> Self {
        Self::new("worker-h100", 202)
    }
}

impl Pocc {
    pub fn new(worker_id: &str, owner_seed: u8) -> Self {
        Self {
            coordinator: key(200),
            governance: key(201),
            worker_owner: key(owner_seed),
            worker_id: worker_id.to_string(),
        }
    }

    pub fn params(&self) -> ProtocolParams {
        ProtocolParams {
            coordinators: vec![address(&self.coordinator)],
            governance: vec![address(&self.governance)],
            price_per_token: PRICE_PER_TOKEN,
            epoch_reward_budget: EPOCH_REWARD_BUDGET,
            epoch_seconds: EPOCH_SECONDS,
        }
    }

    pub fn owner_address(&self) -> String {
        address(&self.worker_owner)
    }

    pub fn governance_address(&self) -> String {
        address(&self.governance)
    }

    /// Genesis accounts the parties need: the worker owner (bond + fees) and governance (fees).
    pub fn genesis_accounts(&self) -> Vec<(String, u64)> {
        vec![
            (self.owner_address(), 10_000),
            (self.governance_address(), 1_000),
        ]
    }

    /// The owner registers the worker with the minimum bond.
    pub fn register(&self, nonce: u64) -> Transaction {
        let action = WorkerBondAction::Register {
            worker_id: self.worker_id.clone(),
            bond_amount: MIN_WORKER_BOND,
            vram_gib: 80,
            endpoint: "127.0.0.1:8080".into(),
        };
        let mut tx =
            Transaction::new_worker_bond(self.owner_address(), nonce, FEE, action).unwrap();
        tx.sign_with(&self.worker_owner);
        tx
    }

    /// A receipt for `tokens` output tokens paid by `client`, signed by the coordinator and the
    /// worker, priced at `PRICE_PER_TOKEN`.
    pub fn receipt(&self, task_id: &str, client: &str, tokens: u64) -> ComputeReceipt {
        let cost = tokens * PRICE_PER_TOKEN;
        let mut receipt = ComputeReceipt::new(
            task_id,
            client,
            &self.worker_id,
            "coordinator-1",
            "model-hash",
            "receipt-anchor",
            tokens,
            cost,
            cost,
        );
        self.sign(&mut receipt);
        receipt
    }

    /// (Re-)sign a receipt as coordinator and worker.
    pub fn sign(&self, receipt: &mut ComputeReceipt) {
        receipt.sign_with(&self.coordinator);
        receipt.sign_as_worker(&self.worker_owner);
    }

    /// A governance transaction to a system recipient.
    pub fn governance_tx(&self, nonce: u64, recipient: &str, payload: Vec<u8>) -> Transaction {
        let mut tx = Transaction::new(self.governance_address(), nonce, recipient, 0, FEE);
        tx.payload = payload;
        tx.sign_with(&self.governance);
        tx
    }

    /// `client` allows the fixture coordinator to charge it up to `amount` (TODO 8.5.1).
    pub fn allow(&self, client: &SigningKey, nonce: u64, amount: u64) -> Transaction {
        let action = AllowanceAction {
            coordinator: address(&self.coordinator),
            amount,
        };
        let mut tx = Transaction::new(address(client), nonce, "system:allowance", 0, FEE);
        tx.payload = action.to_bytes().unwrap();
        tx.sign_with(client);
        tx
    }

    /// Governance closes `epoch_id`.
    pub fn close_epoch(&self, nonce: u64, epoch_id: u64) -> Transaction {
        self.governance_tx(
            nonce,
            "system:epoch_manager",
            epoch_id.to_be_bytes().to_vec(),
        )
    }
}

/// Wait until the current epoch has lasted `EPOCH_SECONDS` of committed block time.
pub fn wait_epoch_over<K, N>(node: &miraset_chain::node::MirasetNode<K, N>)
where
    K: hotstuff_rs::block_tree::pluggables::KVStore,
    N: hotstuff_rs::networking::network::Network + 'static,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !node.epoch_may_close() {
        assert!(
            std::time::Instant::now() < deadline,
            "epoch never became closable"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

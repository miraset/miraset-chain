//! Workload items: run-scoped throwaway keys, signed transfers and coordinator-signed receipts.
//!
//! Every key here is derived from a random run ID and used only for one load run on a devnet;
//! none of them is a validator or faucet key.

use crate::core::types::{ComputeReceipt, Transaction};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

/// Fee on every generated transfer (the devnet has no minimum fee; a constant keeps ordering
/// by sender and nonce, as in `Mempool::insert_tx`).
pub const TRANSFER_FEE: u64 = 10;
pub const TRANSFER_AMOUNT: u64 = 1;
/// Per-receipt client debit, worker reward and token count.
pub const RECEIPT_CLIENT_COST: u64 = 1;
pub const RECEIPT_WORKER_REWARD: u64 = 1;
pub const RECEIPT_TOKENS: u64 = 100;

const DOMAIN: &[u8] = b"miraset-loadgen-v1";

/// Deterministic throwaway key for `(run_id, role, index)`.
pub fn derive_key(run_id: u64, role: &str, index: usize) -> SigningKey {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(run_id.to_be_bytes());
    hasher.update((role.len() as u32).to_be_bytes());
    hasher.update(role.as_bytes());
    hasher.update((index as u64).to_be_bytes());
    SigningKey::from_bytes(&hasher.finalize().into())
}

pub fn address(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

pub fn signed_transfer(
    key: &SigningKey,
    nonce: u64,
    recipient: &str,
    amount: u64,
    fee: u64,
) -> Transaction {
    let mut tx = Transaction::new(address(key), nonce, recipient, amount, fee);
    tx.sign_with(key);
    tx
}

/// Task ID of receipt `seq` of stream `stream`. Zero-padded so a node's mempool, which orders
/// receipts by task ID, takes each stream's receipts in submission order.
pub fn task_id(run_id: u64, stream: usize, seq: u64) -> String {
    format!("lg-{run_id:016x}-{stream:03}-{seq:012}")
}

pub fn signed_receipt(
    coordinator: &SigningKey,
    task_id: String,
    client_address: &str,
    worker_id: &str,
) -> ComputeReceipt {
    let mut receipt = ComputeReceipt::new(
        task_id,
        client_address,
        worker_id,
        "loadgen-coordinator",
        "loadgen-model",
        "loadgen-proof",
        RECEIPT_TOKENS,
        RECEIPT_CLIENT_COST,
        RECEIPT_WORKER_REWARD,
    );
    receipt.sign_with(coordinator);
    receipt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derived_keys_depend_on_run_role_and_index() {
        let base = address(&derive_key(1, "sender", 0));
        assert_eq!(base, address(&derive_key(1, "sender", 0)));
        assert_ne!(base, address(&derive_key(2, "sender", 0)));
        assert_ne!(base, address(&derive_key(1, "coordinator", 0)));
        assert_ne!(base, address(&derive_key(1, "sender", 1)));
    }

    #[test]
    fn test_task_ids_sort_in_submission_order() {
        let ids: Vec<String> = [9, 10, 100, 1_000_000]
            .iter()
            .map(|s| task_id(7, 2, *s))
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert_ne!(task_id(7, 1, 5), task_id(7, 2, 5));
    }

    #[test]
    fn test_generated_items_carry_valid_signatures() {
        let sender = derive_key(3, "sender", 0);
        let tx = signed_transfer(&sender, 4, "recipient", TRANSFER_AMOUNT, TRANSFER_FEE);
        assert!(tx.verify_signature().is_ok());
        assert_eq!(tx.nonce, 4);

        let coordinator = derive_key(3, "coordinator", 0);
        let receipt = signed_receipt(&coordinator, task_id(3, 0, 1), &address(&sender), "w");
        assert!(receipt.verify_signature().is_ok());
        assert!(receipt.client_confirmed);
    }
}

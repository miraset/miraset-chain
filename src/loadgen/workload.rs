//! Workload items: run-scoped throwaway keys, signed transfers and receipts co-signed by the
//! devnet's genesis coordinator and a run-scoped worker.
//!
//! Every derived key here comes from a random run ID and is used only for one load run on a
//! devnet; none of them is a validator, faucet or coordinator key.

use crate::core::types::{ComputeReceipt, Transaction};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

/// Fee on every generated transfer (the devnet has no minimum fee; a constant keeps ordering
/// by sender and nonce, as in `Mempool::insert_tx`).
pub const TRANSFER_FEE: u64 = 10;
pub const TRANSFER_AMOUNT: u64 = 1;
/// Output tokens per receipt; the client pays `RECEIPT_TOKENS × price_per_token` (TODO 8.5.1).
pub const RECEIPT_TOKENS: u64 = 5;

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

/// Worker registered for receipt stream `stream` of run `run_id`.
pub fn worker_id(run_id: u64, stream: usize) -> String {
    format!("lg-{run_id:016x}-worker-{stream:03}")
}

/// A receipt for `RECEIPT_TOKENS` paid by `client_address`, priced at `price_per_token`, signed
/// by the coordinator and by the worker's owner key.
pub fn signed_receipt(
    coordinator: &SigningKey,
    worker_owner: &SigningKey,
    task_id: String,
    client_address: &str,
    worker_id: &str,
    price_per_token: u64,
) -> ComputeReceipt {
    let cost = RECEIPT_TOKENS.saturating_mul(price_per_token);
    let mut receipt = ComputeReceipt::new(
        task_id,
        client_address,
        worker_id,
        "loadgen-coordinator",
        "loadgen-model",
        "loadgen-receipt-anchor",
        RECEIPT_TOKENS,
        cost,
        cost,
    );
    receipt.sign_with(coordinator);
    receipt.sign_as_worker(worker_owner);
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
        let owner = derive_key(3, "worker", 0);
        let receipt = signed_receipt(
            &coordinator,
            &owner,
            task_id(3, 0, 1),
            &address(&sender),
            "w",
            2,
        );
        assert!(receipt.verify_signature().is_ok());
        assert!(receipt.verify_worker_signature(&address(&owner)).is_ok());
        assert_eq!(receipt.client_cost, RECEIPT_TOKENS * 2);
        assert_eq!(receipt.worker_reward, RECEIPT_TOKENS * 2);
        assert!(receipt.client_confirmed);
    }
}

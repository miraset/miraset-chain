use crate::core::types::{ComputeReceipt, Transaction};
use std::sync::{Arc, Mutex};

/// In-memory mempool for pending transactions and compute receipts.
#[derive(Clone, Debug, Default)]
pub struct Mempool {
    pending_txs: Arc<Mutex<Vec<Transaction>>>,
    pending_receipts: Arc<Mutex<Vec<ComputeReceipt>>>,
}

impl Mempool {
    pub fn new() -> Self {
        Self {
            pending_txs: Arc::new(Mutex::new(Vec::new())),
            pending_receipts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Insert transaction into mempool with basic validation.
    pub fn insert_tx(&self, tx: Transaction) -> bool {
        if tx.sender.is_empty() || tx.recipient.is_empty() {
            return false;
        }
        let mut lock = self.pending_txs.lock().expect("mempool lock poisoned");
        // Simple deduplication by sender & nonce
        if lock
            .iter()
            .any(|t| t.sender == tx.sender && t.nonce == tx.nonce)
        {
            return false;
        }
        lock.push(tx);
        // Sort deterministically: higher fee first across senders, with ties broken by (sender, nonce)
        lock.sort_by(|a, b| {
            std::cmp::Reverse(a.fee)
                .cmp(&std::cmp::Reverse(b.fee))
                .then_with(|| a.sender.cmp(&b.sender))
                .then_with(|| a.nonce.cmp(&b.nonce))
        });
        true
    }

    /// Insert verified compute receipt into mempool.
    pub fn insert_receipt(&self, receipt: ComputeReceipt) -> bool {
        if receipt.task_id.is_empty() || receipt.worker_id.is_empty() {
            return false;
        }
        let mut lock = self
            .pending_receipts
            .lock()
            .expect("receipts lock poisoned");
        if lock.iter().any(|r| r.task_id == receipt.task_id) {
            return false;
        }
        lock.push(receipt);
        // Sort deterministically by task_id so receipt execution order is canonical
        lock.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        true
    }

    /// Drain transactions and receipts for inclusion in a new block.
    pub fn drain_for_block(
        &self,
        max_txs: usize,
        max_receipts: usize,
    ) -> (Vec<Transaction>, Vec<ComputeReceipt>) {
        let mut tx_lock = self.pending_txs.lock().expect("mempool lock poisoned");
        let tx_count = max_txs.min(tx_lock.len());
        let mut txs: Vec<Transaction> = tx_lock.drain(0..tx_count).collect();
        // Ensure multiple transactions from the same sender are ordered by ascending nonce
        // while preserving fee-descending order across distinct senders.
        let mut by_sender: std::collections::BTreeMap<String, Vec<Transaction>> =
            std::collections::BTreeMap::new();
        let mut sender_order: Vec<String> = Vec::new();
        for tx in txs.drain(..) {
            if !by_sender.contains_key(&tx.sender) {
                sender_order.push(tx.sender.clone());
            }
            by_sender.entry(tx.sender.clone()).or_default().push(tx);
        }
        for sender in sender_order {
            if let Some(mut sender_txs) = by_sender.remove(&sender) {
                sender_txs.sort_by_key(|t| t.nonce);
                txs.extend(sender_txs);
            }
        }

        let mut receipt_lock = self
            .pending_receipts
            .lock()
            .expect("receipts lock poisoned");
        let receipt_count = max_receipts.min(receipt_lock.len());
        let receipts: Vec<ComputeReceipt> = receipt_lock.drain(0..receipt_count).collect();

        (txs, receipts)
    }

    pub fn tx_count(&self) -> usize {
        self.pending_txs
            .lock()
            .expect("mempool lock poisoned")
            .len()
    }

    pub fn receipt_count(&self) -> usize {
        self.pending_receipts
            .lock()
            .expect("receipts lock poisoned")
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mempool_tx_prioritization_and_deduplication() {
        let pool = Mempool::new();

        let tx1 = Transaction::new("alice", 1, "bob", 100, 10);
        let tx2 = Transaction::new("charlie", 1, "dave", 200, 50); // higher fee
        let tx3 = Transaction::new("alice", 1, "bob", 100, 10); // duplicate nonce for alice

        assert!(pool.insert_tx(tx1.clone()));
        assert!(pool.insert_tx(tx2.clone()));
        assert!(!pool.insert_tx(tx3)); // rejected duplicate

        assert_eq!(pool.tx_count(), 2);

        // tx2 should be prioritized first due to higher fee
        let (drained, _) = pool.drain_for_block(10, 0);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].fee, 50);
        assert_eq!(drained[1].fee, 10);
        assert_eq!(pool.tx_count(), 0);
    }

    #[test]
    fn test_mempool_receipt_handling() {
        let pool = Mempool::new();
        let r1 = ComputeReceipt::new("t1", "client_alice", "w1", "coord", "m1", "p1", 50, 10, 8);
        assert!(pool.insert_receipt(r1.clone()));
        assert!(!pool.insert_receipt(r1)); // duplicate task

        let (_, drained) = pool.drain_for_block(0, 5);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].task_id, "t1");
    }
}

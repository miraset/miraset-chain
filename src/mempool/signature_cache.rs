//! Per-node cache of transactions whose signature verified (TODO 8.5.3, stress test 3 finding
//! 29).
//!
//! A transaction used to be verified up to three times on one node: at RPC admission, in the
//! leader's pre-pass when it built a block, and again when the node validated that block. The
//! cache records each transaction whose signature verified, keyed by the SHA-256 of its
//! canonical bytes (signature included), so later checks on the same node skip it.
//!
//! Only verified transactions are ever recorded, and the key covers every byte the signature
//! check reads, so a cache hit can only skip a check that would pass: block validity does not
//! change. The cache is bounded; the oldest entries leave first, which only costs a re-check.

use crate::core::types::Transaction;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

/// Identity of a transaction including its signature: SHA-256 of its canonical bytes.
pub type TxDigest = [u8; 32];

/// Entries kept: several times the mempool (20 000 transactions) plus the blocks in flight.
/// About 100 000 × 32 bytes plus set overhead, ≈ 8 MB.
pub const SIGNATURE_CACHE_CAPACITY: usize = 100_000;

#[derive(Debug, Default)]
struct Entries {
    digests: HashSet<TxDigest>,
    /// Insertion order, for eviction.
    order: VecDeque<TxDigest>,
}

/// Shared, bounded set of verified transaction digests. Cloning shares the same cache.
#[derive(Clone, Debug)]
pub struct SignatureCache {
    entries: Arc<Mutex<Entries>>,
    capacity: usize,
}

impl Default for SignatureCache {
    fn default() -> Self {
        Self::with_capacity(SIGNATURE_CACHE_CAPACITY)
    }
}

impl SignatureCache {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Arc::default(),
            capacity: capacity.max(1),
        }
    }

    /// Record `tx`, whose signature the caller has just verified.
    pub fn record_verified(&self, tx: &Transaction) {
        if let Some(digest) = tx.canonical_digest() {
            self.insert_verified([digest]);
        }
    }

    /// Record digests of transactions whose signature the caller has just verified.
    pub fn insert_verified(&self, verified: impl IntoIterator<Item = TxDigest>) {
        let mut entries = self.lock();
        for digest in verified {
            if entries.digests.insert(digest) {
                entries.order.push_back(digest);
            }
        }
        while entries.digests.len() > self.capacity {
            let Some(oldest) = entries.order.pop_front() else {
                break;
            };
            entries.digests.remove(&oldest);
        }
    }

    /// For each digest, whether it is recorded (`None`, no canonical encoding, is never).
    pub fn contains_each(&self, digests: &[Option<TxDigest>]) -> Vec<bool> {
        let entries = self.lock();
        digests
            .iter()
            .map(|digest| digest.is_some_and(|d| entries.digests.contains(&d)))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.lock().digests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Entries> {
        // Every mutation completes before the guard drops, so a poisoned lock is consistent.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(nonce: u64) -> Transaction {
        Transaction::new("alice", nonce, "bob", 1, 1)
    }

    #[test]
    fn test_recorded_transactions_hit_and_others_miss() {
        let cache = SignatureCache::default();
        cache.record_verified(&tx(0));
        let digests = [tx(0).canonical_digest(), tx(1).canonical_digest(), None];
        assert_eq!(cache.contains_each(&digests), [true, false, false]);
        // A changed copy of a recorded transaction is another digest.
        let mut changed = tx(0);
        changed.amount += 1;
        assert_eq!(cache.contains_each(&[changed.canonical_digest()]), [false]);
    }

    #[test]
    fn test_oldest_entries_leave_first_at_capacity() {
        let cache = SignatureCache::with_capacity(2);
        for nonce in 0..3 {
            cache.record_verified(&tx(nonce));
        }
        assert_eq!(cache.len(), 2);
        let digests: Vec<_> = (0..3).map(|n| tx(n).canonical_digest()).collect();
        assert_eq!(cache.contains_each(&digests), [false, true, true]);
        // Re-recording an entry does not duplicate it in the eviction order.
        cache.record_verified(&tx(2));
        assert_eq!(cache.len(), 2);
    }
}

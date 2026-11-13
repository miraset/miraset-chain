//! Parallel transaction signature checks (TODO 8.5.3 stress test, finding 21).
//!
//! The stress test showed per-transaction CPU on one thread as the throughput ceiling: the
//! leader and every replica verified each signature serially inside block execution. Signature
//! checks are independent of chain state, so they run here across threads before execution,
//! and execution then skips the transactions found valid.
//!
//! Validity does not change: a transaction with a bad signature still fails exactly where it
//! failed before (validators refuse the block; the leader evicts the candidate), because only
//! transactions whose signature verified are ever exempted from the inline check.

use crate::core::types::Transaction;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;

/// Most threads one batch uses. Five validators share the measurement host's 16 cores, and the
/// node has other work (RPC admission, storage); more threads past this gained little.
const MAX_THREADS: usize = 8;
/// Smallest batch handed to a thread; below this, spawning costs more than it saves.
const MIN_PER_THREAD: usize = 256;

/// Identity of a transaction including its signature: SHA-256 of its canonical bytes.
pub(crate) type TxDigest = [u8; 32];

pub(super) fn digest(tx: &Transaction) -> Option<TxDigest> {
    tx.to_canonical_bytes()
        .ok()
        .map(|bytes| Sha256::digest(&bytes).into())
}

/// How block execution treats transaction signatures.
#[derive(Clone, Debug, Default)]
pub(super) enum SignatureCheck {
    /// Verify every signature inline (the behaviour without a pre-pass).
    #[default]
    Inline,
    /// Every transaction this execution will see was verified (validation of a whole payload).
    AllVerified,
    /// Transactions with these digests were verified; any other is verified inline.
    Verified(Arc<HashSet<TxDigest>>),
}

impl SignatureCheck {
    /// Whether `tx` still needs its signature checked inline.
    pub(super) fn needs_check(&self, tx: &Transaction) -> bool {
        match self {
            Self::Inline => true,
            Self::AllVerified => false,
            Self::Verified(set) => digest(tx).is_none_or(|d| !set.contains(&d)),
        }
    }
}

/// Whether every transaction's signature verifies, checked across threads.
pub(super) fn all_valid(txs: &[Transaction]) -> bool {
    parallel_map(txs, |tx| tx.verify_signature().is_ok())
        .into_iter()
        .all(|ok| ok)
}

/// Leader-side cache of pooled transactions whose signature verified, so each pending
/// transaction is verified once, not once per block it waits through.
#[derive(Debug, Default)]
pub(super) struct VerifiedPool {
    /// Shared with the block being built; updated in place once that block has dropped it.
    verified: Arc<HashSet<TxDigest>>,
}

impl VerifiedPool {
    /// Verify the not-yet-verified transactions of `pending` across threads, forget digests no
    /// longer pending, and return the verified set for this block.
    pub(super) fn refresh(&mut self, pending: &[Transaction]) -> Arc<HashSet<TxDigest>> {
        let digests = parallel_map(pending, digest);
        let fresh: Vec<(TxDigest, &Transaction)> = digests
            .iter()
            .zip(pending)
            .filter_map(|(d, tx)| d.map(|d| (d, tx)))
            .filter(|(d, _)| !self.verified.contains(d))
            .collect();
        let results = parallel_map(&fresh, |(_, tx)| tx.verify_signature().is_ok());
        let pending_set: HashSet<TxDigest> = digests.into_iter().flatten().collect();
        // The previous block's execution has finished with its handle, so this does not copy.
        let verified = Arc::make_mut(&mut self.verified);
        verified.retain(|d| pending_set.contains(d));
        verified.extend(
            fresh
                .iter()
                .zip(results)
                .filter(|(_, ok)| *ok)
                .map(|((d, _), _)| *d),
        );
        Arc::clone(&self.verified)
    }
}

/// `items.iter().map(f)` across up to [`MAX_THREADS`] scoped threads, in order.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(MAX_THREADS);
    let chunk = items.len().div_ceil(threads).max(MIN_PER_THREAD);
    if items.len() <= chunk {
        return items.iter().map(&f).collect();
    }
    std::thread::scope(|scope| {
        let workers: Vec<_> = items
            .chunks(chunk)
            .map(|part| scope.spawn(|| part.iter().map(&f).collect::<Vec<R>>()))
            .collect();
        workers
            .into_iter()
            // A worker's panic is re-raised here, as it would have been inline.
            .flat_map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|p| std::panic::resume_unwind(p))
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn signed(seed: u8, nonce: u64) -> Transaction {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let mut tx = Transaction::new(
            hex::encode(key.verifying_key().to_bytes()),
            nonce,
            "bob",
            1,
            1,
        );
        tx.sign_with(&key);
        tx
    }

    #[test]
    fn test_parallel_map_keeps_order_across_threads() {
        let items: Vec<u32> = (0..5_000).collect();
        assert_eq!(
            parallel_map(&items, |x| x * 2),
            items.iter().map(|x| x * 2).collect::<Vec<_>>()
        );
        assert!(parallel_map(&[] as &[u32], |x| *x).is_empty());
    }

    #[test]
    fn test_parallel_map_at_the_thread_split_boundary() {
        for len in [
            MIN_PER_THREAD - 1,
            MIN_PER_THREAD,
            MIN_PER_THREAD + 1,
            2 * MIN_PER_THREAD + 1,
        ] {
            let items: Vec<usize> = (0..len).collect();
            assert_eq!(parallel_map(&items, |x| *x), items, "len {len}");
        }
    }

    /// Cost of the leader's per-block pre-pass on a full pool (run with `--release --ignored`).
    #[test]
    #[ignore = "timing; run in release"]
    fn bench_refresh_on_a_full_pool() {
        let txs: Vec<Transaction> = (0..20_000u64).map(|n| signed((n % 200) as u8, n)).collect();
        let mut pool = VerifiedPool::default();
        let start = std::time::Instant::now();
        let set = pool.refresh(&txs);
        eprintln!("first refresh (verifies 20 000): {:?}", start.elapsed());
        drop(set);
        let start = std::time::Instant::now();
        let set = pool.refresh(&txs);
        eprintln!("next refresh (all cached): {:?}", start.elapsed());
        assert_eq!(set.len(), 20_000);
        let start = std::time::Instant::now();
        assert!(all_valid(&txs));
        eprintln!("validator pass over 20 000: {:?}", start.elapsed());
    }

    #[test]
    fn test_one_bad_signature_anywhere_fails_the_batch() {
        let mut txs: Vec<Transaction> = (0..1_000).map(|n| signed(3, n)).collect();
        assert!(all_valid(&txs));
        txs[777].amount += 1; // signature no longer covers the contents
        assert!(!all_valid(&txs));
    }

    #[test]
    fn test_verified_pool_exempts_only_transactions_that_verified() {
        let good = signed(4, 0);
        let mut forged = signed(4, 1);
        forged.amount += 1;
        let mut pool = VerifiedPool::default();
        let set = pool.refresh(&[good.clone(), forged.clone()]);
        let check = SignatureCheck::Verified(set);
        assert!(!check.needs_check(&good));
        assert!(
            check.needs_check(&forged),
            "a bad signature is never exempted"
        );
        // A tampered copy of a verified transaction has another digest.
        let mut tampered = good.clone();
        tampered.amount += 1;
        assert!(check.needs_check(&tampered));
        // Digests leave the cache once their transaction is no longer pending.
        let set = pool.refresh(&[]);
        assert!(set.is_empty());
    }
}

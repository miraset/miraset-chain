//! Parallel transaction signature checks (TODO 8.5.3 stress test, finding 21).
//!
//! The stress test showed per-transaction CPU on one thread as the throughput ceiling: the
//! leader and every replica verified each signature serially inside block execution. Signature
//! checks are independent of chain state, so they run here across threads before execution,
//! and execution then skips the transactions found valid.
//!
//! Both passes consult the node's verified-signature cache first (`mempool::SignatureCache`,
//! filled by RPC admission and by these passes), so a transaction is verified once per node:
//! the leader skips what admission verified, and a validator skips what it already saw.
//!
//! Validity does not change: a transaction with a bad signature still fails exactly where it
//! failed before (validators refuse the block; the leader evicts the candidate), because only
//! transactions whose signature verified are ever exempted from the inline check.

use crate::core::types::Transaction;
use crate::mempool::{SignatureCache, TxDigest};
use std::collections::HashSet;
use std::sync::Arc;

/// Most threads one batch uses. Five validators share the measurement host's 16 cores, and the
/// node has other work (RPC admission, storage); more threads past this gained little.
const MAX_THREADS: usize = 8;
/// Smallest batch handed to a thread; below this, spawning costs more than it saves.
const MIN_PER_THREAD: usize = 256;

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
            Self::Verified(set) => tx.canonical_digest().is_none_or(|d| !set.contains(&d)),
        }
    }
}

/// Whether every transaction's signature verifies. Transactions in `cache` are skipped; the
/// rest are verified across threads and, if all pass, recorded in `cache`.
pub(super) fn all_valid(cache: &SignatureCache, txs: &[Transaction]) -> bool {
    let digests = parallel_map(txs, Transaction::canonical_digest);
    let cached = cache.contains_each(&digests);
    let unchecked: Vec<(Option<TxDigest>, &Transaction)> = digests
        .iter()
        .zip(&cached)
        .zip(txs)
        .filter(|((_, hit), _)| !**hit)
        .map(|((digest, _), tx)| (*digest, tx))
        .collect();
    let results = parallel_map(&unchecked, |(_, tx)| tx.verify_signature().is_ok());
    let all_ok = results.iter().all(|ok| *ok);
    // Each transaction's signature stands on its own: record the ones that verified even when
    // another fails, so a proposal re-sent with one forgery does not cost the whole pass again.
    cache.insert_verified(
        unchecked
            .iter()
            .zip(&results)
            .filter_map(|((digest, _), ok)| digest.filter(|_| *ok)),
    );
    all_ok
}

/// Digests of the `pending` transactions whose signature verifies: cached ones directly, the
/// rest verified across threads (and the valid ones recorded in `cache`). The leader builds a
/// block against this set; anything outside it is checked inline.
pub(super) fn verified_pending(
    cache: &SignatureCache,
    pending: &[Transaction],
) -> Arc<HashSet<TxDigest>> {
    let digests = parallel_map(pending, Transaction::canonical_digest);
    let cached = cache.contains_each(&digests);
    let mut verified: HashSet<TxDigest> = digests
        .iter()
        .zip(&cached)
        .filter_map(|(digest, hit)| digest.filter(|_| *hit))
        .collect();
    let unchecked: Vec<(TxDigest, &Transaction)> = digests
        .iter()
        .zip(&cached)
        .zip(pending)
        .filter_map(|((digest, hit), tx)| digest.filter(|_| !*hit).map(|d| (d, tx)))
        .collect();
    let results = parallel_map(&unchecked, |(_, tx)| tx.verify_signature().is_ok());
    let newly: Vec<TxDigest> = unchecked
        .iter()
        .zip(results)
        .filter_map(|((digest, _), ok)| ok.then_some(*digest))
        .collect();
    cache.insert_verified(newly.iter().copied());
    verified.extend(newly);
    Arc::new(verified)
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

    /// Cost of the signature passes on a full pool, uncached and cached (run with `--release --ignored`).
    #[test]
    #[ignore = "timing; run in release"]
    fn bench_signature_passes_on_a_full_pool() {
        let txs: Vec<Transaction> = (0..20_000u64).map(|n| signed((n % 200) as u8, n)).collect();
        let start = std::time::Instant::now();
        assert!(all_valid(&SignatureCache::default(), &txs));
        eprintln!(
            "validator pass over 20 000, uncached: {:?}",
            start.elapsed()
        );
        let cache = SignatureCache::default();
        let start = std::time::Instant::now();
        let set = verified_pending(&cache, &txs);
        eprintln!("leader pre-pass, uncached: {:?}", start.elapsed());
        assert_eq!(set.len(), 20_000);
        let start = std::time::Instant::now();
        let _ = verified_pending(&cache, &txs);
        eprintln!("leader pre-pass, all cached: {:?}", start.elapsed());
        let start = std::time::Instant::now();
        assert!(all_valid(&cache, &txs));
        eprintln!(
            "validator pass over 20 000, all cached: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn test_one_bad_signature_anywhere_fails_the_batch() {
        let cache = SignatureCache::default();
        let mut txs: Vec<Transaction> = (0..1_000).map(|n| signed(3, n)).collect();
        txs[777].amount += 1; // signature no longer covers the contents
        assert!(!all_valid(&cache, &txs));
        assert_eq!(
            cache.len(),
            999,
            "the ones that verified are recorded, the forgery is not"
        );
        assert!(
            !all_valid(&cache, &txs),
            "the forgery still fails when the rest is cached"
        );
        txs[777] = signed(3, 777);
        assert!(all_valid(&cache, &txs));
        assert_eq!(cache.len(), 1_000);
    }

    #[test]
    fn test_cached_transactions_are_not_verified_again_but_forgeries_still_fail() {
        let cache = SignatureCache::default();
        let good = signed(5, 0);
        cache.record_verified(&good);
        let mut forged = signed(5, 1);
        forged.amount += 1;
        assert!(all_valid(&cache, std::slice::from_ref(&good)));
        assert!(!all_valid(&cache, &[good.clone(), forged.clone()]));
        // A tampered copy of a cached transaction is not a hit.
        let mut tampered = good.clone();
        tampered.amount += 1;
        assert!(!all_valid(&cache, &[tampered]));
    }

    #[test]
    fn test_verified_pending_exempts_only_transactions_that_verified() {
        let cache = SignatureCache::default();
        let good = signed(4, 0);
        let admitted = signed(4, 2);
        cache.record_verified(&admitted); // as RPC admission does
        let mut forged = signed(4, 1);
        forged.amount += 1;
        let set = verified_pending(&cache, &[good.clone(), forged.clone(), admitted.clone()]);
        let check = SignatureCheck::Verified(set);
        assert!(!check.needs_check(&good));
        assert!(!check.needs_check(&admitted));
        assert!(
            check.needs_check(&forged),
            "a bad signature is never exempted"
        );
        assert_eq!(
            cache.len(),
            2,
            "the newly verified one is recorded, the forgery is not"
        );
    }
}

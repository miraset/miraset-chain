//! Bounded in-memory mempool for pending transactions and compute receipts (TODO 8.4.3).
//!
//! The node checks every item against committed state before inserting it (see
//! `node::admission`); the pool keeps its own invariants on top:
//!
//! - **Contiguous sender chains.** A sender's pending transactions form one run of consecutive
//!   nonces. A transaction that would leave a gap is refused, and removing one (expiry,
//!   eviction) also removes the sender's later nonces. Nothing pending can wait on a nonce that
//!   is not pending, so it cannot occupy block space it can never use.
//! - **Selection by chain head.** Block production walks sender chains in order of their next
//!   transaction's fee and takes each chain in nonce order (`select_txs`); a high fee on a later
//!   nonce cannot jump ahead of the nonce it depends on.
//! - **Bounds.** Total transactions and receipts, transactions per sender, receipts per client,
//!   a minimum fee, and a time-to-live. A full pool evicts the lowest-fee chain tail for a
//!   better-paying transaction instead of locking new senders out.
//! - **Receipts in arrival order**, so a chosen task id cannot move a receipt to the front.
//!
//! Inserts, removals and expiry are logarithmic or amortised constant; nothing is re-sorted.

use crate::core::types::{ComputeReceipt, Transaction};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Expiry records of removed transactions tolerated before the queue is rebuilt.
const EXPIRY_SLACK: usize = 1024;

/// Bounds of one mempool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MempoolLimits {
    /// Pending transactions across all senders.
    pub max_txs: usize,
    /// Pending compute receipts across all clients.
    pub max_receipts: usize,
    /// Pending transactions of one sender (also how far its nonces may run ahead).
    pub max_txs_per_sender: usize,
    /// Pending receipts naming one client account.
    pub max_receipts_per_client: usize,
    /// Smallest fee a transaction must offer.
    pub min_fee: u64,
    /// Pending items older than this are dropped.
    pub ttl: Duration,
}

impl Default for MempoolLimits {
    fn default() -> Self {
        Self {
            max_txs: 20_000,
            max_receipts: 5_000,
            max_txs_per_sender: 256,
            max_receipts_per_client: 256,
            min_fee: 1,
            ttl: Duration::from_secs(15 * 60),
        }
    }
}

/// Why the pool refused an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InsertError {
    #[error("already exists in mempool")]
    Duplicate,
    #[error("mempool is full")]
    Full,
    #[error("too many pending items from this sender or client")]
    SenderLimit,
    #[error("missing sender, recipient, task or worker identifier")]
    Malformed,
    #[error("fee below the node minimum")]
    FeeTooLow,
    #[error("nonce does not follow the sender's pending transactions")]
    NonceGap,
}

/// What block production decided about one offered transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxVerdict {
    /// Included in the block; offer the sender's next nonce.
    Included,
    /// Already superseded (nonce below the expected one); offer the sender's next nonce.
    Skip,
    /// Nonce does not match; offer nothing more from this sender for this block.
    DropSender,
    /// Failed execution: remove it and the sender's later nonces from the pool, offer nothing
    /// more from this sender. Admission cannot foresee every execution failure (a second
    /// transfer the first one left unaffordable, a malformed system payload); without
    /// eviction such a chain would be offered, and fail, in every block until it expired.
    Evict,
    /// Stop offering transactions for this block.
    Stop,
}

/// What block production decided about one offered receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptVerdict {
    /// Included in the block.
    Included,
    /// Failed execution: remove it from the pool.
    Evict,
    /// Stop offering receipts for this block.
    Stop,
}

#[derive(Debug)]
struct Pending<T> {
    item: T,
    inserted: Instant,
}

#[derive(Debug, Default)]
struct TxPool {
    /// Each sender's pending transactions by nonce (always a contiguous run).
    chains: HashMap<String, BTreeMap<u64, Pending<Transaction>>>,
    /// Chain heads ordered for selection: higher fee first, then sender.
    heads: BTreeSet<(Reverse<u64>, String)>,
    /// Chain tails ordered for eviction: lowest fee first, then sender.
    tails: BTreeSet<(u64, String)>,
    /// Insertion order for expiry; entries whose transaction is gone are skipped.
    expiry: VecDeque<(Instant, String, u64)>,
    len: usize,
}

impl TxPool {
    fn head_key(
        chain: &BTreeMap<u64, Pending<Transaction>>,
        sender: &str,
    ) -> Option<(Reverse<u64>, String)> {
        chain
            .values()
            .next()
            .map(|head| (Reverse(head.item.fee), sender.to_string()))
    }

    fn tail_key(
        chain: &BTreeMap<u64, Pending<Transaction>>,
        sender: &str,
    ) -> Option<(u64, String)> {
        chain
            .values()
            .next_back()
            .map(|tail| (tail.item.fee, sender.to_string()))
    }

    /// Run `change` on one sender's chain, keeping `heads`, `tails` and `len` in step.
    fn with_chain<R>(
        &mut self,
        sender: &str,
        change: impl FnOnce(&mut BTreeMap<u64, Pending<Transaction>>) -> R,
    ) -> R {
        let mut chain = self.chains.remove(sender).unwrap_or_default();
        if let Some(key) = Self::head_key(&chain, sender) {
            self.heads.remove(&key);
        }
        if let Some(key) = Self::tail_key(&chain, sender) {
            self.tails.remove(&key);
        }
        let before = chain.len();
        let result = change(&mut chain);
        self.len = self.len + chain.len() - before;
        if let Some(key) = Self::head_key(&chain, sender) {
            self.heads.insert(key);
        }
        if let Some(key) = Self::tail_key(&chain, sender) {
            self.tails.insert(key);
        }
        if !chain.is_empty() {
            self.chains.insert(sender.to_string(), chain);
        }
        result
    }

    /// Remove `nonce` and every later nonce of `sender` (a gap would strand them).
    fn remove_from(&mut self, sender: &str, nonce: u64) {
        if self.chains.contains_key(sender) {
            self.with_chain(sender, |chain| {
                chain.split_off(&nonce);
            });
        }
    }

    /// Remove every nonce of `sender` below `nonce` (committed or superseded).
    fn remove_below(&mut self, sender: &str, nonce: u64) {
        if self.chains.contains_key(sender) {
            self.with_chain(sender, |chain| {
                let kept = chain.split_off(&nonce);
                *chain = kept;
            });
        }
    }

    fn rebuild_expiry(&mut self) {
        let mut records: Vec<(Instant, String, u64)> = self
            .chains
            .iter()
            .flat_map(|(sender, chain)| {
                chain
                    .iter()
                    .map(move |(nonce, pending)| (pending.inserted, sender.clone(), *nonce))
            })
            .collect();
        records.sort();
        self.expiry = records.into();
    }

    fn inserted_at(&self, sender: &str, nonce: u64) -> Option<Instant> {
        self.chains.get(sender)?.get(&nonce).map(|p| p.inserted)
    }
}

#[derive(Debug, Default)]
struct ReceiptPool {
    /// Arrival sequence -> receipt.
    ordered: BTreeMap<u64, Pending<ComputeReceipt>>,
    by_task: HashMap<String, u64>,
    per_client: HashMap<String, usize>,
    next_seq: u64,
}

impl ReceiptPool {
    fn remove(&mut self, task_id: &str) {
        let Some(seq) = self.by_task.remove(task_id) else {
            return;
        };
        let Some(pending) = self.ordered.remove(&seq) else {
            return;
        };
        let client = &pending.item.client_address;
        if let Some(count) = self.per_client.get_mut(client) {
            *count -= 1;
            if *count == 0 {
                self.per_client.remove(client);
            }
        }
    }
}

/// In-memory mempool shared between RPC admission, block production and the commit hook.
#[derive(Clone, Debug)]
pub struct Mempool {
    limits: MempoolLimits,
    txs: Arc<Mutex<TxPool>>,
    receipts: Arc<Mutex<ReceiptPool>>,
}

impl Default for Mempool {
    fn default() -> Self {
        Self::new()
    }
}

/// A poisoned lock only means another thread panicked; every mutation here completes before
/// anything can panic, so the maps are consistent.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Mempool {
    pub fn new() -> Self {
        Self::with_limits(MempoolLimits::default())
    }

    pub fn with_limits(limits: MempoolLimits) -> Self {
        Self {
            limits,
            txs: Arc::default(),
            receipts: Arc::default(),
        }
    }

    pub fn limits(&self) -> MempoolLimits {
        self.limits
    }

    /// Whether `(sender, nonce)` is pending.
    pub fn contains_tx(&self, sender: &str, nonce: u64) -> bool {
        lock(&self.txs)
            .chains
            .get(sender)
            .is_some_and(|chain| chain.contains_key(&nonce))
    }

    /// Highest pending nonce of `sender`, if it has pending transactions.
    pub fn last_pending_nonce(&self, sender: &str) -> Option<u64> {
        lock(&self.txs)
            .chains
            .get(sender)
            .and_then(|chain| chain.keys().next_back().copied())
    }

    /// Whether a receipt for `task_id` is pending.
    pub fn contains_receipt(&self, task_id: &str) -> bool {
        lock(&self.receipts).by_task.contains_key(task_id)
    }

    /// Insert a transaction admitted by the node. A sender with pending transactions may only
    /// extend its chain by the next nonce; the first pending nonce is the node's to check.
    pub fn insert_tx(&self, tx: Transaction) -> Result<(), InsertError> {
        self.insert_tx_at(tx, Instant::now())
    }

    fn insert_tx_at(&self, tx: Transaction, now: Instant) -> Result<(), InsertError> {
        if tx.sender.is_empty() || tx.recipient.is_empty() {
            return Err(InsertError::Malformed);
        }
        // Expire on insert as well: a node that is not producing blocks must still age items
        // out (amortised constant work).
        self.prune_expired(now);
        if tx.fee < self.limits.min_fee {
            return Err(InsertError::FeeTooLow);
        }
        let mut pool = lock(&self.txs);
        if let Some(chain) = pool.chains.get(&tx.sender) {
            if chain.contains_key(&tx.nonce) {
                return Err(InsertError::Duplicate);
            }
            if chain.len() >= self.limits.max_txs_per_sender {
                return Err(InsertError::SenderLimit);
            }
            let last = chain.keys().next_back().copied();
            if last.and_then(|n| n.checked_add(1)) != Some(tx.nonce) {
                return Err(InsertError::NonceGap);
            }
        }
        if pool.len >= self.limits.max_txs && !evict_for(&mut pool, &tx) {
            return Err(InsertError::Full);
        }
        pool.expiry.push_back((now, tx.sender.clone(), tx.nonce));
        let sender = tx.sender.clone();
        pool.with_chain(&sender, |chain| {
            chain.insert(
                tx.nonce,
                Pending {
                    item: tx,
                    inserted: now,
                },
            );
        });
        Ok(())
    }

    /// Insert a compute receipt admitted by the node.
    pub fn insert_receipt(&self, receipt: ComputeReceipt) -> Result<(), InsertError> {
        self.insert_receipt_at(receipt, Instant::now())
    }

    fn insert_receipt_at(&self, receipt: ComputeReceipt, now: Instant) -> Result<(), InsertError> {
        if receipt.task_id.is_empty() || receipt.worker_id.is_empty() {
            return Err(InsertError::Malformed);
        }
        self.prune_expired(now);
        let mut pool = lock(&self.receipts);
        if pool.by_task.contains_key(&receipt.task_id) {
            return Err(InsertError::Duplicate);
        }
        if pool.ordered.len() >= self.limits.max_receipts {
            return Err(InsertError::Full);
        }
        // Receipts without a client account share no quota (there is no account to protect).
        let client = receipt.client_address.clone();
        if !client.is_empty()
            && pool.per_client.get(&client).copied().unwrap_or(0)
                >= self.limits.max_receipts_per_client
        {
            return Err(InsertError::SenderLimit);
        }
        let seq = pool.next_seq;
        pool.next_seq += 1;
        pool.by_task.insert(receipt.task_id.clone(), seq);
        *pool.per_client.entry(client).or_default() += 1;
        pool.ordered.insert(
            seq,
            Pending {
                item: receipt,
                inserted: now,
            },
        );
        Ok(())
    }

    /// Offer pending transactions to block production, sender chains in order of their head's
    /// fee and each chain in nonce order, until `max` are included or `decide` stops. Nothing
    /// is removed (removal happens at commit), so a view that times out loses nothing.
    pub fn select_txs(
        &self,
        max: usize,
        mut decide: impl FnMut(&Transaction) -> TxVerdict,
    ) -> Vec<Transaction> {
        self.prune_expired(Instant::now());
        let mut pool = lock(&self.txs);
        let mut included = Vec::new();
        let mut evicted: Vec<(String, u64)> = Vec::new();
        'senders: for (_, sender) in &pool.heads {
            let Some(chain) = pool.chains.get(sender) else {
                continue;
            };
            for pending in chain.values() {
                if included.len() >= max {
                    break 'senders;
                }
                match decide(&pending.item) {
                    TxVerdict::Included => included.push(pending.item.clone()),
                    TxVerdict::Skip => {}
                    TxVerdict::DropSender => continue 'senders,
                    TxVerdict::Evict => {
                        evicted.push((sender.clone(), pending.item.nonce));
                        continue 'senders;
                    }
                    TxVerdict::Stop => break 'senders,
                }
            }
        }
        for (sender, nonce) in evicted {
            pool.remove_from(&sender, nonce);
        }
        included
    }

    /// Offer pending receipts in arrival order until `max` are included or `decide` stops.
    pub fn select_receipts(
        &self,
        max: usize,
        mut decide: impl FnMut(&ComputeReceipt) -> ReceiptVerdict,
    ) -> Vec<ComputeReceipt> {
        let mut pool = lock(&self.receipts);
        let mut included = Vec::new();
        let mut evicted = Vec::new();
        for pending in pool.ordered.values() {
            if included.len() >= max {
                break;
            }
            match decide(&pending.item) {
                ReceiptVerdict::Included => included.push(pending.item.clone()),
                ReceiptVerdict::Evict => evicted.push(pending.item.task_id.clone()),
                ReceiptVerdict::Stop => break,
            }
        }
        for task_id in evicted {
            pool.remove(&task_id);
        }
        included
    }

    /// Drop items older than the pool's time-to-live (and the later nonces of expired
    /// transactions, which could no longer apply).
    pub fn prune_expired(&self, now: Instant) {
        let ttl = self.limits.ttl;
        let expired = |since: Instant| now.saturating_duration_since(since) >= ttl;
        {
            let mut pool = lock(&self.txs);
            while let Some((inserted, _, _)) = pool.expiry.front() {
                if !expired(*inserted) {
                    break;
                }
                let Some((inserted, sender, nonce)) = pool.expiry.pop_front() else {
                    break;
                };
                // Skip records of transactions already removed (or re-inserted later).
                if pool.inserted_at(&sender, nonce) == Some(inserted) {
                    pool.remove_from(&sender, nonce);
                }
            }
            // Records of removed transactions only leave at the front; rebuild the queue when
            // they dominate, so churn cannot grow it without bound.
            if pool.expiry.len() > 2 * pool.len + EXPIRY_SLACK {
                pool.rebuild_expiry();
            }
        }
        let mut pool = lock(&self.receipts);
        let stale: Vec<String> = pool
            .ordered
            .values()
            .take_while(|pending| expired(pending.inserted))
            .map(|pending| pending.item.task_id.clone())
            .collect();
        for task_id in stale {
            pool.remove(&task_id);
        }
    }

    /// Remove transactions from `sender` with `nonce < min_pending_nonce` (committed or
    /// superseded by a committed transaction with the same nonce).
    pub fn prune_sender_up_to_nonce(&self, sender: &str, min_pending_nonce: u64) {
        lock(&self.txs).remove_below(sender, min_pending_nonce);
    }

    /// Remove `(sender, nonce)` and the sender's later nonces, which would wait on it forever.
    pub fn remove_tx(&self, sender: &str, nonce: u64) {
        lock(&self.txs).remove_from(sender, nonce);
    }

    /// Remove a specific compute receipt `task_id` from the mempool.
    pub fn remove_receipt(&self, task_id: &str) {
        lock(&self.receipts).remove(task_id);
    }

    /// A copy of every pending transaction (for checks done outside the pool lock, such as
    /// the leader's parallel signature pre-pass).
    pub fn pending_txs(&self) -> Vec<Transaction> {
        let pool = lock(&self.txs);
        pool.chains
            .values()
            .flat_map(|chain| chain.values().map(|pending| pending.item.clone()))
            .collect()
    }

    pub fn tx_count(&self) -> usize {
        lock(&self.txs).len
    }

    pub fn receipt_count(&self) -> usize {
        lock(&self.receipts).ordered.len()
    }
}

/// Make room in a full pool for `tx` by evicting the lowest-fee chain tail, if it pays less
/// than `tx` and is not the transaction `tx` builds on.
fn evict_for(pool: &mut TxPool, tx: &Transaction) -> bool {
    let Some((fee, sender)) = pool.tails.first().cloned() else {
        return false;
    };
    if fee >= tx.fee || sender == tx.sender {
        return false;
    }
    let Some(nonce) = pool
        .chains
        .get(&sender)
        .and_then(|c| c.keys().next_back().copied())
    else {
        return false;
    };
    pool.remove_from(&sender, nonce);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(sender: &str, nonce: u64, fee: u64) -> Transaction {
        Transaction::new(sender, nonce, "bob", 1, fee)
    }

    fn receipt(task: &str, client: &str) -> ComputeReceipt {
        ComputeReceipt::new(task, client, "w1", "coord", "m1", "p1", 50, 10, 8)
    }

    fn limits(max_txs: usize, max_receipts: usize, per_sender: usize) -> MempoolLimits {
        MempoolLimits {
            max_txs,
            max_receipts,
            max_txs_per_sender: per_sender,
            max_receipts_per_client: per_sender,
            min_fee: 1,
            ttl: Duration::from_secs(60),
        }
    }

    fn select_all(pool: &Mempool) -> Vec<(String, u64)> {
        pool.select_txs(usize::MAX, |_| TxVerdict::Included)
            .into_iter()
            .map(|t| (t.sender, t.nonce))
            .collect()
    }

    fn assert_consistent(pool: &Mempool) {
        let inner = lock(&pool.txs);
        let total: usize = inner.chains.values().map(BTreeMap::len).sum();
        assert_eq!(total, inner.len);
        assert_eq!(inner.heads.len(), inner.chains.len());
        assert_eq!(inner.tails.len(), inner.chains.len());
        for chain in inner.chains.values() {
            let nonces: Vec<u64> = chain.keys().copied().collect();
            assert!(
                nonces.windows(2).all(|w| w[1] == w[0] + 1),
                "chain {nonces:?}"
            );
        }
    }

    #[test]
    fn test_selection_follows_head_fee_then_each_chain_in_nonce_order() {
        let pool = Mempool::new();
        pool.insert_tx(tx("alice", 1, 10)).unwrap();
        pool.insert_tx(tx("alice", 2, 90)).unwrap(); // pays more but waits on nonce 1
        pool.insert_tx(tx("bob", 0, 50)).unwrap();
        assert_eq!(
            pool.insert_tx(tx("bob", 0, 99)),
            Err(InsertError::Duplicate)
        );

        let order = select_all(&pool);
        assert_eq!(
            order,
            vec![("bob".into(), 0), ("alice".into(), 1), ("alice".into(), 2)]
        );
        assert_eq!(pool.tx_count(), 3, "selection does not remove");
        assert_consistent(&pool);
    }

    #[test]
    fn test_nonce_gaps_fee_floor_and_sender_limit_are_refused() {
        let pool = Mempool::with_limits(limits(10, 10, 2));
        pool.insert_tx(tx("alice", 5, 1)).unwrap();
        assert_eq!(
            pool.insert_tx(tx("alice", 7, 1)),
            Err(InsertError::NonceGap)
        );
        assert_eq!(
            pool.insert_tx(tx("alice", 4, 1)),
            Err(InsertError::NonceGap)
        );
        pool.insert_tx(tx("alice", 6, 1)).unwrap();
        assert_eq!(
            pool.insert_tx(tx("alice", 7, 1)),
            Err(InsertError::SenderLimit)
        );
        assert_eq!(pool.insert_tx(tx("bob", 0, 0)), Err(InsertError::FeeTooLow));
        assert_consistent(&pool);
    }

    #[test]
    fn test_full_pool_evicts_the_cheapest_tail_for_a_better_fee() {
        let pool = Mempool::with_limits(limits(3, 10, 10));
        pool.insert_tx(tx("alice", 0, 5)).unwrap();
        pool.insert_tx(tx("alice", 1, 2)).unwrap();
        pool.insert_tx(tx("bob", 0, 3)).unwrap();
        // Not better than the cheapest tail (alice/1, fee 2): refused.
        assert_eq!(pool.insert_tx(tx("carol", 0, 2)), Err(InsertError::Full));
        // Better: alice/1 goes.
        pool.insert_tx(tx("carol", 0, 4)).unwrap();
        assert!(!pool.contains_tx("alice", 1));
        assert_eq!(pool.tx_count(), 3);
        assert_consistent(&pool);
    }

    #[test]
    fn test_removing_a_nonce_drops_the_later_ones_and_frees_quota() {
        let pool = Mempool::with_limits(limits(10, 10, 3));
        for nonce in 0..3 {
            pool.insert_tx(tx("alice", nonce, 1)).unwrap();
        }
        pool.remove_tx("alice", 1);
        assert_eq!(select_all(&pool), vec![("alice".into(), 0)]);
        pool.insert_tx(tx("alice", 1, 1)).unwrap();
        pool.prune_sender_up_to_nonce("alice", 1);
        assert_eq!(select_all(&pool), vec![("alice".into(), 1)]);
        assert_eq!(pool.last_pending_nonce("alice"), Some(1));
        pool.prune_sender_up_to_nonce("alice", 2);
        assert_eq!(pool.tx_count(), 0);
        assert_eq!(pool.last_pending_nonce("alice"), None);
        assert_consistent(&pool);
    }

    #[test]
    fn test_selection_verdicts_skip_drop_and_stop() {
        let pool = Mempool::new();
        for nonce in 0..3 {
            pool.insert_tx(tx("alice", nonce, 9)).unwrap();
        }
        pool.insert_tx(tx("bob", 0, 5)).unwrap();
        pool.insert_tx(tx("bob", 1, 5)).unwrap();
        pool.insert_tx(tx("carol", 0, 1)).unwrap();

        // alice/0 superseded (skip), alice/1 cannot apply (drop alice), bob fine, stop at carol.
        let picked = pool.select_txs(10, |t| match (t.sender.as_str(), t.nonce) {
            ("alice", 0) => TxVerdict::Skip,
            ("alice", _) => TxVerdict::DropSender,
            ("carol", _) => TxVerdict::Stop,
            _ => TxVerdict::Included,
        });
        let picked: Vec<(&str, u64)> = picked
            .iter()
            .map(|t| (t.sender.as_str(), t.nonce))
            .collect();
        assert_eq!(picked, vec![("bob", 0), ("bob", 1)]);
        assert_eq!(pool.select_txs(1, |_| TxVerdict::Included).len(), 1);
    }

    #[test]
    fn test_expired_transactions_take_their_later_nonces_with_them() {
        let pool = Mempool::with_limits(limits(10, 10, 10));
        let start = Instant::now();
        pool.insert_tx_at(tx("alice", 0, 1), start).unwrap();
        pool.insert_tx_at(tx("bob", 0, 1), start + Duration::from_secs(30))
            .unwrap();
        pool.insert_tx_at(tx("alice", 1, 1), start + Duration::from_secs(30))
            .unwrap();
        pool.insert_receipt_at(receipt("old", "c"), start).unwrap();
        pool.insert_receipt_at(receipt("new", "c"), start + Duration::from_secs(30))
            .unwrap();

        pool.prune_expired(start + Duration::from_secs(61));
        assert_eq!(select_all(&pool), vec![("bob".into(), 0)]);
        assert!(!pool.contains_receipt("old"));
        assert!(pool.contains_receipt("new"));
        assert_consistent(&pool);
    }

    #[test]
    fn test_receipts_arrive_in_order_with_per_client_and_total_bounds() {
        let pool = Mempool::with_limits(limits(10, 3, 2));
        pool.insert_receipt(receipt("t2", "alice")).unwrap();
        pool.insert_receipt(receipt("t1", "alice")).unwrap();
        assert_eq!(
            pool.insert_receipt(receipt("t3", "alice")),
            Err(InsertError::SenderLimit)
        );
        assert_eq!(
            pool.insert_receipt(receipt("t1", "bob")),
            Err(InsertError::Duplicate)
        );
        pool.insert_receipt(receipt("t0", "bob")).unwrap();
        assert_eq!(
            pool.insert_receipt(receipt("t4", "carol")),
            Err(InsertError::Full)
        );

        let ids: Vec<String> = pool
            .select_receipts(10, |_| ReceiptVerdict::Included)
            .into_iter()
            .map(|r| r.task_id)
            .collect();
        assert_eq!(ids, vec!["t2", "t1", "t0"]);
        pool.remove_receipt("t2");
        pool.insert_receipt(receipt("t3", "alice")).unwrap();
    }

    #[test]
    fn test_evicted_candidates_leave_the_pool_with_their_later_nonces() {
        let pool = Mempool::new();
        for nonce in 0..3 {
            pool.insert_tx(tx("alice", nonce, 9)).unwrap();
        }
        pool.insert_tx(tx("bob", 0, 1)).unwrap();
        let picked = pool.select_txs(10, |t| match (t.sender.as_str(), t.nonce) {
            ("alice", 1) => TxVerdict::Evict,
            _ => TxVerdict::Included,
        });
        assert_eq!(picked.len(), 2, "alice/0 and bob/0");
        assert_eq!(
            select_all(&pool),
            vec![("alice".into(), 0), ("bob".into(), 0)]
        );
        assert_consistent(&pool);

        pool.insert_receipt(receipt("bad", "c")).unwrap();
        pool.insert_receipt(receipt("good", "c")).unwrap();
        let kept = pool.select_receipts(10, |r| {
            if r.task_id == "bad" {
                ReceiptVerdict::Evict
            } else {
                ReceiptVerdict::Included
            }
        });
        assert_eq!(kept.len(), 1);
        assert!(!pool.contains_receipt("bad"));
        assert!(pool.contains_receipt("good"));
    }

    #[test]
    fn test_inserts_expire_old_items_without_block_production() {
        let pool = Mempool::with_limits(limits(10, 10, 10));
        let start = Instant::now();
        pool.insert_tx_at(tx("alice", 0, 1), start).unwrap();
        pool.insert_receipt_at(receipt("old", "c"), start).unwrap();
        let later = start + Duration::from_secs(61);
        pool.insert_tx_at(tx("bob", 0, 1), later).unwrap();
        assert!(!pool.contains_tx("alice", 0));
        pool.insert_receipt_at(receipt("new", "c"), later).unwrap();
        assert!(!pool.contains_receipt("old"));
        assert_consistent(&pool);
    }

    #[test]
    fn test_expiry_records_of_removed_transactions_are_compacted() {
        let pool = Mempool::new();
        for round in 0..(EXPIRY_SLACK as u64 + 10) {
            pool.insert_tx(tx("alice", round, 1)).unwrap();
            pool.prune_sender_up_to_nonce("alice", round + 1);
        }
        pool.prune_expired(Instant::now());
        assert!(lock(&pool.txs).expiry.len() <= EXPIRY_SLACK);
        assert_consistent(&pool);
    }

    #[test]
    fn test_receipts_without_a_client_share_no_quota() {
        let pool = Mempool::with_limits(limits(10, 10, 1));
        pool.insert_receipt(receipt("t1", "")).unwrap();
        pool.insert_receipt(receipt("t2", "")).unwrap();
        pool.insert_receipt(receipt("t3", "alice")).unwrap();
        assert_eq!(
            pool.insert_receipt(receipt("t4", "alice")),
            Err(InsertError::SenderLimit)
        );
    }
}

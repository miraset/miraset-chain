//! Empty-block heartbeat (owner decision 2026-11-13, TODO 8.5.3 finding 11).
//!
//! With pacemaker epoch length 1 the leader proposed a block in every view, about 23 empty
//! blocks per second on an idle devnet (≈ 3 GB of chain data per validator per day). Now a
//! leader proposes at once when there is work, and otherwise waits until `interval` has passed
//! since its parent's block time, so an idle chain grows by about one empty block per interval.
//!
//! "Work" is anything in this node's mempool, or a transaction or receipt in a block that is
//! not committed yet: the commit rule needs following blocks in consecutive views to commit
//! it, so those must not be slowed down. Only the leader's local timing changes; validation is unchanged.

use hotstuff_rs::app::ProduceBlockRequest;
use hotstuff_rs::block_tree::accessors::app::AppBlockTreeView;
use hotstuff_rs::block_tree::pluggables::KVStore;
use hotstuff_rs::types::data_types::CryptoHash;
use std::time::{Duration, Instant};

use super::{MirasetApp, now_ms};
use crate::core::types::BlockPayload;
use crate::mempool::Mempool;

/// How often a waiting leader looks for new work.
const POLL: Duration = Duration::from_millis(10);
/// Most uncommitted blocks the work check walks; a longer uncommitted chain counts as work.
const MAX_UNCOMMITTED_WALK: usize = 64;

/// Wait until work arrives or `interval` has passed since the parent's block time.
///
/// Liveness: the wait never exceeds `interval` from now, which the caller keeps at or below
/// half of `max_view_time`, so a waiting leader still proposes before its replicas time out.
pub(super) fn wait_for_work_or_heartbeat<K: KVStore>(
    request: &ProduceBlockRequest<'_, K>,
    mempool: &Mempool,
    interval: Duration,
) {
    let Some(parent) = request.parent_block() else {
        return;
    };
    let tree = request.block_tree();
    if interval.is_zero() || has_work(mempool) || recent_block_has_work(tree, parent) {
        return;
    }
    let interval_ms = u64::try_from(interval.as_millis()).unwrap_or(u64::MAX);
    let due_ms = MirasetApp::read_chain_time_ms(tree).saturating_add(interval_ms);
    let wait = Duration::from_millis(due_ms.saturating_sub(now_ms()));
    wait_until(Instant::now() + wait.min(interval), || has_work(mempool));
}

/// Strict block-rate limit (owner request 2026-11-13): whatever the work, a leader proposes no
/// earlier than `interval` after its parent's block time, so the chain makes at most one block
/// per `interval`. The wait is capped at `interval` from now, like the heartbeat.
pub(super) fn wait_for_min_block_interval<K: KVStore>(
    request: &ProduceBlockRequest<'_, K>,
    interval: Duration,
) {
    if interval.is_zero() || request.parent_block().is_none() {
        return;
    }
    let interval_ms = u64::try_from(interval.as_millis()).unwrap_or(u64::MAX);
    let due_ms = MirasetApp::read_chain_time_ms(request.block_tree()).saturating_add(interval_ms);
    let wait = Duration::from_millis(due_ms.saturating_sub(now_ms()));
    wait_until(Instant::now() + wait.min(interval), || false);
}

/// Sleep until `deadline` or until `work` reports something to do, checking every [`POLL`].
///
/// The caller caps the deadline at `interval` from now: the parent's block time comes from
/// another validator's clock, and a clock ahead of ours must not stretch the wait past the
/// interval (which [`crate::node::NodeConfig::effective_empty_block_interval`] keeps below
/// half the view time).
fn wait_until(deadline: Instant, mut work: impl FnMut() -> bool) {
    loop {
        let now = Instant::now();
        if now >= deadline || work() {
            return;
        }
        std::thread::sleep(POLL.min(deadline - now));
    }
}

fn has_work(mempool: &Mempool) -> bool {
    mempool.tx_count() > 0 || mempool.receipt_count() > 0
}

/// Whether `block` or an uncommitted ancestor carries a transaction or receipt.
///
/// The walk covers every uncommitted block, not a fixed 3-chain: after a view timeout a block
/// with work needs three blocks in consecutive views before it commits, and those may come
/// several blocks later (review H1). A block counts as committed when the committed height
/// index points at it. A block that cannot be read counts as work, and so does a chain with
/// more than [`MAX_UNCOMMITTED_WALK`] uncommitted blocks, so the leader never waits on a chain
/// it cannot account for.
fn recent_block_has_work<K: KVStore>(tree: &AppBlockTreeView<'_, K>, block: CryptoHash) -> bool {
    let mut cursor = Some(block);
    for _ in 0..MAX_UNCOMMITTED_WALK {
        let Some(hash) = cursor else {
            return false;
        };
        match is_committed(tree, &hash) {
            Some(true) => return false,
            Some(false) => {}
            None => return true,
        }
        if block_has_work(tree, &hash) {
            return true;
        }
        cursor = match tree.block_justify(&hash) {
            Ok(justify) if !justify.is_genesis_pc() => Some(justify.block),
            Ok(_) => None,
            Err(_) => return true,
        };
    }
    true
}

/// `None` when the block or the height index cannot be read.
fn is_committed<K: KVStore>(tree: &AppBlockTreeView<'_, K>, block: &CryptoHash) -> Option<bool> {
    let height = tree.block_height(block).ok()??;
    let at_height = tree.block_at_height(height).ok()?;
    Some(at_height.as_ref() == Some(block))
}

/// Whether `block` carries a transaction or receipt; unreadable payloads count as work.
fn block_has_work<K: KVStore>(tree: &AppBlockTreeView<'_, K>, block: &CryptoHash) -> bool {
    let Ok(Some(data)) = tree.block_data(block) else {
        return true;
    };
    let Some(datum) = data.vec().first() else {
        return true;
    };
    !matches!(
        BlockPayload::from_bytes(datum.bytes()),
        Ok(payload) if payload.transactions.is_empty() && payload.compute_receipts.is_empty()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const SLACK: Duration = Duration::from_millis(500);

    #[test]
    fn wait_returns_at_once_when_work_is_pending() {
        let start = Instant::now();
        wait_until(start + Duration::from_secs(10), || true);
        assert!(start.elapsed() < SLACK);
    }

    #[test]
    fn wait_returns_at_the_deadline_without_work() {
        let start = Instant::now();
        let interval = Duration::from_millis(150);
        wait_until(start + interval, || false);
        let took = start.elapsed();
        assert!(took >= interval, "returned early: {took:?}");
        assert!(took < interval + SLACK, "overslept: {took:?}");
    }

    #[test]
    fn wait_returns_when_work_arrives_mid_wait() {
        let start = Instant::now();
        let polls = Cell::new(0u32);
        wait_until(start + Duration::from_secs(10), || {
            polls.set(polls.get() + 1);
            polls.get() > 5
        });
        assert!(start.elapsed() < SLACK + POLL * 6);
    }

    #[test]
    fn mempool_work_includes_transactions() {
        let mempool = Mempool::new();
        assert!(!has_work(&mempool));
        let key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let mut tx = crate::core::types::Transaction::new(
            hex::encode(key.verifying_key().to_bytes()),
            0,
            "bob",
            1,
            1,
        );
        tx.sign_with(&key);
        assert!(mempool.insert_tx(tx).is_ok());
        assert!(has_work(&mempool));
    }
}

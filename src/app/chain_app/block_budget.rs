//! Leader-side adaptive block size (TODO 8.5.3, owner decision 2026-11-15).
//!
//! With a lossy WAN link, TCP carries about 1 MB/s per stream, so a block of up to
//! `MAX_BLOCK_BYTES` cannot reach a quorum within one view. The view times out, the mempool
//! grows, and the next leader proposes a full block again: the chain stops committing
//! (`docs/performance-baseline.md`, WAN under load). This budget shrinks the blocks this node
//! proposes while views keep timing out and grows them back as blocks commit.
//!
//! - **Decrease:** a view timeout that follows another timeout with no commit in between halves
//!   the budget, down to [`MIN_BLOCK_BUDGET_BYTES`]. A single timeout followed by commits (for
//!   example a crashed leader's view, once per round) does not shrink it.
//! - **Increase:** every commit adds [`BLOCK_BUDGET_RECOVERY_BYTES`], up to the block limit.
//!
//! The budget only limits what this node proposes. Block validity is unchanged: validators
//! still accept any block up to `MAX_BLOCK_BYTES`, so nodes may hold different budgets.

use std::sync::Mutex;

use super::MAX_BLOCK_BYTES;

/// Smallest payload budget a leader shrinks to (still thousands of transfers per block).
pub const MIN_BLOCK_BUDGET_BYTES: usize = 256 * 1024;
/// Payload budget regained per committed block (full size again after ~16 commits from the
/// floor).
pub const BLOCK_BUDGET_RECOVERY_BYTES: usize = 512 * 1024;

#[derive(Debug)]
struct BudgetState {
    bytes: usize,
    /// A view timed out since the last commit.
    timed_out_since_commit: bool,
}

/// Payload byte budget for the blocks this node proposes. Shared between the app (reads it
/// when producing a block) and the consensus event handlers (update it).
#[derive(Debug)]
pub struct BlockBudget {
    state: Mutex<BudgetState>,
}

impl Default for BlockBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockBudget {
    /// Starts at the full block limit.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(BudgetState {
                bytes: MAX_BLOCK_BYTES,
                timed_out_since_commit: false,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, BudgetState> {
        // The state stays consistent under any panic (plain field writes), so a poisoned lock
        // is still usable.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Payload bytes the next proposed block may use (`MIN_BLOCK_BUDGET_BYTES..=MAX_BLOCK_BYTES`).
    pub fn bytes(&self) -> usize {
        self.state().bytes
    }

    /// A view of this node timed out. Returns the budget afterwards.
    pub fn record_view_timeout(&self) -> usize {
        let mut state = self.state();
        if state.timed_out_since_commit {
            state.bytes = (state.bytes / 2).max(MIN_BLOCK_BUDGET_BYTES);
        }
        state.timed_out_since_commit = true;
        state.bytes
    }

    /// This node committed a block. Returns the budget afterwards.
    pub fn record_commit(&self) -> usize {
        let mut state = self.state();
        state.timed_out_since_commit = false;
        state.bytes = state
            .bytes
            .saturating_add(BLOCK_BUDGET_RECOVERY_BYTES)
            .min(MAX_BLOCK_BYTES);
        state.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_the_block_limit() {
        assert_eq!(BlockBudget::new().bytes(), MAX_BLOCK_BYTES);
    }

    #[test]
    fn a_single_timeout_between_commits_does_not_shrink_the_budget() {
        // A crashed validator's view times out once per round; the others commit.
        let budget = BlockBudget::new();
        for _ in 0..10 {
            assert_eq!(budget.record_view_timeout(), MAX_BLOCK_BYTES);
            budget.record_commit();
            budget.record_commit();
        }
        assert_eq!(budget.bytes(), MAX_BLOCK_BYTES);
    }

    #[test]
    fn consecutive_timeouts_halve_the_budget_down_to_the_floor() {
        let budget = BlockBudget::new();
        assert_eq!(budget.record_view_timeout(), MAX_BLOCK_BYTES);
        assert_eq!(budget.record_view_timeout(), MAX_BLOCK_BYTES / 2);
        assert_eq!(budget.record_view_timeout(), MAX_BLOCK_BYTES / 4);
        for _ in 0..20 {
            budget.record_view_timeout();
        }
        assert_eq!(budget.bytes(), MIN_BLOCK_BUDGET_BYTES);
    }

    #[test]
    fn commits_grow_the_budget_back_to_the_limit() {
        let budget = BlockBudget::new();
        for _ in 0..30 {
            budget.record_view_timeout();
        }
        assert_eq!(
            budget.record_commit(),
            MIN_BLOCK_BUDGET_BYTES + BLOCK_BUDGET_RECOVERY_BYTES
        );
        let commits_to_full =
            (MAX_BLOCK_BYTES - MIN_BLOCK_BUDGET_BYTES).div_ceil(BLOCK_BUDGET_RECOVERY_BYTES);
        for _ in 1..commits_to_full {
            budget.record_commit();
        }
        assert_eq!(budget.bytes(), MAX_BLOCK_BYTES);
        assert_eq!(budget.record_commit(), MAX_BLOCK_BYTES);
    }

    #[test]
    fn a_commit_resets_the_timeout_streak() {
        let budget = BlockBudget::new();
        budget.record_view_timeout();
        budget.record_view_timeout(); // halved
        let halved = budget.bytes();
        budget.record_commit();
        // First timeout after a commit: no decrease.
        assert_eq!(
            budget.record_view_timeout(),
            halved + BLOCK_BUDGET_RECOVERY_BYTES
        );
    }
}

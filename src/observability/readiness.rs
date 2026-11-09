//! Liveness, readiness and drain state (TODO 8.4.4, PRD R12).
//!
//! - **Live**: the process is serving and consensus has not halted for safety.
//! - **Ready**: live, not draining, and this node finalized a block within the configured
//!   maximum commit age. A node that is still recovering, partitioned from a quorum, or in a
//!   validator set that cannot finalize is therefore not ready.
//!
//! Limitation: a node that is catching up through block sync also commits blocks, so it can be
//! reported ready before it reaches the tip.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Default for `--ready-max-commit-age`.
pub const DEFAULT_READY_MAX_COMMIT_AGE: Duration = Duration::from_secs(30);

/// Why a node is not ready. Each variant has a fixed label for responses and metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotReadyReason {
    Halted,
    Draining,
    NoCommitYet,
    CommitStale,
}

impl NotReadyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            NotReadyReason::Halted => "halted",
            NotReadyReason::Draining => "draining",
            NotReadyReason::NoCommitYet => "no_commit_yet",
            NotReadyReason::CommitStale => "commit_stale",
        }
    }
}

/// Inputs to the readiness decision, sampled at probe time.
#[derive(Debug, Clone, Copy)]
pub struct ReadinessInput {
    pub halted: bool,
    pub draining: bool,
    /// Time since this process last observed a committed block; `None` before the first one.
    pub last_commit_age: Option<Duration>,
    pub max_commit_age: Duration,
}

/// Decide readiness. Checks run in severity order, so the reported reason is the most serious.
pub fn evaluate_readiness(input: ReadinessInput) -> Result<(), NotReadyReason> {
    if input.halted {
        return Err(NotReadyReason::Halted);
    }
    if input.draining {
        return Err(NotReadyReason::Draining);
    }
    match input.last_commit_age {
        None => Err(NotReadyReason::NoCommitYet),
        Some(age) if age > input.max_commit_age => Err(NotReadyReason::CommitStale),
        Some(_) => Ok(()),
    }
}

/// Process lifecycle flag shared by RPC admission, probes and shutdown.
#[derive(Debug, Default)]
pub struct Lifecycle {
    draining: AtomicBool,
}

impl Lifecycle {
    /// Stop admitting transactions and receipts and report not ready. It cannot be undone.
    pub fn begin_drain(&self) {
        self.draining.store(true, Ordering::Release);
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: Duration = Duration::from_secs(30);

    fn input(halted: bool, draining: bool, age: Option<u64>) -> ReadinessInput {
        ReadinessInput {
            halted,
            draining,
            last_commit_age: age.map(Duration::from_secs),
            max_commit_age: MAX,
        }
    }

    #[test]
    fn test_ready_only_with_recent_commit_and_no_halt_or_drain() {
        assert_eq!(evaluate_readiness(input(false, false, Some(1))), Ok(()));
        assert_eq!(evaluate_readiness(input(false, false, Some(30))), Ok(()));
        assert_eq!(
            evaluate_readiness(input(false, false, Some(31))),
            Err(NotReadyReason::CommitStale)
        );
        assert_eq!(
            evaluate_readiness(input(false, false, None)),
            Err(NotReadyReason::NoCommitYet)
        );
    }

    #[test]
    fn test_halt_outranks_drain_and_drain_outranks_commit_state() {
        assert_eq!(
            evaluate_readiness(input(true, true, Some(1))),
            Err(NotReadyReason::Halted)
        );
        assert_eq!(
            evaluate_readiness(input(false, true, Some(1))),
            Err(NotReadyReason::Draining)
        );
    }

    #[test]
    fn test_drain_is_one_way() {
        let lifecycle = Lifecycle::default();
        assert!(!lifecycle.is_draining());
        lifecycle.begin_drain();
        lifecycle.begin_drain();
        assert!(lifecycle.is_draining());
    }

    #[test]
    fn test_reason_labels_are_fixed() {
        let labels: Vec<_> = [
            NotReadyReason::Halted,
            NotReadyReason::Draining,
            NotReadyReason::NoCommitYet,
            NotReadyReason::CommitStale,
        ]
        .iter()
        .map(|r| r.as_str())
        .collect();
        assert_eq!(
            labels,
            ["halted", "draining", "no_commit_yet", "commit_stale"]
        );
    }
}

//! Phase-vote refusal counters shared between a signing-guard layer and `/metrics`
//! (TODO 8.4.4, PRD R12).
//!
//! Each guard layer (`GuardedSigner` before signing, `GuardedNetwork` before sending) owns one
//! `VoteRefusals` and the node's `NodeMetrics` exports it under a fixed `layer` label.

use std::sync::atomic::{AtomicU64, Ordering};

/// Where a phase vote was refused. Fixed set of label values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteGuardLayer {
    /// The in-process `GuardedSigner` refused to sign. A validator using a remote signer has
    /// no signer-layer series: those refusals are counted inside the signer process.
    Signer,
    /// `GuardedNetwork` dropped an already signed vote before it left the node.
    Transport,
}

impl VoteGuardLayer {
    pub(crate) const ALL: [VoteGuardLayer; 2] = [VoteGuardLayer::Signer, VoteGuardLayer::Transport];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            VoteGuardLayer::Signer => "signer",
            VoteGuardLayer::Transport => "transport",
        }
    }

    pub(crate) fn index(self) -> usize {
        self as usize
    }
}

/// Refused phase votes by reason. Process-local; resets on restart.
#[derive(Debug, Default)]
pub struct VoteRefusals {
    stale: AtomicU64,
    conflicting: AtomicU64,
    below_lock: AtomicU64,
}

impl VoteRefusals {
    /// A vote for a view below the guard record.
    pub fn record_stale(&self) {
        self.stale.fetch_add(1, Ordering::Relaxed);
    }

    /// A different vote than the one already recorded for the same view.
    pub fn record_conflicting(&self) {
        self.conflicting.fetch_add(1, Ordering::Relaxed);
    }

    /// A vote building on a certificate below the guard's lock floor (TODO 8.5.2).
    pub fn record_below_lock(&self) {
        self.below_lock.fetch_add(1, Ordering::Relaxed);
    }

    pub fn stale(&self) -> u64 {
        self.stale.load(Ordering::Relaxed)
    }

    pub fn conflicting(&self) -> u64 {
        self.conflicting.load(Ordering::Relaxed)
    }

    pub fn below_lock(&self) -> u64 {
        self.below_lock.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_refusals_count_by_reason() {
        let refusals = VoteRefusals::default();
        refusals.record_stale();
        refusals.record_stale();
        refusals.record_conflicting();
        refusals.record_below_lock();
        assert_eq!(
            (
                refusals.stale(),
                refusals.conflicting(),
                refusals.below_lock()
            ),
            (2, 1, 1)
        );
    }

    #[test]
    fn test_layer_labels_and_indexes_are_fixed() {
        for (i, layer) in VoteGuardLayer::ALL.iter().enumerate() {
            assert_eq!(layer.index(), i);
        }
        let labels: Vec<_> = VoteGuardLayer::ALL.iter().map(|l| l.as_str()).collect();
        assert_eq!(labels, ["signer", "transport"]);
    }
}

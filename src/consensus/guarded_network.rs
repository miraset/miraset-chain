//! Transport-boundary signing guard (`docs/adr/proposal-vote-safety-state.md` D2/D3).
//!
//! `GuardedNetwork` must be the outermost `Network` handed to the replica: our transports
//! deliver self-addressed messages inside `send`/`broadcast`, so wrapping them here also covers
//! a replica's vote to itself when it is the next leader.
//!
//! Per outgoing phase vote (checked on both `send` and `broadcast`):
//! - fresh or identical re-send: forwarded;
//! - stale (view below the guard record) or conflicting (same view, different vote): dropped
//!   and counted, node keeps running. Both happen legitimately after a chain-data wipe or
//!   restore, so they are refusals, not faults;
//! - guard fault (storage failure, wrong chain): dropped and the node halts.
//!
//! After a halt, no outgoing message is forwarded.

use hotstuff_rs::hotstuff::messages::HotStuffMessage;
use hotstuff_rs::networking::messages::{Message, ProgressMessage};
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::update_sets::ValidatorSetUpdates;
use hotstuff_rs::types::validator_set::{ValidatorSet, VerifyingKey};
use std::sync::Arc;

use super::safety_halt::SafetyHalt;
use super::signing_guard::{Authorization, SigningGuard};
use crate::observability::VoteRefusals;

#[derive(Clone)]
pub struct GuardedNetwork<N: Network> {
    inner: N,
    guard: SigningGuard,
    halt: SafetyHalt,
    refusals: Arc<VoteRefusals>,
}

impl<N: Network> GuardedNetwork<N> {
    pub fn new(inner: N, guard: SigningGuard, halt: SafetyHalt) -> Self {
        Self {
            inner,
            guard,
            halt,
            refusals: Arc::new(VoteRefusals::default()),
        }
    }

    /// Number of stale phase votes dropped since start.
    pub fn stale_votes_dropped(&self) -> u64 {
        self.refusals.stale()
    }

    /// Number of conflicting same-view phase votes refused since start.
    pub fn conflicting_votes_refused(&self) -> u64 {
        self.refusals.conflicting()
    }

    /// Refusal counters, shared with `/metrics` (`layer="transport"`).
    pub fn refusals(&self) -> Arc<VoteRefusals> {
        Arc::clone(&self.refusals)
    }

    pub fn halt(&self) -> &SafetyHalt {
        &self.halt
    }

    fn admit(&self, message: &Message) -> bool {
        if self.halt.is_halted() {
            return false;
        }
        let Message::ProgressMessage(ProgressMessage::HotStuffMessage(HotStuffMessage::PhaseVote(
            vote,
        ))) = message
        else {
            return true;
        };
        match self.guard.authorize(vote) {
            Ok(Authorization::Fresh | Authorization::IdenticalResend) => true,
            Ok(Authorization::Stale { last_view }) => {
                self.refusals.record_stale();
                tracing::warn!(
                    target: crate::observability::logging::SAFETY_LOG_TARGET,
                    "signing guard: dropped stale phase vote for view {} (last signed view {last_view})",
                    vote.view.int()
                );
                false
            }
            Ok(Authorization::Conflicting { view }) => {
                self.refusals.record_conflicting();
                tracing::error!(
                    target: crate::observability::logging::SAFETY_LOG_TARGET,
                    "signing guard: REFUSED conflicting phase vote for view {view} (already signed a different vote in this view)"
                );
                false
            }
            Ok(Authorization::BelowLock {
                justify_view,
                lock_floor,
            }) => {
                // Not produced without lock context; handled for exhaustiveness.
                self.refusals.record_below_lock();
                tracing::error!(
                    target: crate::observability::logging::SAFETY_LOG_TARGET,
                    "signing guard: dropped phase vote building on view {justify_view} below lock floor {lock_floor}"
                );
                false
            }
            Ok(Authorization::InvalidLockContext { view }) => {
                // Not produced without lock context; handled for exhaustiveness.
                tracing::error!(
                    target: crate::observability::logging::SAFETY_LOG_TARGET,
                    "signing guard: dropped phase vote for view {view}: invalid lock context"
                );
                false
            }
            Err(err) => {
                self.halt.trigger(format!("signing guard: {err}"));
                false
            }
        }
    }
}

impl<N: Network> Network for GuardedNetwork<N> {
    fn init_validator_set(&mut self, validator_set: ValidatorSet) {
        self.inner.init_validator_set(validator_set);
    }

    fn update_validator_set(&mut self, updates: ValidatorSetUpdates) {
        self.inner.update_validator_set(updates);
    }

    fn broadcast(&mut self, message: Message) {
        if self.admit(&message) {
            self.inner.broadcast(message);
        }
    }

    fn send(&mut self, peer: VerifyingKey, message: Message) {
        if self.admit(&message) {
            self.inner.send(peer, message);
        }
    }

    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        self.inner.recv()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::signing_guard::tests::vote;
    use crate::consensus::signing_guard::{GuardIdentity, OpenMode};
    use hotstuff_rs::hotstuff::types::Phase;
    use std::sync::Mutex;
    use tempfile::TempDir;

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<Message>>>);

    impl Recorder {
        fn count(&self) -> usize {
            self.0.lock().unwrap().len()
        }
    }

    impl Network for Recorder {
        fn init_validator_set(&mut self, _: ValidatorSet) {}
        fn update_validator_set(&mut self, _: ValidatorSetUpdates) {}
        fn broadcast(&mut self, message: Message) {
            self.0.lock().unwrap().push(message);
        }
        fn send(&mut self, _: VerifyingKey, message: Message) {
            self.0.lock().unwrap().push(message);
        }
        fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
            None
        }
    }

    fn vote_msg(view: u64, block: u8) -> Message {
        HotStuffMessage::PhaseVote(vote(1337, view, block, Phase::Generic)).into()
    }

    fn setup() -> (TempDir, Recorder, GuardedNetwork<Recorder>, VerifyingKey) {
        let dir = TempDir::new().unwrap();
        let identity = GuardIdentity {
            chain_id: 1337,
            validator: [1u8; 32],
            genesis_hash: [2u8; 32],
        };
        let guard = SigningGuard::open(
            &dir.path().join("guard.redb"),
            identity,
            OpenMode::CreateIfMissing,
        )
        .unwrap();
        let recorder = Recorder::default();
        let net = GuardedNetwork::new(recorder.clone(), guard, SafetyHalt::new());
        let peer = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]).verifying_key();
        (dir, recorder, net, peer)
    }

    #[test]
    fn test_fresh_and_resent_votes_are_forwarded_on_send_and_broadcast() {
        let (_dir, recorder, mut net, peer) = setup();
        net.send(peer, vote_msg(1, 0xaa));
        net.send(peer, vote_msg(1, 0xaa));
        net.broadcast(vote_msg(2, 0xbb));
        assert_eq!(recorder.count(), 3);
        assert!(!net.halt().is_halted());
    }

    #[test]
    fn test_stale_vote_is_dropped_without_halting() {
        let (_dir, recorder, mut net, peer) = setup();
        net.send(peer, vote_msg(5, 0xaa));
        net.send(peer, vote_msg(4, 0xcc));
        assert_eq!(recorder.count(), 1);
        assert_eq!(net.stale_votes_dropped(), 1);
        assert!(!net.halt().is_halted());
    }

    #[test]
    fn test_conflicting_vote_is_refused_without_halting() {
        let (_dir, recorder, mut net, peer) = setup();
        net.send(peer, vote_msg(5, 0xaa));
        net.broadcast(vote_msg(5, 0xbb));
        assert_eq!(recorder.count(), 1);
        assert_eq!(net.conflicting_votes_refused(), 1);
        assert!(!net.halt().is_halted());
    }

    #[test]
    fn test_guard_fault_halts_and_blocks_all_further_traffic() {
        let (_dir, recorder, mut net, peer) = setup();
        net.send(peer, vote_msg(5, 0xaa));
        let wrong_chain = HotStuffMessage::PhaseVote(vote(1338, 6, 0xaa, Phase::Generic));
        net.send(peer, wrong_chain.into());
        assert!(net.halt().is_halted());
        assert!(net.halt().reason().unwrap().contains("chain"));

        net.send(peer, vote_msg(7, 0xaa));
        assert_eq!(recorder.count(), 1, "nothing leaves a halted node");
    }
}

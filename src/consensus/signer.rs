//! Validator signer with slashing protection inside it (TODO 8.3.4, key custody D1+D2).
//!
//! The replica asks for every signature as a typed `SignRequest` (vendored patch #9). For a
//! phase vote the `SigningGuard` decides **before** any signature exists, which is what
//! HotStuff-rs.md §5.8 asks for. Timeout votes and block advertisements cannot equivocate and are
//! signed directly. `GuardedNetwork` stays as a second, independent layer; it shares the same
//! guard and sees the signed vote as an identical re-send.

use hotstuff_rs::types::crypto_primitives::VerifyingKey;
use hotstuff_rs::types::data_types::SignatureBytes;
use hotstuff_rs::types::signer::{LocalKeySigner, SignError, SignRequest, ValidatorSigner};
use std::sync::Arc;

use super::signing_guard::{Authorization, LockContext, SigningGuard, VoteRecord};
use crate::observability::VoteRefusals;

/// `LocalKeySigner` plus the durable signing guard.
pub struct GuardedSigner {
    inner: LocalKeySigner,
    guard: SigningGuard,
    refusals: Arc<VoteRefusals>,
}

impl GuardedSigner {
    /// The guard must belong to the same validator key.
    pub fn new(key: ed25519_dalek::SigningKey, guard: SigningGuard) -> Result<Arc<Self>, String> {
        let inner = LocalKeySigner::new(key);
        if inner.public_key().to_bytes() != guard.identity().validator {
            return Err("signing guard belongs to a different validator key".into());
        }
        Ok(Arc::new(Self {
            inner,
            guard,
            refusals: Arc::new(VoteRefusals::default()),
        }))
    }

    /// Phase votes refused because their view is below the guard record.
    pub fn stale_refused(&self) -> u64 {
        self.refusals.stale()
    }

    /// Phase votes refused because they build on a certificate below the lock floor.
    pub fn below_lock_refused(&self) -> u64 {
        self.refusals.below_lock()
    }

    /// Phase votes refused because a different vote was already signed in that view.
    pub fn conflicting_refused(&self) -> u64 {
        self.refusals.conflicting()
    }

    /// Refusal counters, shared with `/metrics` (`layer="signer"`).
    pub fn refusals(&self) -> Arc<VoteRefusals> {
        Arc::clone(&self.refusals)
    }

    /// Last phase-vote view recorded by the guard, if any.
    pub fn last_signed_view(&self) -> Result<Option<u64>, super::SigningGuardError> {
        Ok(self.guard.last_vote()?.map(|record| record.view))
    }

    /// Highest HotStuff locked view the guard recorded at vote time (TODO 8.5.2).
    pub fn lock_floor(&self) -> Result<u64, super::SigningGuardError> {
        self.guard.lock_floor()
    }

    /// The chain this signer (and its guard) is bound to.
    pub fn chain_id(&self) -> u64 {
        self.guard.identity().chain_id
    }

    fn authorize_phase_vote(&self, request: &SignRequest) -> Result<(), SignError> {
        let SignRequest::PhaseVote {
            chain_id,
            view,
            block,
            phase,
            justify_view,
            locked_view,
        } = request
        else {
            return Ok(());
        };
        let record = VoteRecord::from_parts(view.int(), block.bytes(), *phase);
        let lock = LockContext {
            justify_view: justify_view.int(),
            locked_view: locked_view.int(),
        };
        match self
            .guard
            .authorize_record(chain_id.int(), record, Some(lock))
        {
            Ok(Authorization::Fresh | Authorization::IdenticalResend) => Ok(()),
            Ok(Authorization::Stale { last_view }) => {
                self.refusals.record_stale();
                tracing::warn!(
                    target: crate::observability::logging::SAFETY_LOG_TARGET,
                    "signing guard: refused to sign stale phase vote for view {} (last signed view {last_view})",
                    view.int()
                );
                Err(SignError::Refused(format!(
                    "stale phase vote for view {}",
                    view.int()
                )))
            }
            Ok(Authorization::Conflicting { view }) => {
                self.refusals.record_conflicting();
                tracing::error!(
                    target: crate::observability::logging::SAFETY_LOG_TARGET,
                    "signing guard: REFUSED to sign conflicting phase vote for view {view} (already signed a different vote in this view)"
                );
                Err(SignError::Refused(format!(
                    "conflicting phase vote for view {view}"
                )))
            }
            Ok(Authorization::BelowLock {
                justify_view,
                lock_floor,
            }) => {
                self.refusals.record_below_lock();
                tracing::error!(
                    target: crate::observability::logging::SAFETY_LOG_TARGET,
                    "signing guard: REFUSED phase vote for view {} building on view {justify_view} below lock floor {lock_floor}",
                    view.int()
                );
                Err(SignError::Refused(format!(
                    "phase vote for view {} below lock floor {lock_floor}",
                    view.int()
                )))
            }
            Ok(Authorization::InvalidLockContext { view }) => {
                tracing::error!(
                    target: crate::observability::logging::SAFETY_LOG_TARGET,
                    "signing guard: REFUSED phase vote for view {view}: lock context above the vote's view"
                );
                Err(SignError::Refused(format!(
                    "invalid lock context for view {view}"
                )))
            }
            Err(err) => Err(SignError::Failed(format!("signing guard: {err}"))),
        }
    }
}

impl ValidatorSigner for GuardedSigner {
    fn public_key(&self) -> VerifyingKey {
        self.inner.public_key()
    }

    fn sign(&self, request: &SignRequest) -> Result<SignatureBytes, SignError> {
        // Purpose binding: never sign anything for another chain, vote or not.
        let found = request.chain_id().int();
        if found != self.chain_id() {
            return Err(SignError::Failed(format!(
                "request for chain {found}, signer is bound to chain {}",
                self.chain_id()
            )));
        }
        self.authorize_phase_vote(request)?;
        self.inner.sign(request)
    }
}

#[cfg(test)]
#[path = "signer_tests.rs"]
mod tests;

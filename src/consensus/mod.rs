//! Consensus safety around the vendored `hotstuff_rs` replica (TODO 8.3.2, PRD R07).

pub mod guarded_network;
pub mod safety_halt;
pub mod signer;
pub mod signing_guard;

pub use guarded_network::GuardedNetwork;
pub use safety_halt::SafetyHalt;
pub use signer::GuardedSigner;
pub use signing_guard::{
    Authorization, GuardIdentity, LockContext, OpenMode, SigningGuard, SigningGuardError,
    VoteRecord,
};

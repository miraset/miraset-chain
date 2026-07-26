#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod crypto;
pub mod types;

pub use crypto::{Address, KeyPair, verify_signature};
pub use types::{
    Block, Event, JobStatus, MIRASET_CHAIN_ID, Object, ObjectData, ObjectId, Transaction, Version,
    WorkerStatus, new_object_id, sign_transaction, verify_transaction_signature,
};

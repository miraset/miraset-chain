//! Durable phase-vote signing guard (TODO 8.3.2, PRD R07;
//! `docs/adr/proposal-vote-safety-state.md` D2).
//!
//! The guard lives in its own Redb file, separate from the chain database, so wiping, resyncing
//! or restoring chain data cannot reset it. A phase vote may leave the node only if its view is
//! above the last recorded vote, or if it is byte-for-byte the last recorded vote (an idempotent
//! re-send). The new record is committed with Redb's immediate (fsync) durability before the
//! vote is allowed out.

use hotstuff_rs::hotstuff::messages::PhaseVote;
use hotstuff_rs::hotstuff::types::Phase;
use redb::{Database, DatabaseError, ReadableDatabase, TableDefinition};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const GUARD_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("signing_guard");
const IDENTITY_KEY: &str = "identity";
const LAST_VOTE_KEY: &str = "last_phase_vote";
const FORMAT_VERSION: u8 = 1;
const IDENTITY_LEN: usize = 1 + 8 + 32 + 32;
const VOTE_RECORD_LEN: usize = 1 + 8 + 32 + 1;

/// What the guard is bound to. A guard file never authorizes votes for another identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuardIdentity {
    pub chain_id: u64,
    pub validator: [u8; 32],
    pub genesis_hash: [u8; 32],
}

/// The last phase vote this validator allowed out of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoteRecord {
    pub view: u64,
    pub block: [u8; 32],
    pub phase: u8,
}

/// How to treat a missing guard file (proposal decision 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    /// Refuse to start without an existing guard. Used whenever the chain database is
    /// non-empty: a lost guard must not be silently recreated.
    RequireExisting,
    /// Create the guard if it does not exist (first start, or `--init-signing-guard`).
    CreateIfMissing,
}

impl OpenMode {
    /// Startup rule (proposal decision 4): once the chain database holds data, this validator
    /// may already have signed, so the guard must already exist unless the operator explicitly
    /// asks to initialize it.
    pub fn for_startup(chain_has_data: bool, init_requested: bool) -> Self {
        if chain_has_data && !init_requested {
            Self::RequireExisting
        } else {
            Self::CreateIfMissing
        }
    }
}

/// Outcome of a vote that does not indicate a fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authorization {
    /// View above the last record; the record was durably advanced.
    Fresh,
    /// Identical to the last record; nothing new is signed.
    IdenticalResend,
    /// View below the last record (e.g. catching up after a chain-data wipe). Drop the vote.
    Stale { last_view: u64 },
    /// Same view as the last record but a different block or phase. Signing it would be
    /// equivocation. Drop the vote; this is expected after a chain-data wipe or restore, when
    /// the replica reaches the last signed view again on a different block.
    Conflicting { view: u64 },
}

/// Faults: the guard cannot make a trustworthy decision, so the node must halt.
/// Messages never contain key material.
#[derive(Debug, thiserror::Error)]
pub enum SigningGuardError {
    #[error(
        "signing guard not found at {0}; pass --init-signing-guard only if this validator key has never signed on this chain"
    )]
    Missing(PathBuf),
    #[error("signing guard is already open in another process")]
    InUse,
    #[error("signing guard belongs to a different chain, validator or genesis")]
    IdentityMismatch,
    #[error("signing guard is corrupt: {0}")]
    Corrupt(&'static str),
    #[error("phase vote is for chain {found}, guard is bound to chain {expected}")]
    WrongChain { found: u64, expected: u64 },
    #[error("signing guard storage error: {0}")]
    Storage(String),
}

fn storage_error(err: impl std::fmt::Display) -> SigningGuardError {
    SigningGuardError::Storage(err.to_string())
}

fn phase_code(phase: Phase) -> u8 {
    match phase {
        Phase::Generic => 0,
        Phase::Prepare => 1,
        Phase::Precommit => 2,
        Phase::Commit => 3,
        Phase::Decide => 4,
    }
}

const MAX_PHASE_CODE: u8 = 4;

impl GuardIdentity {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(IDENTITY_LEN);
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&self.chain_id.to_be_bytes());
        out.extend_from_slice(&self.validator);
        out.extend_from_slice(&self.genesis_hash);
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, SigningGuardError> {
        if bytes.len() != IDENTITY_LEN || bytes[0] != FORMAT_VERSION {
            return Err(SigningGuardError::Corrupt("identity record"));
        }
        Ok(Self {
            chain_id: u64::from_be_bytes(bytes[1..9].try_into().expect("8 bytes")),
            validator: bytes[9..41].try_into().expect("32 bytes"),
            genesis_hash: bytes[41..73].try_into().expect("32 bytes"),
        })
    }
}

impl VoteRecord {
    fn from_vote(vote: &PhaseVote) -> Self {
        Self {
            view: vote.view.int(),
            block: vote.block.bytes(),
            phase: phase_code(vote.phase),
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(VOTE_RECORD_LEN);
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&self.view.to_be_bytes());
        out.extend_from_slice(&self.block);
        out.push(self.phase);
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, SigningGuardError> {
        if bytes.len() != VOTE_RECORD_LEN || bytes[0] != FORMAT_VERSION {
            return Err(SigningGuardError::Corrupt("vote record"));
        }
        let phase = bytes[41];
        if phase > MAX_PHASE_CODE {
            return Err(SigningGuardError::Corrupt("vote record phase"));
        }
        Ok(Self {
            view: u64::from_be_bytes(bytes[1..9].try_into().expect("8 bytes")),
            block: bytes[9..41].try_into().expect("32 bytes"),
            phase,
        })
    }
}

struct GuardState {
    db: Database,
    last: Option<VoteRecord>,
}

/// Shared handle; clones refer to the same file and in-memory record.
#[derive(Clone)]
pub struct SigningGuard {
    identity: GuardIdentity,
    state: Arc<Mutex<GuardState>>,
}

impl SigningGuard {
    /// Open the guard at `path`, binding it to `identity` on first creation and refusing a file
    /// bound to anything else. Redb's file lock rejects a second process on the same host.
    pub fn open(
        path: &Path,
        identity: GuardIdentity,
        mode: OpenMode,
    ) -> Result<Self, SigningGuardError> {
        let existed = path.exists();
        if !existed && mode == OpenMode::RequireExisting {
            return Err(SigningGuardError::Missing(path.to_path_buf()));
        }
        let db = Database::create(path).map_err(|err| match err {
            DatabaseError::DatabaseAlreadyOpen => SigningGuardError::InUse,
            other => storage_error(other),
        })?;

        let (stored_identity, last) = read_records(&db)?;
        match stored_identity {
            Some(stored) if stored != identity => return Err(SigningGuardError::IdentityMismatch),
            Some(_) => {}
            None if last.is_some() => {
                return Err(SigningGuardError::Corrupt("vote without identity"));
            }
            None if existed && mode == OpenMode::RequireExisting => {
                return Err(SigningGuardError::Missing(path.to_path_buf()));
            }
            None => write_record(&db, IDENTITY_KEY, &identity.encode())?,
        }

        Ok(Self {
            identity,
            state: Arc::new(Mutex::new(GuardState { db, last })),
        })
    }

    pub fn identity(&self) -> GuardIdentity {
        self.identity
    }

    /// Last vote allowed out, if any.
    pub fn last_vote(&self) -> Result<Option<VoteRecord>, SigningGuardError> {
        Ok(self.lock()?.last)
    }

    /// Decide whether `vote` may leave the node; on `Fresh` the record is durable on return.
    pub fn authorize(&self, vote: &PhaseVote) -> Result<Authorization, SigningGuardError> {
        let found = vote.chain_id.int();
        if found != self.identity.chain_id {
            return Err(SigningGuardError::WrongChain {
                found,
                expected: self.identity.chain_id,
            });
        }
        let candidate = VoteRecord::from_vote(vote);
        let mut state = self.lock()?;
        if let Some(last) = state.last {
            if candidate.view < last.view {
                return Ok(Authorization::Stale {
                    last_view: last.view,
                });
            }
            if candidate.view == last.view {
                return Ok(if candidate == last {
                    Authorization::IdenticalResend
                } else {
                    Authorization::Conflicting {
                        view: candidate.view,
                    }
                });
            }
        }
        write_record(&state.db, LAST_VOTE_KEY, &candidate.encode())?;
        state.last = Some(candidate);
        Ok(Authorization::Fresh)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, GuardState>, SigningGuardError> {
        self.state
            .lock()
            .map_err(|_| SigningGuardError::Storage("signing guard lock poisoned".into()))
    }
}

type StoredRecords = (Option<GuardIdentity>, Option<VoteRecord>);

fn read_records(db: &Database) -> Result<StoredRecords, SigningGuardError> {
    let txn = db.begin_read().map_err(storage_error)?;
    let table = match txn.open_table(GUARD_TABLE) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok((None, None)),
        Err(err) => return Err(storage_error(err)),
    };
    let identity = table
        .get(IDENTITY_KEY)
        .map_err(storage_error)?
        .map(|value| GuardIdentity::decode(value.value()))
        .transpose()?;
    let last = table
        .get(LAST_VOTE_KEY)
        .map_err(storage_error)?
        .map(|value| VoteRecord::decode(value.value()))
        .transpose()?;
    Ok((identity, last))
}

/// Commit one record with Redb's default immediate (fsync) durability.
fn write_record(db: &Database, key: &str, value: &[u8]) -> Result<(), SigningGuardError> {
    let txn = db.begin_write().map_err(storage_error)?;
    {
        let mut table = txn.open_table(GUARD_TABLE).map_err(storage_error)?;
        table.insert(key, value).map_err(storage_error)?;
    }
    txn.commit().map_err(storage_error)
}

#[cfg(test)]
#[path = "signing_guard_tests.rs"]
pub(crate) mod tests;

//! Conversions between the gRPC wire messages and `hotstuff_rs` sign requests.
//!
//! Decoding validates every field (32-byte block hash, known phase, a request present), so the
//! signer never signs a request it cannot interpret.

use hotstuff_rs::hotstuff::types::Phase;
use hotstuff_rs::types::data_types::{BlockHeight, ChainID, CryptoHash, ViewNumber};
use hotstuff_rs::types::signer::SignRequest;

use super::proto;
use super::proto::sign_request::Request;

const BLOCK_HASH_BYTES: usize = 32;

fn phase_to_wire(phase: Phase) -> proto::Phase {
    match phase {
        Phase::Generic => proto::Phase::Generic,
        Phase::Prepare => proto::Phase::Prepare,
        Phase::Precommit => proto::Phase::Precommit,
        Phase::Commit => proto::Phase::Commit,
        Phase::Decide => proto::Phase::Decide,
    }
}

fn phase_from_wire(raw: i32) -> Result<Phase, String> {
    match proto::Phase::try_from(raw) {
        Ok(proto::Phase::Generic) => Ok(Phase::Generic),
        Ok(proto::Phase::Prepare) => Ok(Phase::Prepare),
        Ok(proto::Phase::Precommit) => Ok(Phase::Precommit),
        Ok(proto::Phase::Commit) => Ok(Phase::Commit),
        Ok(proto::Phase::Decide) => Ok(Phase::Decide),
        Ok(proto::Phase::Unspecified) | Err(_) => Err(format!("unknown phase {raw}")),
    }
}

/// Encode a sign request for the wire.
pub fn to_wire(request: &SignRequest) -> proto::SignRequest {
    let request = match request {
        SignRequest::PhaseVote {
            chain_id,
            view,
            block,
            phase,
            justify_view,
            locked_view,
        } => Request::PhaseVote(proto::PhaseVote {
            chain_id: chain_id.int(),
            view: view.int(),
            block: block.bytes().to_vec(),
            phase: phase_to_wire(*phase) as i32,
            justify_view: Some(justify_view.int()),
            locked_view: Some(locked_view.int()),
        }),
        SignRequest::TimeoutVote { chain_id, view } => Request::TimeoutVote(proto::TimeoutVote {
            chain_id: chain_id.int(),
            view: view.int(),
        }),
        SignRequest::AdvertiseBlock {
            chain_id,
            highest_committed_block_height,
        } => Request::AdvertiseBlock(proto::AdvertiseBlock {
            chain_id: chain_id.int(),
            highest_committed_block_height: highest_committed_block_height.int(),
        }),
        SignRequest::TransportIdentity {
            chain_id,
            noise_static_pubkey,
            not_after,
        } => Request::TransportIdentity(proto::TransportIdentity {
            chain_id: chain_id.int(),
            noise_static_pubkey: noise_static_pubkey.to_vec(),
            not_after: *not_after,
        }),
    };
    proto::SignRequest {
        request: Some(request),
    }
}

/// Decode and validate a sign request from the wire.
pub fn from_wire(request: &proto::SignRequest) -> Result<SignRequest, String> {
    match request.request.as_ref().ok_or("empty sign request")? {
        Request::PhaseVote(vote) => {
            let block: [u8; BLOCK_HASH_BYTES] = vote
                .block
                .as_slice()
                .try_into()
                .map_err(|_| format!("block hash must be {BLOCK_HASH_BYTES} bytes"))?;
            Ok(SignRequest::PhaseVote {
                chain_id: ChainID::new(vote.chain_id),
                view: ViewNumber::new(vote.view),
                block: CryptoHash::new(block),
                phase: phase_from_wire(vote.phase)?,
                justify_view: ViewNumber::new(
                    vote.justify_view
                        .ok_or("phase vote without lock context (justify_view)")?,
                ),
                locked_view: ViewNumber::new(
                    vote.locked_view
                        .ok_or("phase vote without lock context (locked_view)")?,
                ),
            })
        }
        Request::TimeoutVote(vote) => Ok(SignRequest::TimeoutVote {
            chain_id: ChainID::new(vote.chain_id),
            view: ViewNumber::new(vote.view),
        }),
        Request::AdvertiseBlock(advertise) => Ok(SignRequest::AdvertiseBlock {
            chain_id: ChainID::new(advertise.chain_id),
            highest_committed_block_height: BlockHeight::new(
                advertise.highest_committed_block_height,
            ),
        }),
        Request::TransportIdentity(identity) => Ok(SignRequest::TransportIdentity {
            chain_id: ChainID::new(identity.chain_id),
            noise_static_pubkey: identity
                .noise_static_pubkey
                .as_slice()
                .try_into()
                .map_err(|_| "noise static key must be 32 bytes".to_string())?,
            not_after: identity.not_after,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requests() -> Vec<SignRequest> {
        let chain_id = ChainID::new(1337);
        let mut all: Vec<SignRequest> = [
            Phase::Generic,
            Phase::Prepare,
            Phase::Precommit,
            Phase::Commit,
            Phase::Decide,
        ]
        .into_iter()
        .map(|phase| SignRequest::PhaseVote {
            chain_id,
            view: ViewNumber::new(9),
            block: CryptoHash::new([4u8; 32]),
            phase,
            justify_view: ViewNumber::new(8),
            locked_view: ViewNumber::new(6),
        })
        .collect();
        all.push(SignRequest::TimeoutVote {
            chain_id,
            view: ViewNumber::new(10),
        });
        all.push(SignRequest::AdvertiseBlock {
            chain_id,
            highest_committed_block_height: BlockHeight::new(3),
        });
        all.push(SignRequest::TransportIdentity {
            chain_id,
            noise_static_pubkey: [9u8; 32],
            not_after: 1_800_000_000,
        });
        all
    }

    #[test]
    fn test_every_request_round_trips_with_identical_signing_bytes() {
        for request in requests() {
            let decoded = from_wire(&to_wire(&request)).unwrap();
            assert!(decoded == request);
            assert_eq!(decoded.signing_bytes(), request.signing_bytes());
        }
    }

    #[test]
    fn test_invalid_wire_requests_are_rejected() {
        let empty = proto::SignRequest { request: None };
        assert!(from_wire(&empty).is_err());

        let vote_with = |block: Vec<u8>, phase: i32, context: Option<u64>| proto::SignRequest {
            request: Some(Request::PhaseVote(proto::PhaseVote {
                chain_id: 1,
                view: 1,
                block,
                phase,
                justify_view: context,
                locked_view: context,
            })),
        };
        let vote = |block: Vec<u8>, phase: i32| vote_with(block, phase, Some(0));
        let generic = proto::Phase::Generic as i32;
        assert!(from_wire(&vote(vec![0u8; 31], generic)).is_err());
        assert!(from_wire(&vote(vec![0u8; 33], generic)).is_err());
        assert!(from_wire(&vote(vec![0u8; 32], proto::Phase::Unspecified as i32)).is_err());
        assert!(from_wire(&vote(vec![0u8; 32], 99)).is_err());
        assert!(from_wire(&vote(vec![0u8; 32], generic)).is_ok());
        // TODO 8.5.2: an older sender without lock context is refused, not silently unchecked.
        assert!(from_wire(&vote_with(vec![0u8; 32], generic, None)).is_err());
        let identity = |key: Vec<u8>| proto::SignRequest {
            request: Some(Request::TransportIdentity(proto::TransportIdentity {
                chain_id: 1,
                noise_static_pubkey: key,
                not_after: 1_800_000_000,
            })),
        };
        assert!(from_wire(&identity(vec![0u8; 31])).is_err());
        assert!(from_wire(&identity(vec![0u8; 32])).is_ok());
    }
}

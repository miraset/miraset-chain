//! MIRASET PATCH #8 regression test (`MIRASET_PATCHES.md`).
//!
//! A replica already at view 10 with no stored TC (e.g. after a chain-data wipe) receives a
//! timeout vote carrying an older fallback TC for view 0. Upstream tried to "advance" to view 1,
//! violating `update_view`'s precondition; the error killed the algorithm thread. The patched
//! pacemaker keeps the newer `highest_tc` and stays at view 10.

#[path = "../../tests/common/mem_db.rs"]
mod mem_db;

use super::*;
use crate::block_tree::accessors::internal::BlockTreeSingleton;
use crate::pacemaker::types::TimeoutCertificate;
use crate::types::data_types::{Power, SignatureSet};
use crate::types::update_sets::{AppStateUpdates, ValidatorSetUpdates};
use ed25519_dalek::SigningKey;
use mem_db::MemDB;

const CHAIN: ChainID = ChainID::new(7);
const EPOCH_LENGTH: u32 = 10;
const CURRENT_VIEW: u64 = 10;
const OLD_TC_VIEW: u64 = 0;

#[derive(Clone)]
struct NullNetwork;

impl Network for NullNetwork {
    fn init_validator_set(&mut self, _: ValidatorSet) {}
    fn update_validator_set(&mut self, _: ValidatorSetUpdates) {}
    fn broadcast(&mut self, _: Message) {}
    fn send(&mut self, _: VerifyingKey, _: Message) {}
    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        None
    }
}

fn timeout_vote(
    key: &SigningKey,
    view: u64,
    highest_tc: Option<TimeoutCertificate>,
) -> TimeoutVote {
    let keypair = Keypair::new(key.clone());
    match PacemakerMessage::timeout_vote(&keypair, CHAIN, ViewNumber::new(view), highest_tc)
        .expect("local signer never refuses")
    {
        PacemakerMessage::TimeoutVote(vote) => vote,
        PacemakerMessage::AdvanceView(_) => unreachable!("timeout_vote builds a TimeoutVote"),
    }
}

/// A TC for `view` signed by every validator in `validator_set`.
fn full_tc(view: u64, keys: &[&SigningKey], validator_set: &ValidatorSet) -> TimeoutCertificate {
    let mut signatures = SignatureSet::new(validator_set.len());
    for key in keys {
        let position = validator_set
            .position(&key.verifying_key())
            .expect("signer is a validator");
        signatures.set(position, Some(timeout_vote(key, view, None).signature));
    }
    TimeoutCertificate {
        chain_id: CHAIN,
        view: ViewNumber::new(view),
        signatures,
    }
}

#[test]
fn test_older_fallback_tc_does_not_move_the_view_backwards() {
    let me = SigningKey::from_bytes(&[1u8; 32]);
    let peer = SigningKey::from_bytes(&[2u8; 32]);
    let mut validator_set = ValidatorSet::new();
    validator_set.put(&me.verifying_key(), Power::new(1));
    validator_set.put(&peer.verifying_key(), Power::new(1));
    let validator_set_state =
        ValidatorSetState::new(validator_set.clone(), validator_set.clone(), None, true);

    // Fresh block tree: no highest TC, as after a chain-data wipe.
    let mut block_tree = BlockTreeSingleton::new(MemDB::new());
    block_tree
        .initialize(&AppStateUpdates::new(), &validator_set_state)
        .unwrap();
    assert!(block_tree.highest_tc().unwrap().is_none());

    // The replica has already reached view 10 (through phase certificates).
    let config = PacemakerConfiguration {
        chain_id: CHAIN,
        keypair: Keypair::new(me.clone()),
        epoch_length: EpochLength::new(EPOCH_LENGTH),
        max_view_time: Duration::from_secs(1),
    };
    let mut pacemaker = Pacemaker::new(
        config,
        SenderHandle::new(NullNetwork),
        ViewNumber::new(CURRENT_VIEW),
        &validator_set_state,
        None,
    )
    .unwrap();

    // The peer's timeout vote for the current epoch-change view carries an older, valid TC.
    // One vote of power 1 is below the quorum of 2, so the fallback-TC branch runs.
    let old_tc = full_tc(OLD_TC_VIEW, &[&me, &peer], &validator_set);
    let vote = timeout_vote(&peer, CURRENT_VIEW, Some(old_tc));

    pacemaker
        .on_receive_msg(
            PacemakerMessage::TimeoutVote(vote),
            &peer.verifying_key(),
            &mut block_tree,
        )
        .expect("an older fallback TC must not be an error");

    assert_eq!(pacemaker.query().view, ViewNumber::new(CURRENT_VIEW));
    let stored = block_tree
        .highest_tc()
        .unwrap()
        .expect("fallback TC is stored");
    assert_eq!(stored.view, ViewNumber::new(OLD_TC_VIEW));
}

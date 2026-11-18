//! MIRASET PATCH #20 regression test (`MIRASET_PATCHES.md`): a replica's own `AdvertiseBlock`
//! (delivered back to it by a broadcast) does not make it a sync server of itself.

use super::*;
use ed25519_dalek::SigningKey;

#[test]
fn test_own_advertisement_does_not_register_a_sync_server() {
    let local = SigningKey::from_bytes(&[7u8; 32]).verifying_key();
    let peer = SigningKey::from_bytes(&[8u8; 32]).verifying_key();
    let mut state = BlockSyncClientState::initialize();

    state.register_advertised_sync_server(&local, local, BlockHeight::new(10));
    state.register_advertised_sync_server(&local, peer, BlockHeight::new(10));

    assert!(!state.available_sync_servers.contains_key(&local));
    assert!(state.available_sync_servers.contains_key(&peer));
    assert_eq!(
        state.random_sync_server(&Some(BlockHeight::new(0))),
        Some(peer)
    );
}

//! MIRASET PATCH #13 regression test (`MIRASET_PATCHES.md`): a blacklisting stays in force
//! until it expires, then ends.

use super::*;
use ed25519_dalek::SigningKey;

const EXPIRY: Duration = Duration::from_secs(1);

#[test]
fn test_blacklisting_lasts_until_its_expiry() {
    let server = SigningKey::from_bytes(&[5u8; 32]).verifying_key();
    let mut state = BlockSyncClientState::initialize();
    state.register_or_update_sync_server(server, BlockHeight::new(10));
    state.blacklist_sync_server(server, EXPIRY);
    assert!(!state.available_sync_servers.contains_key(&server));

    // Upstream lifted the blacklisting here, on the first tick after it was imposed.
    state.remove_expired_blacklisted_servers();
    assert!(
        state.blacklist_contains_server_address(&server),
        "blacklisting lifted before it expired"
    );

    std::thread::sleep(EXPIRY + Duration::from_millis(50));
    state.remove_expired_blacklisted_servers();
    assert!(!state.blacklist_contains_server_address(&server));
}

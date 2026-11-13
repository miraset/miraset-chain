use ed25519_dalek::SigningKey;
use hotstuff_rs::block_sync::messages::{BlockSyncMessage, BlockSyncRequest};
use hotstuff_rs::networking::messages::Message;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::data_types::{BlockHeight, ChainID};
use miraset_chain::network::{TcpNetwork, TransportIdentity};
use std::collections::HashMap;
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind temp listener");
    listener.local_addr().expect("valid address").port()
}

#[test]
fn test_tcp_network_direct_send_and_broadcast() {
    let sk1 = SigningKey::from_bytes(&[1u8; 32]);
    let vk1 = sk1.verifying_key();
    let port1 = free_port();
    let addr1 = format!("127.0.0.1:{}", port1).parse().unwrap();

    let sk2 = SigningKey::from_bytes(&[2u8; 32]);
    let vk2 = sk2.verifying_key();
    let port2 = free_port();
    let addr2 = format!("127.0.0.1:{}", port2).parse().unwrap();

    let mut peer_map = HashMap::new();
    peer_map.insert(vk1, addr1);
    peer_map.insert(vk2, addr2);

    let identity =
        |key: &SigningKey| TransportIdentity::for_key(key.clone(), 42, [0u8; 32]).unwrap();
    let (mut net1, runtime1) = TcpNetwork::bind(identity(&sk1), addr1, peer_map.clone()).unwrap();
    let (mut net2, runtime2) = TcpNetwork::bind(identity(&sk2), addr2, peer_map).unwrap();

    // Small warmup delay for server listener threads to start accept loops
    thread::sleep(Duration::from_millis(100));

    // Send a message from net1 to net2
    let msg = Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(BlockSyncRequest {
        chain_id: ChainID::new(42),
        start_height: BlockHeight::new(10),
        limit: 5,
    }));

    net1.send(vk2, msg.clone());

    // Wait and receive on net2
    let mut received = None;
    for _ in 0..30 {
        if let Some((sender, received_msg)) = net2.recv() {
            received = Some((sender, received_msg));
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    assert!(received.is_some(), "net2 did not receive frame from net1");
    let (sender, received_msg) = received.unwrap();
    assert_eq!(sender, vk1);
    match received_msg {
        Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(req)) => {
            assert_eq!(req.start_height, BlockHeight::new(10));
            assert_eq!(req.limit, 5);
        }
        _ => panic!("unexpected message variant"),
    }

    // Broadcast from net2: both net1 and local net2 should receive
    let bcast_msg =
        Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(BlockSyncRequest {
            chain_id: ChainID::new(42),
            start_height: BlockHeight::new(20),
            limit: 1,
        }));

    net2.broadcast(bcast_msg);

    let mut net1_got_bcast = false;
    for _ in 0..30 {
        if let Some((sender, Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(req)))) =
            net1.recv()
            && sender == vk2
            && req.start_height == BlockHeight::new(20)
            && req.limit == 1
        {
            net1_got_bcast = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(net1_got_bcast, "net1 did not receive broadcast from net2");

    drop(runtime1);
    drop(runtime2);
}

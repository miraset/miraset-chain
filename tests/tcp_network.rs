use ed25519_dalek::SigningKey;
use hotstuff_rs::block_sync::messages::{BlockSyncMessage, BlockSyncRequest};
use hotstuff_rs::networking::messages::Message;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::data_types::{BlockHeight, ChainID};
use miraset_chain::network::TcpNetwork;
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

    let (mut net1, runtime1) = TcpNetwork::bind(vk1, addr1, peer_map.clone()).unwrap();
    let (mut net2, runtime2) = TcpNetwork::bind(vk2, addr2, peer_map).unwrap();

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

// ---- TODO 8.5.2 F6: transport memory bounds ----------------------------------------------

use borsh::BorshSerialize;
use hotstuff_rs::block_sync::messages::BlockSyncResponse;
use hotstuff_rs::hotstuff::types::PhaseCertificate;
use hotstuff_rs::types::block::Block;
use hotstuff_rs::types::data_types::{CryptoHash, Data, Datum};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Instant;

const LARGE_PAYLOAD: usize = 1 << 20;
/// The receiver's per-peer inbound budget is 64 MiB (4 × 16 MiB frames).
const BUDGET_FRAMES: usize = 64;
/// Kernel socket buffers on loopback can hold a few more megabytes beyond the budget.
const SOCKET_SLACK_FRAMES: usize = 32;
const FLOOD_FRAMES: usize = 300;

/// A one-node transport that accepts frames claiming `peer`'s key.
fn receiver_with_peer(peer: &SigningKey) -> (TcpNetwork, impl Drop, SocketAddr) {
    let me = SigningKey::from_bytes(&[30u8; 32]).verifying_key();
    let addr: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    let peer_addr: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    let peers = HashMap::from([(me, addr), (peer.verifying_key(), peer_addr)]);
    let (net, runtime) = TcpNetwork::bind(me, addr, peers).unwrap();
    thread::sleep(Duration::from_millis(100));
    (net, runtime, addr)
}

fn frame_for(sender: &SigningKey, message: &Message) -> Vec<u8> {
    let body = message.try_to_vec().unwrap();
    let length = u32::try_from(32 + body.len()).unwrap();
    let mut frame = length.to_be_bytes().to_vec();
    frame.extend_from_slice(&sender.verifying_key().to_bytes());
    frame.extend_from_slice(&body);
    frame
}

/// A valid message of about 1 MiB.
fn large_message() -> Message {
    let block = Block::new(
        BlockHeight::new(1),
        PhaseCertificate::genesis_pc(),
        CryptoHash::new([0u8; 32]),
        Data::new(vec![Datum::new(vec![7u8; LARGE_PAYLOAD])]),
    );
    Message::from(BlockSyncResponse {
        blocks: vec![block],
        highest_pc: PhaseCertificate::genesis_pc(),
    })
}

#[test]
fn test_non_member_frame_is_refused_before_its_body_is_read() {
    let member = SigningKey::from_bytes(&[31u8; 32]);
    let outsider = SigningKey::from_bytes(&[32u8; 32]);
    let (_net, _runtime, addr) = receiver_with_peer(&member);

    let mut stream = TcpStream::connect(addr).unwrap();
    let mut header = u32::try_from(16 * 1024 * 1024)
        .unwrap()
        .to_be_bytes()
        .to_vec();
    header.extend_from_slice(&outsider.verifying_key().to_bytes());
    stream.write_all(&header).unwrap();

    // The receiver closes the connection at once; it neither allocates nor waits for 16 MiB.
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let started = Instant::now();
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => {}
        Err(e) if e.kind() == ErrorKind::ConnectionReset => {}
        other => panic!(
            "connection still open after {:?}: {other:?}",
            started.elapsed()
        ),
    }
}

#[test]
fn test_inbound_bytes_from_one_peer_are_bounded_by_backpressure() {
    let peer = SigningKey::from_bytes(&[33u8; 32]);
    let (mut net, _runtime, addr) = receiver_with_peer(&peer);
    let frame = frame_for(&peer, &large_message());

    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    // The replica never calls `recv`: frames pile up until the peer's budget is full and the
    // reader stops reading, so the sender's writes block.
    let mut sent = 0;
    while sent < FLOOD_FRAMES && stream.write_all(&frame).is_ok() {
        sent += 1;
    }
    assert!(
        sent < BUDGET_FRAMES + SOCKET_SLACK_FRAMES,
        "receiver kept buffering: {sent} frames of 1 MiB accepted"
    );

    // Taking messages returns budget. The flooding connection may now hold a partial frame, so
    // prove the reader resumes with a fresh connection and a message we can recognise.
    drop(stream);
    let marker = Message::from(BlockSyncRequest {
        chain_id: ChainID::new(42),
        start_height: BlockHeight::new(424_242),
        limit: 1,
    });
    let mut fresh = TcpStream::connect(addr).unwrap();
    fresh
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    fresh.write_all(&frame_for(&peer, &marker)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut drained = 0;
    let found = loop {
        assert!(
            Instant::now() < deadline,
            "marker not delivered after draining {drained} messages"
        );
        match net.recv() {
            Some((_, Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(r))))
                if r.start_height == BlockHeight::new(424_242) =>
            {
                break true;
            }
            Some(_) => drained += 1,
            None => {}
        }
    };
    assert!(found);
    assert!(drained > 0, "flooded frames were never delivered");
}

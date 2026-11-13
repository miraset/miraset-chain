//! Inbound transport tests with real authenticated sessions (TODO 8.4.1; memory bounds 8.5.2 F6).

use super::*;
use crate::network::TcpNetwork;
use borsh::BorshSerialize;
use ed25519_dalek::SigningKey;
use hotstuff_rs::block_sync::messages::{BlockSyncMessage, BlockSyncRequest, BlockSyncResponse};
use hotstuff_rs::hotstuff::types::PhaseCertificate;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::block::Block;
use hotstuff_rs::types::data_types::{BlockHeight, ChainID, CryptoHash, Data, Datum};
use std::io::Write;
use std::net::{SocketAddr, TcpListener};

const CHAIN: u64 = 42;
const GENESIS: [u8; 32] = [0u8; 32];
const RECEIVER: u8 = 30;
const PEER: u8 = 31;
const OUTSIDER: u8 = 39;
const LARGE_PAYLOAD: usize = 1 << 20;
/// The receiver's per-peer inbound budget is 64 MiB.
const BUDGET_FRAMES: usize = 64;
/// Loopback socket buffers can hold a few more megabytes beyond the budget.
const SOCKET_SLACK_FRAMES: usize = 32;
const FLOOD_FRAMES: usize = 300;
const WAIT: Duration = Duration::from_secs(10);

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn identity(seed: u8) -> Arc<TransportIdentity> {
    TransportIdentity::for_key(key(seed), CHAIN, GENESIS).unwrap()
}

fn free_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn address_book(seeds: &[u8]) -> HashMap<VerifyingKey, String> {
    seeds
        .iter()
        .map(|s| (key(*s).verifying_key(), free_addr().to_string()))
        .collect()
}

/// A receiver whose only peer is `PEER`.
fn receiver() -> (TcpNetwork, crate::network::NetworkRuntime, SocketAddr) {
    let addr = free_addr();
    let peers = HashMap::from([
        (key(RECEIVER).verifying_key(), addr.to_string()),
        (key(PEER).verifying_key(), free_addr().to_string()),
    ]);
    let (net, runtime) = TcpNetwork::bind_with_hosts(identity(RECEIVER), addr, peers).unwrap();
    thread::sleep(Duration::from_millis(100));
    (net, runtime, addr)
}

/// Dial the receiver as `seed` and complete the handshake.
fn dial(seed: u8, addr: SocketAddr) -> io::Result<(TcpStream, Session)> {
    let mut stream = TcpStream::connect(addr)?;
    let peers = address_book(&[RECEIVER, seed]);
    let session = identity(seed).initiate(
        &mut stream,
        &peers,
        &key(RECEIVER).verifying_key(),
        Instant::now() + Duration::from_secs(2),
    )?;
    Ok((stream, session))
}

fn message_bytes(message: &Message) -> Vec<u8> {
    message.try_to_vec().unwrap()
}

fn marker(height: u64) -> Message {
    Message::from(BlockSyncRequest {
        chain_id: ChainID::new(CHAIN),
        start_height: BlockHeight::new(height),
        limit: 1,
    })
}

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

/// Receive until `height`'s marker arrives; returns how many other messages came first.
fn receive_marker(net: &mut TcpNetwork, height: u64) -> usize {
    let deadline = Instant::now() + WAIT;
    let mut others = 0;
    loop {
        assert!(Instant::now() < deadline, "marker {height} not delivered");
        match net.recv() {
            Some((origin, Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(r))))
                if r.start_height == BlockHeight::new(height) =>
            {
                assert_eq!(
                    origin,
                    key(PEER).verifying_key(),
                    "origin is the proven key"
                );
                return others;
            }
            Some(_) => others += 1,
            None => {}
        }
    }
}

fn wait_for_rejection(net: &TcpNetwork, reason: Rejection) {
    let deadline = Instant::now() + WAIT;
    while net.rejections().get(reason) == 0 {
        assert!(
            Instant::now() < deadline,
            "{} not recorded",
            reason.as_str()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn connection_closed(stream: &mut TcpStream) -> bool {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => true,
        Err(e) => e.kind() == io::ErrorKind::ConnectionReset,
        Ok(_) => false,
    }
}

#[test]
fn test_authenticated_peer_messages_arrive_with_the_proven_origin() {
    let (mut net, _runtime, addr) = receiver();
    let (mut stream, mut session) = dial(PEER, addr).unwrap();
    let frame = session.seal_frame(&message_bytes(&marker(7))).unwrap();
    stream.write_all(&frame).unwrap();
    assert_eq!(receive_marker(&mut net, 7), 0);
    assert_eq!(
        net.traffic().received(),
        frame.len() as u64,
        "received bytes are counted on the wire"
    );
}

/// In Noise `XX` the initiator finishes when it sends the last message, before the responder
/// checks it, so the outsider's own handshake "succeeds"; the receiver then refuses it, closes
/// the connection, and delivers nothing.
#[test]
fn test_non_validator_is_refused_in_the_handshake() {
    let (mut net, _runtime, addr) = receiver();
    let mut stream = TcpStream::connect(addr).unwrap();
    let peers = address_book(&[RECEIVER, OUTSIDER]);
    let mut session = identity(OUTSIDER)
        .initiate(
            &mut stream,
            &peers,
            &key(RECEIVER).verifying_key(),
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
    let _ = stream.write_all(&session.seal_frame(&message_bytes(&marker(5))).unwrap());
    wait_for_rejection(&net, Rejection::UnknownValidator);
    assert!(connection_closed(&mut stream));
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        assert!(net.recv().is_none(), "outsider's message was delivered");
    }
}

#[test]
fn test_inbound_bytes_from_one_peer_are_bounded_by_backpressure() {
    let (mut net, _runtime, addr) = receiver();
    let (mut stream, mut session) = dial(PEER, addr).unwrap();
    let body = message_bytes(&large_message());
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    // The replica never calls `recv`: frames pile up until the budget is full and the reader
    // stops reading, so the sender's writes block.
    let mut sent = 0;
    while sent < FLOOD_FRAMES {
        let frame = session.seal_frame(&body).unwrap();
        if stream.write_all(&frame).is_err() {
            break;
        }
        sent += 1;
    }
    assert!(
        sent < BUDGET_FRAMES + SOCKET_SLACK_FRAMES,
        "receiver kept buffering: {sent} frames of 1 MiB accepted"
    );

    // Taking messages returns budget; a fresh session (the flooding one may hold a partial
    // frame) is read again.
    drop(stream);
    let (mut fresh, mut session) = dial(PEER, addr).unwrap();
    fresh
        .write_all(&session.seal_frame(&message_bytes(&marker(9))).unwrap())
        .unwrap();
    assert!(
        receive_marker(&mut net, 9) > 0,
        "flooded frames were never delivered"
    );
}

#[test]
fn test_new_connection_from_a_validator_replaces_the_old_one() {
    let (mut net, _runtime, addr) = receiver();
    let (mut first, _first_session) = dial(PEER, addr).unwrap();
    let (mut second, mut second_session) = dial(PEER, addr).unwrap();
    assert!(connection_closed(&mut first), "old connection still open");
    second
        .write_all(
            &second_session
                .seal_frame(&message_bytes(&marker(11)))
                .unwrap(),
        )
        .unwrap();
    receive_marker(&mut net, 11);
}

#[test]
fn test_pending_handshake_flood_does_not_block_a_validator() {
    let (mut net, _runtime, addr) = receiver();
    // One host can hold only MAX_PENDING_PER_IP slots; the rest of its idle connections are
    // refused at once, so they cannot fill the slots a validator needs.
    let idle: Vec<TcpStream> = (0..MAX_PENDING_HANDSHAKES + 4)
        .map(|_| TcpStream::connect(addr).unwrap())
        .collect();
    wait_for_rejection(&net, Rejection::SlotsFull);
    // The idle ones time out of their slots; then a real validator gets in.
    let deadline = Instant::now() + WAIT;
    let (mut stream, mut session) = loop {
        match dial(PEER, addr) {
            Ok(connected) => break connected,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(200)),
            Err(error) => panic!("validator never got a slot: {error}"),
        }
    };
    stream
        .write_all(&session.seal_frame(&message_bytes(&marker(13))).unwrap())
        .unwrap();
    receive_marker(&mut net, 13);
    assert!(net.rejections().get(Rejection::HandshakeTimeout) > 0);
    drop(idle);
}

#[test]
fn test_oversized_frame_length_closes_the_connection() {
    let (net, _runtime, addr) = receiver();
    let (mut stream, _session) = dial(PEER, addr).unwrap();
    let length = u32::try_from(MAX_FRAME_BYTES + 1).unwrap();
    stream.write_all(&length.to_be_bytes()).unwrap();
    assert!(connection_closed(&mut stream));
    wait_for_rejection(&net, Rejection::FrameTooLarge);
}

#[test]
fn test_tampered_chunk_closes_the_connection() {
    let (net, _runtime, addr) = receiver();
    let (mut stream, mut session) = dial(PEER, addr).unwrap();
    let mut frame = session.seal_frame(&message_bytes(&marker(15))).unwrap();
    let last = frame.len() - 1;
    frame[last] ^= 1;
    stream.write_all(&frame).unwrap();
    assert!(connection_closed(&mut stream));
    wait_for_rejection(&net, Rejection::DecryptFailed);
}

/// While one host keeps its idle connections open, a validator from another address still
/// completes its handshake at once (per-IP pending cap). Linux routes all of 127.0.0.0/8 to
/// loopback.
#[cfg(target_os = "linux")]
#[test]
fn test_sustained_flood_from_one_host_does_not_lock_out_another() {
    let (mut net, _runtime, addr) = receiver();
    let idle: Vec<TcpStream> = (0..MAX_PENDING_HANDSHAKES + 4)
        .map(|_| TcpStream::connect(addr).unwrap())
        .collect();
    wait_for_rejection(&net, Rejection::SlotsFull);

    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    socket
        .bind(&"127.0.0.2:0".parse::<SocketAddr>().unwrap().into())
        .unwrap();
    socket.connect(&addr.into()).unwrap();
    let mut stream: TcpStream = socket.into();
    let peers = address_book(&[RECEIVER, PEER]);
    let mut session = identity(PEER)
        .initiate(
            &mut stream,
            &peers,
            &key(RECEIVER).verifying_key(),
            Instant::now() + Duration::from_secs(2),
        )
        .expect("validator handshake while the flood is held");
    stream
        .write_all(&session.seal_frame(&message_bytes(&marker(17))).unwrap())
        .unwrap();
    receive_marker(&mut net, 17);
    drop(idle);
}

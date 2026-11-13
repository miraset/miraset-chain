//! Adversarial transport tests through the public API (TODO 8.4.1, proposal §6 "N1").
//!
//! Before 8.4.1 a frame named its sender, so any host reaching a P2P port could inject a
//! proposal as the leader. Now every connection must complete a Noise handshake that proves a
//! genesis validator key; anything else is refused and counted, and never reaches the replica.

use borsh::BorshSerialize;
use ed25519_dalek::SigningKey;
use hotstuff_rs::hotstuff::messages::{HotStuffMessage, Proposal};
use hotstuff_rs::hotstuff::types::PhaseCertificate;
use hotstuff_rs::networking::messages::Message;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::block::Block;
use hotstuff_rs::types::data_types::{BlockHeight, ChainID, CryptoHash, Data, ViewNumber};
use miraset_chain::network::{Rejection, TcpNetwork, TransportIdentity};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

const CHAIN: u64 = 77;
const GENESIS: [u8; 32] = [3u8; 32];
const WAIT: Duration = Duration::from_secs(10);

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn free_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

/// Validator 1 listening; validator 2 (the "leader") is configured but offline.
fn victim() -> (TcpNetwork, impl Drop, SocketAddr) {
    let addr = free_addr();
    let peers = HashMap::from([
        (key(1).verifying_key(), addr),
        (key(2).verifying_key(), free_addr()),
    ]);
    let identity = TransportIdentity::for_key(key(1), CHAIN, GENESIS).unwrap();
    let (net, runtime) = TcpNetwork::bind(identity, addr, peers).unwrap();
    thread::sleep(Duration::from_millis(100));
    (net, runtime, addr)
}

fn spoofed_proposal() -> Message {
    let block = Block::new(
        BlockHeight::new(1),
        PhaseCertificate::genesis_pc(),
        CryptoHash::new([0u8; 32]),
        Data::new(vec![]),
    );
    Message::from(HotStuffMessage::Proposal(Proposal {
        chain_id: ChainID::new(CHAIN),
        view: ViewNumber::new(1),
        block,
    }))
}

fn nothing_delivered(net: &mut TcpNetwork, for_how_long: Duration) {
    let deadline = Instant::now() + for_how_long;
    while Instant::now() < deadline {
        assert!(
            net.recv().is_none(),
            "an unauthenticated message reached the replica"
        );
    }
}

fn closed_by_peer(stream: &mut TcpStream) -> bool {
    stream.set_read_timeout(Some(WAIT)).unwrap();
    let mut buf = [0u8; 256];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return true,
            Ok(_) => continue, // handshake bytes the victim sent before refusing
            Err(e) => return e.kind() == ErrorKind::ConnectionReset,
        }
    }
}

/// N1: the pre-8.4.1 frame format (length ‖ claimed sender key ‖ borsh message) naming the
/// leader is refused before any message is decoded.
#[test]
fn test_raw_spoofed_leader_proposal_never_reaches_the_replica() {
    let (mut net, _runtime, addr) = victim();
    let body = spoofed_proposal().try_to_vec().unwrap();
    let mut frame = u32::try_from(32 + body.len())
        .unwrap()
        .to_be_bytes()
        .to_vec();
    frame.extend_from_slice(&key(2).verifying_key().to_bytes());
    frame.extend_from_slice(&body);

    let mut stream = TcpStream::connect(addr).unwrap();
    // The victim may reset the connection mid-write; that is a valid refusal too.
    let _ = stream.write_all(&frame);
    assert!(closed_by_peer(&mut stream), "victim kept the connection");
    nothing_delivered(&mut net, Duration::from_millis(500));
    let rejections = net.rejections();
    let deadline = Instant::now() + WAIT;
    while rejections.get(Rejection::HandshakeFailed) + rejections.get(Rejection::HandshakeTimeout)
        == 0
    {
        assert!(Instant::now() < deadline, "refusal not counted");
        thread::sleep(Duration::from_millis(20));
    }
}

/// A host that speaks Noise correctly but holds no genesis validator key (here: it claims to be
/// on the network with its own key) is refused and delivers nothing.
#[test]
fn test_noise_capable_outsider_is_refused() {
    let (mut net, _runtime, victim_addr) = victim();
    // The outsider runs a full transport of its own that lists the victim as a peer.
    let outsider_addr = free_addr();
    let outsider_peers = HashMap::from([
        (key(9).verifying_key(), outsider_addr),
        (key(1).verifying_key(), victim_addr),
    ]);
    let identity = TransportIdentity::for_key(key(9), CHAIN, GENESIS).unwrap();
    let (mut outsider, _outsider_runtime) =
        TcpNetwork::bind(identity, outsider_addr, outsider_peers).unwrap();
    let deadline = Instant::now() + WAIT;
    while net.rejections().get(Rejection::UnknownValidator) == 0 {
        assert!(Instant::now() < deadline, "outsider never refused");
        outsider.send(key(1).verifying_key(), spoofed_proposal());
        thread::sleep(Duration::from_millis(100));
    }
    nothing_delivered(&mut net, Duration::from_millis(500));
}

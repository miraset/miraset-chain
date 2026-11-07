use borsh::{BorshDeserialize, BorshSerialize};
use hotstuff_rs::networking::messages::Message;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::crypto_primitives::VerifyingKey;
use hotstuff_rs::types::update_sets::ValidatorSetUpdates;
use hotstuff_rs::types::validator_set::ValidatorSet;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_INBOUND_CONNECTIONS: usize = 64;
const NETWORK_QUEUE_CAPACITY: usize = 4096;

struct OutboundMessage {
    destination: Option<VerifyingKey>,
    message: Message,
}

/// Runtime background threads managing TCP send and receive loops.
pub struct NetworkRuntime {
    shutdown: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Drop for NetworkRuntime {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// Length-delimited framed TCP Network adapter implementing `hotstuff_rs::networking::network::Network`.
#[derive(Clone)]
pub struct TcpNetwork {
    peer_addresses: Arc<HashMap<VerifyingKey, SocketAddr>>,
    outbound: SyncSender<OutboundMessage>,
    inbound: Arc<Mutex<Receiver<(VerifyingKey, Message)>>>,
}

impl TcpNetwork {
    /// Bind a new TCP server on `address` with a configured peer address table.
    pub fn bind(
        local_key: VerifyingKey,
        address: SocketAddr,
        peer_addresses: HashMap<VerifyingKey, SocketAddr>,
    ) -> io::Result<(Self, NetworkRuntime)> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;

        let (outbound_tx, outbound_rx) = mpsc::sync_channel(NETWORK_QUEUE_CAPACITY);
        let (inbound_tx, inbound_rx) = mpsc::sync_channel(NETWORK_QUEUE_CAPACITY);

        let peer_addresses = Arc::new(peer_addresses);
        let shutdown = Arc::new(AtomicBool::new(false));

        let sender_peers = Arc::clone(&peer_addresses);
        let sender_shutdown = Arc::clone(&shutdown);
        let local_inbound = inbound_tx.clone();
        let sender_thread = thread::Builder::new()
            .name("tcp-network-send".to_string())
            .spawn(move || {
                run_outbound(
                    outbound_rx,
                    sender_peers,
                    local_key,
                    local_inbound,
                    sender_shutdown,
                );
            })?;

        let receiver_peers = Arc::clone(&peer_addresses);
        let receiver_shutdown = Arc::clone(&shutdown);
        let receiver_thread = thread::Builder::new()
            .name("tcp-network-recv".to_string())
            .spawn(move || {
                run_inbound(listener, inbound_tx, receiver_peers, receiver_shutdown);
            })?;

        Ok((
            Self {
                peer_addresses,
                outbound: outbound_tx,
                inbound: Arc::new(Mutex::new(inbound_rx)),
            },
            NetworkRuntime {
                shutdown,
                threads: vec![sender_thread, receiver_thread],
            },
        ))
    }

    fn enqueue(&self, destination: Option<VerifyingKey>, message: Message) {
        let _ = self.outbound.try_send(OutboundMessage {
            destination,
            message,
        });
    }

    /// Retrieve configured peer addresses.
    pub fn peer_addresses(&self) -> &Arc<HashMap<VerifyingKey, SocketAddr>> {
        &self.peer_addresses
    }
}

impl Network for TcpNetwork {
    fn init_validator_set(&mut self, _validators: ValidatorSet) {}

    fn update_validator_set(&mut self, _updates: ValidatorSetUpdates) {}

    fn broadcast(&mut self, message: Message) {
        self.enqueue(None, message);
    }

    fn send(&mut self, peer: VerifyingKey, message: Message) {
        self.enqueue(Some(peer), message);
    }

    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        match self.inbound.lock().ok()?.try_recv() {
            Ok(message) => Some(message),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }
}

fn run_outbound(
    receiver: Receiver<OutboundMessage>,
    peers: Arc<HashMap<VerifyingKey, SocketAddr>>,
    local_key: VerifyingKey,
    local_inbound: SyncSender<(VerifyingKey, Message)>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Acquire) {
        let outbound = match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };

        let broadcast = outbound.destination.is_none();
        let destinations: Vec<_> = match outbound.destination {
            Some(key) if key != local_key => peers.get(&key).copied().into_iter().collect(),
            Some(_) => Vec::new(),
            None => peers
                .iter()
                .filter(|(key, _)| **key != local_key)
                .map(|(_, address)| *address)
                .collect(),
        };

        if broadcast || outbound.destination == Some(local_key) {
            let _ = local_inbound.try_send((local_key, outbound.message.clone()));
        }

        let message_bytes = match outbound.message.try_to_vec() {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };

        for destination in destinations {
            let _ = send_frame(destination, local_key.to_bytes(), &message_bytes);
        }
    }
}

fn send_frame(address: SocketAddr, sender: [u8; 32], message: &[u8]) -> io::Result<()> {
    let frame_len = sender.len().saturating_add(message.len());
    if frame_len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Frame exceeds max size",
        ));
    }
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(&(frame_len as u32).to_be_bytes())?;
    stream.write_all(&sender)?;
    stream.write_all(message)?;
    Ok(())
}

fn run_inbound(
    listener: TcpListener,
    sender: SyncSender<(VerifyingKey, Message)>,
    peers: Arc<HashMap<VerifyingKey, SocketAddr>>,
    shutdown: Arc<AtomicBool>,
) {
    let active_connections = Arc::new(AtomicUsize::new(0));
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                if active_connections.fetch_add(1, Ordering::AcqRel) >= MAX_INBOUND_CONNECTIONS {
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                    continue;
                }
                let sender = sender.clone();
                let peers = Arc::clone(&peers);
                let shutdown = Arc::clone(&shutdown);
                let connection_counter = Arc::clone(&active_connections);

                let _ = thread::Builder::new()
                    .name("tcp-peer-reader".to_string())
                    .spawn(move || {
                        let _guard = ActiveConn(connection_counter);
                        let _ = receive_frame(stream, sender, &peers, &shutdown);
                    });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

struct ActiveConn(Arc<AtomicUsize>);

impl Drop for ActiveConn {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn receive_frame(
    mut stream: TcpStream,
    sender: SyncSender<(VerifyingKey, Message)>,
    peers: &HashMap<VerifyingKey, SocketAddr>,
    shutdown: &AtomicBool,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut length_bytes = [0u8; 4];
    stream.read_exact(&mut length_bytes)?;
    let length = u32::from_be_bytes(length_bytes) as usize;
    if !(32..=MAX_FRAME_BYTES).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid frame length",
        ));
    }

    let mut frame = vec![0u8; length];
    stream.read_exact(&mut frame)?;

    let sender_bytes: [u8; 32] = frame[..32]
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid sender key"))?;

    let peer = VerifyingKey::from_bytes(&sender_bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    if !peers.contains_key(&peer) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "peer not in validator set",
        ));
    }

    let message = Message::try_from_slice(&frame[32..])
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    if shutdown.load(Ordering::Acquire) {
        return Ok(());
    }

    match sender.try_send((peer, message)) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "inbound queue full",
        )),
        Err(TrySendError::Disconnected(_)) => Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "inbound queue closed",
        )),
    }
}

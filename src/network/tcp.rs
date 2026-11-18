//! Static-peer TCP transport for `hotstuff_rs`, authenticated with Noise (TODO 8.4.1).
//!
//! Each configured peer gets its own outbound worker with a bounded queue and a persistent
//! connection, so an unreachable or slow peer only delays (and eventually drops) its own
//! messages instead of stalling delivery to the rest of the validator set. Peer addresses are
//! `host:port` strings resolved at connect time, so container/DNS names work.
//!
//! **Identity (8.4.1, `docs/adr/proposal-transport-identity.md`).** Every connection starts with
//! a Noise `XX` handshake in which each side proves its validator key (`noise.rs`). Outbound
//! connections accept only the validator they dialled; inbound ones (`inbound.rs`) only
//! configured validators, and the message origin is the proven key. A frame is
//! `u32 BE plaintext length ‖ encrypted chunks`; nothing in it names the sender.
//!
//! **Memory (8.5.2 F6).** Queues are bounded in bytes as well as messages: per-validator and
//! global inbound budgets (with TCP backpressure), and a per-peer outbound budget (frames over it
//! are dropped). Reservations travel with the queued item and return when it is dropped.

use borsh::BorshSerialize;
use hotstuff_rs::networking::messages::Message;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::crypto_primitives::VerifyingKey;
use hotstuff_rs::types::update_sets::ValidatorSetUpdates;
use hotstuff_rs::types::validator_set::ValidatorSet;
use socket2::{SockRef, TcpKeepalive};
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::byte_budget::{ByteBudget, Reservation};
use super::inbound::{Acceptor, Budgets, HANDSHAKE_TIMEOUT};
use super::noise::{Session, TransportIdentity, TransportRejections, rejection_of};
use crate::observability::TransportTraffic;
use crate::observability::send_delay::MessageKind;

/// Largest plaintext message (borsh `Message`).
pub(crate) const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const INBOUND_QUEUE_CAPACITY: usize = 4096;
const PEER_QUEUE_CAPACITY: usize = 1024;
/// Bytes of frames from one validator waiting for the replica (room for four maximum frames).
const PEER_INBOUND_BYTES: usize = 4 * MAX_FRAME_BYTES;
/// Bytes of frames queued for one peer.
const PEER_OUTBOUND_BYTES: usize = 4 * MAX_FRAME_BYTES;
/// Bytes of frames from all validators waiting for the replica.
const GLOBAL_INBOUND_BYTES: usize = 16 * MAX_FRAME_BYTES;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);
/// After a failed connect, messages to that peer are dropped (not queued) for this long.
const RECONNECT_BACKOFF: Duration = Duration::from_millis(200);
/// Abort a connection whose sent data stays unacknowledged this long (Linux `TCP_USER_TIMEOUT`).
/// Without it, a peer that vanished without a RST (partition, IP change) black-holes writes into
/// the kernel send buffer for many minutes while `write_all` keeps reporting success.
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
const TCP_USER_TIMEOUT: Duration = Duration::from_secs(3);
const KEEPALIVE_IDLE: Duration = Duration::from_secs(2);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(1);
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// `hotstuff_rs` polls `recv` in a `yield_now` loop; wait briefly so idle nodes don't spin a core.
const RECV_WAIT: Duration = Duration::from_millis(1);

type PeerQueues = Arc<HashMap<VerifyingKey, PeerQueue>>;

/// Outbound queue for one peer and the byte budget its queued messages hold.
struct PeerQueue {
    sender: SyncSender<Outbound>,
    budget: Arc<ByteBudget>,
}

/// A plaintext message body queued for a peer and the outbound bytes it holds until written or
/// dropped, with its kind and queueing time for the send-delay histogram (8.5.3). Each worker
/// encrypts it in its own session.
type Outbound = (Arc<Vec<u8>>, Reservation, MessageKind, Instant);

/// A received message, its authenticated origin, and the inbound bytes it holds until the
/// replica takes it.
pub(crate) type Inbound = (VerifyingKey, Message, Reservation);

/// Background threads (listener and per-peer senders) owned by a [`TcpNetwork`].
/// Dropping it stops the transport and releases the listening socket.
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

/// Authenticated TCP adapter implementing `hotstuff_rs::networking::network::Network`.
#[derive(Clone)]
pub struct TcpNetwork {
    local_key: VerifyingKey,
    peer_addresses: Arc<HashMap<VerifyingKey, String>>,
    peer_queues: PeerQueues,
    local_inbound: SyncSender<Inbound>,
    inbound: Arc<Mutex<Receiver<Inbound>>>,
    dropped_messages: Arc<AtomicU64>,
    rejections: Arc<TransportRejections>,
    traffic: Arc<TransportTraffic>,
}

impl TcpNetwork {
    /// Bind with numeric peer socket addresses.
    pub fn bind(
        identity: Arc<TransportIdentity>,
        address: SocketAddr,
        peer_addresses: HashMap<VerifyingKey, SocketAddr>,
    ) -> io::Result<(Self, NetworkRuntime)> {
        let peers = peer_addresses
            .into_iter()
            .map(|(key, addr)| (key, addr.to_string()))
            .collect();
        Self::bind_with_hosts(identity, address, peers)
    }

    /// Bind with `host:port` peer addresses that are resolved on every (re)connect. Only the
    /// validators in `peer_addresses` can connect or be connected to.
    pub fn bind_with_hosts(
        identity: Arc<TransportIdentity>,
        address: SocketAddr,
        peer_addresses: HashMap<VerifyingKey, String>,
    ) -> io::Result<(Self, NetworkRuntime)> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        super::host_tuning::warn_if_slow_start_after_idle();

        let local_key = identity.validator();
        let shutdown = Arc::new(AtomicBool::new(false));
        let dropped_messages = Arc::new(AtomicU64::new(0));
        let rejections = Arc::new(TransportRejections::default());
        let traffic = Arc::new(TransportTraffic::default());
        let (inbound_tx, inbound_rx) = mpsc::sync_channel(INBOUND_QUEUE_CAPACITY);
        let peer_addresses = Arc::new(peer_addresses);

        let mut threads = Vec::new();
        let mut queues = HashMap::new();
        for (peer, host) in peer_addresses.iter().filter(|(k, _)| **k != local_key) {
            let (tx, rx) = mpsc::sync_channel(PEER_QUEUE_CAPACITY);
            let budget = Arc::new(ByteBudget::new(PEER_OUTBOUND_BYTES));
            queues.insert(*peer, PeerQueue { sender: tx, budget });
            let worker = PeerWorker {
                peer: *peer,
                host: host.clone(),
                identity: Arc::clone(&identity),
                peers: Arc::clone(&peer_addresses),
                queue: rx,
                shutdown: Arc::clone(&shutdown),
                dropped: Arc::clone(&dropped_messages),
                rejections: Arc::clone(&rejections),
                traffic: Arc::clone(&traffic),
            };
            threads.push(
                thread::Builder::new()
                    .name("tcp-peer-send".to_string())
                    .spawn(move || worker.run())?,
            );
        }

        let budgets: Budgets = Arc::new(
            peer_addresses
                .keys()
                .map(|peer| (*peer, Arc::new(ByteBudget::new(PEER_INBOUND_BYTES))))
                .collect(),
        );
        let acceptor = Acceptor {
            listener,
            identity,
            peers: Arc::clone(&peer_addresses),
            inbound: inbound_tx.clone(),
            budgets,
            global_budget: Arc::new(ByteBudget::new(GLOBAL_INBOUND_BYTES)),
            rejections: Arc::clone(&rejections),
            traffic: Arc::clone(&traffic),
            shutdown: Arc::clone(&shutdown),
        };
        threads.push(
            thread::Builder::new()
                .name("tcp-network-accept".to_string())
                .spawn(move || acceptor.run())?,
        );

        Ok((
            Self {
                local_key,
                peer_addresses,
                peer_queues: Arc::new(queues),
                local_inbound: inbound_tx,
                inbound: Arc::new(Mutex::new(inbound_rx)),
                dropped_messages,
                rejections,
                traffic,
            },
            NetworkRuntime { shutdown, threads },
        ))
    }

    /// Retrieve configured peer addresses.
    pub fn peer_addresses(&self) -> &Arc<HashMap<VerifyingKey, String>> {
        &self.peer_addresses
    }

    /// Number of outbound messages dropped (full queue, unreachable peer, encode failure).
    pub fn dropped_messages(&self) -> u64 {
        self.dropped_messages.load(Ordering::Relaxed)
    }

    /// Shared handle to the dropped-message counter, for metrics export.
    pub fn dropped_messages_counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.dropped_messages)
    }

    /// P2P bytes sent and received on the wire (TODO 8.5.3 I7).
    pub fn traffic(&self) -> Arc<TransportTraffic> {
        Arc::clone(&self.traffic)
    }

    /// Refused or closed connections by reason (handshake, attestation, framing, slots).
    pub fn rejections(&self) -> Arc<TransportRejections> {
        Arc::clone(&self.rejections)
    }

    fn deliver_local(&self, message: Message) {
        if self
            .local_inbound
            .try_send((self.local_key, message, Reservation::none()))
            .is_err()
        {
            self.dropped_messages.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn enqueue(&self, peer: &VerifyingKey, body: &Arc<Vec<u8>>, kind: MessageKind) {
        let Some(queue) = self.peer_queues.get(peer) else {
            return;
        };
        let Some(reservation) = Reservation::try_new(&[&queue.budget], body.len()) else {
            self.dropped_messages.fetch_add(1, Ordering::Relaxed);
            return;
        };
        // On a full queue the item (and its reservation) is dropped here.
        if queue
            .sender
            .try_send((Arc::clone(body), reservation, kind, Instant::now()))
            .is_err()
        {
            self.dropped_messages.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn encode(&self, message: &Message) -> Option<Arc<Vec<u8>>> {
        match message.try_to_vec() {
            Ok(body) if !body.is_empty() && body.len() <= MAX_FRAME_BYTES => Some(Arc::new(body)),
            _ => {
                self.dropped_messages.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }
}

impl Network for TcpNetwork {
    fn init_validator_set(&mut self, _validators: ValidatorSet) {}

    fn update_validator_set(&mut self, _updates: ValidatorSetUpdates) {}

    fn broadcast(&mut self, message: Message) {
        if let Some(body) = self.encode(&message) {
            let kind = MessageKind::of(&message);
            for peer in self.peer_queues.keys() {
                self.enqueue(peer, &body, kind);
            }
        }
        self.deliver_local(message);
    }

    fn send(&mut self, peer: VerifyingKey, message: Message) {
        if peer == self.local_key {
            self.deliver_local(message);
        } else if let Some(body) = self.encode(&message) {
            self.enqueue(&peer, &body, MessageKind::of(&message));
        }
    }

    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        // Dropping the reservation returns the message's bytes to its budgets.
        let (peer, message, _reservation) =
            self.inbound.lock().ok()?.recv_timeout(RECV_WAIT).ok()?;
        Some((peer, message))
    }
}

/// An authenticated outbound connection.
struct Connection {
    stream: TcpStream,
    session: Session,
}

/// Outbound worker for one peer: owns a persistent authenticated connection and a bounded queue.
struct PeerWorker {
    peer: VerifyingKey,
    host: String,
    identity: Arc<TransportIdentity>,
    peers: Arc<HashMap<VerifyingKey, String>>,
    queue: Receiver<Outbound>,
    shutdown: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    rejections: Arc<TransportRejections>,
    traffic: Arc<TransportTraffic>,
}

impl PeerWorker {
    fn run(self) {
        let mut connection: Option<Connection> = None;
        let mut retry_after = Instant::now();
        while !self.shutdown.load(Ordering::Acquire) {
            // The reservation is returned when this iteration ends, written or not.
            let (body, _reservation, kind, queued) =
                match self.queue.recv_timeout(WORKER_POLL_INTERVAL) {
                    Ok(item) => item,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return,
                };
            let delivered = if connection.is_none() && Instant::now() < retry_after {
                false
            } else if self.write_frame(&mut connection, &body) {
                self.traffic.record_send_delay(kind, queued.elapsed());
                true
            } else {
                retry_after = Instant::now() + RECONNECT_BACKOFF;
                false
            };
            if !delivered {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Encrypt and write on the existing connection, reconnecting once if it is absent or broken.
    fn write_frame(&self, connection: &mut Option<Connection>, body: &[u8]) -> bool {
        for _ in 0..2 {
            if connection.is_none() {
                *connection = self.connect().ok();
            }
            let Some(conn) = connection.as_mut() else {
                return false;
            };
            let started = Instant::now();
            let written = conn.session.seal_frame(body).and_then(|frame| {
                conn.stream.write_all(&frame)?;
                Ok(frame.len())
            });
            match written {
                Ok(bytes) => {
                    self.traffic.record_sent(bytes);
                    return true;
                }
                // A failed write drops the connection, the rest of this frame and whatever the
                // peer had not yet read (8.5.3: visible, not only counted as dropped).
                Err(error) => tracing::warn!(
                    address = %self.host,
                    error_kind = ?error.kind(),
                    frame_bytes = body.len(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "outbound peer write failed; reconnecting"
                ),
            }
            *connection = None;
        }
        false
    }

    fn connect(&self) -> io::Result<Connection> {
        let mut last_error = io::Error::new(io::ErrorKind::NotFound, "peer address unresolved");
        for address in self.host.to_socket_addrs()? {
            match self.connect_to(address) {
                Ok(connection) => return Ok(connection),
                Err(error) => {
                    if let Some(reason) = rejection_of(&error) {
                        self.rejections.record(reason);
                        tracing::warn!(
                            reason = reason.as_str(),
                            %address,
                            "outbound peer connection refused"
                        );
                    }
                    last_error = error;
                }
            }
        }
        Err(last_error)
    }

    fn connect_to(&self, address: SocketAddr) -> io::Result<Connection> {
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
        stream.set_nodelay(true)?;
        stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
        detect_dead_peers(&stream)?;
        let session = self.identity.initiate(
            &mut stream,
            &self.peers,
            &self.peer,
            Instant::now() + HANDSHAKE_TIMEOUT,
        )?;
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        Ok(Connection { stream, session })
    }
}

/// Make silently-dead peers (no FIN/RST) surface as I/O errors within seconds.
pub(crate) fn detect_dead_peers(stream: &TcpStream) -> io::Result<()> {
    let socket = SockRef::from(stream);
    socket.set_tcp_keepalive(
        &TcpKeepalive::new()
            .with_time(KEEPALIVE_IDLE)
            .with_interval(KEEPALIVE_INTERVAL),
    )?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    socket.set_tcp_user_timeout(Some(TCP_USER_TIMEOUT))?;
    Ok(())
}

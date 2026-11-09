//! Static-peer, length-framed TCP transport for `hotstuff_rs`.
//!
//! Each configured peer gets its own outbound worker with a bounded queue and a persistent
//! connection, so an unreachable or slow peer only delays (and eventually drops) its own
//! messages instead of stalling delivery to the rest of the validator set. Peer addresses are
//! `host:port` strings resolved at connect time, so container/DNS names work.
//!
//! Frame format: `u32 BE length` || `32-byte sender verifying key` || `borsh(Message)`.
//! The sender key is self-declared and only checked against the configured peer table; it is
//! NOT authenticated (TODO 8.4.1).

use borsh::{BorshDeserialize, BorshSerialize};
use hotstuff_rs::networking::messages::Message;
use hotstuff_rs::networking::network::Network;
use hotstuff_rs::types::crypto_primitives::VerifyingKey;
use hotstuff_rs::types::update_sets::ValidatorSetUpdates;
use hotstuff_rs::types::validator_set::ValidatorSet;
use socket2::{SockRef, TcpKeepalive};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const SENDER_KEY_BYTES: usize = 32;
const MAX_INBOUND_CONNECTIONS: usize = 64;
const INBOUND_QUEUE_CAPACITY: usize = 4096;
const PEER_QUEUE_CAPACITY: usize = 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);
/// After a failed connect, messages to that peer are dropped (not queued) for this long.
const RECONNECT_BACKOFF: Duration = Duration::from_millis(200);
/// Read timeout used to periodically observe shutdown on idle inbound connections.
const READ_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Abort a connection whose sent data stays unacknowledged this long (Linux `TCP_USER_TIMEOUT`).
/// Without it, a peer that vanished without a RST (partition, IP change) black-holes writes into
/// the kernel send buffer for many minutes while `write_all` keeps reporting success.
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
const TCP_USER_TIMEOUT: Duration = Duration::from_secs(3);
const KEEPALIVE_IDLE: Duration = Duration::from_secs(2);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(1);
/// Live validators send block-sync advertisements every ~500ms, so an inbound connection silent
/// for this long is dead; closing it frees its slot in `MAX_INBOUND_CONNECTIONS`.
const INBOUND_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(5);
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// `hotstuff_rs` polls `recv` in a `yield_now` loop; wait briefly so idle nodes don't spin a core.
const RECV_WAIT: Duration = Duration::from_millis(1);

type PeerQueues = Arc<HashMap<VerifyingKey, SyncSender<Arc<Vec<u8>>>>>;

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

/// Length-delimited framed TCP adapter implementing `hotstuff_rs::networking::network::Network`.
#[derive(Clone)]
pub struct TcpNetwork {
    local_key: VerifyingKey,
    peer_addresses: Arc<HashMap<VerifyingKey, String>>,
    peer_queues: PeerQueues,
    local_inbound: SyncSender<(VerifyingKey, Message)>,
    inbound: Arc<Mutex<Receiver<(VerifyingKey, Message)>>>,
    dropped_messages: Arc<AtomicU64>,
}

impl TcpNetwork {
    /// Bind with numeric peer socket addresses.
    pub fn bind(
        local_key: VerifyingKey,
        address: SocketAddr,
        peer_addresses: HashMap<VerifyingKey, SocketAddr>,
    ) -> io::Result<(Self, NetworkRuntime)> {
        let peers = peer_addresses
            .into_iter()
            .map(|(key, addr)| (key, addr.to_string()))
            .collect();
        Self::bind_with_hosts(local_key, address, peers)
    }

    /// Bind with `host:port` peer addresses that are resolved on every (re)connect.
    pub fn bind_with_hosts(
        local_key: VerifyingKey,
        address: SocketAddr,
        peer_addresses: HashMap<VerifyingKey, String>,
    ) -> io::Result<(Self, NetworkRuntime)> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let dropped_messages = Arc::new(AtomicU64::new(0));
        let (inbound_tx, inbound_rx) = mpsc::sync_channel(INBOUND_QUEUE_CAPACITY);
        let peer_addresses = Arc::new(peer_addresses);

        let mut threads = Vec::new();
        let mut queues = HashMap::new();
        for (peer, host) in peer_addresses.iter().filter(|(k, _)| **k != local_key) {
            let (tx, rx) = mpsc::sync_channel(PEER_QUEUE_CAPACITY);
            queues.insert(*peer, tx);
            let worker = PeerWorker {
                host: host.clone(),
                queue: rx,
                shutdown: Arc::clone(&shutdown),
                dropped: Arc::clone(&dropped_messages),
            };
            threads.push(
                thread::Builder::new()
                    .name("tcp-peer-send".to_string())
                    .spawn(move || worker.run())?,
            );
        }

        let acceptor = Acceptor {
            listener,
            inbound: inbound_tx.clone(),
            peers: Arc::clone(&peer_addresses),
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

    fn deliver_local(&self, message: Message) {
        if self
            .local_inbound
            .try_send((self.local_key, message))
            .is_err()
        {
            self.dropped_messages.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn enqueue(&self, peer: &VerifyingKey, frame: &Arc<Vec<u8>>) {
        let Some(queue) = self.peer_queues.get(peer) else {
            return;
        };
        if queue.try_send(Arc::clone(frame)).is_err() {
            self.dropped_messages.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn encode(&self, message: &Message) -> Option<Arc<Vec<u8>>> {
        match encode_frame(self.local_key.to_bytes(), message) {
            Ok(frame) => Some(Arc::new(frame)),
            Err(_) => {
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
        if let Some(frame) = self.encode(&message) {
            for peer in self.peer_queues.keys() {
                self.enqueue(peer, &frame);
            }
        }
        self.deliver_local(message);
    }

    fn send(&mut self, peer: VerifyingKey, message: Message) {
        if peer == self.local_key {
            self.deliver_local(message);
        } else if let Some(frame) = self.encode(&message) {
            self.enqueue(&peer, &frame);
        }
    }

    fn recv(&mut self) -> Option<(VerifyingKey, Message)> {
        self.inbound.lock().ok()?.recv_timeout(RECV_WAIT).ok()
    }
}

fn encode_frame(sender: [u8; SENDER_KEY_BYTES], message: &Message) -> io::Result<Vec<u8>> {
    let body = message.try_to_vec()?;
    let frame_len = SENDER_KEY_BYTES.saturating_add(body.len());
    if frame_len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds max size",
        ));
    }
    let length = u32::try_from(frame_len)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length overflow"))?;
    let mut frame = Vec::with_capacity(4 + frame_len);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&sender);
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Outbound worker for one peer: owns a persistent connection and a bounded queue.
struct PeerWorker {
    host: String,
    queue: Receiver<Arc<Vec<u8>>>,
    shutdown: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
}

impl PeerWorker {
    fn run(self) {
        let mut stream: Option<TcpStream> = None;
        let mut retry_after = Instant::now();
        while !self.shutdown.load(Ordering::Acquire) {
            let frame = match self.queue.recv_timeout(WORKER_POLL_INTERVAL) {
                Ok(frame) => frame,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            if stream.is_none() && Instant::now() < retry_after {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if !self.write_frame(&mut stream, &frame) {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                retry_after = Instant::now() + RECONNECT_BACKOFF;
            }
        }
    }

    /// Write on the existing connection, reconnecting once if it is absent or broken.
    fn write_frame(&self, stream: &mut Option<TcpStream>, frame: &[u8]) -> bool {
        for _ in 0..2 {
            if stream.is_none() {
                *stream = self.connect().ok();
            }
            let Some(conn) = stream.as_mut() else {
                return false;
            };
            if conn.write_all(frame).is_ok() {
                return true;
            }
            *stream = None;
        }
        false
    }

    fn connect(&self) -> io::Result<TcpStream> {
        let mut last_error = io::Error::new(io::ErrorKind::NotFound, "peer address unresolved");
        for address in self.host.to_socket_addrs()? {
            match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
                Ok(stream) => {
                    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
                    stream.set_nodelay(true)?;
                    detect_dead_peers(&stream)?;
                    return Ok(stream);
                }
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }
}

/// Accepts inbound connections and spawns a bounded number of frame readers.
struct Acceptor {
    listener: TcpListener,
    inbound: SyncSender<(VerifyingKey, Message)>,
    peers: Arc<HashMap<VerifyingKey, String>>,
    shutdown: Arc<AtomicBool>,
}

impl Acceptor {
    fn run(self) {
        let active = Arc::new(AtomicUsize::new(0));
        while !self.shutdown.load(Ordering::Acquire) {
            match self.listener.accept() {
                Ok((stream, _)) => self.spawn_reader(stream, &active),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(ACCEPT_POLL_INTERVAL);
                }
                Err(_) => thread::sleep(WORKER_POLL_INTERVAL),
            }
        }
    }

    fn spawn_reader(&self, stream: TcpStream, active: &Arc<AtomicUsize>) {
        if active.fetch_add(1, Ordering::AcqRel) >= MAX_INBOUND_CONNECTIONS {
            active.fetch_sub(1, Ordering::AcqRel);
            return;
        }
        let reader = FrameReader {
            stream,
            inbound: self.inbound.clone(),
            peers: Arc::clone(&self.peers),
            shutdown: Arc::clone(&self.shutdown),
        };
        let guard = ActiveConn(Arc::clone(active));
        let spawned = thread::Builder::new()
            .name("tcp-peer-reader".to_string())
            .spawn(move || {
                let _guard = guard;
                let _ = reader.run();
            });
        // On spawn failure the closure (and its guard) is dropped, releasing the slot.
        drop(spawned);
    }
}

struct ActiveConn(Arc<AtomicUsize>);

impl Drop for ActiveConn {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Reads frames from one persistent inbound connection until EOF, error, or shutdown.
struct FrameReader {
    stream: TcpStream,
    inbound: SyncSender<(VerifyingKey, Message)>,
    peers: Arc<HashMap<VerifyingKey, String>>,
    shutdown: Arc<AtomicBool>,
}

impl FrameReader {
    fn run(mut self) -> io::Result<()> {
        self.stream.set_read_timeout(Some(READ_POLL_INTERVAL))?;
        detect_dead_peers(&self.stream)?;
        loop {
            let mut length_bytes = [0u8; 4];
            if !self.read_full(&mut length_bytes)? {
                return Ok(());
            }
            let length = u32::from_be_bytes(length_bytes) as usize;
            if !(SENDER_KEY_BYTES..=MAX_FRAME_BYTES).contains(&length) {
                return Err(invalid("invalid frame length"));
            }
            let mut frame = vec![0u8; length];
            if !self.read_full(&mut frame)? {
                return Ok(());
            }
            self.dispatch(&frame)?;
        }
    }

    /// Fill `buf`, retrying across read timeouts without losing partial progress.
    /// Returns `Ok(false)` on shutdown or clean EOF before any byte of `buf` was read.
    fn read_full(&mut self, buf: &mut [u8]) -> io::Result<bool> {
        let mut filled = 0;
        let mut last_progress = Instant::now();
        while filled < buf.len() {
            if last_progress.elapsed() >= INBOUND_IDLE_TIMEOUT {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "inbound peer connection idle",
                ));
            }
            if self.shutdown.load(Ordering::Acquire) {
                return Ok(false);
            }
            match self.stream.read(&mut buf[filled..]) {
                Ok(0) if filled == 0 => return Ok(false),
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => {
                    filled += n;
                    last_progress = Instant::now();
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    fn dispatch(&self, frame: &[u8]) -> io::Result<()> {
        let (sender_bytes, body) = frame.split_at(SENDER_KEY_BYTES);
        let sender_bytes: [u8; SENDER_KEY_BYTES] = sender_bytes
            .try_into()
            .map_err(|_| invalid("invalid sender key"))?;
        let peer = VerifyingKey::from_bytes(&sender_bytes).map_err(|_| invalid("bad key"))?;
        if !self.peers.contains_key(&peer) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "peer not in configured validator set",
            ));
        }
        let message = Message::try_from_slice(body).map_err(|_| invalid("bad message"))?;
        match self.inbound.try_send((peer, message)) {
            // A full inbound queue drops this message but keeps the connection.
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }
}

/// Make silently-dead peers (no FIN/RST) surface as I/O errors within seconds.
fn detect_dead_peers(stream: &TcpStream) -> io::Result<()> {
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

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

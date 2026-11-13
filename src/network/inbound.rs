//! Inbound side of the TCP transport: accept, authenticate (Noise + attestation), then read
//! encrypted frames from one validator per connection (TODO 8.4.1; memory bounds from 8.5.2 F6).
//!
//! - At most `MAX_PENDING_HANDSHAKES` connections may be unauthenticated at once; more are
//!   refused (`slots_full`), and each must finish its handshake within `HANDSHAKE_TIMEOUT`.
//! - Each validator has at most one authenticated inbound connection: a new one replaces the old,
//!   so a restarted peer reconnects at once and a stale connection cannot pin resources.
//! - The message origin is the validator the session proved, never a field in the frame.

use borsh::BorshDeserialize;
use hotstuff_rs::networking::messages::Message;
use hotstuff_rs::types::crypto_primitives::VerifyingKey;
use std::collections::HashMap;
use std::io::{self, Read};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::byte_budget::{ByteBudget, Reservation};
use super::noise::{
    MAX_CHUNK_PLAINTEXT, Rejection, Session, TransportIdentity, TransportRejections, refused,
    rejection_of,
};
use super::tcp::{Inbound, MAX_FRAME_BYTES, detect_dead_peers};
use crate::observability::TransportTraffic;

pub(crate) const MAX_PENDING_HANDSHAKES: usize = 16;
/// Pending handshakes per source IP, so one host cannot occupy every slot.
pub(crate) const MAX_PENDING_PER_IP: usize = 4;
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
/// How often a reader waiting for inbound budget re-checks it (and shutdown).
const BUDGET_WAIT: Duration = Duration::from_millis(5);
/// A reader that cannot reserve budget for this long closes its connection (the peer
/// reconnects), so a stuck budget cannot park connections indefinitely.
const BUDGET_WAIT_LIMIT: Duration = Duration::from_secs(30);
/// A frame body holding a reservation must arrive within this long (16 MiB at about 0.5 MiB/s),
/// so trickled bytes cannot pin a budget.
const BODY_READ_LIMIT: Duration = Duration::from_secs(30);
/// Read timeout used to periodically observe shutdown and replacement on idle connections.
const READ_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Live validators send block-sync advertisements every ~500ms, so an inbound connection silent
/// for this long is dead.
const INBOUND_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(5);
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);

pub(crate) type Budgets = Arc<HashMap<VerifyingKey, Arc<ByteBudget>>>;

/// The current authenticated inbound connection per validator (its close flag).
type Sessions = Arc<Mutex<HashMap<VerifyingKey, Arc<AtomicBool>>>>;

/// Accepts inbound connections and runs one authenticated reader per connection.
pub(crate) struct Acceptor {
    pub(crate) listener: TcpListener,
    pub(crate) identity: Arc<TransportIdentity>,
    pub(crate) peers: Arc<HashMap<VerifyingKey, String>>,
    pub(crate) inbound: SyncSender<Inbound>,
    pub(crate) budgets: Budgets,
    pub(crate) global_budget: Arc<ByteBudget>,
    pub(crate) rejections: Arc<TransportRejections>,
    pub(crate) traffic: Arc<TransportTraffic>,
    pub(crate) shutdown: Arc<AtomicBool>,
}

impl Acceptor {
    pub(crate) fn run(self) {
        let pending = Arc::new(Pending::default());
        let sessions: Sessions = Arc::default();
        let this = Arc::new(self);
        while !this.shutdown.load(Ordering::Acquire) {
            match this.listener.accept() {
                Ok((stream, from)) => Self::spawn(&this, stream, from.ip(), &pending, &sessions),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(ACCEPT_POLL_INTERVAL);
                }
                Err(_) => thread::sleep(ACCEPT_ERROR_BACKOFF),
            }
        }
    }

    fn spawn(
        this: &Arc<Self>,
        stream: TcpStream,
        from: IpAddr,
        pending: &Arc<Pending>,
        sessions: &Sessions,
    ) {
        let Some(slot) = PendingSlot::acquire(pending, from) else {
            this.rejections.record(Rejection::SlotsFull);
            return; // Dropping the stream closes it.
        };
        let acceptor = Arc::clone(this);
        let sessions = Arc::clone(sessions);
        let spawned = thread::Builder::new()
            .name("tcp-peer-reader".to_string())
            .spawn(move || {
                if let Err(error) = acceptor.serve(stream, slot, &sessions)
                    && let Some(reason) = rejection_of(&error)
                {
                    acceptor.rejections.record(reason);
                    // Unauthenticated hosts can trigger this at will: keep it out of warn logs.
                    tracing::debug!(reason = reason.as_str(), "inbound peer connection refused");
                }
            });
        // On spawn failure the closure (and its slot) is dropped, releasing the slot.
        drop(spawned);
    }

    /// Authenticate, register as this validator's inbound connection, then read frames.
    fn serve(
        &self,
        mut stream: TcpStream,
        mut slot: PendingSlot,
        sessions: &Sessions,
    ) -> io::Result<()> {
        stream.set_nodelay(true)?;
        stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
        detect_dead_peers(&stream)?;
        let session =
            self.identity
                .respond(&mut stream, &self.peers, Instant::now() + HANDSHAKE_TIMEOUT)?;
        slot.release();
        let peer = session.peer();
        let closed = Arc::new(AtomicBool::new(false));
        if let Ok(mut current) = sessions.lock()
            && let Some(previous) = current.insert(peer, Arc::clone(&closed))
        {
            previous.store(true, Ordering::Release);
        }
        let budget = self
            .budgets
            .get(&peer)
            .cloned()
            .ok_or_else(|| refused(Rejection::UnknownValidator))?;
        let mut reader = FrameReader {
            stream,
            session,
            peer_budget: budget,
            global_budget: Arc::clone(&self.global_budget),
            inbound: self.inbound.clone(),
            traffic: Arc::clone(&self.traffic),
            shutdown: Arc::clone(&self.shutdown),
            closed: Arc::clone(&closed),
        };
        let result = reader.run();
        if let Ok(mut current) = sessions.lock()
            && current
                .get(&peer)
                .is_some_and(|flag| Arc::ptr_eq(flag, &closed))
        {
            current.remove(&peer);
        }
        result
    }
}

/// Unauthenticated connections in total and per source IP.
#[derive(Default)]
struct Pending {
    by_ip: Mutex<HashMap<IpAddr, usize>>,
}

/// A pre-authentication slot, returned when the handshake ends (either way).
struct PendingSlot(Option<(Arc<Pending>, IpAddr)>);

impl PendingSlot {
    fn acquire(pending: &Arc<Pending>, ip: IpAddr) -> Option<Self> {
        let mut by_ip = pending.by_ip.lock().ok()?;
        let total: usize = by_ip.values().sum();
        let from_ip = by_ip.get(&ip).copied().unwrap_or(0);
        if total >= MAX_PENDING_HANDSHAKES || from_ip >= MAX_PENDING_PER_IP {
            return None;
        }
        by_ip.insert(ip, from_ip + 1);
        Some(Self(Some((Arc::clone(pending), ip))))
    }

    fn release(&mut self) {
        let Some((pending, ip)) = self.0.take() else {
            return;
        };
        if let Ok(mut by_ip) = pending.by_ip.lock()
            && let Some(count) = by_ip.get_mut(&ip)
        {
            *count -= 1;
            if *count == 0 {
                by_ip.remove(&ip);
            }
        }
    }
}

impl Drop for PendingSlot {
    fn drop(&mut self) {
        self.release();
    }
}

/// Reads encrypted frames from one authenticated connection until EOF, error, shutdown or
/// replacement by a newer connection from the same validator.
struct FrameReader {
    stream: TcpStream,
    session: Session,
    peer_budget: Arc<ByteBudget>,
    global_budget: Arc<ByteBudget>,
    inbound: SyncSender<Inbound>,
    traffic: Arc<TransportTraffic>,
    shutdown: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
}

impl FrameReader {
    fn run(&mut self) -> io::Result<()> {
        self.stream.set_read_timeout(Some(READ_POLL_INTERVAL))?;
        let mut sealed = vec![0u8; MAX_CHUNK_PLAINTEXT + Session::TAG];
        loop {
            let mut length_bytes = [0u8; 4];
            if !self.read_full(&mut length_bytes, None)? {
                return Ok(());
            }
            let length = u32::from_be_bytes(length_bytes) as usize;
            if length == 0 || length > MAX_FRAME_BYTES {
                return Err(refused(Rejection::FrameTooLarge));
            }
            let Some(reservation) = self.reserve(length)? else {
                return Ok(());
            };
            let mut body = vec![0u8; length];
            let deadline = Instant::now() + BODY_READ_LIMIT;
            for chunk in body.chunks_mut(MAX_CHUNK_PLAINTEXT) {
                let sealed_chunk = &mut sealed[..chunk.len() + Session::TAG];
                if !self.read_full(sealed_chunk, Some(deadline))? {
                    return Ok(());
                }
                self.session.open_chunk(sealed_chunk, chunk)?;
                self.traffic.record_received(sealed_chunk.len());
            }
            self.traffic.record_received(length_bytes.len());
            self.dispatch(&body, reservation)?;
        }
    }

    /// Wait (TCP backpressure) until `length` bytes fit in this validator's and the global
    /// inbound budget. `None` on shutdown or replacement.
    fn reserve(&self, length: usize) -> io::Result<Option<Reservation>> {
        let give_up = Instant::now() + BUDGET_WAIT_LIMIT;
        loop {
            if let Some(reservation) =
                Reservation::try_new(&[&self.peer_budget, &self.global_budget], length)
            {
                return Ok(Some(reservation));
            }
            if self.stopping() {
                return Ok(None);
            }
            if Instant::now() >= give_up {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "inbound budget stayed full",
                ));
            }
            thread::sleep(BUDGET_WAIT);
        }
    }

    fn stopping(&self) -> bool {
        self.shutdown.load(Ordering::Acquire) || self.closed.load(Ordering::Acquire)
    }

    /// Fill `buf`, retrying across read timeouts without losing partial progress.
    /// Returns `Ok(false)` on shutdown, replacement, or clean EOF before any byte of `buf`. With
    /// a `deadline`, the read must finish by then whatever the progress.
    fn read_full(&mut self, buf: &mut [u8], deadline: Option<Instant>) -> io::Result<bool> {
        let mut filled = 0;
        let mut last_progress = Instant::now();
        while filled < buf.len() {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "inbound frame body too slow",
                ));
            }
            if last_progress.elapsed() >= INBOUND_IDLE_TIMEOUT {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "inbound peer connection idle",
                ));
            }
            if self.stopping() {
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

    /// Decode and queue one message from the authenticated peer with its reservation (returned
    /// when the replica takes it, or here if the message is dropped).
    fn dispatch(&self, body: &[u8], reservation: Reservation) -> io::Result<()> {
        let message = Message::try_from_slice(body)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad message"))?;
        match self
            .inbound
            .try_send((self.session.peer(), message, reservation))
        {
            // A full inbound queue drops this message but keeps the connection.
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }
}

#[cfg(test)]
#[path = "inbound_tests.rs"]
mod tests;

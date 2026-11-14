//! Bounds on RPC connections (stress test 2, `docs/performance-baseline.md` finding 26: under
//! four load generators a validator's listener ran out of file descriptors and logged
//! `accept error: Too many open files`).
//!
//! - **Total:** at most `limit` connections. At the bound the listener stops accepting until
//!   one closes: new clients wait in the kernel's backlog instead of failing, and descriptors
//!   stay available for P2P and storage.
//! - **Per client address:** at most a quarter of the total, so one source cannot hold every
//!   slot; connections over it are closed at once.
//! - **Idle:** a connection with no bytes read or written for [`IDLE_TIMEOUT`] is closed, so
//!   silent clients cannot hold slots (review 2026-11-14). SSE streams send a keep-alive
//!   comment every 15 s, which counts as activity.

use axum::serve::Listener;
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, Sleep};

/// Most RPC connections, whatever the descriptor limit.
pub const MAX_RPC_CONNECTIONS: usize = 4_096;
/// A connection with no traffic either way for this long is closed.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Descriptors kept for everything else (P2P peers, storage, the signing guard, metrics).
const RESERVED_DESCRIPTORS: u64 = 256;
/// Fewest RPC connections allowed, even under a very low descriptor limit.
const MIN_RPC_CONNECTIONS: usize = 32;

/// RPC connections allowed under a soft open-file limit of `soft_limit` (`None`: unknown).
pub fn connection_limit(soft_limit: Option<u64>) -> usize {
    let Some(soft) = soft_limit else {
        return MAX_RPC_CONNECTIONS;
    };
    let available = soft.saturating_sub(RESERVED_DESCRIPTORS) / 2;
    usize::try_from(available)
        .unwrap_or(usize::MAX)
        .clamp(MIN_RPC_CONNECTIONS, MAX_RPC_CONNECTIONS)
}

/// Connections one client address may hold under a total of `limit`.
fn per_address_limit(limit: usize) -> usize {
    (limit / 4).max(1)
}

type AddressCounts = Arc<Mutex<HashMap<IpAddr, usize>>>;

/// A TCP listener enforcing the total, per-address and idle bounds.
pub struct LimitedListener {
    inner: TcpListener,
    permits: Arc<Semaphore>,
    per_address: usize,
    counts: AddressCounts,
    idle_timeout: Duration,
}

impl LimitedListener {
    pub fn new(inner: TcpListener, limit: usize) -> Self {
        Self::with_idle_timeout(inner, limit, IDLE_TIMEOUT)
    }

    pub fn with_idle_timeout(inner: TcpListener, limit: usize, idle_timeout: Duration) -> Self {
        Self {
            inner,
            permits: Arc::new(Semaphore::new(limit)),
            per_address: per_address_limit(limit),
            counts: AddressCounts::default(),
            idle_timeout,
        }
    }
}

impl Listener for LimitedListener {
    type Io = LimitedStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let permit = Arc::clone(&self.permits)
                .acquire_owned()
                .await
                .expect("the connection semaphore is never closed");
            let (io, addr) = Listener::accept(&mut self.inner).await;
            let Some(slot) = AddressSlot::take(&self.counts, addr.ip(), self.per_address) else {
                // Over this address's share: close at once (dropping `io`) and free the slot.
                tracing::debug!(%addr, "RPC connection refused: per-address limit");
                continue;
            };
            let idle = Box::pin(tokio::time::sleep(self.idle_timeout));
            return (
                LimitedStream {
                    io,
                    idle,
                    idle_timeout: self.idle_timeout,
                    _slot: slot,
                    _permit: permit,
                },
                addr,
            );
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

/// One connection counted against its client address; released when dropped.
struct AddressSlot {
    counts: AddressCounts,
    ip: IpAddr,
}

impl AddressSlot {
    fn take(counts: &AddressCounts, ip: IpAddr, limit: usize) -> Option<Self> {
        let mut map = counts.lock().unwrap_or_else(PoisonError::into_inner);
        let held = map.entry(ip).or_insert(0);
        if *held >= limit {
            return None;
        }
        *held += 1;
        Some(Self {
            counts: Arc::clone(counts),
            ip,
        })
    }
}

impl Drop for AddressSlot {
    fn drop(&mut self) {
        let mut map = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(held) = map.get_mut(&self.ip) {
            *held -= 1;
            if *held == 0 {
                map.remove(&self.ip);
            }
        }
    }
}

/// A connection that returns its slots when dropped and fails reads once it has been idle.
pub struct LimitedStream {
    io: TcpStream,
    /// Fires [`IDLE_TIMEOUT`] after the last byte read or written.
    idle: Pin<Box<Sleep>>,
    idle_timeout: Duration,
    _slot: AddressSlot,
    _permit: OwnedSemaphorePermit,
}

impl LimitedStream {
    /// Disable Nagle's algorithm (small JSON responses go out at once).
    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.io.set_nodelay(nodelay)
    }

    fn touch(&mut self) {
        self.idle.as_mut().reset(Instant::now() + self.idle_timeout);
    }
}

impl AsyncRead for LimitedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        match Pin::new(&mut self.io).poll_read(cx, buf) {
            Poll::Ready(result) => {
                if buf.filled().len() > before {
                    self.touch();
                }
                Poll::Ready(result)
            }
            // Nothing to read: the connection stays open only until the idle deadline.
            Poll::Pending => match self.idle.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "RPC connection idle",
                ))),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

impl AsyncWrite for LimitedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.io).poll_write(cx, buf);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            self.touch();
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.io).poll_write_vectored(cx, bufs);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            self.touch();
        }
        result
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }
}

#[cfg(test)]
#[path = "connections_tests.rs"]
mod tests;

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn test_connection_limit_leaves_descriptors_for_the_node() {
    assert_eq!(connection_limit(None), MAX_RPC_CONNECTIONS);
    assert_eq!(connection_limit(Some(1_024)), 384);
    assert_eq!(connection_limit(Some(524_288)), MAX_RPC_CONNECTIONS);
    assert_eq!(connection_limit(Some(256)), MIN_RPC_CONNECTIONS);
    assert_eq!(connection_limit(Some(64)), MIN_RPC_CONNECTIONS);
    assert_eq!(connection_limit(Some(u64::MAX)), MAX_RPC_CONNECTIONS);
}

async fn bound(limit: usize, idle: Duration) -> (LimitedListener, SocketAddr) {
    let inner = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = inner.local_addr().unwrap();
    (LimitedListener::with_idle_timeout(inner, limit, idle), addr)
}

#[tokio::test]
async fn test_listener_holds_at_most_limit_connections() {
    // Per-address share is a quarter of the total; every client here is 127.0.0.1.
    let (mut listener, addr) = bound(8, IDLE_TIMEOUT).await;
    let clients: Vec<TcpStream> = futures_connect(addr, 3).await;
    let (first, _) = listener.accept().await;
    let (second, _) = listener.accept().await;
    // The third connection from the same address is over its share (2 of 8): closed at once.
    let third = tokio::time::timeout(Duration::from_millis(300), listener.accept()).await;
    assert!(
        third.is_err(),
        "a third connection from one address was kept"
    );
    drop(first);
    let again = TcpStream::connect(addr).await.unwrap();
    let accepted = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await;
    assert!(
        accepted.is_ok(),
        "a slot freed by a closed connection is reused"
    );
    drop((second, again, clients));
}

#[tokio::test]
async fn test_total_bound_makes_new_clients_wait() {
    let (mut listener, addr) = bound(1, IDLE_TIMEOUT).await;
    let _a = TcpStream::connect(addr).await.unwrap();
    let _b = TcpStream::connect(addr).await.unwrap(); // waits in the backlog
    let (first, _) = listener.accept().await;
    let waiting = tokio::time::timeout(Duration::from_millis(300), listener.accept()).await;
    assert!(waiting.is_err(), "accepted over the total bound");
    drop(first);
    let accepted = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await;
    assert!(
        accepted.is_ok(),
        "the waiting client is accepted once a slot frees"
    );
}

#[tokio::test]
async fn test_idle_connection_is_closed_and_active_one_is_not() {
    let idle_timeout = Duration::from_millis(300);
    let (mut listener, addr) = bound(8, idle_timeout).await;
    let mut client = TcpStream::connect(addr).await.unwrap();
    let (mut server, _) = listener.accept().await;
    // Traffic keeps it open past the idle timeout.
    for _ in 0..4 {
        tokio::time::sleep(idle_timeout / 2).await;
        server.write_all(b"x").await.unwrap();
        let mut byte = [0u8; 1];
        client.read_exact(&mut byte).await.unwrap();
    }
    // Then silence: the server's read fails once the deadline passes.
    let mut buf = [0u8; 8];
    let read = tokio::time::timeout(idle_timeout * 4, server.read(&mut buf)).await;
    let error = read
        .expect("idle read did not end")
        .expect_err("idle read succeeded");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

async fn futures_connect(addr: SocketAddr, n: usize) -> Vec<TcpStream> {
    let mut clients = Vec::with_capacity(n);
    for _ in 0..n {
        clients.push(TcpStream::connect(addr).await.unwrap());
    }
    clients
}

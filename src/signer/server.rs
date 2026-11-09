//! gRPC signer server over a Unix domain socket (key custody D3).
//!
//! The server owns the validator key and the slashing-protection guard through a
//! `GuardedSigner`; the node holds neither. Access control is layered:
//! - the socket file is created mode 0600;
//! - each connection's peer uid (`SO_PEERCRED`) must equal the server's own uid.
//!
//! A signer fault (guard storage failure, request for another chain) halts the server: every
//! later request fails, and `serve` returns an error so the process exits non-zero.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hotstuff_rs::types::signer::{SignError, ValidatorSigner as _};
use tokio::net::{UnixListener, UnixStream};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{Request, Response, Status};

use super::convert::from_wire;
use super::proto;
use super::proto::validator_signer_server::{ValidatorSigner, ValidatorSignerServer};
use crate::consensus::{GuardedSigner, SafetyHalt};

/// Owner-only socket permissions.
const SOCKET_MODE: u32 = 0o600;
/// After shutdown or a halt, connected clients get this long before the server stops anyway.
/// tonic's graceful shutdown otherwise waits for every client to disconnect, so a halted signer
/// with a connected validator would never exit.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
const HALT_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// Whether a connecting process may use the signer.
pub fn peer_allowed(peer_uid: u32, server_uid: u32) -> bool {
    peer_uid == server_uid
}

pub struct SignerService {
    signer: Arc<GuardedSigner>,
    halt: SafetyHalt,
}

impl SignerService {
    pub fn new(signer: Arc<GuardedSigner>, halt: SafetyHalt) -> Self {
        Self { signer, halt }
    }

    fn check_running(&self) -> Result<(), Status> {
        match self.halt.reason() {
            Some(reason) => Err(Status::unavailable(format!("signer halted: {reason}"))),
            None => Ok(()),
        }
    }
}

#[tonic::async_trait]
impl ValidatorSigner for SignerService {
    async fn get_public_key(
        &self,
        _: Request<proto::GetPublicKeyRequest>,
    ) -> Result<Response<proto::GetPublicKeyResponse>, Status> {
        self.check_running()?;
        Ok(Response::new(proto::GetPublicKeyResponse {
            public_key: self.signer.public_key().to_bytes().to_vec(),
            chain_id: self.signer.chain_id(),
        }))
    }

    async fn sign(
        &self,
        request: Request<proto::SignRequest>,
    ) -> Result<Response<proto::SignResponse>, Status> {
        self.check_running()?;
        let request = from_wire(request.get_ref()).map_err(Status::invalid_argument)?;
        let signer = Arc::clone(&self.signer);
        // The guard fsyncs; keep blocking I/O off the async workers.
        let outcome = tokio::task::spawn_blocking(move || signer.sign(&request))
            .await
            .map_err(|e| Status::internal(format!("signing task failed: {e}")))?;
        let result = match outcome {
            Ok(signature) => proto::sign_response::Result::Signature(signature.bytes().to_vec()),
            Err(SignError::Refused(reason)) => proto::sign_response::Result::Refused(reason),
            Err(SignError::Failed(reason)) => {
                self.halt.trigger(format!("signer: {reason}"));
                return Err(Status::internal(reason));
            }
        };
        Ok(Response::new(proto::SignResponse {
            result: Some(result),
        }))
    }
}

/// Bind the socket: replace a stale socket left by a crash, refuse anything else at `path`.
pub fn bind_socket(path: &Path) -> Result<UnixListener, String> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if !metadata.file_type().is_socket() {
            return Err(format!(
                "{} exists and is not a socket; refusing to replace it",
                path.display()
            ));
        }
        std::fs::remove_file(path)
            .map_err(|e| format!("cannot remove stale socket {}: {e}", path.display()))?;
    }
    let listener = UnixListener::bind(path)
        .map_err(|e| format!("cannot bind signer socket {}: {e}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(SOCKET_MODE))
        .map_err(|e| format!("cannot restrict signer socket {}: {e}", path.display()))?;
    Ok(listener)
}

/// Serve until `shutdown` resolves or the signer halts. Connections from other uids are dropped.
pub async fn serve(
    listener: UnixListener,
    socket_path: PathBuf,
    service: SignerService,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), String> {
    let server_uid = crate::keys::effective_uid();
    let halt = service.halt.clone();
    let incoming = UnixListenerStream::new(listener).filter(move |conn| match conn {
        Ok(stream) => connection_allowed(stream, server_uid),
        Err(_) => true,
    });
    // One "stop" signal (shutdown requested or halted) starts the graceful shutdown; if clients
    // keep it open past SHUTDOWN_GRACE, the server future is dropped, closing them.
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let server = tonic::transport::Server::builder()
        .add_service(ValidatorSignerServer::new(service))
        .serve_with_incoming_shutdown(incoming, async move {
            let _ = stop_rx.wait_for(|stop| *stop).await;
        });
    let halt_for_stop = halt.clone();
    let stopper = async move {
        let halted = async {
            while !halt_for_stop.is_halted() {
                tokio::time::sleep(HALT_POLL).await;
            }
        };
        tokio::select! {
            () = shutdown => {}
            () = halted => {}
        }
        let _ = stop_tx.send(true);
        tokio::time::sleep(SHUTDOWN_GRACE).await;
    };
    let result = tokio::select! {
        result = server => result,
        () = stopper => Ok(()),
    };
    let _ = std::fs::remove_file(&socket_path);
    result.map_err(|e| format!("signer server failed: {e}"))?;
    match halt.reason() {
        Some(reason) => Err(format!("signer halted: {reason}")),
        None => Ok(()),
    }
}

fn connection_allowed(stream: &UnixStream, server_uid: u32) -> bool {
    match stream.peer_cred() {
        Ok(cred) if peer_allowed(cred.uid(), server_uid) => true,
        Ok(cred) => {
            tracing::warn!(
                "signer: rejected connection from uid {} (only uid {server_uid} may sign)",
                cred.uid()
            );
            false
        }
        Err(e) => {
            tracing::warn!("signer: rejected connection without peer credentials: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_only_the_server_uid_may_connect() {
        assert!(peer_allowed(10001, 10001));
        assert!(!peer_allowed(0, 10001));
        assert!(!peer_allowed(10002, 10001));
    }

    #[tokio::test]
    async fn test_bind_replaces_stale_socket_but_not_other_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signer.sock");

        drop(bind_socket(&path).unwrap());
        assert!(path.exists(), "stale socket left behind");
        let listener = bind_socket(&path).expect("stale socket is replaced");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, SOCKET_MODE);
        drop(listener);

        let regular = dir.path().join("not-a-socket");
        std::fs::write(&regular, b"keep me").unwrap();
        assert!(bind_socket(&regular).is_err());
        assert_eq!(std::fs::read(&regular).unwrap(), b"keep me");
    }
}

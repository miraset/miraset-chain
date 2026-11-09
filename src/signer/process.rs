//! The `miraset-chain signer` process (key custody D3).

use std::path::PathBuf;
use std::sync::Arc;

use crate::consensus::{GuardIdentity, GuardedSigner, OpenMode, SafetyHalt, SigningGuard};
use crate::keys::read_key_file;
use crate::node::{GenesisConfig, genesis_state_commitment};

use super::server::{SignerService, bind_socket, serve};

/// Exit status when the signer stops because of a safety halt (same as a validator halt).
pub const SIGNER_HALT_EXIT_CODE: i32 = 3;

pub struct SignerOptions {
    pub genesis: PathBuf,
    pub key_file: PathBuf,
    pub signing_guard: PathBuf,
    pub socket: PathBuf,
    /// The signer cannot know whether its key has signed before, so it never creates a guard
    /// implicitly: the first start needs this flag.
    pub init_signing_guard: bool,
}

/// How the signer process ended.
pub enum SignerExit {
    /// Startup or transport error.
    Error(String),
    /// Safety halt (guard fault, request for another chain).
    Halted(String),
}

/// Load key, genesis and guard, then serve until `shutdown` or a halt.
pub async fn run_signer(
    options: SignerOptions,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), SignerExit> {
    let signer = load_signer(&options).map_err(SignerExit::Error)?;
    let last_view = signer
        .last_signed_view()
        .map_err(|e| SignerExit::Error(e.to_string()))?;
    let listener = bind_socket(&options.socket).map_err(SignerExit::Error)?;
    // No structured fields: operators and tests read "last signed view: N)" at the line end.
    tracing::info!(
        "MIRASET validator signer (experimental, ADR-0016 Proposed); validator {}; chain {}; \
         Socket: {}; signing guard {} (last signed view: {})",
        hex::encode(signer_public_key(&signer)),
        signer.chain_id(),
        options.socket.display(),
        options.signing_guard.display(),
        last_view.map_or_else(|| "none".to_string(), |view| view.to_string())
    );

    let halt = SafetyHalt::new();
    let service = SignerService::new(signer, halt.clone());
    serve(listener, options.socket.clone(), service, shutdown)
        .await
        .map_err(|e| match halt.reason() {
            Some(reason) => SignerExit::Halted(reason.to_string()),
            None => SignerExit::Error(e),
        })
}

fn signer_public_key(signer: &GuardedSigner) -> [u8; 32] {
    use hotstuff_rs::types::signer::ValidatorSigner;
    signer.public_key().to_bytes()
}

fn load_signer(options: &SignerOptions) -> Result<Arc<GuardedSigner>, String> {
    let genesis = GenesisConfig::load(&options.genesis).map_err(|e| e.to_string())?;
    let key = read_key_file(&options.key_file)?;
    let vk = key.verifying_key();
    if !genesis.is_validator(&vk) {
        return Err(format!(
            "key {} is not a validator in {}",
            hex::encode(vk.to_bytes()),
            options.genesis.display()
        ));
    }
    if let Some(parent) = options.signing_guard.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let identity = GuardIdentity {
        chain_id: genesis.chain_id,
        validator: vk.to_bytes(),
        genesis_hash: genesis_state_commitment(&genesis.initial_accounts()),
    };
    let mode = if options.init_signing_guard {
        OpenMode::CreateIfMissing
    } else {
        OpenMode::RequireExisting
    };
    let guard =
        SigningGuard::open(&options.signing_guard, identity, mode).map_err(|e| e.to_string())?;
    GuardedSigner::new(key, guard)
}

//! Remote validator signer over gRPC (TODO 8.3.4, `docs/adr/proposal-validator-key-custody.md`
//! D3 and decision 2).
//!
//! `miraset-chain signer` runs [`server`] in its own process, holding the validator key and the
//! slashing-protection guard. A validator started with `--remote-signer` uses
//! [`client::RemoteSigner`] and holds no key. Transport: gRPC over a Unix domain socket with
//! peer-uid checks; a cross-host transport (mTLS) depends on TODO 8.4.1.

pub mod client;
pub mod convert;
pub mod process;
pub mod server;

pub mod proto {
    #![allow(clippy::all, missing_docs)]
    tonic::include_proto!("miraset.signer.v1");
}

pub use client::{RemoteSigner, RemoteSignerExpectation};
pub use server::{SignerService, bind_socket, serve};

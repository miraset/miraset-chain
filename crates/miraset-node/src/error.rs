use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

/// Storage-level errors.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sled error: {0}")]
    Sled(#[from] sled::Error),
    #[error("bincode error: {0}")]
    Bincode(#[from] bincode::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("corrupt value for key {key}: {details}")]
    CorruptValue { key: String, details: String },
}

/// State-level errors (block production, object lifecycle, etc.).
#[derive(Debug, Error)]
pub enum StateError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("serialization error: {0}")]
    Serialization(#[from] bincode::Error),
    #[error("no genesis block available")]
    NoGenesis,
    #[error("block not found: {0}")]
    BlockNotFound(u64),
    #[error("object not found: {0}")]
    ObjectNotFound(String),
    #[error("worker not found: {0}")]
    WorkerNotFound(String),
    #[error("job not found: {0}")]
    JobNotFound(String),
    #[error("not owner")]
    NotOwner,
    #[error("version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u64, got: u64 },
    #[error("{0}")]
    Other(String),
}

impl From<String> for StateError {
    fn from(value: String) -> Self {
        Self::Other(value)
    }
}

/// Transaction validation errors.
#[derive(Debug, Error)]
pub enum TxError {
    #[error("invalid signature")]
    InvalidSignature,
    #[error("invalid nonce: expected {expected}, got {got}")]
    InvalidNonce { expected: u64, got: u64 },
    #[error("insufficient balance")]
    InsufficientBalance,
    #[error("invalid message length")]
    InvalidMessageLength,
    #[error("object not found")]
    ObjectNotFound,
    #[error("escrow exceeds balance")]
    InsufficientEscrow,
    #[error("serialization error: {0}")]
    Serialization(#[from] bincode::Error),
    #[error("{0}")]
    Other(String),
}

impl From<String> for TxError {
    fn from(value: String) -> Self {
        Self::Other(value)
    }
}

impl TxError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::InvalidSignature
            | Self::InvalidNonce { .. }
            | Self::InvalidMessageLength
            | Self::ObjectNotFound
            | Self::InsufficientBalance
            | Self::InsufficientEscrow
            | Self::Other(_) => StatusCode::BAD_REQUEST,
            Self::Serialization(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for TxError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = Json(serde_json::json!({"error": self.to_string()}));
        (status, body).into_response()
    }
}

impl StateError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::BlockNotFound(_)
            | Self::ObjectNotFound(_)
            | Self::WorkerNotFound(_)
            | Self::JobNotFound(_) => StatusCode::NOT_FOUND,
            Self::NotOwner | Self::VersionMismatch { .. } | Self::Other(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::Storage(_) | Self::Serialization(_) | Self::NoGenesis => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

impl IntoResponse for StateError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = Json(serde_json::json!({"error": self.to_string()}));
        (status, body).into_response()
    }
}

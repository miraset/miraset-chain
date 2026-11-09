//! Logs, metrics, health probes and lifecycle (TODO 8.4.4, PRD R12).

pub mod logging;
pub mod metrics;
pub mod readiness;

pub use logging::{LogFormat, init_logging};
pub use metrics::{AdmissionKind, AdmissionReject, NodeMetrics, ScrapeState};
pub use readiness::{
    DEFAULT_READY_MAX_COMMIT_AGE, Lifecycle, NotReadyReason, ReadinessInput, evaluate_readiness,
};

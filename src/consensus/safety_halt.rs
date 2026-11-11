//! Process-wide consensus safety halt (`docs/adr/proposal-vote-safety-state.md` D3).
//!
//! Once triggered, the node sends no further consensus messages, `/v1/health` reports 503 with the
//! reason, and the validator binary exits non-zero after a grace period so the orchestrator
//! restarts it. The first reason wins; later triggers are ignored.
//!
//! Besides explicit triggers (signing guard faults, chain-store write failures), a panic on any
//! `hotstuff_rs` thread (`hs-algorithm`, `hs-poller`, `hs-sync-server`, `hs-event-bus`) halts the
//! node once [`SafetyHalt::install_consensus_panic_hook`] is installed: such a panic stops
//! consensus for good and must not leave `/v1/health` reporting `ok`.

use std::sync::{Arc, OnceLock};

/// Thread-name prefix of the vendored `hotstuff_rs` threads (vendored patch #5).
const CONSENSUS_THREAD_PREFIX: &str = "hs-";
/// Keep halt reasons short; panic messages can embed large debug output.
const MAX_PANIC_MESSAGE_CHARS: usize = 200;

/// Halt reason for a panic, or `None` if the panicking thread is not a consensus thread.
pub fn consensus_thread_panic_reason(thread_name: Option<&str>, message: &str) -> Option<String> {
    let name = thread_name.filter(|name| name.starts_with(CONSENSUS_THREAD_PREFIX))?;
    let message: String = message.chars().take(MAX_PANIC_MESSAGE_CHARS).collect();
    Some(format!("consensus thread {name} panicked: {message}"))
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_string())
}

#[derive(Clone, Default, Debug)]
pub struct SafetyHalt(Arc<OnceLock<String>>);

impl SafetyHalt {
    pub fn new() -> Self {
        Self::default()
    }

    /// Halt with `reason`. Returns `true` if this call triggered the halt.
    pub fn trigger(&self, reason: impl Into<String>) -> bool {
        let reason = reason.into();
        let triggered = self.0.set(reason).is_ok();
        if triggered {
            tracing::error!(target: crate::observability::logging::SAFETY_LOG_TARGET, "SAFETY HALT: {}", self.reason().unwrap_or_default());
        }
        triggered
    }

    pub fn is_halted(&self) -> bool {
        self.0.get().is_some()
    }

    pub fn reason(&self) -> Option<&str> {
        self.0.get().map(String::as_str)
    }

    /// Install a process-wide panic hook that halts on any consensus-thread panic. The previous
    /// hook still runs, so panic output is unchanged. Call once, from the binary's `main`.
    pub fn install_consensus_panic_hook(&self) {
        let halt = self.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            if let Some(reason) = consensus_thread_panic_reason(thread.name(), &panic_message(info))
            {
                halt.trigger(reason);
            }
            previous(info);
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_reason_wins_and_is_shared_across_clones() {
        let halt = SafetyHalt::new();
        let observer = halt.clone();
        assert!(!observer.is_halted());
        assert!(halt.trigger("first"));
        assert!(!halt.trigger("second"));
        assert_eq!(observer.reason(), Some("first"));
    }

    #[test]
    fn test_only_consensus_thread_panics_produce_a_halt_reason() {
        assert_eq!(
            consensus_thread_panic_reason(Some("hs-algorithm"), "Pacemaker failure!"),
            Some("consensus thread hs-algorithm panicked: Pacemaker failure!".to_string())
        );
        assert_eq!(
            consensus_thread_panic_reason(Some("tokio-runtime-worker"), "x"),
            None
        );
        assert_eq!(consensus_thread_panic_reason(None, "x"), None);
    }

    #[test]
    fn test_panic_halt_reason_is_truncated() {
        let long = "x".repeat(MAX_PANIC_MESSAGE_CHARS * 3);
        let reason = consensus_thread_panic_reason(Some("hs-poller"), &long).unwrap();
        assert!(reason.len() < MAX_PANIC_MESSAGE_CHARS + 50);
    }
}

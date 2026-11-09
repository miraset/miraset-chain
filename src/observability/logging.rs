//! Structured logging setup (TODO 8.4.4, PRD R12).
//!
//! All diagnostics go to stderr through `tracing`, as human-readable text or one JSON object per
//! line. The vendored `hotstuff_rs` logs through the `log` crate; those records are bridged into
//! the same subscriber. Filtering follows `RUST_LOG` (default `info`).
//!
//! Secret safety: key types never implement `Display` with secret bytes and their `Debug` is
//! redacted (`ed25519-dalek`, `src/keys.rs`), and no code path formats key material into a log
//! event. `tests/e2e_observability.rs` checks the real binary's output for the key.
//!
//! Safety events (safety halt, refused or dropped votes) use the separate target
//! [`SAFETY_LOG_TARGET`], which is always enabled at `warn` whatever `RUST_LOG` says, so a
//! filter cannot hide the `SAFETY HALT` line that operators and the fault suite look for.

use clap::ValueEnum;
use tracing_subscriber::EnvFilter;

const DEFAULT_FILTER: &str = "info";

/// `tracing` target for consensus safety events; not under the crate prefix, so filters such
/// as `miraset_chain=off` do not reach it.
pub const SAFETY_LOG_TARGET: &str = "miraset_safety";

/// Log output format for `--log-format`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}

/// Install the global subscriber. A second call (for example in tests) is a no-op.
pub fn init_logging(format: LogFormat) {
    let (filter, rejected) = build_filter(std::env::var("RUST_LOG").ok().as_deref());
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true);
    let _ = match format {
        LogFormat::Text => builder.with_ansi(false).try_init(),
        LogFormat::Json => builder.json().flatten_event(true).try_init(),
    };
    if let Some(error) = rejected {
        tracing::warn!("RUST_LOG is invalid ({error}); using `{DEFAULT_FILTER}`");
    }
}

/// Parse `RUST_LOG` (default `info`) and append the safety floor. Returns the parse error when
/// the variable was set but invalid, so the caller can warn once logging works.
fn build_filter(rust_log: Option<&str>) -> (EnvFilter, Option<String>) {
    let (filter, rejected) = match rust_log.filter(|v| !v.trim().is_empty()) {
        None => (EnvFilter::new(DEFAULT_FILTER), None),
        Some(spec) => match EnvFilter::try_new(spec) {
            Ok(filter) => (filter, None),
            Err(error) => (EnvFilter::new(DEFAULT_FILTER), Some(error.to_string())),
        },
    };
    let floor = format!("{SAFETY_LOG_TARGET}=warn")
        .parse()
        .expect("static safety directive parses");
    (filter.add_directive(floor), rejected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Level;
    use tracing::level_filters::LevelFilter;

    fn max_level(spec: Option<&str>) -> Option<LevelFilter> {
        build_filter(spec).0.max_level_hint()
    }

    #[test]
    fn test_unset_or_blank_rust_log_uses_info() {
        assert!(build_filter(None).1.is_none());
        assert!(build_filter(Some("  ")).1.is_none());
        assert_eq!(max_level(None), Some(LevelFilter::INFO));
    }

    #[test]
    fn test_invalid_rust_log_is_reported_and_falls_back() {
        let (_, rejected) = build_filter(Some("miraset_chain=notalevel"));
        assert!(rejected.is_some());
    }

    #[test]
    fn test_safety_target_survives_restrictive_filters() {
        for spec in [
            "off",
            "error",
            "miraset_chain=off",
            "warn,miraset_safety=off",
        ] {
            let rendered = build_filter(Some(spec)).0.to_string();
            assert!(
                rendered.contains("miraset_safety=warn"),
                "{spec}: {rendered}"
            );
            // The floor keeps warn events enabled somewhere even under `off`.
            assert!(max_level(Some(spec)) >= Some(LevelFilter::from_level(Level::WARN)));
        }
    }
}

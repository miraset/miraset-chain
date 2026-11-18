//! Host TCP settings the transport depends on (TODO 8.5.3 findings 36–37).
//!
//! Linux resets a connection's congestion window to its initial 10 segments after it idles
//! for one retransmission timeout (`net.ipv4.tcp_slow_start_after_idle=1`, the default). The
//! transport keeps one TCP stream per validator pair, and consensus traffic is bursty, so every
//! proposal started from about 290 KB/s at 50 ms RTT. In the 2026-11-18 WAN measurement this was
//! the cause of late proposals and of a lagging validator never catching up; with the setting
//! at 0 the node stayed within 5 blocks and 1 000 tx/s committed in full. The setting belongs
//! to the network namespace (one per container), so the node can only report it.

const SLOW_START_AFTER_IDLE: &str = "/proc/sys/net/ipv4/tcp_slow_start_after_idle";

/// Whether the contents of `tcp_slow_start_after_idle` mean the setting is on.
fn slow_start_after_idle_enabled(contents: &str) -> bool {
    contents.trim() != "0"
}

/// Log a warning at start if the host restarts TCP slow start after idle. Silent where the
/// setting cannot be read (not Linux, restricted `/proc`).
pub(crate) fn warn_if_slow_start_after_idle() {
    let Ok(contents) = std::fs::read_to_string(SLOW_START_AFTER_IDLE) else {
        return;
    };
    if slow_start_after_idle_enabled(&contents) {
        tracing::warn!(
            setting = "net.ipv4.tcp_slow_start_after_idle",
            value = contents.trim(),
            "TCP restarts slow start after idle: on WAN links validators fall behind; \
             set it to 0 for the node's network namespace (docs/devnet.md)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::slow_start_after_idle_enabled;

    #[test]
    fn only_zero_disables_slow_start_after_idle() {
        assert!(slow_start_after_idle_enabled("1\n"));
        assert!(!slow_start_after_idle_enabled("0\n"));
        assert!(!slow_start_after_idle_enabled("0"));
    }
}

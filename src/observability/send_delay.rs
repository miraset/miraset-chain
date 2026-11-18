//! Outbound send delay by message kind (TODO 8.5.3 findings 36–37).
//!
//! Each peer has one outbound queue in front of one TCP stream. On a lossy WAN link that stream
//! carries about 1 MB/s, so a message may wait behind queued proposals and block-sync responses
//! for seconds. The delay of a message is measured from when the replica hands it to the
//! transport until its frame is fully written to the socket (the kernel's send buffer, not the
//! peer). Exported as `miraset_p2p_send_delay_seconds{kind}`. Only written frames are recorded:
//! messages dropped by a full queue or a failed write (counted in `miraset_p2p_dropped`) are not,
//! and a reconnect is included in the delay of the message that triggered it, so the longest
//! delays are under-reported.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use hotstuff_rs::block_sync::messages::BlockSyncMessage;
use hotstuff_rs::hotstuff::messages::HotStuffMessage;
use hotstuff_rs::networking::messages::{Message, ProgressMessage};

/// Upper bounds of the histogram buckets, in seconds (`+Inf` is implied).
pub const SEND_DELAY_BUCKETS_SECS: [f64; 10] =
    [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0];

/// What an outbound message is, for [`SendDelays`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Proposal,
    Vote,
    NewView,
    Nudge,
    Pacemaker,
    SyncAdvertise,
    SyncRequest,
    SyncResponse,
}

impl MessageKind {
    const ALL: [MessageKind; 8] = [
        MessageKind::Proposal,
        MessageKind::Vote,
        MessageKind::NewView,
        MessageKind::Nudge,
        MessageKind::Pacemaker,
        MessageKind::SyncAdvertise,
        MessageKind::SyncRequest,
        MessageKind::SyncResponse,
    ];

    pub fn of(message: &Message) -> Self {
        match message {
            Message::ProgressMessage(ProgressMessage::HotStuffMessage(m)) => match m {
                HotStuffMessage::Proposal(_) => MessageKind::Proposal,
                HotStuffMessage::PhaseVote(_) => MessageKind::Vote,
                HotStuffMessage::NewView(_) => MessageKind::NewView,
                HotStuffMessage::Nudge(_) => MessageKind::Nudge,
            },
            Message::ProgressMessage(ProgressMessage::PacemakerMessage(_)) => {
                MessageKind::Pacemaker
            }
            Message::ProgressMessage(ProgressMessage::BlockSyncAdvertiseMessage(_)) => {
                MessageKind::SyncAdvertise
            }
            Message::BlockSyncMessage(BlockSyncMessage::BlockSyncRequest(_)) => {
                MessageKind::SyncRequest
            }
            Message::BlockSyncMessage(BlockSyncMessage::BlockSyncResponse(_)) => {
                MessageKind::SyncResponse
            }
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn label(self) -> &'static str {
        match self {
            MessageKind::Proposal => "proposal",
            MessageKind::Vote => "vote",
            MessageKind::NewView => "new_view",
            MessageKind::Nudge => "nudge",
            MessageKind::Pacemaker => "pacemaker",
            MessageKind::SyncAdvertise => "sync_advertise",
            MessageKind::SyncRequest => "sync_request",
            MessageKind::SyncResponse => "sync_response",
        }
    }
}

const KINDS: usize = MessageKind::ALL.len();
const BUCKETS: usize = SEND_DELAY_BUCKETS_SECS.len() + 1;

/// Cumulative send-delay histograms, one per [`MessageKind`] (lock-free).
#[derive(Debug, Default)]
pub struct SendDelays {
    /// Non-cumulative bucket counts; the last bucket is `+Inf`.
    buckets: [[AtomicU64; BUCKETS]; KINDS],
    sum_micros: [AtomicU64; KINDS],
}

impl SendDelays {
    /// One message of `kind` was written `delay` after it was queued.
    pub fn record(&self, kind: MessageKind, delay: Duration) {
        let secs = delay.as_secs_f64();
        let bucket = SEND_DELAY_BUCKETS_SECS
            .iter()
            .position(|&bound| secs <= bound)
            .unwrap_or(BUCKETS - 1);
        self.buckets[kind.index()][bucket].fetch_add(1, Ordering::Relaxed);
        self.sum_micros[kind.index()].fetch_add(
            u64::try_from(delay.as_micros()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

    /// Prometheus text for `miraset_p2p_send_delay_seconds` (kinds never sent are omitted).
    pub fn render(&self, out: &mut String) {
        let _ = writeln!(
            out,
            "# HELP miraset_p2p_send_delay_seconds Time from queueing an outbound P2P message \
             to writing its frame, by message kind.\n\
             # TYPE miraset_p2p_send_delay_seconds histogram"
        );
        for kind in MessageKind::ALL {
            let counts: Vec<u64> = self.buckets[kind.index()]
                .iter()
                .map(|c| c.load(Ordering::Relaxed))
                .collect();
            let total: u64 = counts.iter().sum();
            if total == 0 {
                continue;
            }
            let label = kind.label();
            let mut cumulative = 0;
            for (bound, count) in SEND_DELAY_BUCKETS_SECS.iter().zip(&counts) {
                cumulative += count;
                let _ = writeln!(
                    out,
                    "miraset_p2p_send_delay_seconds_bucket{{kind=\"{label}\",le=\"{bound}\"}} \
                     {cumulative}"
                );
            }
            let sum = self.sum_micros[kind.index()].load(Ordering::Relaxed) as f64 / 1e6;
            let _ = writeln!(
                out,
                "miraset_p2p_send_delay_seconds_bucket{{kind=\"{label}\",le=\"+Inf\"}} {total}\n\
                 miraset_p2p_send_delay_seconds_sum{{kind=\"{label}\"}} {sum}\n\
                 miraset_p2p_send_delay_seconds_count{{kind=\"{label}\"}} {total}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(delays: &SendDelays) -> String {
        let mut out = String::new();
        delays.render(&mut out);
        out
    }

    #[test]
    fn delays_fall_into_cumulative_buckets_per_kind() {
        let delays = SendDelays::default();
        delays.record(MessageKind::Proposal, Duration::from_millis(5));
        delays.record(MessageKind::Proposal, Duration::from_millis(700));
        delays.record(MessageKind::Proposal, Duration::from_secs(60));
        let out = rendered(&delays);
        assert!(out.contains("bucket{kind=\"proposal\",le=\"0.01\"} 1\n"));
        assert!(out.contains("bucket{kind=\"proposal\",le=\"0.5\"} 1\n"));
        assert!(out.contains("bucket{kind=\"proposal\",le=\"1\"} 2\n"));
        assert!(out.contains("bucket{kind=\"proposal\",le=\"30\"} 2\n"));
        assert!(out.contains("bucket{kind=\"proposal\",le=\"+Inf\"} 3\n"));
        assert!(out.contains("count{kind=\"proposal\"} 3\n"));
        assert!(out.contains("sum{kind=\"proposal\"} 60.705\n"));
    }

    #[test]
    fn a_delay_on_a_bucket_bound_counts_in_that_bucket() {
        let delays = SendDelays::default();
        delays.record(MessageKind::Vote, Duration::from_millis(100));
        assert!(rendered(&delays).contains("bucket{kind=\"vote\",le=\"0.1\"} 1\n"));
    }

    #[test]
    fn kinds_never_sent_are_omitted() {
        let delays = SendDelays::default();
        delays.record(MessageKind::SyncResponse, Duration::from_millis(1));
        let out = rendered(&delays);
        assert!(out.contains("kind=\"sync_response\""));
        assert!(!out.contains("kind=\"proposal\""));
    }
}

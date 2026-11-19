//! Per-view consensus trace (TODO 8.5.3 finding 36), off by default.
//!
//! About one view in three times out on a lossy WAN devnet although the transport drops nothing,
//! and a counter cannot say where such a view loses its time. With
//! `RUST_LOG=info,miraset_chain::view_trace=debug` every node logs one line per consensus event
//! of a view: start, proposal sent or received (with the block's payload size), votes received,
//! certificates collected and timeout. `ts_ms` is the event's own timestamp (Unix ms, taken on
//! the consensus thread), so lines from all nodes of one host can be merged into a timeline even
//! when the event bus delivers them late.

use std::time::{SystemTime, UNIX_EPOCH};

use hotstuff_rs::events::{
    CollectPCEvent, CollectTCEvent, ProposeEvent, ReceivePhaseVoteEvent, ReceiveProposalEvent,
    StartViewEvent, ViewTimeoutEvent,
};
use hotstuff_rs::types::block::Block;
use hotstuff_rs::types::crypto_primitives::VerifyingKey;

const TARGET: &str = "miraset_chain::view_trace";

fn unix_ms(timestamp: SystemTime) -> u128 {
    timestamp
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn payload_bytes(block: &Block) -> usize {
    block.data.vec().iter().map(|d| d.bytes().len()).sum()
}

/// First 4 bytes of a validator key in hex, as the start of the `validator=` startup log field.
fn key_prefix(key: &VerifyingKey) -> String {
    hex::encode(&key.to_bytes()[..4])
}

pub fn start_view(event: &StartViewEvent) {
    tracing::debug!(target: TARGET, view = event.view.int(), ts_ms = %unix_ms(event.timestamp), "start_view");
}

pub fn propose(event: &ProposeEvent) {
    let block = &event.proposal.block;
    tracing::debug!(
        target: TARGET,
        view = event.proposal.view.int(),
        height = block.height.int(),
        payload_bytes = payload_bytes(block),
        ts_ms = %unix_ms(event.timestamp),
        "propose"
    );
}

pub fn receive_proposal(event: &ReceiveProposalEvent) {
    let block = &event.proposal.block;
    tracing::debug!(
        target: TARGET,
        view = event.proposal.view.int(),
        from = %key_prefix(&event.origin),
        height = block.height.int(),
        payload_bytes = payload_bytes(block),
        ts_ms = %unix_ms(event.timestamp),
        "receive_proposal"
    );
}

pub fn receive_phase_vote(event: &ReceivePhaseVoteEvent) {
    tracing::debug!(
        target: TARGET,
        view = event.phase_vote.view.int(),
        from = %key_prefix(&event.origin),
        phase = ?event.phase_vote.phase,
        ts_ms = %unix_ms(event.timestamp),
        "receive_vote"
    );
}

pub fn collect_pc(event: &CollectPCEvent) {
    tracing::debug!(
        target: TARGET,
        view = event.phase_certificate.view.int(),
        phase = ?event.phase_certificate.phase,
        ts_ms = %unix_ms(event.timestamp),
        "collect_pc"
    );
}

pub fn view_timeout(event: &ViewTimeoutEvent) {
    tracing::debug!(target: TARGET, view = event.view.int(), ts_ms = %unix_ms(event.timestamp), "view_timeout");
}

pub fn collect_tc(event: &CollectTCEvent) {
    tracing::debug!(
        target: TARGET,
        view = event.timeout_certificate.view.int(),
        ts_ms = %unix_ms(event.timestamp),
        "collect_tc"
    );
}

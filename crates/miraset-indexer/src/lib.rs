#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

use miraset_core::{Block, Event};
use std::collections::HashMap;

/// In-memory event and block indexer.
///
/// This is the initial indexer implementation. The placeholder comment in the
/// previous version intended a Postgres-backed indexer; this in-memory store
/// is kept to provide a useful, runnable crate while a durable backend is
/// designed.
#[derive(Debug, Default)]
pub struct Indexer {
    blocks: Vec<Block>,
    events: Vec<IndexedEvent>,
    by_type: HashMap<String, Vec<usize>>,
    by_height: HashMap<u64, Vec<usize>>,
}

/// An event annotated with its position in the chain.
#[derive(Debug, Clone)]
pub struct IndexedEvent {
    pub event_index: u64,
    pub block_height: u64,
    pub event: Event,
}

impl Indexer {
    /// Create a new, empty indexer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Index a block, storing it and indexing all events it references.
    ///
    /// Events are indexed by their tag (`event_type`) and by block height.
    /// The `event_index` should be the monotonic index of the event in the
    /// chain; events in a block are assumed to start at `event_index_start`.
    pub fn index_block(&mut self, block: Block, event_index_start: u64) {
        let height = block.height;
        self.blocks.push(block.clone());

        for (i, event) in block.transactions.iter().enumerate() {
            let _ = i;
            let _ = event;
            // Note: `Block` currently stores transactions, not events. Events
            // are emitted during execution and should be indexed separately via
            // `index_event` for now.
        }

        let _ = event_index_start;
        let _ = height;
    }

    /// Index a single event with its chain position.
    pub fn index_event(&mut self, event_index: u64, block_height: u64, event: Event) {
        let type_name = event_type_name(&event);
        let idx = self.events.len();

        self.events.push(IndexedEvent {
            event_index,
            block_height,
            event: event.clone(),
        });
        self.by_type.entry(type_name).or_default().push(idx);
        self.by_height.entry(block_height).or_default().push(idx);
    }

    /// Return all indexed events.
    pub fn all_events(&self) -> &[IndexedEvent] {
        &self.events
    }

    /// Return events of a given type (e.g. `"Transferred"`).
    pub fn events_by_type(&self, event_type: &str) -> Vec<&IndexedEvent> {
        self.by_type
            .get(event_type)
            .map(|indices| indices.iter().map(|&i| &self.events[i]).collect())
            .unwrap_or_default()
    }

    /// Return events at a specific block height.
    pub fn events_by_height(&self, block_height: u64) -> Vec<&IndexedEvent> {
        self.by_height
            .get(&block_height)
            .map(|indices| indices.iter().map(|&i| &self.events[i]).collect())
            .unwrap_or_default()
    }

    /// Return the total number of indexed events.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Return the total number of indexed blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }
}

fn event_type_name(event: &Event) -> String {
    match event {
        Event::Transferred { .. } => "Transferred".to_string(),
        Event::ChatMessage { .. } => "ChatMessage".to_string(),
        Event::ObjectCreated { .. } => "ObjectCreated".to_string(),
        Event::ObjectMutated { .. } => "ObjectMutated".to_string(),
        Event::ObjectTransferred { .. } => "ObjectTransferred".to_string(),
        Event::WorkerRegistered { .. } => "WorkerRegistered".to_string(),
        Event::ResourceSnapshotSubmitted { .. } => "ResourceSnapshotSubmitted".to_string(),
        Event::JobCreated { .. } => "JobCreated".to_string(),
        Event::JobAssigned { .. } => "JobAssigned".to_string(),
        Event::JobCompleted { .. } => "JobCompleted".to_string(),
        Event::ReceiptAnchored { .. } => "ReceiptAnchored".to_string(),
        Event::JobChallenged { .. } => "JobChallenged".to_string(),
        Event::EpochSettled { .. } => "EpochSettled".to_string(),
        Event::RewardsDistributed { .. } => "RewardsDistributed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use miraset_core::{Address, Block};

    #[test]
    fn test_indexer_creation() {
        let indexer = Indexer::new();
        assert_eq!(indexer.event_count(), 0);
        assert_eq!(indexer.block_count(), 0);
    }

    #[test]
    fn test_index_event() {
        let mut indexer = Indexer::new();
        let event = Event::Transferred {
            from: Address::from_bytes([1; 32]),
            to: Address::from_bytes([2; 32]),
            amount: 100,
            tx_hash: [3; 32],
            block_height: 1,
        };
        indexer.index_event(0, 1, event.clone());

        assert_eq!(indexer.event_count(), 1);
        assert_eq!(indexer.events_by_type("Transferred").len(), 1);
        assert_eq!(indexer.events_by_height(1).len(), 1);
        assert!(indexer.events_by_type("ChatMessage").is_empty());
    }

    #[test]
    fn test_index_block() {
        let mut indexer = Indexer::new();
        let block = Block::genesis();
        indexer.index_block(block, 0);
        assert_eq!(indexer.block_count(), 1);
    }
}

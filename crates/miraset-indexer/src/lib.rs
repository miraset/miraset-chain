// Placeholder for indexer - will be expanded in future iterations
// This crate will consume events from the chain and store them in Postgres

pub struct Indexer;

impl Indexer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Indexer {
    fn default() -> Self {
        Self::new()
    }
}

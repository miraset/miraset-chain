use miraset_core::{Address, Block, Event, Transaction};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct State {
    inner: Arc<RwLock<StateInner>>,
}

struct StateInner {
    balances: HashMap<Address, u64>,
    nonces: HashMap<Address, u64>,
    blocks: Vec<Block>,
    pending_txs: Vec<Transaction>,
    events: Vec<Event>,
}

impl State {
    pub fn new() -> Self {
        let genesis = Block::genesis();
        Self {
            inner: Arc::new(RwLock::new(StateInner {
                balances: HashMap::new(),
                nonces: HashMap::new(),
                blocks: vec![genesis],
                pending_txs: Vec::new(),
                events: Vec::new(),
            })),
        }
    }

    pub fn get_balance(&self, addr: &Address) -> u64 {
        self.inner.read().balances.get(addr).copied().unwrap_or(0)
    }

    pub fn get_nonce(&self, addr: &Address) -> u64 {
        self.inner.read().nonces.get(addr).copied().unwrap_or(0)
    }

    pub fn add_balance(&self, addr: &Address, amount: u64) {
        let mut w = self.inner.write();
        *w.balances.entry(*addr).or_insert(0) += amount;
    }

    pub fn submit_transaction(&self, tx: Transaction) -> Result<(), String> {
        // Basic validation
        let from = tx.from();
        let nonce = tx.nonce();
        let signature = tx.signature();

        // Verify signature
        let tx_copy = tx.clone();
        let mut tx_for_hash = tx_copy;
        // Remove signature for hashing
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            Transaction::ChatSend { signature, .. } => *signature = [0; 64],
            Transaction::WorkerRegister { signature, .. } => *signature = [0; 64],
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        if !miraset_core::verify_signature(from, &msg, signature) {
            return Err("Invalid signature".into());
        }

        let mut w = self.inner.write();

        // Check nonce
        let current_nonce = w.nonces.get(from).copied().unwrap_or(0);
        if nonce != current_nonce {
            return Err(format!(
                "Invalid nonce: expected {}, got {}",
                current_nonce, nonce
            ));
        }

        // Type-specific validation
        match &tx {
            Transaction::Transfer { to, amount, .. } => {
                let balance = w.balances.get(from).copied().unwrap_or(0);
                if balance < *amount {
                    return Err("Insufficient balance".into());
                }
            }
            Transaction::ChatSend { message, .. } => {
                if message.is_empty() || message.len() > 1000 {
                    return Err("Invalid message length".into());
                }
            }
            Transaction::WorkerRegister { .. } => {
                // No additional validation for MVP
            }
        }

        w.pending_txs.push(tx);
        Ok(())
    }

    pub fn produce_block(&self) -> Block {
        let mut w = self.inner.write();
        let prev = w.blocks.last().unwrap();
        let height = prev.height + 1;
        let prev_hash = prev.hash();

        let transactions = std::mem::take(&mut w.pending_txs);

        // Execute transactions
        for tx in &transactions {
            // Update nonce
            let from = tx.from();
            *w.nonces.entry(*from).or_insert(0) += 1;

            // Execute
            match tx {
                Transaction::Transfer { from, to, amount, .. } => {
                    let balance = w.balances.get(from).copied().unwrap_or(0);
                    w.balances.insert(*from, balance - amount);
                    *w.balances.entry(*to).or_insert(0) += amount;
                    w.events.push(Event::Transferred {
                        from: *from,
                        to: *to,
                        amount: *amount,
                        tx_hash: tx.hash(),
                        block_height: height,
                    });
                }
                Transaction::ChatSend { from, message, .. } => {
                    w.events.push(Event::ChatMessage {
                        from: *from,
                        message: message.clone(),
                        tx_hash: tx.hash(),
                        block_height: height,
                        timestamp: chrono::Utc::now(),
                    });
                }
                Transaction::WorkerRegister {
                    from,
                    gpu_model,
                    vram_gib,
                    ..
                } => {
                    w.events.push(Event::WorkerRegistered {
                        address: *from,
                        gpu_model: gpu_model.clone(),
                        vram_gib: *vram_gib,
                        tx_hash: tx.hash(),
                        block_height: height,
                    });
                }
            }
        }

        let block = Block {
            height,
            timestamp: chrono::Utc::now(),
            prev_hash,
            transactions,
            state_root: [0; 32], // Simplified for MVP
        };

        w.blocks.push(block.clone());
        block
    }

    pub fn get_latest_block(&self) -> Block {
        self.inner.read().blocks.last().unwrap().clone()
    }

    pub fn get_block(&self, height: u64) -> Option<Block> {
        self.inner
            .read()
            .blocks
            .iter()
            .find(|b| b.height == height)
            .cloned()
    }

    pub fn get_events(&self, from_height: u64, limit: usize) -> Vec<Event> {
        self.inner
            .read()
            .events
            .iter()
            .filter(|e| match e {
                Event::Transferred { block_height, .. } => *block_height >= from_height,
                Event::ChatMessage { block_height, .. } => *block_height >= from_height,
                Event::WorkerRegistered { block_height, .. } => *block_height >= from_height,
            })
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn get_chat_messages(&self, limit: usize) -> Vec<(Address, String, chrono::DateTime<chrono::Utc>)> {
        self.inner
            .read()
            .events
            .iter()
            .filter_map(|e| match e {
                Event::ChatMessage {
                    from,
                    message,
                    timestamp,
                    ..
                } => Some((*from, message.clone(), *timestamp)),
                _ => None,
            })
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn height(&self) -> u64 {
        self.inner.read().blocks.last().unwrap().height
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

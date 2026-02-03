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
            Transaction::Transfer { amount, .. } => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use miraset_core::KeyPair;

    #[test]
    fn test_state_new() {
        let state = State::new();
        assert_eq!(state.height(), 0);
    }

    #[test]
    fn test_get_balance_zero() {
        let state = State::new();
        let kp = KeyPair::generate();
        assert_eq!(state.get_balance(&kp.address()), 0);
    }

    #[test]
    fn test_add_balance() {
        let state = State::new();
        let kp = KeyPair::generate();

        state.add_balance(&kp.address(), 1000);
        assert_eq!(state.get_balance(&kp.address()), 1000);

        state.add_balance(&kp.address(), 500);
        assert_eq!(state.get_balance(&kp.address()), 1500);
    }

    #[test]
    fn test_get_nonce_initial() {
        let state = State::new();
        let kp = KeyPair::generate();
        assert_eq!(state.get_nonce(&kp.address()), 0);
    }

    #[test]
    fn test_submit_transfer_valid() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 2000);

        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 500,
            nonce: 0,
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::Transfer { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_submit_transfer_insufficient_balance() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 100);

        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 500,
            nonce: 0,
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::Transfer { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Insufficient balance"));
    }

    #[test]
    fn test_submit_transfer_invalid_nonce() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 2000);

        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 500,
            nonce: 5, // Wrong nonce
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::Transfer { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid nonce"));
    }

    #[test]
    fn test_submit_transfer_invalid_signature() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 2000);

        let tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 500,
            nonce: 0,
            signature: [0; 64], // Invalid signature
        };

        let result = state.submit_transaction(tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid signature"));
    }

    #[test]
    fn test_submit_chat_valid() {
        let state = State::new();
        let kp = KeyPair::generate();

        let mut tx = Transaction::ChatSend {
            from: kp.address(),
            message: "Hello, blockchain!".to_string(),
            nonce: 0,
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::ChatSend { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::ChatSend { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_submit_chat_empty_message() {
        let state = State::new();
        let kp = KeyPair::generate();

        let mut tx = Transaction::ChatSend {
            from: kp.address(),
            message: "".to_string(),
            nonce: 0,
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::ChatSend { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::ChatSend { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid message length"));
    }

    #[test]
    fn test_submit_chat_message_too_long() {
        let state = State::new();
        let kp = KeyPair::generate();

        let long_message = "x".repeat(1001);
        let mut tx = Transaction::ChatSend {
            from: kp.address(),
            message: long_message,
            nonce: 0,
            signature: [0; 64],
        };

        // Sign transaction
        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::ChatSend { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::ChatSend { signature, .. } => *signature = sig,
            _ => {}
        }

        let result = state.submit_transaction(tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid message length"));
    }

    #[test]
    fn test_produce_block() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 1000);

        // Submit transaction
        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 300,
            nonce: 0,
            signature: [0; 64],
        };

        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::Transfer { signature, .. } => *signature = sig,
            _ => {}
        }

        state.submit_transaction(tx).unwrap();

        // Produce block
        let block = state.produce_block();

        assert_eq!(block.height, 1);
        assert_eq!(block.transactions.len(), 1);

        // Check balances updated
        assert_eq!(state.get_balance(&kp.address()), 700);
        assert_eq!(state.get_balance(&recipient.address()), 300);

        // Check nonce incremented
        assert_eq!(state.get_nonce(&kp.address()), 1);
    }

    #[test]
    fn test_produce_block_multiple_transactions() {
        let state = State::new();
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();

        state.add_balance(&kp1.address(), 1000);
        state.add_balance(&kp2.address(), 1000);

        // Submit two transactions
        for (kp, amount) in [(kp1, 100), (kp2, 200)] {
            let mut tx = Transaction::ChatSend {
                from: kp.address(),
                message: format!("Message {}", amount),
                nonce: 0,
                signature: [0; 64],
            };

            let mut tx_for_hash = tx.clone();
            match &mut tx_for_hash {
                Transaction::ChatSend { signature, .. } => *signature = [0; 64],
                _ => {}
            }
            let msg = bincode::serialize(&tx_for_hash).unwrap();
            let sig = kp.sign(&msg);

            match &mut tx {
                Transaction::ChatSend { signature, .. } => *signature = sig,
                _ => {}
            }

            state.submit_transaction(tx).unwrap();
        }

        let block = state.produce_block();
        assert_eq!(block.transactions.len(), 2);
        assert_eq!(state.height(), 1);
    }

    #[test]
    fn test_get_latest_block() {
        let state = State::new();
        let latest = state.get_latest_block();
        assert_eq!(latest.height, 0);
    }

    #[test]
    fn test_get_block_by_height() {
        let state = State::new();

        let genesis = state.get_block(0);
        assert!(genesis.is_some());
        assert_eq!(genesis.unwrap().height, 0);

        let nonexistent = state.get_block(999);
        assert!(nonexistent.is_none());
    }

    #[test]
    fn test_get_events() {
        let state = State::new();
        let kp = KeyPair::generate();
        let recipient = KeyPair::generate();

        state.add_balance(&kp.address(), 1000);

        // Submit and produce block
        let mut tx = Transaction::Transfer {
            from: kp.address(),
            to: recipient.address(),
            amount: 100,
            nonce: 0,
            signature: [0; 64],
        };

        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::Transfer { signature, .. } => *signature = sig,
            _ => {}
        }

        state.submit_transaction(tx).unwrap();
        state.produce_block();

        let events = state.get_events(0, 10);
        assert_eq!(events.len(), 1);

        match &events[0] {
            Event::Transferred { from, to, amount, .. } => {
                assert_eq!(*from, kp.address());
                assert_eq!(*to, recipient.address());
                assert_eq!(*amount, 100);
            }
            _ => panic!("Expected Transferred event"),
        }
    }

    #[test]
    fn test_get_chat_messages() {
        let state = State::new();
        let kp = KeyPair::generate();

        // Submit chat messages and produce block after each
        for i in 1..=3 {
            let mut tx = Transaction::ChatSend {
                from: kp.address(),
                message: format!("Message {}", i),
                nonce: i - 1,
                signature: [0; 64],
            };

            let mut tx_for_hash = tx.clone();
            match &mut tx_for_hash {
                Transaction::ChatSend { signature, .. } => *signature = [0; 64],
                _ => {}
            }
            let msg = bincode::serialize(&tx_for_hash).unwrap();
            let sig = kp.sign(&msg);

            match &mut tx {
                Transaction::ChatSend { signature, .. } => *signature = sig,
                _ => {}
            }

            state.submit_transaction(tx).unwrap();
            state.produce_block(); // Produce block to increment nonce
        }

        let messages = state.get_chat_messages(10);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].1, "Message 1");
        assert_eq!(messages[1].1, "Message 2");
        assert_eq!(messages[2].1, "Message 3");
    }

    #[test]
    fn test_worker_register() {
        let state = State::new();
        let kp = KeyPair::generate();

        let mut tx = Transaction::WorkerRegister {
            from: kp.address(),
            gpu_model: "NVIDIA RTX 4090".to_string(),
            vram_gib: 24,
            nonce: 0,
            signature: [0; 64],
        };

        let mut tx_for_hash = tx.clone();
        match &mut tx_for_hash {
            Transaction::WorkerRegister { signature, .. } => *signature = [0; 64],
            _ => {}
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        match &mut tx {
            Transaction::WorkerRegister { signature, .. } => *signature = sig,
            _ => {}
        }

        state.submit_transaction(tx).unwrap();
        state.produce_block();

        let events = state.get_events(0, 10);
        assert_eq!(events.len(), 1);

        match &events[0] {
            Event::WorkerRegistered { address, gpu_model, vram_gib, .. } => {
                assert_eq!(*address, kp.address());
                assert_eq!(gpu_model, "NVIDIA RTX 4090");
                assert_eq!(*vram_gib, 24);
            }
            _ => panic!("Expected WorkerRegistered event"),
        }
    }

    #[test]
    fn test_height() {
        let state = State::new();
        assert_eq!(state.height(), 0);

        state.produce_block();
        assert_eq!(state.height(), 1);

        state.produce_block();
        assert_eq!(state.height(), 2);
    }
}

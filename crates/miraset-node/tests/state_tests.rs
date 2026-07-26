// State unit tests, moved from src/state/mod.rs to keep the source module focused.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use miraset_core::{Event, KeyPair, Transaction};
use miraset_node::State;
use miraset_node::error::TxError;

#[test]
fn test_state_new() {
    let state = State::new();
    assert_eq!(state.height().unwrap(), 0);
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
    if let Transaction::Transfer { signature, .. } = &mut tx_for_hash {
        *signature = [0; 64]
    }
    let msg = bincode::serialize(&tx_for_hash).unwrap();
    let sig = kp.sign(&msg);

    if let Transaction::Transfer { signature, .. } = &mut tx {
        *signature = sig
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
    if let Transaction::Transfer { signature, .. } = &mut tx_for_hash {
        *signature = [0; 64]
    }
    let msg = bincode::serialize(&tx_for_hash).unwrap();
    let sig = kp.sign(&msg);

    if let Transaction::Transfer { signature, .. } = &mut tx {
        *signature = sig
    }

    let result = state.submit_transaction(tx);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), TxError::InsufficientBalance));
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
    if let Transaction::Transfer { signature, .. } = &mut tx_for_hash {
        *signature = [0; 64]
    }
    let msg = bincode::serialize(&tx_for_hash).unwrap();
    let sig = kp.sign(&msg);

    if let Transaction::Transfer { signature, .. } = &mut tx {
        *signature = sig
    }

    let result = state.submit_transaction(tx);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), TxError::InvalidNonce { .. }));
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
    assert!(matches!(result.unwrap_err(), TxError::InvalidSignature));
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
    if let Transaction::ChatSend { signature, .. } = &mut tx_for_hash {
        *signature = [0; 64]
    }
    let msg = bincode::serialize(&tx_for_hash).unwrap();
    let sig = kp.sign(&msg);

    if let Transaction::ChatSend { signature, .. } = &mut tx {
        *signature = sig
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
    if let Transaction::ChatSend { signature, .. } = &mut tx_for_hash {
        *signature = [0; 64]
    }
    let msg = bincode::serialize(&tx_for_hash).unwrap();
    let sig = kp.sign(&msg);

    if let Transaction::ChatSend { signature, .. } = &mut tx {
        *signature = sig
    }

    let result = state.submit_transaction(tx);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), TxError::InvalidMessageLength));
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
    if let Transaction::ChatSend { signature, .. } = &mut tx_for_hash {
        *signature = [0; 64]
    }
    let msg = bincode::serialize(&tx_for_hash).unwrap();
    let sig = kp.sign(&msg);

    if let Transaction::ChatSend { signature, .. } = &mut tx {
        *signature = sig
    }

    let result = state.submit_transaction(tx);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), TxError::InvalidMessageLength));
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
    if let Transaction::Transfer { signature, .. } = &mut tx_for_hash {
        *signature = [0; 64]
    }
    let msg = bincode::serialize(&tx_for_hash).unwrap();
    let sig = kp.sign(&msg);

    if let Transaction::Transfer { signature, .. } = &mut tx {
        *signature = sig
    }

    state.submit_transaction(tx).unwrap();

    // Produce block
    let block = state.produce_block().unwrap();

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
        if let Transaction::ChatSend { signature, .. } = &mut tx_for_hash {
            *signature = [0; 64]
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        if let Transaction::ChatSend { signature, .. } = &mut tx {
            *signature = sig
        }

        state.submit_transaction(tx).unwrap();
    }

    let block = state.produce_block().unwrap();
    assert_eq!(block.transactions.len(), 2);
    assert_eq!(state.height().unwrap(), 1);
}

#[test]
fn test_get_latest_block() {
    let state = State::new();
    let latest = state.get_latest_block().unwrap();
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
    if let Transaction::Transfer { signature, .. } = &mut tx_for_hash {
        *signature = [0; 64]
    }
    let msg = bincode::serialize(&tx_for_hash).unwrap();
    let sig = kp.sign(&msg);

    if let Transaction::Transfer { signature, .. } = &mut tx {
        *signature = sig
    }

    state.submit_transaction(tx).unwrap();
    state.produce_block().unwrap();

    let events = state.get_events(0, 10);
    assert_eq!(events.len(), 1);

    match &events[0] {
        Event::Transferred {
            from, to, amount, ..
        } => {
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
        if let Transaction::ChatSend { signature, .. } = &mut tx_for_hash {
            *signature = [0; 64]
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = kp.sign(&msg);

        if let Transaction::ChatSend { signature, .. } = &mut tx {
            *signature = sig
        }

        state.submit_transaction(tx).unwrap();
        state.produce_block().unwrap(); // Produce block to increment nonce
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

    let mut tx = Transaction::RegisterWorker {
        owner: kp.address(),
        pubkey: kp.address(),
        endpoints: vec!["http://localhost:8080".to_string()],
        gpu_model: "NVIDIA RTX 4090".to_string(),
        vram_total_gib: 24,
        supported_models: vec!["llama-3-8b".to_string()],
        stake_bond: 1000,
        nonce: 0,
        signature: [0; 64],
    };

    let mut tx_for_hash = tx.clone();
    if let Transaction::RegisterWorker { signature, .. } = &mut tx_for_hash {
        *signature = [0; 64]
    }
    let msg = bincode::serialize(&tx_for_hash).unwrap();
    let sig = kp.sign(&msg);

    if let Transaction::RegisterWorker { signature, .. } = &mut tx {
        *signature = sig
    }

    state.submit_transaction(tx).unwrap();
    state.produce_block().unwrap();

    let events = state.get_events(0, 10);
    assert_eq!(events.len(), 1);

    match &events[0] {
        Event::WorkerRegistered {
            owner,
            gpu_model,
            vram_gib,
            ..
        } => {
            assert_eq!(*owner, kp.address());
            assert_eq!(gpu_model, "NVIDIA RTX 4090");
            assert_eq!(*vram_gib, 24);
        }
        _ => panic!("Expected WorkerRegistered event"),
    }
}

#[test]
fn test_height() {
    let state = State::new();
    assert_eq!(state.height().unwrap(), 0);

    state.produce_block().unwrap();
    assert_eq!(state.height().unwrap(), 1);

    state.produce_block().unwrap();
    assert_eq!(state.height().unwrap(), 2);
}

#[test]
fn test_create_job_auto_assigns_worker() {
    let state = State::new();
    let worker_kp = KeyPair::generate();
    let requester_kp = KeyPair::generate();

    state.add_balance(&requester_kp.address(), 100_000);

    // Register a worker that supports the requested model.
    let mut register_tx = Transaction::RegisterWorker {
        owner: worker_kp.address(),
        pubkey: worker_kp.address(),
        endpoints: vec!["http://localhost:8080".to_string()],
        gpu_model: "NVIDIA RTX 4090".to_string(),
        vram_total_gib: 24,
        supported_models: vec!["llama-3-8b".to_string()],
        stake_bond: 1000,
        nonce: 0,
        signature: [0; 64],
    };
    {
        let mut tx_for_hash = register_tx.clone();
        if let Transaction::RegisterWorker { signature, .. } = &mut tx_for_hash {
            *signature = [0; 64];
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = worker_kp.sign(&msg);
        if let Transaction::RegisterWorker { signature, .. } = &mut register_tx {
            *signature = sig;
        }
    }
    state.submit_transaction(register_tx).unwrap();

    // Create a job for the supported model.
    let mut create_tx = Transaction::CreateJob {
        requester: requester_kp.address(),
        model_id: "llama-3-8b".to_string(),
        max_tokens: 1000,
        escrow_amount: 10_000,
        nonce: 0,
        signature: [0; 64],
    };
    {
        let mut tx_for_hash = create_tx.clone();
        if let Transaction::CreateJob { signature, .. } = &mut tx_for_hash {
            *signature = [0; 64];
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = requester_kp.sign(&msg);
        if let Transaction::CreateJob { signature, .. } = &mut create_tx {
            *signature = sig;
        }
    }
    state.submit_transaction(create_tx).unwrap();

    // Produce block: execution should create the job and auto-assign it.
    state.produce_block().unwrap();

    // Three events should be present: worker registration, job creation, and
    // auto-assignment.
    let events = state.get_events(0, 10);
    assert_eq!(events.len(), 3);
    assert!(matches!(events[0], Event::WorkerRegistered { .. }));
    assert!(matches!(events[1], Event::JobCreated { .. }));
    assert!(matches!(events[2], Event::JobAssigned { .. }));

    // Job should be assigned to the registered worker.
    let jobs = state.get_jobs();
    assert_eq!(jobs.len(), 1);
    let (_job_id, job_obj) = &jobs[0];
    match &job_obj.data {
        miraset_core::ObjectData::InferenceJob {
            assigned_worker_id,
            status,
            ..
        } => {
            assert!(assigned_worker_id.is_some());
            assert_eq!(*status, miraset_core::JobStatus::Assigned);
        }
        _ => panic!("expected inference job"),
    }

    // Escrow was deducted.
    assert_eq!(state.get_balance(&requester_kp.address()), 90_000);
}

#[test]
fn test_create_job_insufficient_escrow() {
    let state = State::new();
    let requester_kp = KeyPair::generate();

    state.add_balance(&requester_kp.address(), 1000);

    let mut create_tx = Transaction::CreateJob {
        requester: requester_kp.address(),
        model_id: "llama-3-8b".to_string(),
        max_tokens: 100,
        escrow_amount: 10_000,
        nonce: 0,
        signature: [0; 64],
    };
    {
        let mut tx_for_hash = create_tx.clone();
        if let Transaction::CreateJob { signature, .. } = &mut tx_for_hash {
            *signature = [0; 64];
        }
        let msg = bincode::serialize(&tx_for_hash).unwrap();
        let sig = requester_kp.sign(&msg);
        if let Transaction::CreateJob { signature, .. } = &mut create_tx {
            *signature = sig;
        }
    }

    let result = state.submit_transaction(create_tx);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), TxError::InsufficientEscrow));
}

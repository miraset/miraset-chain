use crate::error::TxError;
use crate::state::State;
use miraset_core::Transaction;

/// Validate a transaction (signature, nonce, type-specific rules) and enqueue
/// it into the pending transaction pool.
///
/// This function is the implementation backing [`State::submit_transaction`].
/// It is kept in a separate module so that the main `State` facade only
/// handles lock acquisition and delegation.
pub(crate) fn submit_transaction(state: &State, tx: Transaction) -> Result<(), TxError> {
    let from = tx.from();
    let nonce = tx.nonce();
    let signature = tx.signature();

    // D6: Verify signature for ALL transaction types using zero-sig canonical pattern
    {
        let mut tx_for_hash = tx.clone();
        // Zero out signature for canonical hashing
        match &mut tx_for_hash {
            Transaction::Transfer { signature, .. }
            | Transaction::ChatSend { signature, .. }
            | Transaction::CreateObject { signature, .. }
            | Transaction::MutateObject { signature, .. }
            | Transaction::TransferObject { signature, .. }
            | Transaction::RegisterWorker { signature, .. }
            | Transaction::SubmitResourceSnapshot { signature, .. }
            | Transaction::CreateJob { signature, .. }
            | Transaction::AssignJob { signature, .. }
            | Transaction::SubmitJobResult { signature, .. }
            | Transaction::AnchorReceipt { signature, .. }
            | Transaction::ChallengeJob { signature, .. } => *signature = [0; 64],
        }
        let msg = bincode::serialize(&tx_for_hash)?;
        if !miraset_core::verify_signature(from, &msg, signature) {
            return Err(TxError::InvalidSignature);
        }
    }

    let mut w = state.inner.write();

    // Check nonce
    let current_nonce = w.nonces.get(from).copied().unwrap_or(0);
    if nonce != current_nonce {
        return Err(TxError::InvalidNonce {
            expected: current_nonce,
            got: nonce,
        });
    }

    // Type-specific validation
    match &tx {
        Transaction::Transfer { amount, .. } => {
            let balance = w.balances.get(from).copied().unwrap_or(0);
            if balance < *amount {
                return Err(TxError::InsufficientBalance);
            }
        }
        Transaction::ChatSend { message, .. } => {
            if message.is_empty() || message.len() > 1000 {
                return Err(TxError::InvalidMessageLength);
            }
        }
        Transaction::MutateObject { object_id, .. } => {
            if !w.objects.contains_key(object_id) {
                return Err(TxError::ObjectNotFound);
            }
        }
        Transaction::CreateJob { escrow_amount, .. } => {
            let balance = w.balances.get(from).copied().unwrap_or(0);
            if balance < *escrow_amount {
                return Err(TxError::InsufficientEscrow);
            }
        }
        _ => {
            // Other transactions validated during execution
        }
    }

    w.pending_txs.push(tx);
    Ok(())
}

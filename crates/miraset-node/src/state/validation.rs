use crate::error::TxError;
use crate::gas::{GasBudget, estimate_gas_for_transaction, gas_cost_tokens};
use crate::state::State;
use miraset_core::Transaction;
use std::sync::Arc;

/// Validate a transaction (signature, nonce, gas, type-specific rules) and
/// enqueue it into the pending transaction pool.
///
/// This function is the implementation backing [`State::submit_transaction`].
/// It is kept in a separate module so that the main `State` facade only
/// handles lock acquisition and delegation.
pub(crate) fn submit_transaction(state: &State, tx: Transaction) -> Result<(), TxError> {
    let from = tx.from();
    let nonce = tx.nonce();

    // D6: Verify signature for ALL transaction types using the chain-scoped
    // canonical message.
    if !miraset_core::verify_transaction_signature(&tx) {
        return Err(TxError::InvalidSignature);
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

    // Mempool cap
    if w.pending_txs.len() >= w.mempool_capacity {
        return Err(TxError::MempoolFull);
    }

    // Gas pre-check: reject txs that cannot possibly afford the estimated
    // execution cost. The tx still needs a gas coin / budget; for the
    // account-model chain we use the sender's balance as the gas payer.
    let gas_config = Arc::clone(&state.gas_config);
    let estimated_gas = estimate_gas_for_transaction(&tx, &gas_config);
    let min_budget = GasBudget::default_budget();
    if estimated_gas > min_budget.max_gas_amount {
        return Err(TxError::GasBudgetExceeded);
    }

    // Type-specific validation (including gas funds)
    match &tx {
        Transaction::Transfer { amount, .. } => {
            let balance = w.balances.get(from).copied().unwrap_or(0);
            let gas_cost = gas_cost_tokens(estimated_gas, gas_config.base_fee.max(1));
            let total = amount
                .checked_add(gas_cost)
                .ok_or(TxError::InsufficientBalance)?;
            if balance < total {
                return Err(TxError::InsufficientBalance);
            }
        }
        Transaction::ChatSend { message, .. } => {
            if message.is_empty() || message.len() > 1000 {
                return Err(TxError::InvalidMessageLength);
            }
            let balance = w.balances.get(from).copied().unwrap_or(0);
            let gas_cost = gas_cost_tokens(estimated_gas, gas_config.base_fee.max(1));
            if balance < gas_cost {
                return Err(TxError::InsufficientGas);
            }
        }
        Transaction::MutateObject { object_id, .. } => {
            if !w.objects.contains_key(object_id) {
                return Err(TxError::ObjectNotFound);
            }
            let balance = w.balances.get(from).copied().unwrap_or(0);
            let gas_cost = gas_cost_tokens(estimated_gas, gas_config.base_fee.max(1));
            if balance < gas_cost {
                return Err(TxError::InsufficientGas);
            }
        }
        Transaction::CreateJob { escrow_amount, .. } => {
            let balance = w.balances.get(from).copied().unwrap_or(0);
            let gas_cost = gas_cost_tokens(estimated_gas, gas_config.base_fee.max(1));
            let total = escrow_amount
                .checked_add(gas_cost)
                .ok_or(TxError::InsufficientEscrow)?;
            if balance < total {
                return Err(TxError::InsufficientEscrow);
            }
        }
        _ => {
            let balance = w.balances.get(from).copied().unwrap_or(0);
            let gas_cost = gas_cost_tokens(estimated_gas, gas_config.base_fee.max(1));
            if balance < gas_cost {
                return Err(TxError::InsufficientGas);
            }
        }
    }

    w.pending_txs.push(tx);
    Ok(())
}

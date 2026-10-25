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
    submit_transaction_inner(state, tx, None)
}

pub(crate) fn submit_job_with_prompt(
    state: &State,
    tx: Transaction,
    prompt: String,
) -> Result<(), TxError> {
    if !matches!(tx, Transaction::CreateJob { .. }) {
        return Err(TxError::Other(
            "only Transaction::CreateJob accepts an off-chain prompt".to_string(),
        ));
    }
    if prompt.trim().is_empty() || prompt.len() > super::MAX_JOB_PROMPT_BYTES {
        return Err(TxError::Other(format!(
            "job prompt must be non-empty and at most {} bytes",
            super::MAX_JOB_PROMPT_BYTES
        )));
    }
    submit_transaction_inner(state, tx, Some(prompt))
}

fn submit_transaction_inner(
    state: &State,
    tx: Transaction,
    prompt: Option<String>,
) -> Result<(), TxError> {
    let from = tx.from();
    let nonce = tx.nonce();

    // D6: Verify signature for ALL transaction types using the chain-scoped
    // canonical message.
    if !miraset_core::verify_transaction_signature(&tx) {
        return Err(TxError::InvalidSignature);
    }

    let persisted_nonce = state.get_nonce(from);
    let persisted_balance = state.get_balance(from);
    let tx_hash = if prompt.is_some() {
        Some(tx.hash().map_err(TxError::Serialization)?)
    } else {
        None
    };

    let mut w = state.inner.write();
    let current_nonce = *w.nonces.entry(*from).or_insert(persisted_nonce);
    let current_balance = *w.balances.entry(*from).or_insert(persisted_balance);

    // Check nonce
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
            let gas_cost = gas_cost_tokens(estimated_gas, gas_config.base_fee.max(1));
            let total = amount
                .checked_add(gas_cost)
                .ok_or(TxError::InsufficientBalance)?;
            if current_balance < total {
                return Err(TxError::InsufficientBalance);
            }
        }
        Transaction::ChatSend { message, .. } => {
            if message.is_empty() || message.len() > 1000 {
                return Err(TxError::InvalidMessageLength);
            }
            let gas_cost = gas_cost_tokens(estimated_gas, gas_config.base_fee.max(1));
            if current_balance < gas_cost {
                return Err(TxError::InsufficientGas);
            }
        }
        Transaction::MutateObject { object_id, .. } => {
            if !w.objects.contains_key(object_id) {
                return Err(TxError::ObjectNotFound);
            }
            let gas_cost = gas_cost_tokens(estimated_gas, gas_config.base_fee.max(1));
            if current_balance < gas_cost {
                return Err(TxError::InsufficientGas);
            }
        }
        Transaction::CreateJob { escrow_amount, .. } => {
            let gas_cost = gas_cost_tokens(estimated_gas, gas_config.base_fee.max(1));
            let total = escrow_amount
                .checked_add(gas_cost)
                .ok_or(TxError::InsufficientEscrow)?;
            if current_balance < total {
                return Err(TxError::InsufficientEscrow);
            }
        }
        _ => {
            let gas_cost = gas_cost_tokens(estimated_gas, gas_config.base_fee.max(1));
            if current_balance < gas_cost {
                return Err(TxError::InsufficientGas);
            }
        }
    }

    if let (Some(prompt), Some(tx_hash)) = (prompt, tx_hash) {
        w.pending_job_prompts.insert(tx_hash, prompt);
    }
    w.pending_txs.push(tx);
    Ok(())
}

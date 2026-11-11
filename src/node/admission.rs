//! Admission checks for submitted transactions and compute receipts (TODO 8.4.3).
//!
//! Items are checked against **committed** state and the node's pending chain before they enter
//! the mempool, so a client cannot fill block production with items that can never apply:
//! forged signatures, replayed nonces or settled tasks, nonce gaps, fees below the floor and
//! transfers the committed balance cannot cover are refused at the RPC. Balances are not
//! accumulated over a sender's pending chain and system payloads are not interpreted here;
//! block production re-checks everything on top of its parent and evicts what fails.

use crate::core::types::{AccountState, ComputeReceipt, Transaction};
use crate::mempool::InsertError;

/// Why a transaction or receipt was not admitted to the mempool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    #[error("already exists in mempool")]
    Duplicate,
    #[error("node is shutting down and not admitting new work")]
    Draining,
    /// The item can never apply as submitted.
    #[error("invalid: {0}")]
    Invalid(&'static str),
    /// The item, or another one with the same nonce or task, is already committed.
    #[error("already committed: {0}")]
    Replay(&'static str),
    #[error("mempool is full")]
    MempoolFull,
    #[error("too many pending transactions from this sender")]
    SenderLimit,
}

impl From<InsertError> for AdmissionError {
    fn from(err: InsertError) -> Self {
        match err {
            InsertError::Duplicate => Self::Duplicate,
            InsertError::Full => Self::MempoolFull,
            InsertError::SenderLimit => Self::SenderLimit,
            InsertError::Malformed => Self::Invalid("missing identifier"),
            InsertError::FeeTooLow => Self::Invalid("fee below the node minimum"),
            InsertError::NonceGap => {
                Self::Invalid("nonce must follow the committed and pending nonces of the sender")
            }
        }
    }
}

/// Check a transaction against its sender's committed account (default for a new account).
///
/// `next_nonce` is the nonce the sender's pending chain needs next: the committed nonce, or
/// one past the sender's last pending nonce. Requiring it keeps every pending chain contiguous,
/// so no admitted transaction waits on a nonce that is not pending. Clients therefore submit a
/// sender's pending sequence to one node (nodes do not share mempools). Cheap checks run before
/// the signature is verified.
pub fn check_transaction(
    tx: &Transaction,
    committed: &AccountState,
    next_nonce: u64,
    min_fee: u64,
) -> Result<(), AdmissionError> {
    if tx.sender.is_empty() || tx.recipient.is_empty() {
        return Err(AdmissionError::Invalid("missing sender or recipient"));
    }
    if tx.sender.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err(AdmissionError::Invalid("sender must be lowercase hex"));
    }
    if tx.fee < min_fee {
        return Err(AdmissionError::Invalid("fee below the node minimum"));
    }
    if tx.nonce < committed.nonce {
        return Err(AdmissionError::Replay("nonce already used"));
    }
    if tx.nonce != next_nonce {
        return Err(AdmissionError::Invalid(
            "nonce must follow the committed and pending nonces of the sender",
        ));
    }
    let required = tx
        .amount
        .checked_add(tx.fee)
        .ok_or(AdmissionError::Invalid("amount + fee overflow"))?;
    if required > committed.balance {
        return Err(AdmissionError::Invalid("insufficient committed balance"));
    }
    tx.verify_signature().map_err(AdmissionError::Invalid)
}

/// Check a compute receipt: task replay, client confirmation, the client's committed balance,
/// then the coordinator signature. `client` is the committed client account when the receipt
/// names one.
pub fn check_receipt(
    receipt: &ComputeReceipt,
    task_settled: bool,
    client: Option<&AccountState>,
) -> Result<(), AdmissionError> {
    if receipt.task_id.is_empty() || receipt.worker_id.is_empty() {
        return Err(AdmissionError::Invalid("missing task or worker identifier"));
    }
    if task_settled {
        return Err(AdmissionError::Replay("task already settled"));
    }
    if !receipt.client_address.is_empty() {
        if !receipt.client_confirmed {
            return Err(AdmissionError::Invalid("client did not confirm delivery"));
        }
        let balance = client.map_or(0, |account| account.balance);
        if receipt.client_cost > balance {
            return Err(AdmissionError::Invalid(
                "insufficient committed client balance",
            ));
        }
    }
    receipt.verify_signature().map_err(AdmissionError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    const MIN_FEE: u64 = 1;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn address(key: &SigningKey) -> String {
        hex::encode(key.verifying_key().to_bytes())
    }

    fn signed_tx(nonce: u64, amount: u64, fee: u64) -> Transaction {
        let sender = key(1);
        let mut tx = Transaction::new(address(&sender), nonce, "bob", amount, fee);
        tx.sign_with(&sender);
        tx
    }

    fn account(balance: u64, nonce: u64) -> AccountState {
        AccountState { balance, nonce }
    }

    fn signed_receipt(client_cost: u64, confirmed: bool) -> ComputeReceipt {
        let mut receipt = ComputeReceipt::new(
            "task-1",
            "client",
            "worker-1",
            "coord",
            "m",
            "p",
            client_cost,
            5,
            5,
        );
        receipt.client_confirmed = confirmed;
        receipt.sign_with(&key(9));
        receipt
    }

    #[test]
    fn test_valid_transaction_is_admitted() {
        assert_eq!(
            check_transaction(&signed_tx(3, 90, 10), &account(100, 3), 3, MIN_FEE),
            Ok(())
        );
        // Extending a pending chain: committed nonce 3, nonces 3..=5 pending.
        assert_eq!(
            check_transaction(&signed_tx(6, 1, 1), &account(100, 3), 6, MIN_FEE),
            Ok(())
        );
    }

    #[test]
    fn test_forged_and_unsigned_transactions_are_invalid() {
        let mut forged = signed_tx(0, 1, 1);
        forged.amount = 2; // signature no longer covers the payload
        assert!(matches!(
            check_transaction(&forged, &account(100, 0), 0, MIN_FEE),
            Err(AdmissionError::Invalid(_))
        ));
        let unsigned = Transaction::new(address(&key(1)), 0, "bob", 1, 1);
        assert!(matches!(
            check_transaction(&unsigned, &account(100, 0), 0, MIN_FEE),
            Err(AdmissionError::Invalid(_))
        ));
    }

    #[test]
    fn test_used_nonce_is_a_replay_and_far_nonce_is_invalid() {
        assert_eq!(
            check_transaction(&signed_tx(2, 1, 1), &account(100, 3), 3, MIN_FEE),
            Err(AdmissionError::Replay("nonce already used"))
        );
        // A gap, before or after pending nonces, is refused.
        assert!(matches!(
            check_transaction(&signed_tx(4, 1, 1), &account(100, 3), 3, MIN_FEE),
            Err(AdmissionError::Invalid(_))
        ));
        assert!(matches!(
            check_transaction(&signed_tx(7, 1, 1), &account(100, 3), 6, MIN_FEE),
            Err(AdmissionError::Invalid(_))
        ));
    }

    #[test]
    fn test_unfunded_or_overflowing_transaction_is_invalid() {
        assert_eq!(
            check_transaction(&signed_tx(0, 95, 10), &account(100, 0), 0, MIN_FEE),
            Err(AdmissionError::Invalid("insufficient committed balance"))
        );
        assert_eq!(
            check_transaction(
                &signed_tx(0, u64::MAX, 1),
                &account(u64::MAX, 0),
                0,
                MIN_FEE
            ),
            Err(AdmissionError::Invalid("amount + fee overflow"))
        );
    }

    #[test]
    fn test_fee_floor_and_uppercase_sender_are_invalid() {
        assert_eq!(
            check_transaction(&signed_tx(0, 1, 0), &account(100, 0), 0, MIN_FEE),
            Err(AdmissionError::Invalid("fee below the node minimum"))
        );
        let mut upper = signed_tx(0, 1, 1);
        upper.sender = upper.sender.to_ascii_uppercase();
        assert_eq!(
            check_transaction(&upper, &account(100, 0), 0, MIN_FEE),
            Err(AdmissionError::Invalid("sender must be lowercase hex"))
        );
    }

    #[test]
    fn test_receipt_checks_signature_replay_confirmation_and_client_funds() {
        let funded = account(100, 0);
        assert_eq!(
            check_receipt(&signed_receipt(10, true), false, Some(&funded)),
            Ok(())
        );
        assert_eq!(
            check_receipt(&signed_receipt(10, true), true, Some(&funded)),
            Err(AdmissionError::Replay("task already settled"))
        );
        assert!(matches!(
            check_receipt(&signed_receipt(10, false), false, Some(&funded)),
            Err(AdmissionError::Invalid(_))
        ));
        assert!(matches!(
            check_receipt(&signed_receipt(10, true), false, None),
            Err(AdmissionError::Invalid(_))
        ));
        let mut tampered = signed_receipt(10, true);
        tampered.client_cost = 1;
        assert!(matches!(
            check_receipt(&tampered, false, Some(&funded)),
            Err(AdmissionError::Invalid(_))
        ));
    }
}

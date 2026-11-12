//! Admission checks for submitted transactions and compute receipts (TODO 8.4.3).
//!
//! Items are checked against **committed** state and the node's pending chain before they enter
//! the mempool, so a client cannot fill block production with items that can never apply:
//! forged signatures, replayed nonces or settled tasks, nonce gaps, fees below the floor and
//! transfers the committed balance cannot cover are refused at the RPC. Balances are not
//! accumulated over a sender's pending chain and system payloads are not interpreted here;
//! block production re-checks everything on top of its parent and evicts what fails.

use crate::core::types::{
    AccountState, ComputeReceipt, MIN_WORKER_BOND, ProtocolParams, Transaction, WorkerBondAction,
    WorkerInfo,
};
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

/// Check a transaction to a system recipient against who may send it (TODO 8.5.1), so a
/// stranger's slash or epoch close is refused here instead of failing in every block until it
/// is evicted. Mirrors the execution rules; ordinary transfers pass unchanged.
pub fn check_system_transaction(
    tx: &Transaction,
    params: &ProtocolParams,
) -> Result<(), AdmissionError> {
    match tx.recipient.as_str() {
        "system:epoch_manager" => {
            if !params.is_governance(&tx.sender) {
                return Err(AdmissionError::Invalid(
                    "only governance may close an epoch",
                ));
            }
            if tx.amount != 0 || tx.payload.len() != 8 {
                return Err(AdmissionError::Invalid(
                    "epoch close carries no amount and an 8-byte epoch id",
                ));
            }
            Ok(())
        }
        "system:worker_registry" => {
            let action: WorkerBondAction = serde_json::from_slice(&tx.payload)
                .map_err(|_| AdmissionError::Invalid("malformed worker registry action"))?;
            match action {
                WorkerBondAction::Register { bond_amount, .. } if tx.amount != bond_amount => Err(
                    AdmissionError::Invalid("registration amount must equal the bond"),
                ),
                WorkerBondAction::Register { .. } => Ok(()),
                WorkerBondAction::Unbond { .. } if tx.amount != 0 => {
                    Err(AdmissionError::Invalid("unbond carries no amount"))
                }
                WorkerBondAction::Unbond { .. } => Ok(()),
                WorkerBondAction::Slash { .. } if !params.is_governance(&tx.sender) => {
                    Err(AdmissionError::Invalid("only governance may slash"))
                }
                WorkerBondAction::Slash { .. } if tx.amount != 0 => {
                    Err(AdmissionError::Invalid("slash carries no amount"))
                }
                WorkerBondAction::Slash { .. } => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

/// Committed state a receipt is judged against.
pub struct ReceiptContext<'a> {
    pub params: &'a ProtocolParams,
    pub task_settled: bool,
    /// The committed worker the receipt names, if registered.
    pub worker: Option<&'a WorkerInfo>,
    /// The committed client account the receipt names, if it exists.
    pub client: Option<&'a AccountState>,
}

/// Check a compute receipt against the settlement rules (TODO 8.5.1, see `ComputeReceipt`):
/// authorized coordinator, task not settled, registered and bonded worker, client confirmation
/// and payment `tokens × price` within the client's committed balance, then the coordinator and
/// worker signatures.
pub fn check_receipt(
    receipt: &ComputeReceipt,
    ctx: &ReceiptContext<'_>,
) -> Result<(), AdmissionError> {
    if receipt.task_id.is_empty() || receipt.worker_id.is_empty() {
        return Err(AdmissionError::Invalid("missing task or worker identifier"));
    }
    if !ctx.params.is_coordinator(&receipt.coordinator_pubkey) {
        return Err(AdmissionError::Invalid("coordinator is not authorized"));
    }
    if ctx.task_settled {
        return Err(AdmissionError::Replay("task already settled"));
    }
    let worker = ctx
        .worker
        .filter(|w| w.active && w.stake >= MIN_WORKER_BOND && !w.owner_address.is_empty())
        .ok_or(AdmissionError::Invalid(
            "worker is not registered with a bond",
        ))?;
    if receipt.client_address.is_empty() || !receipt.client_confirmed {
        return Err(AdmissionError::Invalid(
            "receipt must name a client that confirmed delivery",
        ));
    }
    if receipt.tokens_computed == 0 {
        return Err(AdmissionError::Invalid("receipt has no output tokens"));
    }
    if receipt
        .client_address
        .bytes()
        .any(|b| b.is_ascii_uppercase())
    {
        return Err(AdmissionError::Invalid(
            "client address must be lowercase hex",
        ));
    }
    if receipt.client_address == worker.owner_address {
        return Err(AdmissionError::Invalid(
            "client and worker owner are the same account",
        ));
    }
    let cost = receipt
        .tokens_computed
        .checked_mul(ctx.params.price_per_token)
        .ok_or(AdmissionError::Invalid("cost overflow"))?;
    if receipt.client_cost != cost || receipt.worker_reward != cost {
        return Err(AdmissionError::Invalid(
            "client_cost and worker_reward must equal tokens x price_per_token",
        ));
    }
    if cost > ctx.client.map_or(0, |account| account.balance) {
        return Err(AdmissionError::Invalid(
            "insufficient committed client balance",
        ));
    }
    receipt
        .verify_signature()
        .map_err(AdmissionError::Invalid)?;
    receipt
        .verify_worker_signature(&worker.owner_address)
        .map_err(AdmissionError::Invalid)
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

    const COORDINATOR: u8 = 9;
    const WORKER_OWNER: u8 = 8;
    const PRICE: u64 = 2;

    fn params() -> ProtocolParams {
        ProtocolParams {
            coordinators: vec![address(&key(COORDINATOR))],
            price_per_token: PRICE,
            ..ProtocolParams::default()
        }
    }

    fn worker() -> WorkerInfo {
        WorkerInfo {
            worker_id: "worker-1".into(),
            stake: MIN_WORKER_BOND,
            completed_tasks: 0,
            rewards_accumulated: 0,
            active: true,
            vram_gib: 24,
            uptime_bps: 10_000,
            epoch_tokens: 0,
            owner_address: address(&key(WORKER_OWNER)),
            endpoint: String::new(),
            unbonding: 0,
        }
    }

    /// 5 tokens at price 2: cost 10, signed by the coordinator and the worker.
    fn signed_receipt() -> ComputeReceipt {
        let mut receipt =
            ComputeReceipt::new("task-1", "client", "worker-1", "coord", "m", "p", 5, 10, 10);
        receipt.sign_with(&key(COORDINATOR));
        receipt.sign_as_worker(&key(WORKER_OWNER));
        receipt
    }

    fn resign(mut receipt: ComputeReceipt) -> ComputeReceipt {
        receipt.sign_with(&key(COORDINATOR));
        receipt.sign_as_worker(&key(WORKER_OWNER));
        receipt
    }

    fn check(
        receipt: &ComputeReceipt,
        settled: bool,
        worker: Option<&WorkerInfo>,
        client: Option<&AccountState>,
    ) -> Result<(), AdmissionError> {
        let params = params();
        check_receipt(
            receipt,
            &ReceiptContext {
                params: &params,
                task_settled: settled,
                worker,
                client,
            },
        )
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
    fn test_valid_receipt_is_admitted_and_replay_is_refused() {
        let (w, funded) = (worker(), account(100, 0));
        assert_eq!(
            check(&signed_receipt(), false, Some(&w), Some(&funded)),
            Ok(())
        );
        assert_eq!(
            check(&signed_receipt(), true, Some(&w), Some(&funded)),
            Err(AdmissionError::Replay("task already settled"))
        );
    }

    #[test]
    fn test_receipts_from_unauthorized_coordinators_or_unsigned_by_the_worker_are_invalid() {
        let (w, funded) = (worker(), account(100, 0));
        let invalid = |r: &ComputeReceipt| {
            matches!(
                check(r, false, Some(&w), Some(&funded)),
                Err(AdmissionError::Invalid(_))
            )
        };
        // Self-signed by a key that is not a genesis coordinator.
        let mut forged = signed_receipt();
        forged.sign_with(&key(77));
        assert!(invalid(&forged));
        // No worker signature, or one by another key.
        let mut unsigned = signed_receipt();
        unsigned.worker_signature.clear();
        assert!(invalid(&unsigned));
        let mut other = signed_receipt();
        other.sign_as_worker(&key(78));
        assert!(invalid(&other));
        // Payload changed after signing.
        let mut tampered = signed_receipt();
        tampered.tokens_computed = 50;
        assert!(invalid(&tampered));
    }

    #[test]
    fn test_receipts_need_a_bonded_worker_and_a_paying_client() {
        let funded = account(100, 0);
        let w = worker();
        let invalid =
            |r: &ComputeReceipt, worker: Option<&WorkerInfo>, client: Option<&AccountState>| {
                matches!(
                    check(r, false, worker, client),
                    Err(AdmissionError::Invalid(_))
                )
            };
        // Unknown or unbonded worker.
        assert!(invalid(&signed_receipt(), None, Some(&funded)));
        let unbonded = WorkerInfo {
            stake: MIN_WORKER_BOND - 1,
            active: false,
            ..worker()
        };
        assert!(invalid(&signed_receipt(), Some(&unbonded), Some(&funded)));
        // Cost not equal to tokens x price, in either field.
        let mut cheap = signed_receipt();
        cheap.client_cost = 9;
        assert!(invalid(&resign(cheap), Some(&w), Some(&funded)));
        let mut generous = signed_receipt();
        generous.worker_reward = 11;
        assert!(invalid(&resign(generous), Some(&w), Some(&funded)));
        // No client, unconfirmed delivery, client cannot pay, worker paying itself.
        let mut no_client = signed_receipt();
        no_client.client_address.clear();
        assert!(invalid(&resign(no_client), Some(&w), Some(&funded)));
        let mut unconfirmed = signed_receipt();
        unconfirmed.client_confirmed = false;
        assert!(invalid(&resign(unconfirmed), Some(&w), Some(&funded)));
        assert!(invalid(&signed_receipt(), Some(&w), Some(&account(9, 0))));
        let mut wash = signed_receipt();
        wash.client_address = w.owner_address.clone();
        assert!(invalid(&resign(wash), Some(&w), Some(&funded)));
    }

    #[test]
    fn test_system_transactions_need_the_right_sender_and_no_amount() {
        let governance = key(30);
        let stranger = key(31);
        let params = ProtocolParams {
            governance: vec![address(&governance)],
            ..ProtocolParams::default()
        };
        let tx = |signer: &SigningKey, recipient: &str, amount: u64, payload: Vec<u8>| {
            let mut tx = Transaction::new(address(signer), 0, recipient, amount, 1);
            tx.payload = payload;
            tx
        };
        let close = 0u64.to_be_bytes().to_vec();
        let slash = |amount| {
            WorkerBondAction::Slash {
                worker_id: "w".into(),
                slash_amount: 10,
                reason: "r".into(),
            }
            .to_bytes()
            .map(|p| (amount, p))
            .unwrap()
        };
        let ok = |t: &Transaction| check_system_transaction(t, &params).is_ok();

        assert!(ok(&tx(
            &governance,
            "system:epoch_manager",
            0,
            close.clone()
        )));
        assert!(!ok(&tx(
            &stranger,
            "system:epoch_manager",
            0,
            close.clone()
        )));
        assert!(!ok(&tx(&governance, "system:epoch_manager", 5, close)));
        let (amount, payload) = slash(0);
        assert!(ok(&tx(
            &governance,
            "system:worker_registry",
            amount,
            payload.clone()
        )));
        assert!(!ok(&tx(
            &stranger,
            "system:worker_registry",
            0,
            payload.clone()
        )));
        assert!(!ok(&tx(&governance, "system:worker_registry", 7, payload)));
        assert!(!ok(&tx(
            &stranger,
            "system:worker_registry",
            0,
            b"garbage".to_vec()
        )));
        let unbond = WorkerBondAction::Unbond {
            worker_id: "w".into(),
            amount: 5,
        }
        .to_bytes()
        .unwrap();
        assert!(ok(&tx(
            &stranger,
            "system:worker_registry",
            0,
            unbond.clone()
        )));
        assert!(!ok(&tx(&stranger, "system:worker_registry", 5, unbond)));
        assert!(ok(&tx(&stranger, "bob", 5, Vec::new())));
    }

    #[test]
    fn test_zero_token_and_uppercase_client_receipts_are_invalid() {
        let (w, funded) = (worker(), account(100, 0));
        let mut empty = signed_receipt();
        empty.tokens_computed = 0;
        empty.client_cost = 0;
        empty.worker_reward = 0;
        assert!(matches!(
            check(&resign(empty), false, Some(&w), Some(&funded)),
            Err(AdmissionError::Invalid(_))
        ));
        let mut upper = signed_receipt();
        upper.client_address = "CLIENT".into();
        assert!(matches!(
            check(&resign(upper), false, Some(&w), Some(&funded)),
            Err(AdmissionError::Invalid(_))
        ));
    }
}

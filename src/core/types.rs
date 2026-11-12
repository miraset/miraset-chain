use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Current canonical protocol encoding version (PRD R03).
pub const PROTOCOL_ENCODING_VERSION: u8 = 1;

/// Domain-separation tags for cryptographic signing and commitments (PRD R03).
pub const DOMAIN_TX_SIGN_V1: &[u8] = b"MIRASET:TX:SIGN:v1";
pub const DOMAIN_TX_COMMIT_V1: &[u8] = b"MIRASET:TX:COMMIT:v1";
pub const DOMAIN_RECEIPT_SIGN_V1: &[u8] = b"MIRASET:RECEIPT:SIGN:v1";
/// Worker signatures sign the receipt under their own domain, so a coordinator signature can
/// never stand in for a worker's (or the reverse).
pub const DOMAIN_RECEIPT_WORKER_SIGN_V1: &[u8] = b"MIRASET:RECEIPT:WORKER:v1";
pub const DOMAIN_RECEIPT_COMMIT_V1: &[u8] = b"MIRASET:RECEIPT:COMMIT:v1";
pub const DOMAIN_BLOCK_COMMIT_V1: &[u8] = b"MIRASET:BLOCK:COMMIT:v1";
pub const DOMAIN_STATE_COMMIT_V1: &[u8] = b"MIRASET:STATE:COMMIT:v1";

const TX_MAGIC: &[u8; 4] = b"MTX1";
const RECEIPT_MAGIC: &[u8; 4] = b"MRCP";
const BLOCK_MAGIC: &[u8; 4] = b"MBLK";

fn default_protocol_version() -> u8 {
    PROTOCOL_ENCODING_VERSION
}

/// Transaction representing a state transfer or interaction in MIRASET L1.
///
/// `sender` is the lowercase-hex Ed25519 public key that authorizes this transaction.
/// `verify_signature` rejects any transaction whose `sender` does not match `pubkey`,
/// so account identity is the signing key itself; there is no separate address registry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    #[serde(default = "default_protocol_version")]
    pub version: u8,
    pub sender: String,
    pub nonce: u64,
    pub recipient: String,
    pub amount: u64,
    pub fee: u64,
    pub payload: Vec<u8>,
    /// Hex-encoded Ed25519 public key of the sender. Must equal `sender`.
    #[serde(default)]
    pub pubkey: String,
    /// Hex-encoded Ed25519 signature over `canonical_sign_bytes()`.
    #[serde(default)]
    pub signature: String,
}

/// Appends a length-prefixed byte string, so adjacent variable-length fields in a
/// canonical signing payload cannot be shifted into one another (e.g. recipient
/// bytes bleeding into the following payload bytes).
fn write_length_prefixed(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(bytes);
}

fn read_u8(input: &mut &[u8]) -> Result<u8, &'static str> {
    if input.is_empty() {
        return Err("unexpected end of canonical input (u8)");
    }
    let v = input[0];
    *input = &input[1..];
    Ok(v)
}

fn read_u32(input: &mut &[u8]) -> Result<u32, &'static str> {
    if input.len() < 4 {
        return Err("unexpected end of canonical input (u32)");
    }
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&input[..4]);
    *input = &input[4..];
    Ok(u32::from_be_bytes(arr))
}

fn read_u64(input: &mut &[u8]) -> Result<u64, &'static str> {
    if input.len() < 8 {
        return Err("unexpected end of canonical input (u64)");
    }
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&input[..8]);
    *input = &input[8..];
    Ok(u64::from_be_bytes(arr))
}

fn read_length_prefixed_bytes(input: &mut &[u8]) -> Result<Vec<u8>, &'static str> {
    let len = read_u32(input)? as usize;
    if input.len() < len {
        return Err("unexpected end of canonical input (length-prefixed bytes)");
    }
    let out = input[..len].to_vec();
    *input = &input[len..];
    Ok(out)
}

fn read_length_prefixed_string(input: &mut &[u8]) -> Result<String, &'static str> {
    let bytes = read_length_prefixed_bytes(input)?;
    String::from_utf8(bytes).map_err(|_| "invalid UTF-8 in canonical string field")
}

/// Computes a deterministic domain-separated state commitment root over key-value entries.
pub fn compute_state_commitment(entries: &[(&[u8], &[u8])]) -> [u8; 32] {
    let mut sorted: Vec<(&[u8], &[u8])> = entries.to_vec();
    sorted.sort_by(|(ka, va), (kb, vb)| ka.cmp(kb).then_with(|| va.cmp(vb)));

    let mut hasher = Sha256::new();
    hasher.update(DOMAIN_STATE_COMMIT_V1);
    hasher.update([PROTOCOL_ENCODING_VERSION]);
    hasher.update((sorted.len() as u32).to_be_bytes());
    for (k, v) in sorted {
        hasher.update((k.len() as u32).to_be_bytes());
        hasher.update(k);
        hasher.update((v.len() as u32).to_be_bytes());
        hasher.update(v);
    }
    hasher.finalize().into()
}

/// Minimum worker bond required to register and serve tasks (ADR-0007 / protocol/model.md: 100 MIR).
pub const MIN_WORKER_BOND: u64 = 100;

/// Base slashing penalty for worker receipt or delivery fraud (ADR-0007: 10% base).
pub const WORKER_BASE_SLASH_BPS: u64 = 1000; // 10.00%

/// Actions that a worker or governance/coordinator can execute for worker stake bonds (ADR-0007).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerBondAction {
    /// Register or increase a worker's stake bond.
    Register {
        worker_id: String,
        bond_amount: u64,
        vram_gib: u64,
        endpoint: String,
    },
    /// Unbond stake (returns unbonded amount to worker owner account).
    Unbond { worker_id: String, amount: u64 },
    /// Slash worker bond due to verified fault or challenge (ADR-0007).
    Slash {
        worker_id: String,
        slash_amount: u64,
        reason: String,
    },
}

impl WorkerBondAction {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

impl Transaction {
    pub fn new(
        sender: impl Into<String>,
        nonce: u64,
        recipient: impl Into<String>,
        amount: u64,
        fee: u64,
    ) -> Self {
        Self {
            version: PROTOCOL_ENCODING_VERSION,
            sender: sender.into(),
            nonce,
            recipient: recipient.into(),
            amount,
            fee,
            payload: Vec::new(),
            pubkey: String::new(),
            signature: String::new(),
        }
    }

    /// Construct a worker bond transaction (register, unbond, or slash).
    pub fn new_worker_bond(
        sender: impl Into<String>,
        nonce: u64,
        fee: u64,
        action: WorkerBondAction,
    ) -> Result<Self, serde_json::Error> {
        let payload = action.to_bytes()?;
        let amount = match &action {
            WorkerBondAction::Register { bond_amount, .. } => *bond_amount,
            _ => 0,
        };
        Ok(Self {
            version: PROTOCOL_ENCODING_VERSION,
            sender: sender.into(),
            nonce,
            recipient: "system:worker_registry".to_string(),
            amount,
            fee,
            payload,
            pubkey: String::new(),
            signature: String::new(),
        })
    }

    /// Construct a transaction that triggers epoch finalization and rollover.
    pub fn new_epoch_advance(
        sender: impl Into<String>,
        nonce: u64,
        fee: u64,
        epoch_id: u64,
    ) -> Self {
        Self {
            version: PROTOCOL_ENCODING_VERSION,
            sender: sender.into(),
            nonce,
            recipient: "system:epoch_manager".to_string(),
            amount: 0,
            fee,
            payload: epoch_id.to_be_bytes().to_vec(),
            pubkey: String::new(),
            signature: String::new(),
        }
    }

    /// Canonical message bytes for cryptographic signature verification (domain-separated).
    pub fn canonical_sign_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(DOMAIN_TX_SIGN_V1);
        bytes.push(self.version);
        write_length_prefixed(&mut bytes, self.sender.as_bytes());
        bytes.extend_from_slice(&self.nonce.to_be_bytes());
        write_length_prefixed(&mut bytes, self.recipient.as_bytes());
        bytes.extend_from_slice(&self.amount.to_be_bytes());
        bytes.extend_from_slice(&self.fee.to_be_bytes());
        write_length_prefixed(&mut bytes, &self.payload);
        bytes
    }

    /// Serialize transaction into versioned canonical binary format.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, &'static str> {
        if self.version != PROTOCOL_ENCODING_VERSION {
            return Err("unsupported transaction encoding version");
        }
        let mut buf = Vec::new();
        buf.extend_from_slice(TX_MAGIC);
        buf.push(self.version);
        write_length_prefixed(&mut buf, self.sender.as_bytes());
        buf.extend_from_slice(&self.nonce.to_be_bytes());
        write_length_prefixed(&mut buf, self.recipient.as_bytes());
        buf.extend_from_slice(&self.amount.to_be_bytes());
        buf.extend_from_slice(&self.fee.to_be_bytes());
        write_length_prefixed(&mut buf, &self.payload);
        write_length_prefixed(&mut buf, self.pubkey.as_bytes());
        write_length_prefixed(&mut buf, self.signature.as_bytes());
        Ok(buf)
    }

    /// Deserialize transaction from versioned canonical binary format, rejecting
    /// unsupported versions, malformed lengths, or trailing bytes.
    pub fn from_canonical_bytes(mut input: &[u8]) -> Result<Self, &'static str> {
        if input.len() < 4 || &input[..4] != TX_MAGIC {
            return Err("invalid transaction canonical magic");
        }
        input = &input[4..];
        let version = read_u8(&mut input)?;
        if version != PROTOCOL_ENCODING_VERSION {
            return Err("unsupported transaction encoding version");
        }
        let sender = read_length_prefixed_string(&mut input)?;
        let nonce = read_u64(&mut input)?;
        let recipient = read_length_prefixed_string(&mut input)?;
        let amount = read_u64(&mut input)?;
        let fee = read_u64(&mut input)?;
        let payload = read_length_prefixed_bytes(&mut input)?;
        let pubkey = read_length_prefixed_string(&mut input)?;
        let signature = read_length_prefixed_string(&mut input)?;
        if !input.is_empty() {
            return Err("noncanonical trailing bytes in transaction");
        }
        Ok(Self {
            version,
            sender,
            nonce,
            recipient,
            amount,
            fee,
            payload,
            pubkey,
            signature,
        })
    }

    /// Domain-separated cryptographic commitment hash for this transaction.
    pub fn commitment_hash(&self) -> Result<[u8; 32], &'static str> {
        let canonical = self.to_canonical_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_TX_COMMIT_V1);
        hasher.update(&canonical);
        Ok(hasher.finalize().into())
    }

    /// Sign this transaction using an Ed25519 signing key. Sets `sender` and `pubkey`
    /// to the signer's hex-encoded verifying key, so the signed sender always matches
    /// the key that produced the signature.
    pub fn sign_with(&mut self, signing_key: &ed25519_dalek::SigningKey) {
        use ed25519_dalek::Signer;
        self.pubkey = hex::encode(signing_key.verifying_key().to_bytes());
        self.sender = self.pubkey.clone();
        let sign_bytes = self.canonical_sign_bytes();
        let sig = signing_key.sign(&sign_bytes);
        self.signature = hex::encode(sig.to_bytes());
    }

    /// Verify the sender's signature. Unlike `ComputeReceipt::verify_signature`, a
    /// missing signature is always rejected — transactions move funds and must always
    /// be authenticated.
    pub fn verify_signature(&self) -> Result<(), &'static str> {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        if self.version != PROTOCOL_ENCODING_VERSION {
            return Err("unsupported transaction encoding version");
        }

        if self.signature.is_empty() || self.pubkey.is_empty() {
            return Err("missing transaction signature");
        }

        let pubkey_bytes = hex::decode(&self.pubkey).map_err(|_| "Invalid hex in pubkey")?;
        if pubkey_bytes.len() != 32 {
            return Err("Invalid pubkey length (must be 32 bytes)");
        }
        let mut pk_arr = [0u8; 32];
        pk_arr.copy_from_slice(&pubkey_bytes);
        let verifying_key =
            VerifyingKey::from_bytes(&pk_arr).map_err(|_| "Invalid Ed25519 verifying key")?;

        if !self.sender.eq_ignore_ascii_case(&self.pubkey) {
            return Err("sender does not match signing pubkey");
        }

        let sig_bytes = hex::decode(&self.signature).map_err(|_| "Invalid hex in signature")?;
        if sig_bytes.len() != 64 {
            return Err("Invalid signature length (must be 64 bytes)");
        }
        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);
        let sig = Signature::from_bytes(&sig_arr);

        let sign_bytes = self.canonical_sign_bytes();
        verifying_key
            .verify(&sign_bytes, &sig)
            .map_err(|_| "Ed25519 transaction signature verification failed")
    }
}

/// Compute receipt for MIRASET Proof of Compute & Capacity (PoCC): one settled job (TODO 8.5.1).
///
/// PoCC is Proof of Compute **& Capacity** (SOW §6), an accounting of useful work and available
/// capacity. A receipt does **not** prove that inference was correct or that a customer paid off
/// chain: `proof_hash` is an opaque commitment the coordinator and worker sign (SOW §6.3 receipt
/// anchor, for audit and disputes), and the chain never checks it against model output.
///
/// The chain settles a receipt only if (REWARDS.md §3.3, §6; ADR-0007, ADR-0009):
/// - it is co-signed by a coordinator key listed in genesis (`ProtocolParams::coordinators`);
/// - it is signed by the worker's registered key (the worker's `owner_address`), and the worker
///   is registered with at least `MIN_WORKER_BOND`;
/// - the task was not settled before;
/// - it names a client account that confirmed delivery, and
///   `client_cost == worker_reward == tokens_computed × price_per_token`; the chain debits the
///   client and credits the worker's owner exactly that amount.
///
/// Settlement is immediate; the SOW §8 challenge window is not implemented (disputes use
/// governance slashing).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeReceipt {
    #[serde(default = "default_protocol_version")]
    pub version: u8,
    pub task_id: String,
    pub worker_id: String,
    pub model_hash: String,
    pub proof_hash: String,
    #[serde(default)]
    pub gas_reward: u64,
    /// Client account debited for compute usage.
    #[serde(default)]
    pub client_address: String,
    /// Permissioned Coordinator ID that co-signs the work receipt.
    #[serde(default)]
    pub coordinator_id: String,
    /// Total inference tokens generated and delivered.
    #[serde(default)]
    pub tokens_computed: u64,
    /// Amount debited from the client's balance/escrow.
    #[serde(default)]
    pub client_cost: u64,
    /// Net reward credited to the worker (defaults to gas_reward if 0).
    #[serde(default)]
    pub worker_reward: u64,
    /// Indicates that the client received all generated tokens/output.
    #[serde(default)]
    pub client_confirmed: bool,
    /// Coordinator co-signature or authentication tag (hex-encoded Ed25519 signature).
    #[serde(default)]
    pub signature: String,
    /// Ed25519 hex public key of the coordinator; must be listed in genesis.
    #[serde(default)]
    pub coordinator_pubkey: String,
    /// Worker's Ed25519 signature (hex) over the same signing bytes, by the key registered as
    /// the worker's owner.
    #[serde(default)]
    pub worker_signature: String,
}

impl ComputeReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_id: impl Into<String>,
        client_address: impl Into<String>,
        worker_id: impl Into<String>,
        coordinator_id: impl Into<String>,
        model_hash: impl Into<String>,
        proof_hash: impl Into<String>,
        tokens_computed: u64,
        client_cost: u64,
        worker_reward: u64,
    ) -> Self {
        Self {
            version: PROTOCOL_ENCODING_VERSION,
            task_id: task_id.into(),
            worker_id: worker_id.into(),
            model_hash: model_hash.into(),
            proof_hash: proof_hash.into(),
            gas_reward: worker_reward,
            client_address: client_address.into(),
            coordinator_id: coordinator_id.into(),
            tokens_computed,
            client_cost,
            worker_reward,
            client_confirmed: true,
            signature: String::new(),
            coordinator_pubkey: String::new(),
            worker_signature: String::new(),
        }
    }

    /// Canonical message bytes for cryptographic signature verification (domain-separated, length-prefixed).
    pub fn canonical_sign_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(DOMAIN_RECEIPT_SIGN_V1);
        bytes.push(self.version);
        write_length_prefixed(&mut bytes, self.task_id.as_bytes());
        write_length_prefixed(&mut bytes, self.client_address.as_bytes());
        write_length_prefixed(&mut bytes, self.worker_id.as_bytes());
        write_length_prefixed(&mut bytes, self.coordinator_id.as_bytes());
        write_length_prefixed(&mut bytes, self.model_hash.as_bytes());
        write_length_prefixed(&mut bytes, self.proof_hash.as_bytes());
        bytes.extend_from_slice(&self.tokens_computed.to_be_bytes());
        bytes.extend_from_slice(&self.client_cost.to_be_bytes());
        bytes.extend_from_slice(&self.worker_reward.to_be_bytes());
        bytes.push(u8::from(self.client_confirmed));
        bytes
    }

    /// Serialize compute receipt into versioned canonical binary format.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, &'static str> {
        if self.version != PROTOCOL_ENCODING_VERSION {
            return Err("unsupported receipt encoding version");
        }
        let mut buf = Vec::new();
        buf.extend_from_slice(RECEIPT_MAGIC);
        buf.push(self.version);
        write_length_prefixed(&mut buf, self.task_id.as_bytes());
        write_length_prefixed(&mut buf, self.worker_id.as_bytes());
        write_length_prefixed(&mut buf, self.model_hash.as_bytes());
        write_length_prefixed(&mut buf, self.proof_hash.as_bytes());
        buf.extend_from_slice(&self.gas_reward.to_be_bytes());
        write_length_prefixed(&mut buf, self.client_address.as_bytes());
        write_length_prefixed(&mut buf, self.coordinator_id.as_bytes());
        buf.extend_from_slice(&self.tokens_computed.to_be_bytes());
        buf.extend_from_slice(&self.client_cost.to_be_bytes());
        buf.extend_from_slice(&self.worker_reward.to_be_bytes());
        buf.push(u8::from(self.client_confirmed));
        write_length_prefixed(&mut buf, self.signature.as_bytes());
        write_length_prefixed(&mut buf, self.coordinator_pubkey.as_bytes());
        write_length_prefixed(&mut buf, self.worker_signature.as_bytes());
        Ok(buf)
    }

    /// Deserialize compute receipt from versioned canonical binary format, rejecting
    /// unsupported versions, invalid boolean flags, or trailing bytes.
    pub fn from_canonical_bytes(mut input: &[u8]) -> Result<Self, &'static str> {
        if input.len() < 4 || &input[..4] != RECEIPT_MAGIC {
            return Err("invalid receipt canonical magic");
        }
        input = &input[4..];
        let version = read_u8(&mut input)?;
        if version != PROTOCOL_ENCODING_VERSION {
            return Err("unsupported receipt encoding version");
        }
        let task_id = read_length_prefixed_string(&mut input)?;
        let worker_id = read_length_prefixed_string(&mut input)?;
        let model_hash = read_length_prefixed_string(&mut input)?;
        let proof_hash = read_length_prefixed_string(&mut input)?;
        let gas_reward = read_u64(&mut input)?;
        let client_address = read_length_prefixed_string(&mut input)?;
        let coordinator_id = read_length_prefixed_string(&mut input)?;
        let tokens_computed = read_u64(&mut input)?;
        let client_cost = read_u64(&mut input)?;
        let worker_reward = read_u64(&mut input)?;
        let confirmed_byte = read_u8(&mut input)?;
        let client_confirmed = match confirmed_byte {
            0 => false,
            1 => true,
            _ => return Err("noncanonical boolean value in receipt"),
        };
        let signature = read_length_prefixed_string(&mut input)?;
        let coordinator_pubkey = read_length_prefixed_string(&mut input)?;
        let worker_signature = read_length_prefixed_string(&mut input)?;
        if !input.is_empty() {
            return Err("noncanonical trailing bytes in receipt");
        }
        Ok(Self {
            version,
            task_id,
            worker_id,
            model_hash,
            proof_hash,
            gas_reward,
            client_address,
            coordinator_id,
            tokens_computed,
            client_cost,
            worker_reward,
            client_confirmed,
            signature,
            coordinator_pubkey,
            worker_signature,
        })
    }

    /// Domain-separated cryptographic commitment hash for this receipt.
    pub fn commitment_hash(&self) -> Result<[u8; 32], &'static str> {
        let canonical = self.to_canonical_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_RECEIPT_COMMIT_V1);
        hasher.update(&canonical);
        Ok(hasher.finalize().into())
    }

    /// Sign this receipt using an Ed25519 signing key.
    pub fn sign_with(&mut self, signing_key: &ed25519_dalek::SigningKey) {
        use ed25519_dalek::Signer;
        let sign_bytes = self.canonical_sign_bytes();
        let sig = signing_key.sign(&sign_bytes);
        self.signature = hex::encode(sig.to_bytes());
        self.coordinator_pubkey = hex::encode(signing_key.verifying_key().to_bytes());
    }

    /// Bytes the worker signs: the receipt's signing bytes under the worker domain.
    pub fn worker_sign_bytes(&self) -> Vec<u8> {
        let mut bytes = DOMAIN_RECEIPT_WORKER_SIGN_V1.to_vec();
        bytes.extend_from_slice(&self.canonical_sign_bytes()[DOMAIN_RECEIPT_SIGN_V1.len()..]);
        bytes
    }

    /// Sign this receipt as the worker whose registered key is `signing_key`.
    pub fn sign_as_worker(&mut self, signing_key: &ed25519_dalek::SigningKey) {
        use ed25519_dalek::Signer;
        let sig = signing_key.sign(&self.worker_sign_bytes());
        self.worker_signature = hex::encode(sig.to_bytes());
    }

    /// Verify the worker signature against the worker's registered key (hex).
    pub fn verify_worker_signature(&self, worker_pubkey: &str) -> Result<(), &'static str> {
        if self.worker_signature.is_empty() {
            return Err("missing worker signature");
        }
        verify_ed25519(
            worker_pubkey,
            &self.worker_signature,
            &self.worker_sign_bytes(),
        )
        .map_err(|_| "worker signature verification failed")
    }

    /// Verify the coordinator signature against the key the receipt names. Whether that key is
    /// an authorized coordinator is checked by the caller against `ProtocolParams`.
    pub fn verify_signature(&self) -> Result<(), &'static str> {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        if self.version != PROTOCOL_ENCODING_VERSION {
            return Err("unsupported receipt encoding version");
        }

        if self.signature.is_empty() || self.coordinator_pubkey.is_empty() {
            return Err("missing coordinator signature");
        }

        let pubkey_bytes = hex::decode(&self.coordinator_pubkey)
            .map_err(|_| "Invalid hex in coordinator_pubkey")?;
        if pubkey_bytes.len() != 32 {
            return Err("Invalid coordinator_pubkey length (must be 32 bytes)");
        }
        let mut pk_arr = [0u8; 32];
        pk_arr.copy_from_slice(&pubkey_bytes);
        let verifying_key =
            VerifyingKey::from_bytes(&pk_arr).map_err(|_| "Invalid Ed25519 verifying key")?;

        let sig_bytes = hex::decode(&self.signature).map_err(|_| "Invalid hex in signature")?;
        if sig_bytes.len() != 64 {
            return Err("Invalid signature length (must be 64 bytes)");
        }
        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);
        let sig = Signature::from_bytes(&sig_arr);

        let sign_bytes = self.canonical_sign_bytes();
        verifying_key
            .verify(&sign_bytes, &sig)
            .map_err(|_| "Ed25519 signature verification failed")
    }
}

/// Verify an Ed25519 signature given hex public key and hex signature.
fn verify_ed25519(pubkey_hex: &str, signature_hex: &str, message: &[u8]) -> Result<(), ()> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let pubkey: [u8; 32] = hex::decode(pubkey_hex)
        .map_err(|_| ())?
        .try_into()
        .map_err(|_| ())?;
    let signature: [u8; 64] = hex::decode(signature_hex)
        .map_err(|_| ())?
        .try_into()
        .map_err(|_| ())?;
    let key = VerifyingKey::from_bytes(&pubkey).map_err(|_| ())?;
    key.verify(message, &Signature::from_bytes(&signature))
        .map_err(|_| ())
}

/// Payload of a transaction to `system:allowance` (TODO 8.5.1): the sender (a client account)
/// sets how much `coordinator` may still charge it through receipts. Receipts spend the
/// allowance; setting it again replaces the remainder (0 revokes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowanceAction {
    /// Coordinator public key (hex).
    pub coordinator: String,
    pub amount: u64,
}

impl AllowanceAction {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

/// Largest per-epoch reward budget a genesis may set (10^15 base units).
pub const MAX_EPOCH_REWARD_BUDGET: u64 = 1_000_000_000_000_000;

/// Protocol parameters fixed at genesis (TODO 8.5.1; owner decision 2026-11-12). Stored in
/// genesis state, so the genesis commitment covers them; changing them needs a new genesis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolParams {
    /// Coordinator public keys (hex) allowed to co-sign compute receipts (ADR-0009).
    #[serde(default)]
    pub coordinators: Vec<String>,
    /// Governance public keys (hex) allowed to slash worker bonds and close epochs.
    #[serde(default)]
    pub governance: Vec<String>,
    /// Price of one output token, debited from the client and paid to the worker (REWARDS §6).
    #[serde(default = "default_price_per_token")]
    pub price_per_token: u64,
    /// Protocol reward minted for each epoch (ADR-0004: 5M MIR per month ≈ 6 944 per 60-minute
    /// epoch); undistributed amounts roll over to the next epoch.
    #[serde(default = "default_epoch_reward_budget")]
    pub epoch_reward_budget: u64,
    /// Minimum epoch length in seconds of block time (REWARDS §2.1: 3600). An epoch can close
    /// only this long after it began, so at most one reward budget is minted per period.
    #[serde(default = "default_epoch_seconds")]
    pub epoch_seconds: u64,
}

fn default_price_per_token() -> u64 {
    1
}

fn default_epoch_reward_budget() -> u64 {
    6_944
}

fn default_epoch_seconds() -> u64 {
    3_600
}

impl Default for ProtocolParams {
    fn default() -> Self {
        Self {
            coordinators: Vec::new(),
            governance: Vec::new(),
            price_per_token: default_price_per_token(),
            epoch_reward_budget: default_epoch_reward_budget(),
            epoch_seconds: default_epoch_seconds(),
        }
    }
}

impl ProtocolParams {
    /// Whether `pubkey` (hex, any case) is an authorized receipt coordinator.
    pub fn is_coordinator(&self, pubkey: &str) -> bool {
        self.coordinators
            .iter()
            .any(|k| k.eq_ignore_ascii_case(pubkey))
    }

    /// Whether `pubkey` (hex, any case) is a governance key.
    pub fn is_governance(&self, pubkey: &str) -> bool {
        self.governance
            .iter()
            .any(|k| k.eq_ignore_ascii_case(pubkey))
    }

    /// Keys are 32-byte hex Ed25519 public keys, unique within and across the two lists (one
    /// key, one role); the price and the epoch budget are positive, and the budget is small
    /// enough that rolled-over budgets cannot overflow for an implausible number of epochs.
    pub fn validate(&self) -> Result<(), String> {
        if self.price_per_token == 0 {
            return Err("price_per_token must be positive".into());
        }
        if self.epoch_seconds == 0 {
            return Err("epoch_seconds must be positive".into());
        }
        if self.epoch_reward_budget == 0 || self.epoch_reward_budget > MAX_EPOCH_REWARD_BUDGET {
            return Err(format!(
                "epoch_reward_budget must be in 1..={MAX_EPOCH_REWARD_BUDGET}"
            ));
        }
        let lower: std::collections::BTreeSet<String> = self
            .coordinators
            .iter()
            .map(|k| k.to_ascii_lowercase())
            .collect();
        if let Some(key) = self
            .governance
            .iter()
            .find(|k| lower.contains(&k.to_ascii_lowercase()))
        {
            return Err(format!("key {key} is both coordinator and governance"));
        }
        for (role, keys) in [
            ("coordinator", &self.coordinators),
            ("governance", &self.governance),
        ] {
            let mut seen = std::collections::BTreeSet::new();
            for key in keys {
                let bytes: [u8; 32] = hex::decode(key)
                    .ok()
                    .and_then(|b| b.try_into().ok())
                    .ok_or_else(|| format!("{role} key {key} is not 32-byte hex"))?;
                ed25519_dalek::VerifyingKey::from_bytes(&bytes)
                    .map_err(|_| format!("{role} key {key} is not an Ed25519 public key"))?;
                if !seen.insert(key.to_ascii_lowercase()) {
                    return Err(format!("duplicate {role} key {key}"));
                }
            }
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

/// Event emitted when a block is committed (finalized by consensus).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitEvent {
    pub block_hash: String,
    pub timestamp_secs: u64,
    pub tx_count: usize,
    pub receipt_count: usize,
    /// Height of the committed block (absent if it could not be read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
}

/// Consolidated block payload containing transactions, PoCC receipts, and deterministic state commitment root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockPayload {
    #[serde(default = "default_protocol_version")]
    pub version: u8,
    pub transactions: Vec<Transaction>,
    pub compute_receipts: Vec<ComputeReceipt>,
    /// Deterministic state commitment root over the candidate state updates produced by this block (PRD R04).
    #[serde(default)]
    pub state_root: [u8; 32],
    /// Proposer's wall-clock time in Unix milliseconds (TODO 8.5.1). Strictly after the
    /// parent's; validators also bound how far it may run ahead of their clock.
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl Default for BlockPayload {
    fn default() -> Self {
        Self {
            version: PROTOCOL_ENCODING_VERSION,
            transactions: Vec::new(),
            compute_receipts: Vec::new(),
            state_root: [0u8; 32],
            timestamp_ms: 0,
        }
    }
}

impl BlockPayload {
    pub fn new(transactions: Vec<Transaction>, compute_receipts: Vec<ComputeReceipt>) -> Self {
        Self {
            version: PROTOCOL_ENCODING_VERSION,
            transactions,
            compute_receipts,
            state_root: [0u8; 32],
            timestamp_ms: 0,
        }
    }

    pub fn with_timestamp_ms(mut self, timestamp_ms: u64) -> Self {
        self.timestamp_ms = timestamp_ms;
        self
    }

    pub fn with_state_root(mut self, state_root: [u8; 32]) -> Self {
        self.state_root = state_root;
        self
    }

    /// Encode block payload into versioned canonical binary format.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, &'static str> {
        if self.version != PROTOCOL_ENCODING_VERSION {
            return Err("unsupported block payload encoding version");
        }
        let mut buf = Vec::new();
        buf.extend_from_slice(BLOCK_MAGIC);
        buf.push(self.version);
        buf.extend_from_slice(&self.state_root);
        buf.extend_from_slice(&self.timestamp_ms.to_be_bytes());
        buf.extend_from_slice(&(self.transactions.len() as u32).to_be_bytes());
        for tx in &self.transactions {
            let tx_bytes = tx.to_canonical_bytes()?;
            write_length_prefixed(&mut buf, &tx_bytes);
        }
        buf.extend_from_slice(&(self.compute_receipts.len() as u32).to_be_bytes());
        for receipt in &self.compute_receipts {
            let r_bytes = receipt.to_canonical_bytes()?;
            write_length_prefixed(&mut buf, &r_bytes);
        }
        Ok(buf)
    }

    /// Decode block payload from versioned canonical binary format, rejecting
    /// unsupported versions, malformed inner entries, or trailing bytes.
    pub fn from_canonical_bytes(mut input: &[u8]) -> Result<Self, &'static str> {
        if input.len() < 4 || &input[..4] != BLOCK_MAGIC {
            return Err("invalid block payload canonical magic");
        }
        input = &input[4..];
        let version = read_u8(&mut input)?;
        if version != PROTOCOL_ENCODING_VERSION {
            return Err("unsupported block payload encoding version");
        }
        if input.len() < 32 {
            return Err("unexpected end of canonical input (state_root)");
        }
        let mut state_root = [0u8; 32];
        state_root.copy_from_slice(&input[..32]);
        input = &input[32..];
        let timestamp_ms = read_u64(&mut input)?;
        let tx_count = read_u32(&mut input)? as usize;
        if tx_count > 10_000 {
            return Err("block transaction count exceeds safety limit");
        }
        let mut transactions = Vec::with_capacity(tx_count);
        for _ in 0..tx_count {
            let raw_tx = read_length_prefixed_bytes(&mut input)?;
            transactions.push(Transaction::from_canonical_bytes(&raw_tx)?);
        }
        let receipt_count = read_u32(&mut input)? as usize;
        if receipt_count > 10_000 {
            return Err("block receipt count exceeds safety limit");
        }
        let mut compute_receipts = Vec::with_capacity(receipt_count);
        for _ in 0..receipt_count {
            let raw_rcpt = read_length_prefixed_bytes(&mut input)?;
            compute_receipts.push(ComputeReceipt::from_canonical_bytes(&raw_rcpt)?);
        }
        if !input.is_empty() {
            return Err("noncanonical trailing bytes in block payload");
        }
        Ok(Self {
            version,
            transactions,
            compute_receipts,
            state_root,
            timestamp_ms,
        })
    }

    /// Domain-separated cryptographic commitment hash for this block payload.
    pub fn commitment_hash(&self) -> Result<[u8; 32], &'static str> {
        let canonical = self.to_canonical_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_BLOCK_COMMIT_V1);
        hasher.update(&canonical);
        Ok(hasher.finalize().into())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_canonical_bytes()
            .expect("canonical block payload serialization failed")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        Self::from_canonical_bytes(bytes)
    }
}

/// On-chain account state for balances and replay protection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AccountState {
    pub balance: u64,
    pub nonce: u64,
}

impl AccountState {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// GPU Worker metadata in MIRASET decentralized compute network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub worker_id: String,
    pub stake: u64,
    pub completed_tasks: u64,
    pub rewards_accumulated: u64,
    pub active: bool,
    /// Available VRAM in GiB for capacity scoring (PoCC).
    #[serde(default = "default_vram_gib")]
    pub vram_gib: u64,
    /// Historical uptime in basis points (e.g. 9850 = 98.50%) in [0, 10000].
    #[serde(default = "default_uptime_bps")]
    pub uptime_bps: u64,
    /// Accumulated verified output tokens in current epoch.
    #[serde(default)]
    pub epoch_tokens: u64,
    /// Owner/operator address holding the bond and receiving withdrawals.
    #[serde(default)]
    pub owner_address: String,
    /// Network endpoint or multiaddr for direct peer job dispatch (optional).
    #[serde(default)]
    pub endpoint: String,
    /// Bond withdrawn with `Unbond` but not yet paid out: it stays slashable and is released to
    /// the owner when the current epoch closes (TODO 8.5.1), so unbonding cannot outrun a slash.
    #[serde(default)]
    pub unbonding: u64,
}

impl WorkerInfo {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

fn default_vram_gib() -> u64 {
    24
}

fn default_uptime_bps() -> u64 {
    10000
}

/// Epoch-level PoCC reward distribution parameters (ADR-0004 & REWARDS.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRewardConfig {
    /// 60% compute pool split (basis points: 6000 / 10000).
    pub compute_split_bps: u64,
    /// 40% capacity pool split (basis points: 4000 / 10000).
    pub capacity_split_bps: u64,
    /// Minimum uptime in basis points to qualify for capacity rewards (e.g. 9000 = 90%).
    pub min_uptime_bps: u64,
    /// Saturation cap for VRAM contribution in GiB (e.g. 80 GiB).
    pub vram_cap_gib: u64,
    /// Whether uptime and VRAM are measured (REWARDS §3.1-3.2). Until signed measurements
    /// exist, self-reported values must not earn capacity rewards: the capacity pool is left
    /// undistributed and rolls over (§5.2 zero-denominator rule; owner decision 2026-11-12).
    pub capacity_measured: bool,
}

impl Default for EpochRewardConfig {
    fn default() -> Self {
        Self {
            compute_split_bps: 6000,
            capacity_split_bps: 4000,
            min_uptime_bps: 9000,
            vram_cap_gib: 80,
            capacity_measured: false,
        }
    }
}

/// Result of an epoch reward distribution calculation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRewardDistribution {
    pub total_reward: u64,
    pub compute_pool: u64,
    pub capacity_pool: u64,
    pub worker_rewards: std::collections::BTreeMap<String, u64>,
    /// Part of `total_reward` not paid to any worker (no tokens, no measured capacity); it
    /// rolls over to the next epoch.
    #[serde(default)]
    pub undistributed: u64,
}

impl EpochRewardConfig {
    /// Checked deterministic PoCC reward distribution across workers sorted by `worker_id`
    /// (PRD R04, ADR-0004, REWARDS.md).
    pub fn try_distribute(
        &self,
        total_budget: u64,
        workers: &[WorkerInfo],
    ) -> Result<EpochRewardDistribution, &'static str> {
        if self.compute_split_bps.checked_add(self.capacity_split_bps) != Some(10_000) {
            return Err("invalid reward pool split bps (must sum to 10000)");
        }
        if self.vram_cap_gib == 0 {
            return Err("vram_cap_gib must be non-zero");
        }

        let compute_pool =
            u64::try_from((total_budget as u128 * self.compute_split_bps as u128) / 10000)
                .map_err(|_| "compute pool overflow")?;
        let capacity_pool = total_budget
            .checked_sub(compute_pool)
            .ok_or("capacity pool underflow")?;

        // Sort workers deterministically by worker_id so remainder rounding is independent of input order.
        let mut sorted_workers: Vec<&WorkerInfo> = workers.iter().collect();
        sorted_workers.sort_by(|a, b| a.worker_id.cmp(&b.worker_id));

        let mut worker_rewards = std::collections::BTreeMap::new();
        for w in &sorted_workers {
            worker_rewards.insert(w.worker_id.clone(), 0u64);
        }

        // 1. Compute pool distribution: proportional to epoch_tokens
        // Only workers still bonded share the compute pool (TODO 8.5.1): unbonding does not
        // keep a claim on rewards; their tokens' share stays undistributed and rolls over.
        let mut total_tokens: u128 = 0;
        for w in sorted_workers.iter().filter(|w| w.active) {
            total_tokens = total_tokens
                .checked_add(w.epoch_tokens as u128)
                .ok_or("total epoch tokens overflow")?;
        }
        if total_tokens > 0 && compute_pool > 0 {
            let mut compute_allocated = 0u64;
            let active_workers: Vec<&WorkerInfo> = sorted_workers
                .iter()
                .copied()
                .filter(|w| w.active && w.epoch_tokens > 0)
                .collect();
            for (idx, w) in active_workers.iter().enumerate() {
                let share = if idx == active_workers.len() - 1 {
                    compute_pool
                        .checked_sub(compute_allocated)
                        .ok_or("compute pool remainder underflow")?
                } else {
                    let s = u64::try_from(
                        (compute_pool as u128 * w.epoch_tokens as u128) / total_tokens,
                    )
                    .map_err(|_| "compute share overflow")?;
                    compute_allocated = compute_allocated
                        .checked_add(s)
                        .ok_or("compute allocated overflow")?;
                    s
                };
                if let Some(r) = worker_rewards.get_mut(&w.worker_id) {
                    *r = r.checked_add(share).ok_or("worker reward overflow")?;
                }
            }
        }

        // 2. Capacity pool distribution:
        // ScoreVal = U^2 * (V / V_cap) with fixed-point precision in [0, 10000].
        let mut capacity_scores = Vec::with_capacity(sorted_workers.len());
        let mut total_capacity_score: u128 = 0;

        for w in &sorted_workers {
            if !self.capacity_measured || !w.active || w.uptime_bps < self.min_uptime_bps {
                capacity_scores.push(0u128);
                continue;
            }
            if w.uptime_bps > 10_000 {
                return Err("worker uptime_bps exceeds 10000");
            }

            let u = w.uptime_bps as u128; // in [0, 10000]
            let u_sq = (u * u) / 10000; // in [0, 10000]

            let capped_vram = w.vram_gib.min(self.vram_cap_gib) as u128;
            let v_norm = (capped_vram * 10000) / self.vram_cap_gib as u128; // in [0, 10000]

            let score = (u_sq * v_norm) / 10000;
            total_capacity_score = total_capacity_score
                .checked_add(score)
                .ok_or("total capacity score overflow")?;
            capacity_scores.push(score);
        }

        if total_capacity_score > 0 && capacity_pool > 0 {
            let eligible_workers: Vec<(&WorkerInfo, u128)> = sorted_workers
                .iter()
                .copied()
                .zip(capacity_scores)
                .filter(|(_, score)| *score > 0)
                .collect();

            let mut capacity_allocated = 0u64;
            for (idx, (w, score)) in eligible_workers.iter().enumerate() {
                let share = if idx == eligible_workers.len() - 1 {
                    capacity_pool
                        .checked_sub(capacity_allocated)
                        .ok_or("capacity pool remainder underflow")?
                } else {
                    let s = u64::try_from((capacity_pool as u128 * *score) / total_capacity_score)
                        .map_err(|_| "capacity share overflow")?;
                    capacity_allocated = capacity_allocated
                        .checked_add(s)
                        .ok_or("capacity allocated overflow")?;
                    s
                };
                if let Some(r) = worker_rewards.get_mut(&w.worker_id) {
                    *r = r.checked_add(share).ok_or("worker reward overflow")?;
                }
            }
        }

        let mut distributed = 0u64;
        for reward in worker_rewards.values() {
            distributed = distributed
                .checked_add(*reward)
                .ok_or("distributed rewards overflow")?;
        }
        let undistributed = total_budget
            .checked_sub(distributed)
            .ok_or("distributed more than the budget")?;
        Ok(EpochRewardDistribution {
            total_reward: total_budget,
            compute_pool,
            capacity_pool,
            worker_rewards,
            undistributed,
        })
    }

    /// Calculate PoCC rewards for a set of workers according to canonical ADR-0004 & REWARDS.md formulas:
    /// - ComputeReward_i = R_comp * T_i / Σ T_j
    /// - ScoreVal_i = (U_i / 10000)^2 * (min(V_i, V_cap) / V_cap)
    /// - CapacityReward_i = R_cap * ScoreVal_i / Σ ScoreVal_j
    pub fn distribute(&self, total_budget: u64, workers: &[WorkerInfo]) -> EpochRewardDistribution {
        self.try_distribute(total_budget, workers)
            .expect("reward distribution failed invariant check")
    }
}

/// Status of an accounting epoch in MIRASET L1 (ADR-0003 & REWARDS.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EpochStatus {
    /// Active submission window (~40 mins): workers submit compute receipts.
    Active,
    /// Challenge window (~10-20 mins): receipts can be disputed or audited.
    Challenge,
    /// Finalized: PoCC reward distribution is computed and settled on-chain.
    Finalized,
}

/// Metadata and state for an on-chain 60-minute accounting epoch (ADR-0003).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochInfo {
    pub epoch_id: u64,
    pub status: EpochStatus,
    /// Unix timestamp when epoch began.
    pub start_timestamp: u64,
    /// Total compute tokens settled during this epoch.
    pub total_tokens: u64,
    /// Total compute receipts submitted in this epoch.
    pub total_receipts: u64,
    /// Total protocol reward budget allocated for this epoch.
    pub reward_budget: u64,
    /// Finalized distribution (if finalized).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distribution: Option<EpochRewardDistribution>,
}

impl EpochInfo {
    pub fn new(epoch_id: u64, start_timestamp: u64, reward_budget: u64) -> Self {
        Self {
            epoch_id,
            status: EpochStatus::Active,
            start_timestamp,
            total_tokens: 0,
            total_receipts: 0,
            reward_budget,
            distribution: None,
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_signature_roundtrip() {
        use ed25519_dalek::SigningKey;

        let key = SigningKey::from_bytes(&[11u8; 32]);
        let mut tx = Transaction::new("placeholder", 0, "bob", 100, 5);

        // Unsigned transactions must never verify.
        assert!(tx.verify_signature().is_err());

        tx.sign_with(&key);
        assert_eq!(tx.sender, hex::encode(key.verifying_key().to_bytes()));
        assert!(tx.verify_signature().is_ok());

        // Tampering with any signed field invalidates the signature.
        tx.amount = 999;
        assert!(tx.verify_signature().is_err());
    }

    #[test]
    fn test_transaction_rejects_sender_pubkey_mismatch() {
        use ed25519_dalek::SigningKey;

        let signer = SigningKey::from_bytes(&[12u8; 32]);
        let mut tx = Transaction::new("not-the-signer", 0, "bob", 100, 5);
        tx.sign_with(&signer);
        // Force a mismatch between the claimed sender and the actual signing key.
        tx.sender = "0".repeat(64);
        assert!(tx.verify_signature().is_err());
    }

    #[test]
    fn test_block_payload_serde_roundtrip() {
        let tx = Transaction::new("alice", 1, "bob", 500, 10);
        let receipt = ComputeReceipt::new(
            "task_101",
            "client_bob",
            "worker_gpu_a",
            "coordinator_main",
            "hash_llama_3",
            "proof_abc123",
            100,
            25,
            20,
        );

        let payload = BlockPayload::new(vec![tx], vec![receipt]);
        let bytes = payload.to_bytes();
        let decoded = BlockPayload::from_bytes(&bytes).expect("decoding failed");

        assert_eq!(payload, decoded);
    }

    #[test]
    fn test_epoch_reward_distribution_60_40() {
        // 60% compute, 40% capacity, with measured capacity.
        let config = EpochRewardConfig {
            capacity_measured: true,
            ..EpochRewardConfig::default()
        };

        let w1 = WorkerInfo {
            worker_id: "worker_a".to_string(),
            stake: 1000,
            completed_tasks: 10,
            rewards_accumulated: 0,
            active: true,
            vram_gib: 40,
            uptime_bps: 9900, // 99%
            epoch_tokens: 120_000,
            owner_address: "owner_a".to_string(),
            endpoint: "10.0.0.1:8000".to_string(),
            unbonding: 0,
        };

        let w2 = WorkerInfo {
            worker_id: "worker_b".to_string(),
            stake: 2000,
            completed_tasks: 8,
            rewards_accumulated: 0,
            active: true,
            vram_gib: 80,
            uptime_bps: 9500, // 95%
            epoch_tokens: 80_000,
            owner_address: "owner_b".to_string(),
            endpoint: "10.0.0.2:8000".to_string(),
            unbonding: 0,
        };

        let total_budget = 3000; // 1800 compute, 1200 capacity
        let dist = config.distribute(total_budget, &[w1, w2]);

        assert_eq!(dist.compute_pool, 1800);
        assert_eq!(dist.capacity_pool, 1200);

        // Compute share: total 200k. w1 has 120k (60%), w2 has 80k (40%).
        // w1 compute = 1800 * 60% = 1080.
        // w2 compute = 1800 * 40% = 720.
        let reward_a = dist.worker_rewards.get("worker_a").copied().unwrap();
        let reward_b = dist.worker_rewards.get("worker_b").copied().unwrap();

        assert!(reward_a > 1080, "worker_a should get compute + capacity");
        assert!(reward_b > 720, "worker_b should get compute + capacity");
        assert_eq!(reward_a + reward_b, total_budget);
    }

    #[test]
    fn test_worker_bond_action_serde() {
        let action = WorkerBondAction::Register {
            worker_id: "worker_1".to_string(),
            bond_amount: 500,
            vram_gib: 48,
            endpoint: "127.0.0.1:9000".to_string(),
        };

        let bytes = action.to_bytes().unwrap();
        let decoded = WorkerBondAction::from_bytes(&bytes).unwrap();
        assert_eq!(action, decoded);

        let unbond = WorkerBondAction::Unbond {
            worker_id: "worker_1".to_string(),
            amount: 200,
        };
        let u_bytes = unbond.to_bytes().unwrap();
        let u_decoded = WorkerBondAction::from_bytes(&u_bytes).unwrap();
        assert_eq!(unbond, u_decoded);
    }

    #[test]
    fn test_canonical_encoding_golden_vectors_and_commitments() {
        let tx = Transaction::new("alice", 1, "bob", 500, 10);
        let tx_bytes = tx.to_canonical_bytes().unwrap();
        assert_eq!(&tx_bytes[..4], b"MTX1");
        assert_eq!(tx_bytes[4], PROTOCOL_ENCODING_VERSION);
        assert_eq!(
            hex::encode(&tx_bytes),
            "4d5458310100000005616c696365000000000000000100000003626f6200000000000001f4000000000000000a000000000000000000000000"
        );
        assert_eq!(Transaction::from_canonical_bytes(&tx_bytes).unwrap(), tx);

        let receipt = ComputeReceipt::new("t1", "alice", "w1", "c1", "m1", "p1", 100, 25, 20);
        let r_bytes = receipt.to_canonical_bytes().unwrap();
        assert_eq!(&r_bytes[..4], b"MRCP");
        assert_eq!(r_bytes[4], PROTOCOL_ENCODING_VERSION);
        assert_eq!(
            ComputeReceipt::from_canonical_bytes(&r_bytes).unwrap(),
            receipt
        );

        let tx_commit = tx.commitment_hash().unwrap();
        let r_commit = receipt.commitment_hash().unwrap();
        let block = BlockPayload::new(vec![tx], vec![receipt]);
        let block_commit = block.commitment_hash().unwrap();
        let state_commit =
            compute_state_commitment(&[(b"acc:alice", b"1000"), (b"acc:bob", b"500")]);
        let state_commit_reordered =
            compute_state_commitment(&[(b"acc:bob", b"500"), (b"acc:alice", b"1000")]);

        assert_eq!(state_commit, state_commit_reordered);
        assert_ne!(tx_commit, r_commit);
        assert_ne!(tx_commit, block_commit);
        assert_ne!(block_commit, state_commit);
    }

    #[test]
    fn test_canonical_encoding_rejects_malformed_and_trailing_bytes() {
        let tx = Transaction::new("alice", 1, "bob", 500, 10);
        let mut tx_bytes = tx.to_canonical_bytes().unwrap();
        tx_bytes.push(0xFF); // noncanonical trailing byte
        assert!(Transaction::from_canonical_bytes(&tx_bytes).is_err());

        // Truncated input
        assert!(Transaction::from_canonical_bytes(&tx_bytes[..10]).is_err());
        // Bad magic
        assert!(Transaction::from_canonical_bytes(b"BAD!01").is_err());

        let receipt = ComputeReceipt::new("t1", "alice", "w1", "c1", "m1", "p1", 10, 5, 4);
        let mut r_bytes = receipt.to_canonical_bytes().unwrap();
        r_bytes.push(0x00);
        assert!(ComputeReceipt::from_canonical_bytes(&r_bytes).is_err());

        let block = BlockPayload::new(vec![tx], vec![receipt]);
        let mut b_bytes = block.to_canonical_bytes().unwrap();
        b_bytes.push(0x42);
        assert!(BlockPayload::from_canonical_bytes(&b_bytes).is_err());
    }

    #[test]
    fn test_canonical_encoding_rejects_unsupported_version() {
        let mut tx = Transaction::new("alice", 1, "bob", 500, 10);
        tx.version = 2;
        assert!(tx.to_canonical_bytes().is_err());
        assert!(tx.verify_signature().is_err());

        let mut valid_tx_bytes = Transaction::new("alice", 1, "bob", 500, 10)
            .to_canonical_bytes()
            .unwrap();
        valid_tx_bytes[4] = 99; // tamper version byte
        assert!(Transaction::from_canonical_bytes(&valid_tx_bytes).is_err());

        let mut receipt = ComputeReceipt::new("t1", "alice", "w1", "c1", "m1", "p1", 10, 5, 4);
        receipt.version = 0;
        assert!(receipt.to_canonical_bytes().is_err());
        assert!(receipt.verify_signature().is_err());

        let block = BlockPayload {
            version: 2,
            ..Default::default()
        };
        assert!(block.to_canonical_bytes().is_err());
    }

    fn hex_key(seed: u8) -> String {
        hex::encode(
            ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
                .verifying_key()
                .to_bytes(),
        )
    }

    #[test]
    fn test_protocol_params_validation() {
        let good = ProtocolParams {
            coordinators: vec![hex_key(1)],
            governance: vec![hex_key(2)],
            ..ProtocolParams::default()
        };
        assert!(good.validate().is_ok());
        let bad = [
            ProtocolParams {
                price_per_token: 0,
                ..good.clone()
            },
            ProtocolParams {
                epoch_reward_budget: 0,
                ..good.clone()
            },
            ProtocolParams {
                epoch_reward_budget: MAX_EPOCH_REWARD_BUDGET + 1,
                ..good.clone()
            },
            ProtocolParams {
                coordinators: vec!["zz".into()],
                ..good.clone()
            },
            ProtocolParams {
                coordinators: vec![hex_key(1), hex_key(1).to_uppercase()],
                ..good.clone()
            },
            ProtocolParams {
                governance: vec![hex_key(1)],
                ..good.clone()
            },
        ];
        for params in bad {
            assert!(params.validate().is_err(), "accepted {params:?}");
        }
    }

    #[test]
    fn test_coordinator_and_worker_signatures_are_not_interchangeable() {
        let key = ed25519_dalek::SigningKey::from_bytes(&[5; 32]);
        let mut receipt = ComputeReceipt::new("t", "c", "w", "co", "m", "p", 1, 1, 1);
        receipt.sign_with(&key);
        // The coordinator signature, copied into the worker slot, is not a worker signature.
        receipt.worker_signature = receipt.signature.clone();
        assert!(receipt.verify_worker_signature(&hex_key(5)).is_err());
        receipt.sign_as_worker(&key);
        assert!(receipt.verify_worker_signature(&hex_key(5)).is_ok());
        assert_ne!(receipt.worker_signature, receipt.signature);
    }
}

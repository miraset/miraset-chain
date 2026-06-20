# Miraset Chain Architecture

## Blockchain Layer
**Platform:** Built as a custom Rust blockchain focused on native assets and object-backed protocol state.

**On-Chain State:**
- Worker registrations and capabilities
- Job objects with escrow
- Receipt hash anchors (cryptographic commitments)
- Epoch settlements and reward distribution

## Off-Chain Compute Infrastructure
**Inference Engine:** Ollama or LMStudio (worker's choice)

**Verification Method:**
- Cryptographic receipt hashes anchored on-chain
- Deterministic hash computation from response streams
- Dual signatures (worker + coordinator) for fraud prevention

## Technical Flow
### Job Lifecycle (End-to-End)
1. **Worker Registration**: Submits `WorkerRegistration` on-chain.
2. **Job Creation**: User funds escrow, coordinator creates `InferenceJob`.
3. **Job Assignment**: Coordinator selects worker, updates job on-chain.
4. **Job Execution**: Worker runs inference, streams output to user.
5. **Work Verification**: Worker generates receipt and hash.
6. **On-Chain Settlement**: Receipt hash anchored on-chain, signatures validated.
7. **Epoch Settlement**: Validators aggregate work and distribute rewards (60-min).

## Data Architecture
### On-Chain Objects
- **WorkerRegistration**: ID, owner, pubkey, capabilities, stake.
- **InferenceJob**: ID, requester, model, price, escrow, status.
- **JobResult**: Tokens, receipt hash, signatures.
- **ReceiptAnchor**: Hash commitment on-chain.

### Off-Chain Storage (Indexer)
- **Receipt Payload Store**: Keyed by hash for auditing.
- **Event Index**: History of jobs, workers, and rewards.

## Technology Stack
- **Blockchain**: Custom Rust chain
- **Worker Runtime**: Rust (Axum + Tokio)
- **Inference Engine**: Ollama / LMStudio
- **Storage**: Sled (on-chain), PostgreSQL (indexer)
- **Cryptography**: Blake3, Ed25519

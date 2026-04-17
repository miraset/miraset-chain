# Miraset Chain — Product Overview

## Executive Summary

**Miraset Chain** is a Rust-based, compute-first blockchain that rewards participants for providing GPU capacity and executing AI inference workloads. It combines on-chain settlement with off-chain inference to create an economic layer for distributed AI compute resources.

> Note on implementation vs concept: some documents describe the design using “Sui-like object model” terminology.
> The current implementation is a custom Rust chain with an object-centric state model and native protocol transactions only.

---

## Core Concept

Miraset Chain connects **GPU providers** (workers/validators) with **AI inference consumers** through a blockchain settlement layer, enabling a decentralized marketplace for AI compute.

**Key innovation:** **Proof of Compute & Capacity (PoCC)** — a dual incentive model rewarding both:

- **Capacity** (keeping GPU resources available and being online)
- **Compute** (verifiable inference work; measured in tokens processed)

---

## How It Works

### Participants

#### 1. Workers (GPU Node Operators)
Workers contribute GPU compute resources to the network:
- Register hardware capabilities (VRAM, GPU model, supported models)
- Run a local inference engine (Ollama backend supported; mock fallback exists for dev)
- Execute inference jobs assigned by a scheduler/coordinator (MVP)
- Submit cryptographic hashes/receipts for verifiability
- Earn rewards for both availability and compute performed

#### 2. Users (AI Inference Consumers)
Users request AI inference services:
- Fund jobs / escrow (conceptual)
- Submit inference requests (prompts, model selection, parameters)
- Receive streamed responses from assigned workers
- Pay per-token pricing for consumed compute (conceptual pricing model)

#### 3. Coordinator / Scheduler (MVP Phase)
A scheduler that:
- Matches jobs to capable workers based on requirements
- Monitors job execution
- Can co-sign receipts in a permissioned MVP model (optional)

#### 4. Validators
Validators secure the network:
- Participate in PoCC consensus (capacity + compute-aware)
- Finalize blocks and epoch transitions
- Distribute rewards based on verified contributions

---

## Technical Architecture

### Blockchain Layer

**Implementation:** custom Rust node (`crates/miraset-node`)

**State model:** object-centric (Sui-inspired), implemented in Rust types (`miraset-core::ObjectData`), including:
- `WorkerRegistration`
- `ResourceSnapshot`
- `InferenceJob`
- `JobResult`
- `ReceiptAnchor`
- `EpochBatch`

**Smart contracts:** not supported. The chain exposes only built-in protocol transactions and core asset/object operations.

**Consensus:** PoCC (Proof of Compute & Capacity) implementation lives in `crates/miraset-node/src/pocc.rs` and `pocc_manager.rs`.

### Off-Chain Compute Infrastructure

**Inference execution:** performed off-chain by `miraset-worker`.

**Verification method (MVP):**
- Deterministic receipt payload hashing
- On-chain receipt anchors (hash commitments)
- Worker signatures (and optional coordinator signatures)

### RPC / APIs (current node)

The node exposes an HTTP RPC (Axum) for basic querying and transaction submission:
- balances, nonces
- latest block / block by height
- events, chat messages
- submit transaction

---

## Technology Stack

### Core Components

- **Blockchain node:** Rust
- **Worker runtime:** Rust (`axum` + `tokio`)
- **Inference engine:** Ollama-compatible backend (plus development fallback)
- **Storage:** Sled (node persistence)
- **Cryptography:** Blake3 hashing, signatures (Ed25519-style keypair in `miraset-core`)
- **API protocols:** HTTP (node RPC and worker API)

> Prior versions of this document referenced “Sui (Rust + Move smart contracts)” and “BCS”.
> Those references are obsolete; the current implementation uses Rust serialization (`bincode`/Serde) and a custom node.

---

## Roadmap (high level)

- Expand PoCC into full end-to-end job marketplace settlement (epoch batching, disputes)
- Harden verifiability and receipt formats (canonical serialization, challenge flows)
- Evolve scheduler from permissioned coordinator toward decentralization
- Continue hardening native asset, job, and settlement flows

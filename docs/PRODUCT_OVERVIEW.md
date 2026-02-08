# Miraset Chain — Product Overview

## Executive Summary

**Miraset Chain** is a blockchain-based decentralized GPU compute marketplace that rewards participants for running AI inference workloads. It combines blockchain settlement with off-chain GPU inference to create an economic layer for distributed AI compute resources.

---

## Core Concept

Miraset Chain connects **GPU providers** (workers) with **AI inference consumers** (users) through a blockchain settlement layer, enabling a decentralized marketplace for AI compute. The system rewards both **capacity** (keeping GPU resources available) and **compute** (actual inference work performed).

**Key Innovation:** A dual incentive model called **Proof of Compute & Capacity (PoCC)** that creates sustainable economics for distributed AI infrastructure.

---

## How It Works

### Participants

#### 1. Workers (GPU Node Operators)
Workers are participants who contribute GPU compute resources to the network:
- Register their hardware capabilities (VRAM, GPU model, supported AI models)
- Run local LLM inference engines (Ollama or LMStudio)
- Execute inference jobs assigned by the coordinator
- Submit cryptographic proofs of work completed
- Earn rewards for both availability and compute performed

#### 2. Users (AI Inference Consumers)
Users request AI inference services:
- Fund escrow accounts with tokens
- Submit inference requests (prompts, model selection, parameters)
- Receive streamed responses from assigned workers
- Pay per-token pricing for consumed compute

#### 3. Coordinator (MVP Phase)
A trusted scheduler that:
- Matches jobs to capable workers based on requirements
- Monitors job execution
- Co-signs work receipts for settlement
- *(Post-MVP: becomes decentralized)*

#### 4. Validators
Blockchain validators who:
- Run consensus (BFT/PoS)
- Finalize settlements every epoch (60 minutes)
- Distribute rewards based on verified work
- Secure the network

---

## Technical Architecture

### Blockchain Layer (Sui-Based)

**Platform:** Built on Sui blockchain (v1.9.1 Move VM)

**Why Sui:**
- Lower resource consumption compared to alternatives
- Object-centric state model (perfect for job lifecycle management)
- Production-ready with robust tooling
- Native Move language for smart contracts

**On-Chain State:**
- Worker registrations and capabilities
- Job objects with escrow
- Receipt hash anchors (cryptographic commitments)
- Epoch settlements and reward distribution

**Consensus:** Modified BFT/PoS with GPU validation requirements

### Off-Chain Compute Infrastructure

**Inference Engine:** Ollama or LMStudio (worker's choice)

**Why Off-Chain:**
- GPU inference is too expensive for on-chain execution
- Enables use of existing, optimized inference engines
- Reduces blockchain bloat while maintaining verifiability

**Verification Method:**
- Cryptographic receipt hashes anchored on-chain
- Deterministic hash computation from response streams
- Dual signatures (worker + coordinator) for fraud prevention

### Worker Runtime (Rust)

Custom `miraset-worker` service that:
- Exposes HTTP API for job management and streaming
- Integrates with local inference engines (Ollama/LMStudio)
- Generates signed usage reports
- Computes deterministic receipt hashes
- Manages job lifecycle and state

---

## Proof of Compute & Capacity (PoCC)

The consensus mechanism that powers Miraset Chain's economic model.

### Capacity Rewards

**Purpose:** Compensate workers for maintaining available GPU resources

**Measured by:**
- Uptime (worker health heartbeats)
- VRAM availability snapshots

**Formula:** 
```
R_capacity(worker) ∝ (uptime_score × available_VRAM)
```

**Why it matters:** Incentivizes maintaining a reliable, always-available network of compute resources.

### Compute Rewards

**Purpose:** Pay workers for actual inference work performed

**Measured by:**
- Verified output tokens generated
- Model difficulty multiplier

**Formula:**
```
R_compute(worker) ∝ (total_verified_tokens × model_difficulty_multiplier)
```

**Why it matters:** Ensures fair compensation based on actual work done, with harder models paying more.

---

## Technical Flow

### Job Lifecycle (End-to-End)

```
1. Worker Registration
   └─> Worker submits WorkerRegistration on-chain
   └─> Declares VRAM capacity, endpoints, supported models
   └─> Periodically submits ResourceSnapshot (VRAM availability)

2. Job Creation
   └─> User funds escrow account
   └─> Coordinator creates InferenceJob object
   └─> Specifies model, max_tokens, constraints

3. Job Assignment
   └─> Coordinator selects capable worker
   └─> Updates job assignment on-chain
   └─> Calls worker API: POST /jobs/accept

4. Job Execution
   └─> Worker runs inference locally via Ollama/LMStudio
   └─> Streams output tokens via GET /jobs/{id}/stream
   └─> User receives real-time streamed response

5. Work Verification
   └─> Worker generates receipt payload
   └─> Computes deterministic receipt_hash
   └─> Submits JobResult with signatures

6. On-Chain Settlement
   └─> Receipt hash anchored on-chain (ReceiptAnchor)
   └─> Validates worker + coordinator signatures
   └─> Verifies token count and escrow availability

7. Epoch Settlement (60-minute batches)
   └─> Validators collect all settled jobs
   └─> Aggregate VRAM snapshots and uptime data
   └─> Calculate and distribute rewards
   └─> Process any disputes or penalties
```

---

## Economic Model

### Payment Flow

```
User Escrow → Worker Payment (per-token pricing)
                    ↓
            Receipt Hash Anchored On-Chain
                    ↓
       Epoch Settlement (60-minute batches)
                    ↓
    Capacity + Compute Rewards Distributed
                    ↓
         Protocol Fees (if applicable)
```

### Pricing Structure (MVP)

- **Fixed per-token pricing** based on model complexity
- **Transparent costs** committed in job object at creation
- **Escrow-based security** ensures payment availability

### Future Pricing (Post-MVP)

- Dynamic pricing based on supply/demand
- Quality-of-Service (QoS) multipliers
- Latency-based SLA pricing tiers

---

## Security Model

### Fraud Prevention Mechanisms

#### 1. Receipt Anchoring
- Every completed job stores a cryptographic hash on-chain
- Receipt payload includes: job_id, epoch_id, request_hash, response_stream_hash, output_tokens
- On-chain hash enables integrity verification without storing full payload

#### 2. Dual Signatures
- Worker signs receipt with private key
- Coordinator co-signs (MVP phase)
- Invalid signatures result in automatic rejection

#### 3. Challenge Window
- Post-settlement period for dispute submission
- Anyone can challenge invalid receipts with evidence
- Validators adjudicate disputes

#### 4. Slashing Mechanism
- Workers stake bonds for participation
- Proven fraud results in stake slashing
- Penalties distributed to dispute submitters + burn

### Threat Mitigation

| Threat | Control |
|--------|---------|
| **Fraudulent token reporting** | Signed receipts + coordinator co-signature + challenge window |
| **Uptime spoofing** | Independent validator sampling + heartbeat verification |
| **Coordinator abuse** | Transparent assignment rules + post-MVP decentralization |
| **Replay attacks** | Job ID uniqueness + "settled once" invariant |
| **Consensus faults** | BFT consensus + validator staking |

---

## Data Architecture

### On-Chain Objects (Sui Model)

#### WorkerRegistration
```
- worker_id, owner, pubkey
- endpoints (HTTP API URLs)
- capabilities: gpu_model, vram_total_gib, supported_models
- stake_bond (for slashing)
- status: Active/Jailed
```

#### InferenceJob
```
- job_id, epoch_id, requester
- model, max_tokens, constraints
- assigned_worker_id
- fixed_price_per_token
- escrow_amount
- status: Created/Assigned/Running/Completed/Finalized
```

#### JobResult
```
- job_id, worker_id
- output_tokens
- receipt_hash (cryptographic commitment)
- worker_signature, coordinator_signature
```

#### ReceiptAnchor
```
- job_id, epoch_id
- receipt_hash (SHA256/Blake3)
- timestamp
```

#### EpochBatch
```
- epoch_id
- total_verified_tokens
- capacity_reward_pool, compute_reward_pool
- committee_attestation
```

### Off-Chain Storage (Indexer)

**Receipt Payload Store:**
- Full receipt payloads keyed by receipt_hash
- Enables dispute resolution and auditing
- Storage: PostgreSQL or S3-compatible blob store

**Event Index:**
- Worker registrations and uptime history
- Job lifecycle events
- Reward distribution history
- Queryable API for explorers and dashboards

---

## Technology Stack

### Core Components

- **Blockchain:** Sui (Rust + Move smart contracts)
- **Worker Runtime:** Rust (`axum` + `tokio` async runtime)
- **Inference Engine:** Ollama or LMStudio (interchangeable)
- **Storage:** Sled DB (on-chain state), PostgreSQL (indexer)
- **Cryptography:** Blake3 hashing, Ed25519 signatures
- **API Protocols:** HTTP + SSE streaming (edge), gRPC (internal)

### Development Stack

- **Language:** Rust (performance, safety, async)
- **HTTP Framework:** Axum (ergonomic, fast)
- **Serialization:** Serde + BCS (canonical binary format)
- **Database:** SQLx + PostgreSQL
- **Observability:** Tracing, Prometheus metrics
- **Testing:** Integration tests with Docker Compose

---

## MVP Deliverables

### Phase 1: Core Infrastructure

1. **Worker Registration System**
   - On-chain identity and capability declaration
   - VRAM snapshot submission
   - Uptime scoring mechanism
   - Health check API (`GET /health`)

2. **Job Lifecycle Management**
   - Job creation with escrow lock
   - Coordinator-based assignment
   - Worker execution and streaming
   - Receipt hash anchoring on-chain

3. **Settlement System**
   - 60-minute epoch batching
   - Automated payment distribution (escrow → worker)
   - Reward calculation using PoCC formulas
   - Capacity + Compute reward distribution

4. **Indexer & Explorer**
   - Event tracking for all on-chain actions
   - Receipt payload storage (by hash)
   - Query API for workers, jobs, epochs
   - Basic web explorer interface

---

## Roadmap

### Phase 1: MVP (Current)
- ✅ Sui-based blockchain integration
- ✅ Worker registration and capability tracking
- ✅ Permissioned coordinator for job assignment
- ✅ Basic PoCC consensus
- ✅ Receipt-based settlement with escrow
- ✅ Ollama/LMStudio integration

### Phase 2: Decentralization
- ⏳ Permissionless job marketplace
- ⏳ Decentralized coordinator (committee-based assignment)
- ⏳ Enhanced proof-of-compute (spot-checking, replication)
- ⏳ Dispute resolution automation
- ⏳ Worker reputation system

### Phase 3: Advanced Features
- 🔮 Zero-knowledge proofs for model execution verification
- 🔮 Cross-chain interoperability (bridge to Solana, Ethereum)
- 🔮 QoS/latency SLAs with performance multipliers
- 🔮 Multi-model routing (ensemble inference)
- 🔮 Privacy-preserving inference (TEE, homomorphic encryption)

### Phase 4: Ecosystem
- 🔮 SDK for dApp developers
- 🔮 Marketplace UI for users and workers
- 🔮 Model hosting and versioning
- 🔮 Fine-tuning job support
- 🔮 Enterprise SLA packages

---

## Competitive Advantages

### 1. Lower Blockchain Overhead
- Off-chain inference + on-chain settlement minimizes gas costs
- Only cryptographic commitments stored on-chain
- Scales without blockchain bottleneck

### 2. Dual Incentive Model
- Rewards both availability (capacity) and utilization (compute)
- Prevents "race-to-zero" pricing for workers
- Sustains long-term network health

### 3. Production-Ready Foundation
- Building on Sui avoids custom blockchain maintenance
- Leverages battle-tested consensus and tooling
- Faster time-to-market

### 4. Flexible Infrastructure
- Workers can use any compatible inference engine
- Not locked into specific hardware or software
- Easy migration from existing Ollama/LMStudio setups

### 5. Transparent Economics
- All settlements and rewards verifiable on-chain
- Deterministic reward calculations
- Audit-friendly with indexer support

---

## Use Cases

### 1. Cost-Efficient AI Inference
- **Problem:** Centralized cloud AI APIs are expensive (OpenAI, Anthropic)
- **Solution:** Distributed GPU network offers competitive pricing
- **Benefit:** 50-70% cost reduction for high-volume users

### 2. Decentralized AI Services
- **Problem:** Single points of failure and censorship in centralized AI
- **Solution:** No single entity controls the network
- **Benefit:** Censorship-resistant, always-available AI inference

### 3. GPU Monetization
- **Problem:** Gaming rigs and data center GPUs sit idle
- **Solution:** Earn passive income by running Miraset worker
- **Benefit:** 24/7 revenue from existing hardware

### 4. Privacy-Preserving AI
- **Problem:** Sending sensitive data to centralized AI providers
- **Solution:** Inference runs on distributed, auditable workers
- **Benefit:** Data sovereignty + verifiable execution

### 5. Research & Education
- **Problem:** Academic institutions have limited AI compute budgets
- **Solution:** Access to distributed GPU network at lower cost
- **Benefit:** Democratized access to AI infrastructure

### 6. Enterprise AI Deployment
- **Problem:** Building internal AI infrastructure is expensive
- **Solution:** On-demand GPU compute with SLA guarantees (Phase 3)
- **Benefit:** Scalable AI without capital expenditure

---

## Market Positioning

### Target Segments

**Early Adopters (MVP Phase):**
- Crypto-native developers building AI dApps
- GPU miners seeking post-merge revenue
- Web3 projects requiring AI features

**Growth Phase:**
- Small/medium AI startups with cost constraints
- Independent developers and hobbyists
- Research institutions

**Enterprise Phase (Future):**
- Large-scale AI inference consumers
- SaaS companies embedding AI features
- Gaming/media companies (rendering + AI)

### Competitive Landscape

| Competitor | Strength | Miraset Advantage |
|------------|----------|-------------------|
| **Centralized APIs** (OpenAI, Anthropic) | Easy integration, high quality | Lower cost, censorship-resistant |
| **Decentralized Compute** (Akash, Render) | General-purpose compute | AI-specific optimizations, token economics |
| **Blockchain AI** (Bittensor, Ritual) | Established networks | Sui foundation, simpler architecture |
| **Self-Hosted** (Ollama, vLLM) | Full control | Monetization layer, no ops overhead |

---

## Economic Model Details

### Token Flows

```
┌─────────────────────────────────────────────────┐
│         User Payment (per-token fees)           │
└────────────────┬────────────────────────────────┘
                 │
                 ├─> 85% → Worker Direct Payment
                 ├─> 10% → Protocol Treasury
                 └─> 5%  → Validator Rewards
                 
┌─────────────────────────────────────────────────┐
│      Protocol Rewards (inflation/treasury)      │
└────────────────┬────────────────────────────────┘
                 │
                 ├─> 60% → Compute Reward Pool
                 └─> 40% → Capacity Reward Pool
```

### Reward Distribution (Per Epoch)

**Capacity Rewards:**
```
For each worker i in epoch e:
  R_capacity(i) = (U_i × V_i) / Σ(U_j × V_j) × CapacityPool(e)

Where:
  U_i = uptime score [0-1]
  V_i = available VRAM (GiB)
```

**Compute Rewards:**
```
For each worker i in epoch e:
  R_compute(i) = (T_i × D_i) / Σ(T_j × D_j) × ComputePool(e)

Where:
  T_i = verified output tokens
  D_i = model difficulty multiplier
```

---

## Operations & Governance

### Network Parameters (Configurable)

- `epoch_duration`: 60 minutes (3600 seconds)
- `base_price_per_token`: 0.001 tokens (varies by model)
- `capacity_reward_pool`: 10,000 tokens/epoch
- `compute_reward_pool`: 15,000 tokens/epoch
- `challenge_window`: 24 hours
- `min_worker_stake`: 100 tokens
- `slash_percentage`: 10% of stake

### Governance Process (Post-MVP)

1. **Proposal Submission** (min 1000 tokens staked)
2. **Discussion Period** (7 days)
3. **Voting Period** (14 days)
4. **Execution** (if quorum + majority reached)

**Governable Parameters:**
- Reward pool allocations
- Pricing parameters
- Slashing percentages
- Epoch duration
- Protocol upgrades

---

## Developer Resources

### Getting Started

```bash
# Clone repository
git clone https://github.com/miraset/miraset-chain
cd miraset-chain

# Build all components
cargo build --release

# Run local testnet
./start_demo.sh

# Deploy worker
cd crates/miraset-worker
cargo run --release -- --config worker.toml
```

### API Examples

**Submit Inference Job:**
```bash
curl -X POST https://api.miraset.io/v1/jobs \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "model": "llama3-8b",
    "prompt": "Explain quantum computing",
    "max_tokens": 500
  }'
```

**Stream Worker Output:**
```bash
curl https://worker.miraset.io/jobs/abc123/stream \
  --no-buffer
```

**Query Job Status:**
```bash
curl https://indexer.miraset.io/v1/jobs/abc123
```

---

## Community & Support

### Resources

- **Documentation:** https://docs.miraset.io
- **GitHub:** https://github.com/miraset/miraset-chain
- **Discord:** https://discord.gg/miraset
- **Twitter:** @miraset_chain
- **Blog:** https://blog.miraset.io

### Contributing

We welcome contributions! See `CONTRIBUTING.md` for guidelines.

**Areas for contribution:**
- Core protocol development (Rust)
- Move smart contract development
- Worker runtime optimizations
- Indexer improvements
- Documentation and tutorials
- Testing and security audits

---

## FAQ

### For Workers

**Q: What hardware do I need?**
A: Minimum 8GB VRAM (NVIDIA recommended). RTX 3060+ or similar.

**Q: How much can I earn?**
A: Depends on uptime, VRAM, and jobs executed. Typical: $50-200/month per GPU.

**Q: What models do I need to support?**
A: Your choice. Popular models (Llama, Mistral) get more jobs.

### For Users

**Q: How much does inference cost?**
A: ~50-70% cheaper than centralized APIs. Prices vary by model.

**Q: Is my data private?**
A: Jobs are executed on distributed workers. Use encryption for sensitive data.

**Q: What's the latency?**
A: Similar to centralized APIs (MVP). Phase 3 adds QoS tiers.

### For Validators

**Q: What are validator requirements?**
A: Standard Sui validator requirements + ability to verify GPU proofs.

**Q: What rewards do validators earn?**
A: 5% of job fees + staking rewards from Sui consensus.

---

## Conclusion

Miraset Chain creates a sustainable economic protocol for decentralized GPU compute by combining:

1. **Blockchain settlement** for trust and auditability
2. **Off-chain inference** for performance and cost
3. **Dual incentives** (capacity + compute) for network health
4. **Production-ready foundation** (Sui) for reliability

By rewarding both **availability** and **utilization**, Miraset incentivizes a robust, distributed network of AI inference providers while maintaining the security and transparency of blockchain settlement.

---

**Version:** 1.0 (MVP)  
**Last Updated:** February 2026  
**License:** MIT  
**Contact:** team@miraset.network

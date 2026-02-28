# CoinSecurities — Technical Whitepaper (Draft)

**Version:** 0.1 (Draft)  
**Date:** 2026-02-27  
**Product:** CoinSecurities  
**Base L1:** Miraset Chain  
**Native token on L1:** SECCO  

> This document is a technical draft intended for engineering, product, and ecosystem partners.
> It describes the architecture, core flows, and the synergy between AI portfolio management and a compute-first blockchain (Miraset Chain).

---

## 0. Abstract

CoinSecurities is a SaaS investment platform where users lock capital via smart contracts and grow assets with AI-managed strategies.

The system combines:

- **AI concierge**: an LLM-facing interface that translates user intent into actionable portfolio operations.
- **Decision Engine (Trading Core)**: a deterministic computation layer for strategy selection, portfolio optimization, and risk management.
- **Miraset Chain**: a compute-first blockchain providing settlement, escrow/locking logic, verifiable receipts, and incentives for GPU-providing nodes via **Proof of Compute & Capacity (PoCC)**.

The key design principle is separation of concerns:

- **AI explains and orchestrates** (human-facing interpretation + tool routing).
- **The Decision Engine computes** (auditable, deterministic, testable).
- **The blockchain settles and incentivizes** (state transitions, receipts, accounting, rewards).

---

## 1. Portfolio Management Problematic

### 1.1. The problem

Retail and semi-professional investors typically face a set of recurring problems:

1. **Fragmented execution**
   - Signals, research, and execution live across multiple tools and custodians.

2. **Lack of enforceable constraints**
   - Risk limits are often stated (e.g., “no more than 20% in a single asset”) but not enforced.

3. **Low transparency and auditability**
   - In most managed products, the user can’t easily verify how returns were generated.

4. **Operational overhead**
   - Rebalancing, reinvesting, and monitoring are time-consuming.

5. **Compute asymmetry**
   - Institutional-grade optimization, simulation, and forecasting are compute heavy.

### 1.2. Requirements (technical)

CoinSecurities targets a system that provides:

- **Capital locking**: funds are locked on-chain under explicit contractual rules.
- **Explicit risk profile**: conservative / moderate / aggressive profiles, plus custom constraints.
- **Deterministic strategy computation**: the core “what to do” should be reproducible.
- **Explainability**: users need an understandable rationale for each recommendation.
- **Trust-minimized settlement**: recommendations may be off-chain, but transfers/locking and accounting should be on-chain.

---

## 2. Solutions on AI

CoinSecurities uses two AI-adjacent layers:

1. **AI Concierge (LLM Orchestration Layer)**
2. **Decision Engine (Trading Algorithm / Core)**

### 2.1. AI Concierge (LLM Integration Layer)

The AI Concierge is a user-facing agent that:

- Understands natural language intent.
- Chooses which module of the Decision Engine to call.
- Interprets the returned structured result into a human-readable explanation.

**Interaction chain:**

`User Query → LLM Analysis → Decision Engine Computation → LLM Interpretation → Response`

#### 2.1.1. Concierge goals

- Provide a low-friction user experience (“Ask for your best next action in plain English”).
- Enforce safe routing (“Not allowed assets”, liquidity requirements, risk limits).
- Produce consistent and explainable recommendations.

#### 2.1.2. Guardrails

- The LLM is treated as a **planner + narrator**, not a source of truth.
- Portfolio actions must be expressed as **structured operations** (orders, rebalance operations, constraints updates) before they can be executed.
- All executions that impact capital must go through blockchain-validated state transitions.

### 2.2. Decision Engine (Trading Algorithm / Core)

The Decision Engine is a modular computation layer.

Suggested MVP modules (as a stable contract between Concierge and Core):

- **User Profiling Service**
  - risk profile (conservative / moderate / aggressive)
  - constraints (blocked assets/currencies, liquidity constraints)
  - behavior history (optional in MVP)

- **Portfolio Optimization Module**
  - objective function (risk-adjusted return)
  - constraints enforcement
  - outputs: target weights, rebalance deltas, expected risk

- **Risk Management Module**
  - limit checks (concentration, drawdown, liquidity)
  - alert events (e.g., “>50% in one asset”)

- **Reinvestment Strategy Module** (optional)
  - automatic reinvestment rules
  - periodic rebalancing schedule

**Key contract:** the Decision Engine returns a deterministic result (JSON-like structure) that can be audited and tested.

### 2.3. Product API surface (SaaS)

An indicative REST API surface (MVP), aligned with the concierge architecture:

`/api/v1`

- `/auth`
- `/user`
- `/chat`
- `/portfolio`
- `/recommendations`
- `/transactions`
- `/analytics`
- `/blockchain`

This is an application-layer API and is independent from the Miraset Chain protocol RPC.

---

## 3. Blockchain (Miraset Chain)

CoinSecurities uses **Miraset Chain** as its L1 settlement and incentive layer.

### 3.1. What Miraset Chain is

Miraset Chain is a compute-first blockchain designed to reward nodes for:

1. **Capacity**: being online and providing GPU capacity (VRAM)
2. **Compute**: executing useful workloads (LLM inference) measured in **verified tokens**

This incentive model is called **Proof of Compute & Capacity (PoCC)**.

Miraset Chain follows an object-centric state model (Sui-like in concept):

- Worker registrations
- Inference jobs
- Results + receipt anchors
- Epoch settlement objects

> Note: the current implementation is a Rust-based chain with object-centric state types.
> Some conceptual docs describe Sui/Move; public-facing docs must match the implemented design.

### 3.2. Consensus (PoCC)

PoCC combines:

- **Proof of Capacity**: uptime + VRAM availability
- **Proof of Compute**: verified tokens processed

Validators must meet minimum hardware/software requirements (in the reference implementation):

- VRAM minimum: 16 GiB
- Models loaded: at least 3 (1× large 13B+, 2× medium 7B+)
- Stake minimum: 10B units (in smallest units)
- Minimum uptime: 85%

### 3.3. Receipts and verifiability

Each completed job produces:

- a **receipt payload** (request hash + response stream hash + token counts + timestamps)
- a **receipt hash** anchored on-chain

This design allows:

- auditability without posting private prompts/outputs on-chain
- dispute/challenge flows when a receipt is suspected to be invalid

### 3.4. Token economics (SECCO)

SECCO is the native token of Miraset Chain.

CoinSecurities uses SECCO for:

- gas / transaction fees (chain-level)
- collateral / stake (validators and potentially workers)
- reward distribution (PoCC)
- application-level operations (escrow, locks, strategy settlement), where applicable

#### 3.4.1. Draft block rewards (product-level draft)

A draft reward envelope discussed for SECCO (subject to governance parameters):

- **Capacity reward per block:** 1 SECCO
- **Compute reward per block:** 0…10 SECCO
  - distributed proportionally across participants by their compute contribution during that block

> Implementation note: current code uses a 5-second block time constant in PoCC and a per-epoch reward model is also described in docs.
> CoinSecurities treats these numbers as *protocol parameters* that can be set by governance; the public spec should not hardcode Sui block time unless the chain targets it.

### 3.5. Practical chain interfaces (current implementation)

The current node exposes an HTTP RPC (Axum) for basic chain state and transaction submission:

- `GET /balance/{address}`
- `GET /nonce/{address}`
- `GET /block/latest`
- `GET /block/{height}`
- `GET /events`
- `GET /chat/messages`
- `POST /tx/submit`

The worker runtime exposes an Ollama-like HTTP API:

- `GET /health`
- `POST /jobs/accept`
- `POST /jobs/run`
- `GET /jobs/:id/status`
- `GET /jobs/:id/stream`
- `POST /jobs/:id/report`

### 3.6. Job lifecycle (conceptual)

1. Worker registers on-chain with endpoints + GPU capabilities.
2. Coordinator (or policy engine) assigns an inference job.
3. Worker runs inference off-chain and streams output.
4. Worker submits signed result + anchors receipt hash on-chain.
5. Settlement logic accounts for verified tokens and distributes rewards.

---

## 4. Synergy (AI × Blockchain)

CoinSecurities treats Miraset Chain as an economic coordination system for compute.

### 4.1. Why AI needs the chain

- The Decision Engine can be compute-heavy (portfolio optimization, simulations, scenario analysis).
- A decentralized compute market provides elastic access to GPU resources.

### 4.2. Why the chain benefits from CoinSecurities

- The application generates sustained demand for inference workloads.
- This demand helps bootstrap PoCC rewards into a functioning compute economy.

### 4.3. Priority access (“master account”) concept

Because node operators contribute compute to the network, CoinSecurities’ trading core can be given a privileged policy identity ("master account") that may reserve up to **n%** of network compute capacity for internal needs.

This can be implemented as a governance-controlled mechanism, e.g.:

- quota-based scheduling
- priority fee markets
- reserved worker pools

### 4.4. User participation in computation (future directions)

Several integration patterns exist for “users contribute compute to improve predictions”:

- Centralized AI with on-chain staking on prediction outcomes
- Decentralized client-side inference with on-chain staking
- Federated learning

These are explicitly future directions and must not be represented as implemented features in the MVP.

---

## 5. Conclusion

CoinSecurities is built as a modular system:

- An AI concierge that converts human intent into structured portfolio actions.
- A deterministic Decision Engine that produces testable and auditable recommendations.
- A compute-first blockchain (Miraset Chain) that provides settlement, verifiability, and incentives for GPU-providing nodes via PoCC.

By aligning portfolio optimization demand with a verifiable and incentivized compute network, CoinSecurities aims to deliver a scalable and transparent investment operations layer.

---

## Appendix A — Source references (project-internal)

- `docs/ARCHITECTURE.md` — system architecture
- `docs/SOW.md` — scope and object model
- `docs/REWARDS.md` — canonical economics formulas
- `docs/POCC_IMPLEMENTATION.md` — PoCC implementation notes
- `crates/miraset-node` — node, state, RPC, PoCC modules
- `crates/miraset-worker` — worker runtime, receipts, node client


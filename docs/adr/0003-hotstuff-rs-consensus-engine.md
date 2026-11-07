# ADR-0003: HotStuff-rs as MIRASET L1 Consensus Engine and Hexagonal Architecture

**Date**: 2026-11-07  
**Status**: accepted  
**Deciders**: @daochild, Copilot Architecture Team  

## Context

MIRASET Chain requires a Byzantine Fault Tolerant (BFT) consensus engine capable of coordinating decentralized GPU compute verification (Proof-of-Useful-Compute / PoCC), ledger state transitions, and fast block finality. 

Initial exploratory documentation evaluated hybrid approaches including CometBFT (Tendermint). However, deploying CometBFT introduced significant cross-language impedance mismatches (Go daemon vs. Rust application logic), inter-process ABCI serialization overhead, and dual-process operational friction. 

The consensus engine must:
1. Be written natively in Rust to allow compiled single-binary execution and zero-copy memory access.
2. Provide deterministic pipelined consensus with linear message complexity ($O(n)$) during leader transitions.
3. Maintain clean architectural separation between consensus mechanics, networking, persistent storage, and execution logic.

## Decision

We adopt `hotstuff_rs` (v0.4.0) as the primary consensus engine for MIRASET L1 and organize the node implementation using **Hexagonal Architecture (Ports and Adapters)**:

1. **Storage Adapter (Port: `KVStore`)**:
   Implement `hotstuff_rs::block_tree::pluggables::{KVStore, WriteBatch, KVGet}`. Provide `MemKVStore` for in-memory testing and automated CI, with a pluggable path to durable storage (RocksDB) for production.
2. **Network Adapter (Port: `Network`)**:
   Implement `hotstuff_rs::networking::network::Network`. Provide `ChannelNetworkHub` and `ChannelNetwork` for local devnet clusters and unit/integration testing, mapping directly to `hotstuff_rs` message passing.
3. **Execution Engine (Port: `App<K>`)**:
   Implement `MirasetApp` fulfilling `hotstuff_rs::app::App<K>`. It executes deterministic block production (`produce_block`) and state transition validation (`validate_block`), maintaining account balances and decentralized GPU worker state (task completion, stake, PoCC rewards).
4. **Mempool & Node Orchestrator**:
   Implement `Mempool` with fee-based priority sorting and nonce deduplication. Encapsulate `ReplicaSpec::builder()` lifecycle, genesis block tree bootstrapping, and query cameras into `MirasetNode`.

## Alternatives Considered

### Alternative 1: CometBFT (Tendermint Core via ABCI)
- **Pros**: Mature battle-tested BFT consensus in Cosmos ecosystem, instant single-slot finality.
- **Cons**: Foreign Go binary, ABCI IPC latency, operational overhead managing two separate runtime processes.
- **Why not**: Breaks Rust-native single-binary packaging; impedes tight integration with GPU compute verifiers.

### Alternative 2: Narwhal & Bullshark (DAG-based Consensus)
- **Pros**: High transaction throughput via asynchronous mempool DAG separation.
- **Cons**: Substantial architectural complexity, higher bandwidth requirements, and immature standalone Rust libraries.
- **Why not**: Excessive complexity for Phase 1 MIRASET requirements.

### Alternative 3: Custom PBFT/Raft Implementation
- **Pros**: Complete internal control over consensus code.
- **Cons**: Extreme engineering risk, audit complexity, and high likelihood of subtle liveness/safety bugs.
- **Why not**: Pre-existing verified implementations like HotStuff-rs provide formal safety guarantees.

## Consequences

### Positive
- **Single-Binary Rust Node**: Consensus, mempool, storage, and PoCC verifiers run within a unified type-safe Rust runtime.
- **Linear Communication Complexity**: HotStuff chained consensus achieves $O(n)$ message exchanges for view changes using threshold phase certificates (Ed25519 signatures).
- **Testability & Determinism**: In-memory channel network and memory KV store allow fully automated multi-validator consensus tests executing in milliseconds without OS sockets or disk I/O.
- **Decoupled Architecture**: Hexagonal traits isolate HotStuff-rs internals from MIRASET domain logic.

### Negative
- **3-Chain Commit Latency**: Chained HotStuff requires 3 consecutive valid blocks on a branch to reach absolute finality (Commit phase).
- **Custom Production Adapters Required**: HotStuff-rs intentionally leaves P2P transport and disk storage to the user, requiring MIRASET to maintain production-grade adapters (e.g., libp2p and RocksDB).

### Risks and Mitigations
- **Risk**: Pacemaker view timeouts if network partitions occur.
  - **Mitigation**: Configurable adaptive view timeouts, exponential backoff, and robust BlockSync fallback subprotocols provided by `hotstuff_rs`.

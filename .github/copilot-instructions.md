# Copilot instructions

Follow the shared repository guidance in [`AGENTS.md`](../AGENTS.md). This file adds the repository-specific workflows and architectural boundaries Copilot should preserve.

## Build, test, and lint

The Rust workspace uses the toolchain pinned in `rust-toolchain.toml` (1.97.1).

```bash
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D clippy::unwrap_used -D clippy::expect_used
cargo test --workspace
```

Run one Rust test by package and name filter, for example:

```bash
cargo test -p miraset-core test_block_bincode_stability
cargo test -p miraset-node --test state_tests test_submit_transfer_valid
cargo test -p miraset-node --test integration_tests test_rpc_submit_transaction
```

The node integration tests start their own in-process RPC server. They do not need a separately running node.

The desktop wallet is a separate Next.js + Tauri project in `wallet/`. From that directory, run `bun install` first, then use `bun run lint`, `bun run build`, `bun run tauri:dev`, or `bun run tauri:build`. There is no wallet test script.

## Architecture and runtime flow

- `crates/miraset-core/src/types.rs` defines the shared transaction, object, block, and event data model; core cryptography and signing live in `crates/miraset-core`.
- The `miraset-cli` binary starts the single-author devnet node. `miraset-node` owns state, validation, block production, persistence, and the Axum RPC API. State mutation for produced blocks is implemented in `crates/miraset-node/src/state/`; `rpc.rs` is the HTTP boundary.
- A submitted transaction is validated and queued by the node, then applied during block production. The producer updates epoch status before each block and dispatches worker job assignments after block execution.
- The worker is a separate Axum service in `crates/miraset-worker`. It accepts assigned jobs, calls a pluggable inference backend, creates deterministic receipts, and submits results to the node.
- `miraset-indexer` currently provides in-memory block/event indexing. The desktop wallet lives independently under `wallet/`; it is not part of the Rust workspace.
- PoCC validator/proposer code is scaffolding and is not wired into block production; do not assume the devnet block producer provides multi-node consensus.

For API changes, keep the handlers and their contracts aligned: node endpoints are documented in `docs/API_NODE.md`, and worker endpoints in `docs/API_WORKER.md`.

## Repository-specific invariants

- Account balances/nonces and object-based state coexist. Changes to either path must preserve the other where applicable.
- `bincode` is pinned to exactly `=1.3.3`. Transactions, blocks, and object data participate in signing, persistence, IDs, or hashes; changing their serialized form can break compatibility. Preserve the serialization-stability tests unless intentionally changing that format.
- Worker receipt hashes use deterministic bincode serialization and BLAKE3 (`crates/miraset-worker/src/receipt.rs`); preserve their canonical input and field ordering.
- CLI configuration precedence is CLI arguments, then `MIRASET_` environment variables, then `miraset.toml`, then defaults. Keep new configuration consistent with that precedence.
- State internals use `Arc<RwLock<StateInner>>` with `parking_lot`; preserve the established lock ordering when touching shared state.
- Crate roots deny Clippy `unwrap_used` and `expect_used`; test modules locally allow them. CI runs formatting, Clippy, and workspace tests as shown above.

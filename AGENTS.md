# AGENTS.md

## Repo quick map
- Rust workspace root in `Cargo.toml`; main crates: `miraset-core` (types/crypto), `miraset-node` (state+RPC+storage), `miraset-cli` (binary `miraset`), `miraset-worker` (HTTP worker), `miraset-wallet` (key store), `miraset-tui` (terminal UI).
- Desktop wallet is separate in `wallet/` (Next.js + Tauri); packaging scripts live in `tools/launcher/`.
- `miraset-indexer` is currently a placeholder (`crates/miraset-indexer/src/lib.rs`).

## Big-picture architecture (code-first)
- On-chain transaction/object model is defined in `crates/miraset-core/src/types.rs` (`Transaction`, `ObjectData`, `Event`).
- Node runtime keeps in-memory state plus optional Sled persistence: `State::new_with_storage` and block production in `crates/miraset-node/src/state.rs`.
- RPC boundary is Axum in `crates/miraset-node/src/rpc.rs` (`/balance`, `/nonce`, `/block/*`, `/events`, `/chat/messages`, `/tx/submit`).
- Worker is a separate Axum service in `crates/miraset-worker/src/lib.rs`; it accepts jobs, runs inference via `backend.rs`, generates receipt hashes in `receipt.rs`, and submits chain tx via `node_client.rs`.
- Current Move integration is scaffold/placeholder (not full VM execution): see comments and behavior in `crates/miraset-node/src/move_vm.rs` and `crates/miraset-node/src/executor.rs`.

## Runtime data flow you should preserve
- CLI starts node (`miraset node start`), opens `Storage`, seeds a fixed devnet genesis account, spawns block producer, then serves RPC (`crates/miraset-cli/src/main.rs`).
- Worker startup tries on-chain registration immediately; failure is logged and service still starts (`crates/miraset-worker/src/main.rs`).
- Tx path is JSON `Transaction` -> `POST /tx/submit` -> `State::submit_transaction` validation -> included by periodic `run_block_producer`.
- Event persistence uses JSON in Sled (`Storage::save_event`), while blocks use bincode (`Storage::save_block`).

## Developer workflows (known-working patterns from repo)
- Build all Rust crates: `cargo build --workspace` (or `--release`).
- Start local node: `cargo run --bin miraset -- node start`.
- Start worker: `cargo run --bin miraset-worker`.
- Run Rust tests: `cargo test --workspace`.
- Integration tests in `tests/integration_tests.rs` assume node RPC already running at `http://127.0.0.1:9944`.
- Full desktop bundle: `tools/launcher/build-and-package.bat` (Windows) or `./tools/launcher/build-and-package.sh` (bash).

## Project-specific conventions and gotchas
- Config precedence in CLI is intended as CLI > env > file > defaults (`load_config`/`apply_env_overrides` in `crates/miraset-cli/src/main.rs`).
- Env var names use `MIRASET_` prefix (`MIRASET_RPC_ADDR`, `MIRASET_STORAGE_PATH`, `MIRASET_BLOCK_INTERVAL`).
- Signatures are usually computed over serialized tx with signature field zeroed first (see transfer/chat signing in `crates/miraset-cli/src/main.rs` and verification in `State::submit_transaction`).
- Receipt hash logic is deterministic bincode + blake3 (`crates/miraset-worker/src/receipt.rs`); preserve this when changing receipt fields.
- Object-centric features coexist with legacy account balances/nonces in `State`; avoid breaking both paths during refactors.

## Integration surfaces
- Node API contract: `docs/API_NODE.md` plus concrete handler code in `crates/miraset-node/src/rpc.rs`.
- Worker API contract: `docs/API_WORKER.md` plus concrete router in `crates/miraset-worker/src/lib.rs`.
- External AI backend default is Ollama (`/api/generate`, `/api/tags`) with mock fallback when unavailable (`crates/miraset-worker/src/backend.rs`).
- Multi-node Docker dev setup is defined in `docker-compose.yml` (ports 9944-9947).

## Existing agent-instruction sources scanned
- `README.md`, `wallet/README.md`, `wallet/src-tauri/README.md`, `tools/launcher/README.md`, `crates/miraset-launcher/README.md`, `crates/miraset-worker/README.md`.
- No dedicated repo-level AI rules file was found in the scanned patterns (`AGENT.md`, `CLAUDE.md`, `.cursorrules`, etc.).


# AGENTS.md

Primary instructions are located at:
- node_modules/@daochild/agents-config/AGENTS.md

Follow all instructions from that file unless overridden below.

## Repo quick map
- Rust workspace root in `Cargo.toml`; main crates: `miraset-core` (types/crypto), `miraset-node` (state+RPC+storage), `miraset-cli` (binary `miraset`), `miraset-worker` (HTTP worker), `miraset-wallet` (key store), `miraset-tui` (terminal UI), `miraset-launcher` (desktop launcher).
- Desktop wallet is separate in `wallet/` (Next.js + Tauri); packaging scripts live in `tools/launcher/`.
- `miraset-indexer` provides an in-memory event and block indexer (`crates/miraset-indexer/src/lib.rs`); a durable backend is intended for future iterations.
- Workspace uses `edition = "2024"` and pins `bincode = "=1.3.3"` (exact version required for deterministic serialization across crates).

## Big-picture architecture (code-first)
- On-chain transaction/object model is defined in `crates/miraset-core/src/types.rs` (`Transaction`, `ObjectData`, `Event`).
- Node runtime keeps in-memory state plus optional Sled persistence: `State::new_with_storage` and block production in `crates/miraset-node/src/state.rs`.
- RPC boundary is Axum in `crates/miraset-node/src/rpc.rs` (`/balance`, `/nonce`, `/block/*`, `/events`, `/chat/messages`, `/tx/submit`, `/health`, `/status`, `/ping`, `/jobs`, `/jobs/submit`, `/jobs/{id}`, `/workers`, `/epoch`).
- Job coordinator is integrated into block production: `POST /jobs/submit` accepts a signed `Transaction::CreateJob`, routes it through the transaction pipeline, and block execution auto-assigns the job to a suitable active worker. After the block is produced, the node dispatches an HTTP call to the worker's `/jobs/accept` endpoint.
- Worker is a separate Axum service in `crates/miraset-worker/src/lib.rs`; it accepts jobs, runs inference via `backend.rs`, generates receipt hashes in `receipt.rs`, and submits chain tx via `node_client.rs`. Worker endpoints: `/health`, `/ping`, `/jobs/accept`, `/jobs/run`, `/jobs/{id}/stream`, `/jobs/{id}/report`, `/jobs/{id}/status`.
- Epoch management in `crates/miraset-node/src/epoch.rs`: 60-min epochs with submit/challenge windows, worker stats tracking, and reward distribution (70% capacity / 30% compute split).
- Gas metering system in `crates/miraset-node/src/gas.rs` and executor in `crates/miraset-node/src/executor.rs`: defines `GasConfig`, `GasBudget`, `GasStatus` with per-operation costs. **Note:** gas is currently only enforced in `ExecutionContext::execute_transaction`, NOT in the main `State::produce_block` path.
- PoCC (Proof of Compute Contribution) consensus scaffolding in `crates/miraset-node/src/pocc.rs` and `crates/miraset-node/src/pocc_manager.rs`: defines `Validator`, `ValidatorSet`, `PoccConsensus` with stake-weighted proposer selection. Not yet wired into block production.
- Current Move integration is scaffold/placeholder (not full VM execution). `crates/miraset-node/src/move_vm.rs` does not exist. `crates/miraset-node/src/executor.rs` defines a parallel `ExecutionContext` that is currently not invoked from the main `State::produce_block` path; all transaction execution happens in `crates/miraset-node/src/state/execution.rs`.

## Runtime data flow you should preserve
- CLI starts node (`miraset node start`), opens `Storage`, seeds a fixed devnet genesis account (`[1u8; 32]` secret key, 1 trillion tokens), spawns block producer, then serves RPC (`crates/miraset-cli/src/main.rs`).
- Block producer loop (`run_block_producer` in `crates/miraset-node/src/lib.rs`) calls `state.update_epoch()` to auto-advance epoch status before each `produce_block()`.
- Worker startup tries on-chain registration immediately; failure is logged and service still starts (`crates/miraset-worker/src/main.rs`). On success, starts a heartbeat loop (30s interval) that submits `SubmitResourceSnapshot` TX with VRAM availability data.
- Tx path is JSON `Transaction` -> `POST /tx/submit` -> `State::submit_transaction` validation -> included by periodic `run_block_producer`.
- Event persistence uses JSON in Sled (`Storage::save_event`), while blocks use bincode (`Storage::save_block`). Balances and nonces also persisted to Sled and lazily loaded on cache miss.

## Developer workflows (known-working patterns from repo)
- Build all Rust crates: `cargo build --workspace` (or `--release`).
- Start local node: `cargo run --bin miraset -- node start`.
- Start worker: `cargo run --bin miraset-worker`.
- Run Rust tests: `cargo test --workspace`.
- Integration tests in `tests/integration_tests.rs` assume node RPC already running at `http://127.0.0.1:9944`.
- Full desktop bundle: `tools/launcher/build-and-package.bat` (Windows) or `./tools/launcher/build-and-package.sh` (bash).

## Project-specific conventions and gotchas
- Config precedence in CLI is intended as CLI > env > file > defaults (`load_config`/`apply_env_overrides` in `crates/miraset-cli/src/main.rs`). Config file is `miraset.toml` at project root (auto-discovered).
- Env var names use `MIRASET_` prefix (`MIRASET_RPC_ADDR`, `MIRASET_STORAGE_PATH`, `MIRASET_BLOCK_INTERVAL`). Worker backend selection also uses `MIRASET_` prefix: `MIRASET_BACKEND_TYPE` (default `ollama`; accepted: `ollama`, `openai` and aliases `lmstudio`/`vllm`/`localai`/`groq`/`openrouter`/`together`/`fireworks`/`anyscale`, `openllm`/`bentoml`, `llamacpp`, `tgi`), `MIRASET_BACKEND_URL` (per-backend / per-preset default), `MIRASET_BACKEND_API_KEY` (optional Bearer auth; required for cloud OpenAI-compatible providers like OpenAI/Groq/OpenRouter), `MIRASET_SUPPORTED_MODELS` (comma-separated model id list; overrides `BackendType::default_models()`).
- Wallet keystore path is `~/.miraset/wallet.json` (uses `HOME` or `USERPROFILE` on Windows).
- Signatures are usually computed over serialized tx with signature field zeroed first (see transfer/chat signing in `crates/miraset-cli/src/main.rs` and verification in `State::submit_transaction`).
- Receipt hash logic is deterministic bincode + blake3 (`crates/miraset-worker/src/receipt.rs`); preserve this when changing receipt fields.
- Object-centric features coexist with legacy account balances/nonces in `State`; avoid breaking both paths during refactors.
- `bincode` is pinned to exact `=1.3.3` across the workspace; do not upgrade without verifying all stored/hashed data remains compatible.
- `State` inner fields are behind `Arc<RwLock<StateInner>>` (using `parking_lot::RwLock`); always acquire locks in consistent order to avoid deadlocks.
- `InferenceBackend` trait in the worker uses `async_trait`; any new backend implementation must be `Send + Sync`.

## Integration surfaces
- Node API contract: `docs/API_NODE.md` plus concrete handler code in `crates/miraset-node/src/rpc.rs`. Includes job coordinator endpoints (`/jobs/submit`, `/jobs`, `/jobs/{id}`, `/workers`, `/epoch`).
- Worker API contract: `docs/API_WORKER.md` plus concrete router in `crates/miraset-worker/src/lib.rs`. Job lifecycle endpoints: `/jobs/accept` (POST), `/jobs/run` (POST), `/jobs/{id}/stream` (GET), `/jobs/{id}/report` (POST), `/jobs/{id}/status` (GET).
- External AI backend is selected via `MIRASET_BACKEND_TYPE` (default `ollama`). Implementations in `crates/miraset-worker/src/backend.rs`: `OllamaBackend` (`/api/generate`, `/api/tags`), `OpenAiCompatibleBackend` (`/v1/chat/completions`, `/v1/models`; covers LM Studio, vLLM, LocalAI, llama.cpp OpenAI shim, TGI OpenAI shim, and cloud OpenAI/Groq/OpenRouter/Together — cloud requires `MIRASET_BACKEND_API_KEY`), `OpenLlmBackend` (BentoML OpenLLM `/v1/chat/completions`, `/v1/models`), `LlamaCppBackend` (native llama.cpp `/completion`, `/props`, `/health`; one model per server, ignores per-request `model`), `TgiBackend` (HuggingFace TGI `/generate`, `/info`; one model per instance). All backends share a deterministic mock-fallback when the engine is unreachable. Per-backend default model sets are exposed by `BackendType::default_models()`.
- Multi-node Docker dev setup is defined in `docker-compose.yml` (ports 9944-9947). Single-node test setup in `docker-compose.test.yml`.

## Code quality / linting policy
- Every crate root in `crates/` enables `#![warn(clippy::pedantic)]` and `#![deny(clippy::unwrap_used, clippy::expect_used)]`.
- Tests are allowed to use `unwrap()`/`expect()` via `#![allow(clippy::unwrap_used, clippy::expect_used)]` inside each `#[cfg(test)]` module.
- CI enforces the safety-critical lint set: `cargo clippy --workspace --all-targets -- -D clippy::unwrap_used -D clippy::expect_used`.
- `package.json` scripts match CI:
  - `npm run lint` → `cargo clippy --workspace --all-targets -- -D clippy::unwrap_used -D clippy::expect_used`
  - `npm run lint:pedantic` → runs with `-W clippy::pedantic` as a non-blocking baseline
- `clippy.toml` contains project-level clippy configuration (MSRV, thresholds, exported-API tolerance, unwrap-in-tests allowance).
- `rust-toolchain.toml` pins the toolchain to `1.97.1` with `rustfmt` and `clippy` components.
- `cargo fmt --all -- --check` and `cargo test --workspace` are also CI gates.
- `Object::new` and `Object::hash` return `Result<_, bincode::Error>`; callers must propagate or handle the error.
- `State::create_object_from_data` now returns `Result<ObjectId, StateError>`.

## Existing agent-instruction sources scanned
- `README.md`, `wallet/README.md`, `wallet/src-tauri/README.md`, `tools/launcher/README.md`, `crates/miraset-launcher/README.md`, `crates/miraset-worker/README.md`.
- No dedicated repo-level AI rules file was found in the scanned patterns (`AGENT.md`, `CLAUDE.md`, `.cursorrules`, etc.).


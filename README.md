# Miraset Chain

Decentralized GPU compute marketplace for AI inference.

## Quick Start

### 1. Build the Project
Ensure you have Rust installed.
```bash
cargo build --release
```

### 2. Start Local Node
This starts the blockchain node with RPC at `127.0.0.1:9944`.
```bash
cargo run --bin miraset -- node start
```

### 3. Start Worker
In a new terminal, start the worker. It will connect to the node and your local Ollama/LMStudio instance.
```bash
cargo run --bin miraset-worker
```

### 4. Run Wallet
```bash
cd wallet && bun run tauri:dev
```

### 5. Run Tests (optinal)
```bash
cargo test --workspace
```

## Resource Benchmark

Measures block-production resources (wall-time, throughput, CPU time and
utilisation, peak RAM and its share of system RAM, serialized block size)
for blocks with 0, 1, 100, 1,000 and 10,000 transactions (10,000 =
mempool cap, the practical max txs per block):

```bash
npm run bench:resources
```

On completion the benchmark writes `reports/block-resource-bench.json`
and `reports/block-resource-bench.md`.

### VPS package simulation

Simulates the node + benchmark on each Namecheap VPS package
(Spark/Pulsar/Quasar/Magnetar/Hypernova) by running them inside cgroup
scopes with the package's CPU/RAM limits (Linux + systemd user session):

```bash
npm run bench:vps            # all packages
bash scripts/bench-vps-sim.sh spark pulsar   # subset
```

Writes per-package results to `reports/vps-sim/` and an aggregated
per-package report to `reports/vps-simulation.md` (JSON twin next to it).

## Local Docker Inference Test

The test compose stack runs one node, one worker, and Ollama from the official
`ollama/ollama` image. It downloads the model on first start and keeps model
data in a Docker volume. Ollama uses the host's CPU by default; NVIDIA GPU
passthrough can be enabled with the optional Compose override below.

1. Set a fresh 32-byte hex dispatch secret and start the services:

   ```bash
   export MIRASET_DISPATCH_SECRET="$(openssl rand -hex 32)"
   export MIRASET_WORKER_SECRET_KEY="$(openssl rand -hex 32)"
   # Optional: change the default model from gemma3:4b
   # export OLLAMA_MODEL=llama3.2:latest
   docker compose -f docker-compose.test.yml up --build
   ```

   To use a different Ollama model, set `OLLAMA_MODEL` before startup (the
   default is `gemma3:4b`); Compose pulls it automatically.

   To enable an NVIDIA GPU, install the NVIDIA Container Toolkit and start
   with the GPU override:

   ```bash
   docker compose -f docker-compose.test.yml -f docker-compose.test.gpu.yml up --build
   ```

2. Import the fixed devnet account and the worker key into the local CLI
   wallet. The genesis key is public test-only; don't use it outside this
   local devnet:

   ```bash
   cargo run -p miraset-cli -- wallet import local-dev 0101010101010101010101010101010101010101010101010101010101010101
   cargo run -p miraset-cli -- wallet import local-worker "$MIRASET_WORKER_SECRET_KEY"
   ```

   The worker account needs gas to register. Copy its address from the import
   output, fund it from the devnet account, then restart the worker:

   ```bash
   cargo run -p miraset-cli -- wallet transfer local-dev <worker-address> 1000000000
   docker compose -f docker-compose.test.yml restart worker
   ```

3. Check `curl http://127.0.0.1:9944/workers` shows the worker as active, then
   submit a job through node RPC. Use the same model configured in
   `OLLAMA_MODEL`:

   ```bash
   cargo run -p miraset-cli -- job submit local-dev \
     --model "${OLLAMA_MODEL:-gemma3:4b}" \
     --prompt "Explain what a blockchain node does in one sentence." \
     --max-tokens 128 \
     --escrow 10000
   ```

The CLI prints the job ID and polls node RPC until the job is completed. To
inspect the services manually, use `curl http://127.0.0.1:9944/workers` and
`curl http://127.0.0.1:9944/jobs/<job-id>/result`. Ollama fallback is disabled
in this compose file, so inference errors fail instead of returning a
synthetic response. The prompt stays in node memory until dispatch and is lost
if the node restarts before then.

## Documentation
- [Concepts & Overview](docs/concepts/overview.md)
- [Technical Architecture](docs/architecture/technical_details.md)
- [Economics (PoCC)](docs/economics/model.md)
- [Security Model](docs/security/fraud_prevention.md)

## Project Structure
- `miraset-core`: Core types and cryptography.
- `miraset-node`: Blockchain state, RPC, and storage.
- `miraset-cli`: CLI for node management and wallet.
- `miraset-worker`: Off-chain worker for AI inference.
- `miraset-wallet`: Keystore management.
- `wallet/`: Desktop wallet (Next.js + Tauri).

## Roadmap
- **Phase 1 (MVP)**: Core infrastructure, basic PoCC, Ollama integration.
- **Phase 2**: Decentralized coordinator, reputation system.
- **Phase 3**: ZK proofs for verification, cross-chain support.

## License
MIT

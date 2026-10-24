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

## Local Docker Inference Test

The test compose stack runs one node and one worker. Ollama stays on the host
so it can use the host's CPU/GPU. The worker calls the host Ollama API, while
clients submit jobs to the node RPC.

1. Download the configured model (default: `gemma3:4b`), then start Ollama in
   a terminal with a host-reachable listener:

   ```bash
   ollama pull gemma3:4b
   OLLAMA_HOST=0.0.0.0:11434 ollama serve
   ```

   Binding Ollama to all host interfaces can expose it to your LAN; use this
   only on a trusted machine/network and restrict access with your firewall.
2. In another terminal, set a fresh 32-byte hex dispatch secret and start the
   node and worker containers:

   ```bash
   export MIRASET_DISPATCH_SECRET="$(openssl rand -hex 32)"
   docker compose -f docker-compose.test.yml up --build
   ```

3. Import the fixed devnet genesis account into the local CLI wallet (test
   key only; never use it outside this local devnet):

   ```bash
   cargo run -p miraset-cli -- wallet import local-dev 0101010101010101010101010101010101010101010101010101010101010101
   ```

4. Submit a job through node RPC and wait for the Ollama-backed result:

   ```bash
   cargo run -p miraset-cli -- job submit local-dev \
     --model gemma3:4b \
     --prompt "Explain what a blockchain node does in one sentence." \
     --max-tokens 128 \
     --escrow 10000
   ```

The CLI prints the job ID and polls node RPC until the job is completed. To
inspect the services manually, use `curl http://127.0.0.1:9944/workers` and
`curl http://127.0.0.1:9944/jobs/<job-id>/result`. Ollama fallback is disabled
in this compose file, so a missing host/model causes the job to fail instead
of returning a synthetic response. The prompt stays in node memory until
dispatch and is lost if the node restarts before then.

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

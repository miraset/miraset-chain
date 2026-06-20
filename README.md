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

### 4. Run Tests
```bash
cargo test --workspace
```

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

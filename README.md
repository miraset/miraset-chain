## Miraset Chain

**Proof of Compute & Capacity (PoCC)** blockchain for GPU inference rewards.

### Why "miraset"?

`Mimir` — source of wisdom.  
`Ra` — ancient sun god, the power of origin.  
`Set` — the power of transformation.

---

## ✨ New Features (v0.1.1)

### 🤖 Worker Integration (NEW!)
- **AI Inference** - Ollama/vLLM backend with automatic fallback
- **Verifiable Receipts** - Cryptographic proof of computation
- **On-Chain Registration** - Workers register with GPU specs
- **Job Execution** - Accept, execute, and report jobs
- **Real AI Models** - Tested with gemma3, llama3.3, deepseek-r1

**Quick Test:**
```bash
# Terminal 1: Node
cargo run --bin miraset -- node start

# Terminal 2: Worker
cargo run --bin miraset-worker

# Terminal 3: Test
./test_worker_e2e.sh
```

**Full Guide:** See `FINAL_STATUS.md` for complete worker documentation.

### 💾 Persistent Storage
- **Sled integration** - pure Rust embedded database
- **No data loss** - survive restarts and crashes  
- **Fast recovery** - instant reload from storage
- **Cross-platform** - works on Windows without LLVM

### 🐳 Docker Multi-Node
- **4-node setup** ready for BFT consensus testing
- **Isolated containers** with persistent volumes
- **Easy deployment** with docker-compose

---

## Quick Start

### Prerequisites
- Rust 1.70+ (`rustup`)
- Cargo
- Docker & Docker Compose (optional, for multi-node)

### Build
```bash
cargo build --release --workspace
```

Binaries will be in `target/release/`:
- `miraset` — CLI tool (with persistence!)
- `miraset-tui` — Terminal UI

### Run Local Devnet (with Persistence)

Start a local development node:
```bash
./target/release/miraset node start
```

Data automatically saved to `.data/` directory!

**Configuration Options:**

```bash
# Custom storage path
./target/release/miraset node start --storage-path /my/data

# Custom RPC address
./target/release/miraset node start --rpc-addr 0.0.0.0:9944

# Custom block interval (seconds)
./target/release/miraset node start --block-interval 10
```

**Config File** (`miraset.toml` in project root):
```toml
[node]
rpc_addr = "127.0.0.1:9944"
storage_path = ".data"
block_interval = 5
```

**Environment Variables:**
```bash
export MIRASET_RPC_ADDR="127.0.0.1:9944"
export MIRASET_STORAGE_PATH=".data"
export MIRASET_BLOCK_INTERVAL="5"
```

Precedence: CLI flags > Env vars > Config file > Defaults

### Run Multi-Node Setup (Docker)

```bash
# Build images
docker-compose build

# Start 4 nodes
docker-compose up -d

# Test nodes
./test_docker_nodes.sh

# View logs
docker-compose logs -f
```

Nodes available at:
- Node 1: `localhost:9944`
- Node 2: `localhost:9945`  
- Node 3: `localhost:9946`
- Node 4: `localhost:9947`

```bash
./target/release/miraset node start
```

This starts:
- Block producer (5-second blocks)
- RPC server on `http://127.0.0.1:9944`
- Genesis account with 1B tokens (secret key printed to console)

### CLI Usage

**Create wallet:**
```bash
./target/release/miraset wallet new alice
```

**Check balance:**
```bash
./target/release/miraset wallet balance alice
```

**Transfer tokens:**
```bash
./target/release/miraset wallet transfer alice <address> 1000
```

**Send chat message:**
```bash
./target/release/miraset chat send alice "Hello Miraset!"
```

**List chat:**
```bash
./target/release/miraset chat list
```

### Terminal UI

Run the TUI:
```bash
./target/release/miraset-tui
```

**Controls:**
- `1` — Wallet tab
- `2` — Chat tab
- `3` — Chain info tab
- `R` — Refresh data
- `Q` — Quit

Chat: type message and press Enter to send.

---

## Architecture

See `docs/`:
- `docs/SOW.md` — Full specification
- `docs/REWARDS.md` — Economic model & formulas
- `docs/ARCHITECTURE.md` — System design
- `docs/DATA.md` — Research notes

### MVP Features

✅ Local devnet (single-node BFT)  
✅ Wallet (keypairs, balances, transfers)  
✅ On-chain chat  
✅ CLI & TUI  
✅ Transaction signing & verification  
✅ Event indexing  
✅ **Persistent storage (Sled)**  
✅ **Docker multi-node setup**

### Roadmap

- [x] Persistent storage (Sled) ✅ **NEW**
- [x] Docker multi-node deployment ✅ **NEW**
- [ ] P2P networking layer
- [ ] BFT consensus implementation
- [ ] PoCC capacity attestation (VRAM + uptime)
- [ ] GPU job execution & rewards
- [ ] Batched epoch settlement
- [ ] ZK receipt anchoring
- [ ] Sui Move integration

---

## 📚 Documentation

- **[QUICKSTART.md](QUICKSTART.md)** - Quick start guide
- **[USER_GUIDE.md](USER_GUIDE.md)** - Complete user manual
- **[CONFIGURATION.md](CONFIGURATION.md)** - Configuration options & precedence ✨ **NEW**
- **[PERSISTENCE.md](PERSISTENCE.md)** - Storage & multi-node guide ✨ **NEW**
- **[DOCKER.md](DOCKER.md)** - Docker detailed guide ✨ **NEW**
- **[TESTING.md](TESTING.md)** - Test documentation
- **[FINAL_REPORT.md](FINAL_REPORT.md)** - Project summary
- **[docs/](docs/)** - Architecture & specifications

---

## License

MIT


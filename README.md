## Miraset Chain

**Proof of Compute & Capacity (PoCC)** blockchain for GPU inference rewards.

### Why "miraset"?

`Mimir` — source of wisdom.  
`Ra` — ancient sun god, the power of origin.  
`Set` — the power of transformation.

---

## ✨ New Features (v0.1.1)

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

Data automatically saved to `./data/` directory!

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
- **[PERSISTENCE.md](PERSISTENCE.md)** - Storage & multi-node guide ✨ **NEW**
- **[DOCKER.md](DOCKER.md)** - Docker detailed guide ✨ **NEW**
- **[TESTING.md](TESTING.md)** - Test documentation
- **[FINAL_REPORT.md](FINAL_REPORT.md)** - Project summary
- **[docs/](docs/)** - Architecture & specifications

---

## License

MIT

